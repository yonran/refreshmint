/**
 * paypal scraper for Refreshmint.
 */

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
 * @param {ScrapeContext} context
 * @returns {Promise<object>}
 */
async function handleLogin(context) {
    const page = context.mainPage;
    refreshmint.log('State: Login Page');

    try {
        // Check for MFA first
        const mfaInput = page.getByRole('spinbutton', { name: '1-6' });
        if (await mfaInput.isVisible()) {
            return await handleMfa(context);
        }
    } catch (_e) {
        // Ignore timeout
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
    } catch (_e) {
        // Ignore timeout
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
    } catch (_e) {
        // Ignore timeout
    }

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

main().catch((err) => {
    refreshmint.log(`Fatal error: ${err.message}`);
    if (err.stack) refreshmint.log(err.stack);
});
