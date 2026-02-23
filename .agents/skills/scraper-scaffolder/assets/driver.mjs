/**
 * {{name}} scraper for Refreshmint.
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
async function handleLogin(context) {
    const page = context.mainPage;
    refreshmint.log('State: Login Page');

    // Implement login field detection and filling
    const hasLoginFields = await page.evaluate(`(function() {
        // e.g. return !!(document.querySelector('#userId-input') && document.querySelector('#password-input'));
        return false; 
    })()`);

    if (!hasLoginFields) {
        refreshmint.log('Login fields not found, waiting...');
        return { progressName: 'waiting for login fields' };
    }

    refreshmint.log('Filling credentials...');
    // e.g. await page.type('#userId-input', '{{username_secret}}');
    // await humanPace(page, 200, 500);
    // await page.type('#password-input', '{{password_secret}}');
    // await humanPace(page, 800, 1500);

    refreshmint.log('Clicking Sign in');
    // e.g. await page.click('#signin-button');
    // await waitMs(page, 5000);

    return { progressName: 'login submitted' };
}

/**
 * @param {ScrapeContext} context
 * @returns {Promise<object>}
 */
async function _handleMfa(context) {
    const _page = context.mainPage;
    refreshmint.log('State: MFA required');
    const _code = await refreshmint.prompt('Enter MFA code:');
    // Implement filling MFA code here
    return { progressName: 'mfa handled' };
}

async function main() {
    refreshmint.log('{{name}} scraper starting');
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

        // Implement state routing based on URL or DOM content
        if (url.includes('login') || url.includes('signin')) {
            stepReturn = await handleLogin(context);
        } else if (url === 'about:blank') {
            await context.mainPage.goto('{{base_url}}');
            stepReturn = { progressName: 'navigating home' };
        } else {
            refreshmint.log(`Unexpected URL: ${url}`);
            // await context.mainPage.goto('{{base_url}}');
            stepReturn = { progressName: 'lost' };
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
