/**
 * @typedef {object} ScrapeContext
 * @property {PageApi} mainPage
 * @property {number} currentStep
 * @property {string[]} progressNames
 * @property {Set<string>} progressNamesSet
 * @property {number} lastProgressStep
 *
 * @typedef {object} StepReturn
 * @property {string} progressName
 * @property {boolean} [done]
 */

/**
 * @param {ScrapeContext} context
 * @returns {Promise<StepReturn>}
 */
async function navigateToLogin(context) {
    const url =
        'https://accountmanager.providentcu.org/ProvidentOnlineBanking/SignIn.aspx';
    refreshmint.log(`navigating to ${url}`);
    await context.mainPage.goto(url);
    return { progressName: `navigate to ${url}` };
}

/**
 * @param {ScrapeContext} _context
 * @returns {Promise<StepReturn>}
 */
async function scrapeLoginPage(_context) {
    refreshmint.log(`we're on login page`);
    // refreshmint.log(`login snapshot: ${await context.mainPage.snapshot()}`);
    return {
        progressName: 'snapshot login page',
        done: true,
    };
}

async function main() {
    refreshmint.log('calling pages');
    const pages = await browser.pages();
    refreshmint.log(`pages returned`);
    const mainPage = pages[0];
    refreshmint.log(`page ${mainPage}`);
    if (mainPage == null) {
        throw new Error('expected at least one page');
    }
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
        const urlWithoutFragment = url.split('#', 2)[0];
        /** @type {StepReturn} */
        let stepReturn;
        if (
            urlWithoutFragment ===
            'https://accountmanager.providentcu.org/ProvidentOnlineBanking/SignIn.aspx'
        ) {
            refreshmint.log('a');
            stepReturn = await scrapeLoginPage(context);
        } else {
            refreshmint.log('b');
            stepReturn = await navigateToLogin(context);
        }

        const progressName = stepReturn.progressName;
        context.progressNames.push(progressName);
        if (!context.progressNamesSet.has(progressName)) {
            context.progressNamesSet.add(progressName);
            context.lastProgressStep = context.currentStep;
        }
        if (context.currentStep - context.lastProgressStep > 3) {
            throw new Error('no progress in last 3 steps');
        }
        if (stepReturn.done) {
            refreshmint.log('step is done');
            break;
        }
    }
}
refreshmint.log('starting main');
await main();
