/**
 * paypal scraper for Refreshmint.
 */

import { inspect } from 'refreshmint:util';

/**
 * @typedef {object} ScrapeContext
 * @property {PageApi} mainPage
 * @property {number} currentStep
 * @property {string[]} progressNames
 * @property {Set<string>} progressNamesSet
 * @property {number} lastProgressStep
 */

async function waitMs(page, ms) {
    await page.evaluate(`new Promise(r => setTimeout(r, ${ms}))`);
}

async function humanPace(page, minMs, maxMs) {
    const delta = maxMs - minMs;
    const ms = minMs + Math.floor(Math.random() * (delta + 1));
    await waitMs(page, ms);
}

/**
 * @param {unknown} x
 * @returns {string}
 */
function assertString(x) {
    if (typeof x === 'string') {
        return x;
    }
    throw new Error('expected string; got ' + typeof x);
}

/**
 * PayPal shows a cookie-consent banner ("We currently use cookies...")
 * pinned to the bottom of the viewport, which overlaps the "Next" button on
 * the login form directly above it. Root cause of the observed
 * "no progress in last 20 steps" failures: the banner intercepted every
 * click on "Next", the resulting actionability-timeout error was silently
 * swallowed by a broad `catch (_e) { // Ignore timeout }`, and the loop kept
 * refilling the same email field step after step with no diagnostic of why.
 * Dismiss the banner before touching any login-form control so clicks land
 * on the real target.
 *
 * @param {PageApi} page
 * @returns {Promise<boolean>} true if a banner was found and dismissed
 */
async function dismissCookieBanner(page) {
    const declineButton = page.getByRole('button', { name: 'No, I decline' });
    if (await declineButton.isVisible()) {
        refreshmint.log('State: cookie consent banner - declining');
        await declineButton.click();
        await waitMs(page, 500);
        return true;
    }
    return false;
}

/**
 * @param {ScrapeContext} context
 * @returns {Promise<object>}
 */
async function handleMfa(context) {
    const page = context.mainPage;
    refreshmint.log('State: MFA required');

    // We only prompt if we are still waiting on MFA
    const code = await refreshmint.prompt('Enter MFA code (6 digits):');

    if (code && code.length === 6) {
        for (let i = 0; i < 6; i++) {
            const spinbutton = page.getByRole('spinbutton', {
                name: `${i + 1}-6`,
            });
            await spinbutton.fill(code[i]);
            await waitMs(page, 50);
        }
        await humanPace(page, 200, 500);
        await page.getByRole('button', { name: 'Submit' }).click();
        await waitMs(page, 4000);
        return { progressName: 'mfa submitted' };
    }

    return { progressName: 'mfa prompted' };
}

/**
 * PayPal's anti-automation defense can replace the sign-in form with a
 * "You have been blocked" page while the URL stays on /signin, which looks
 * identical to the real login state from the URL alone. That page never
 * resolves on its own, so looping on it for 20 steps only produces a
 * generic, unhelpful "no progress" error. Detect it explicitly and fail
 * fast with the real reason instead.
 *
 * @param {PageApi} page
 * @returns {Promise<void>}
 */
async function checkForBotBlock(page) {
    const text = assertString(
        await page.evaluate(
            `document.body ? document.body.innerText.slice(0, 300) : ''`,
        ),
    );
    if (text.includes('You have been blocked')) {
        throw new Error(
            `PayPal blocked automated access (bot-detection page shown instead of the login form): ${text.trim()}`,
        );
    }

    const hybridCaptcha = page.locator('#splitHybridCaptcha');
    const passwordCaptcha = page.locator('#splitPasswordCaptcha');
    if (
        (await hybridCaptcha.isVisible()) ||
        (await passwordCaptcha.isVisible())
    ) {
        throw new Error(
            'PayPal requires a visible CAPTCHA; continue this login in a debug session so the user can solve it',
        );
    }
}

/**
 * @param {ScrapeContext} context
 * @returns {Promise<object>}
 */
async function handleLogin(context) {
    const page = context.mainPage;
    refreshmint.log('State: Login Page');

    // Fail fast instead of looping: this page never advances on its own.
    await checkForBotBlock(page);

    // Clear the cookie-consent overlay next: it can cover the "Next"/
    // "Log In"/"Submit" buttons below it and silently intercept clicks.
    if (await dismissCookieBanner(page)) {
        return { progressName: 'cookie banner dismissed' };
    }

    try {
        // Check for MFA first
        const mfaInput = page.getByRole('spinbutton', { name: '1-6' });
        if (await mfaInput.isVisible()) {
            return await handleMfa(context);
        }
    } catch (e) {
        refreshmint.log(`MFA input check failed: ${inspect(e)}`);
    }

    try {
        const passwordInput = page.getByRole('textbox', { name: 'Password' });
        if (await passwordInput.isVisible()) {
            refreshmint.log('Filling password...');
            await passwordInput.fill('paypal_password');
            await humanPace(page, 500, 1000);
            await page.getByRole('button', { name: 'Log In' }).click();
            await waitMs(page, 4000);
            return { progressName: 'password submitted' };
        }
    } catch (e) {
        refreshmint.log(`Password step failed: ${inspect(e)}`);
        throw e;
    }

    try {
        const emailInput = page.getByRole('textbox', {
            name: 'Email or mobile number',
        });
        if (await emailInput.isVisible()) {
            refreshmint.log('Filling email...');
            await emailInput.fill('paypal_username');
            await humanPace(page, 200, 500);
            await page.getByRole('button', { name: 'Next' }).click();
            await waitMs(page, 2000);
            return { progressName: 'email submitted' };
        }
    } catch (e) {
        refreshmint.log(`Email step failed: ${inspect(e)}`);
        throw e;
    }

    refreshmint.log(`Login page snapshot: ${await page.snapshot()}`);
    return { progressName: 'waiting on login page' };
}

async function main() {
    refreshmint.log('paypal scraper starting');
    const pages = await browser.pages();
    const mainPage = pages[0];
    if (mainPage == null) throw new Error('expected at least one page');

    /** @type {ScrapeContext} */
    const context = {
        mainPage,
        currentStep: 0,
        progressNames: [],
        progressNamesSet: new Set(),
        lastProgressStep: 0,
    };

    while (true) {
        context.currentStep++;
        const url = await context.mainPage.url();
        refreshmint.log(`Step ${context.currentStep}: URL=${url}`);

        let stepReturn;

        if (url.includes('/signin') || url.includes('/auth/')) {
            stepReturn = await handleLogin(context);
        } else if (
            url.includes('/myaccount/summary') ||
            url.includes('/myaccount/activities') ||
            url.includes('/myaccount/statements') ||
            url.includes('/reports/')
        ) {
            refreshmint.log(
                'Login successful! Base login implementation complete.',
            );
            stepReturn = { progressName: 'login complete', done: true };
        } else if (url === 'about:blank') {
            await context.mainPage.goto('https://www.paypal.com/signin');
            stepReturn = { progressName: 'navigating home' };
        } else {
            refreshmint.log(`Unexpected URL: ${url}`);
            await context.mainPage.goto(
                'https://www.paypal.com/myaccount/summary',
            );
            stepReturn = { progressName: 'lost, navigating to summary' };
        }

        const progressName = stepReturn.progressName;
        context.progressNames.push(progressName);
        if (!context.progressNamesSet.has(progressName)) {
            context.progressNamesSet.add(progressName);
            context.lastProgressStep = context.currentStep;
        }

        if (context.currentStep - context.lastProgressStep > 20) {
            throw new Error('no progress in last 20 steps');
        }
        if (stepReturn && stepReturn.done) {
            refreshmint.log('Scraping complete');
            break;
        }
        await humanPace(context.mainPage, 1000, 2000);
    }
}

// Fail loud: await main() so any thrown error rejects the top-level promise and
// the scrape is recorded as failed. A top-level `.catch` that only logs resolves
// the promise, making every scrape report success even when nothing was captured.
await main();
