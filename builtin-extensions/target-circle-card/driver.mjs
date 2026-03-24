/**
 * target-circle-card scraper for Refreshmint.
 *
 * Scrapes transaction history from https://mytargetcirclecard.target.com/
 * (Target's RedCard / Circle Card credit card portal, powered by TD Bank).
 *
 * TODO: Inspect the login page to fill in selector details.
 */

const ORIGIN = 'https://mytargetcirclecard.target.com';
const LOGIN_URL = `${ORIGIN}/`;

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
 * @returns {Promise<{progressName: string}>}
 */
async function handleLogin(context) {
    const _page = context.mainPage;
    refreshmint.log('State: Login Page');

    // TODO: inspect login page to identify selectors
    // page.type() fires CDP key events that React/framework event handlers pick up.
    // e.g.:
    // await page.type('#username', 'target_circle_card_username');
    // await humanPace(page, 300, 600);
    // await page.type('#password', 'target_circle_card_password');
    // await humanPace(page, 500, 900);
    // await page.click('#signin-button');
    // try { await waitMs(page, 4000); } catch (_e) { /* navigated on success */ }

    return { progressName: 'login stub - not yet implemented' };
}

/**
 * @param {ScrapeContext} context
 * @returns {Promise<{progressName: string, done?: boolean}>}
 */
async function _handleDashboard(context) {
    const _page = context.mainPage;
    refreshmint.log('State: Dashboard / Authenticated');

    // TODO: scrape transaction history or statements from the dashboard.
    // Look for a "Transactions" or "Activity" link and navigate there.

    return { progressName: 'dashboard stub - not yet implemented', done: true };
}

async function main() {
    refreshmint.log('target-circle-card scraper starting');
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

        /** @type {{progressName: string, done?: boolean}} */
        let stepReturn;

        if (url === 'about:blank' || !url.startsWith(ORIGIN)) {
            refreshmint.log(`Navigating to ${LOGIN_URL}`);
            await context.mainPage.goto(LOGIN_URL, {
                waitUntil: 'load',
                timeout: 30000,
            });
            stepReturn = { progressName: 'navigating to login' };
        } else if (
            url.startsWith(ORIGIN) /* TODO: narrow to login URL pattern */
        ) {
            stepReturn = await handleLogin(context);
        } else {
            refreshmint.log(`Unexpected URL: ${url}`);
            stepReturn = { progressName: 'unexpected url' };
        }

        const progressName = stepReturn.progressName;
        context.progressNames.push(progressName);
        if (!context.progressNamesSet.has(progressName)) {
            context.progressNamesSet.add(progressName);
            context.lastProgressStep = context.currentStep;
        }

        if (context.currentStep - context.lastProgressStep > 6) {
            throw new Error('no progress in last 6 steps');
        }
        if (stepReturn.done) {
            refreshmint.log('Scraping complete');
            break;
        }
        await humanPace(context.mainPage, 800, 1400);
    }
}

main().catch((err) => {
    refreshmint.log(`Fatal error: ${err.message}`);
    if (err.stack) refreshmint.log(err.stack);
});
