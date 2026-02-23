/**
 * Chase scraper for Refreshmint.
 */

const BASE_URL = 'https://www.chase.com';
// const LOGON_URL = 'https://secure.chase.com/web/auth/dashboard';

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
async function handleLogin(context) {
    const page = context.mainPage;
    refreshmint.log('State: Login Page');

    // Check if we are in the logon iframe/view
    const hasLoginFields = await page.evaluate(`(function() {
        return !!(document.querySelector('#userId-input') && document.querySelector('#password-input'));
    })()`);

    if (!hasLoginFields) {
        refreshmint.log(
            'Login fields not found, waiting or looking for "Sign in" button',
        );
        const clickedSignIn = await page.evaluate(`(function() {
            const btn = Array.from(document.querySelectorAll('a, button')).find(el => el.textContent.trim().toLowerCase() === 'sign in');
            if (btn) {
                btn.click();
                return true;
            }
            return false;
        })()`);
        if (clickedSignIn) {
            await waitMs(page, 3000);
            return { progressName: 'clicked sign in' };
        }
        return { progressName: 'waiting for login fields' };
    }

    refreshmint.log('Filling credentials...');
    await page.type('#userId-input', 'chase_username');
    await humanPace(page, 200, 500);
    await page.type('#password-input', 'chase_password');
    await humanPace(page, 800, 1500);

    refreshmint.log('Clicking Sign in');
    await page.click('#signin-button');
    await waitMs(page, 5000);

    return { progressName: 'login submitted' };
}

/*
 * ScrapeContext context
 * Returns Promise<object>
 */
/*
async function handleMfa(context) {
    const page = context.mainPage;
    refreshmint.log('State: MFA required');
    // Implement MFA flow (choosing method, entering code)
    // For now, just prompt
    const code = await refreshmint.prompt('Enter MFA code:');
    // ... implement filling code ...
    return { progressName: 'mfa handled' };
}
*/

async function main() {
    refreshmint.log('Chase scraper starting');
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

        if (url.includes('chase.com/web/auth/dashboard')) {
            refreshmint.log('At dashboard');
            stepReturn = { progressName: 'dashboard', done: true };
        } else if (url.includes('logon') || url.includes('signin')) {
            stepReturn = await handleLogin(context);
        } else if (url === 'about:blank' || url.includes('chase.com')) {
            if (url === 'about:blank') {
                await mainPage.goto(BASE_URL);
                stepReturn = { progressName: 'navigating to chase' };
            } else {
                stepReturn = await handleLogin(context);
            }
        } else {
            refreshmint.log(`Unexpected URL: ${url}`);
            await mainPage.goto(BASE_URL);
            stepReturn = { progressName: 'lost, navigating home' };
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
        if (stepReturn.done) {
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
