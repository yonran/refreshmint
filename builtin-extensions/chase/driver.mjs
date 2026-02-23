/**
 * Chase scraper for Refreshmint.
 */

const BASE_URL = 'https://www.chase.com';

/**
 * @typedef {object} AccountInfo
 * @property {string} name e.g. "CHASE SAVINGS (...6870)"
 * @property {string} last4 e.g. "6870"
 * @property {string} label e.g. "chase_savings_6870"
 *
 * @typedef {object} ScrapeContext
 * @property {PageApi} mainPage
 * @property {number} currentStep
 * @property {string[]} progressNames
 * @property {Set<string>} progressNamesSet
 * @property {number} lastProgressStep
 * @property {AccountInfo[]} accounts
 * @property {boolean} activityDone
 * @property {boolean} statementsDone
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
 * @return {boolean}
 */
function assertBoolean(x) {
    if (typeof x === 'boolean') {
        return x;
    }
    throw new Error('expected boolean; got ' + typeof x);
}

async function logSnapshot(page, tag, track = 'state-loop') {
    try {
        const diff = await page.snapshot({ incremental: true, track });
        refreshmint.log(`${tag} snapshot: ${diff}`);
    } catch (e) {
        refreshmint.log(`${tag} snapshot failed: ${e}`);
    }
}

function getLabel(accountName) {
    // e.g. "CHASE SAVINGS (...6870)" -> "chase_savings_6870"
    return accountName
        .toLowerCase()
        .replace(/\(\.\.\.(\d{4})\)/, '$1')
        .replace(/[^a-z0-9]+/g, '_')
        .replace(/^_+|_+$/g, '');
}

/**
 * @param {ScrapeContext} context
 * @returns {Promise<object>}
 */
async function handleLogin(context) {
    const page = context.mainPage;
    const url = await page.url();
    refreshmint.log('Intent: Log in to Chase. URL: ' + url);

    // 1. Check for login iframe
    const framesJson = await page.frames();
    const frames = JSON.parse(framesJson);
    refreshmint.log(
        `Found ${frames.length} frames: ${frames.map((f) => `${f.id || 'no-id'}:${f.name || 'no-name'}:${f.url}`).join(', ')}`,
    );
    const logonFrame = frames.find(
        (f) =>
            f.id === 'logonbox' ||
            f.name === 'logonbox' ||
            f.url.includes('logonbox'),
    );

    if (logonFrame) {
        refreshmint.log(
            `Detected logonbox iframe (id=${logonFrame.id}). Switching context...`,
        );
        try {
            await page.switchToFrame(
                logonFrame.id || logonFrame.name || logonFrame.url,
            );
            const hasUserInFrame = await page.isVisible('#userId-input');
            refreshmint.log(
                `Iframe context switched. hasUserInFrame=${hasUserInFrame}`,
            );
            if (hasUserInFrame) {
                refreshmint.log('Filling login fields in logonbox iframe...');
                await page.fill('#userId-input', 'chase_username');
                await humanPace(page, 200, 500);
                await page.fill('#password-input', 'chase_password');
                await humanPace(page, 800, 1500);
                refreshmint.log('Clicking Sign in in iframe');
                await page.click('#signin-button');
                await page.switchToMainFrame();
                await waitMs(page, 5000);
                return { progressName: 'login submitted (frame)' };
            }
            await page.switchToMainFrame();
        } catch (e) {
            refreshmint.log('iframe interaction failed: ' + e);
            await page.switchToMainFrame();
        }
    }

    // 2. Try to fill in main DOM
    const hasUser = await page.isVisible('#userId-input');
    if (hasUser) {
        refreshmint.log('Filling login fields in main DOM...');
        await page.fill('#userId-input', 'chase_username');
        await humanPace(page, 200, 500);
        await page.fill('#password-input', 'chase_password');
        await humanPace(page, 800, 1500);
        refreshmint.log('Clicking Sign in');
        await page.click('#signin-button');
        await waitMs(page, 5000);
        return { progressName: 'login submitted (main)' };
    }

    // 3. Look for Sign in button (homepage variant)
    const clickedSignIn = await page.evaluate(`(function() {
            const btn = Array.from(document.querySelectorAll('a, button')).find(el => el.textContent.trim().toLowerCase() === 'sign in');
            if (btn) { btn.click(); return true; }
            return false;
        })()`);
    if (assertBoolean(clickedSignIn)) {
        refreshmint.log('Clicked "Sign in" button on page');
        await waitMs(page, 3000);
        return { progressName: 'clicked sign in' };
    }

    await logSnapshot(page, 'Login wait');
    return { progressName: 'waiting for login fields' };
}

/**
 * @param {ScrapeContext} context
 * @returns {Promise<object>}
 */
async function handleMfa(context) {
    const page = context.mainPage;
    refreshmint.log('Intent: Handle MFA identity confirmation');

    const snapshotJson = await page.snapshot({});
    const snapshot = JSON.parse(snapshotJson);

    // Method selection
    const hasText = snapshot.find(
        (el) => el.text && el.text.includes('Get a text'),
    );
    if (hasText) {
        const method = await refreshmint.prompt(
            'Choose MFA method (text/call):',
        );
        const target = method.toLowerCase().includes('call')
            ? 'Get a call'
            : 'Get a text';
        refreshmint.log(`Selecting MFA method: ${target}`);
        await page.evaluate(`(function(t) {
            const el = Array.from(document.querySelectorAll('label')).find(l => l.innerText.includes(t));
            if (el) el.click();
        })("${target}")`);
        await humanPace(page, 500, 1000);
        await page.click('button#next-button');
        await waitMs(page, 3000);
        return { progressName: 'mfa method selected' };
    }

    // Code entry
    const hasCode = snapshot.find(
        (el) => el.selectorHint && el.selectorHint.includes('#otp-code-input'),
    );
    if (hasCode) {
        const code = await refreshmint.prompt('Enter MFA code:');
        refreshmint.log('Filling MFA code...');
        await page.fill('#otp-code-input', code);
        await humanPace(page, 500, 1000);
        await page.click('button#next-button');
        await waitMs(page, 5000);
        return { progressName: 'mfa code submitted' };
    }

    await logSnapshot(page, 'MFA wait');
    return { progressName: 'waiting for mfa' };
}

/**
 * @param {ScrapeContext} context
 * @returns {Promise<object>}
 */
async function identifyAccountsOnDashboard(context) {
    const page = context.mainPage;
    refreshmint.log('Intent: Identify accounts on dashboard');

    // Discover accounts
    refreshmint.log('Searching for accounts in dashboard DOM...');
    const accountsJson = /** @type {string} */ (
        await page.evaluate(`(function() {
            const all = Array.from(document.querySelectorAll('button, a, span, h3'));
            // Look for account patterns like "CHASE SAVINGS (...6870)" or "Freedom (...8354)"
            const matches = all.filter(el => /\\(\\.\\.\\.\\d{4}\\)/.test(el.textContent));
            const seen = new Set();
            const out = [];
            matches.forEach(el => {
                const name = el.textContent.trim().replace(/\\s+/g, ' ');
                if (seen.has(name)) return;
                seen.add(name);
                const m = name.match(/\\(\\.\\.\\.(\\d{4})\\)/);
                out.push({
                    name: name,
                    last4: m ? m[1] : ''
                });
            });
            return JSON.stringify(out);
        })()`)
    );
    const discovered = JSON.parse(accountsJson);
    context.accounts = discovered.map((a) => ({
        ...a,
        label: getLabel(a.name),
    }));

    if (context.accounts.length > 0) {
        refreshmint.log(
            `Discovered ${context.accounts.length} accounts: ${context.accounts.map((a) => a.name).join(', ')}`,
        );
        return { progressName: 'dashboard (accounts discovered)', done: true };
    }

    refreshmint.log('No accounts discovered yet.');
    await logSnapshot(page, 'Dashboard discovery');
    return { progressName: 'dashboard (waiting for accounts)' };
}

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
        accounts: [],
        activityDone: false,
        statementsDone: false,
    };

    while (true) {
        context.currentStep++;
        const url = await context.mainPage.url();
        refreshmint.log(`Step ${context.currentStep}: URL=${url}`);

        let stepReturn;

        let hasLogonIframe = false;
        try {
            const framesJson = await mainPage.frames();
            const frames = JSON.parse(framesJson);
            hasLogonIframe = frames.some(
                (f) => f.id === 'logonbox' || f.name === 'logonbox',
            );
        } catch (_e) {
            // Ignore frame check errors
        }

        if (url.includes('ConfirmYourIdentity')) {
            stepReturn = await handleMfa(context);
        } else if (url.includes('/dashboard') && !hasLogonIframe) {
            stepReturn = await identifyAccountsOnDashboard(context);
        } else if (url.includes('chase.com')) {
            // Likely login wall, breakout needed, or intermediate page
            stepReturn = await handleLogin(context);
        } else {
            refreshmint.log('Not on Chase. Navigating to homepage...');
            await context.mainPage.goto(BASE_URL);
            await context.mainPage.waitForLoadState('load', undefined);
            stepReturn = { progressName: 'navigating to home' };
        }

        const progressName = stepReturn.progressName;
        context.progressNames.push(progressName);
        if (!context.progressNamesSet.has(progressName)) {
            context.progressNamesSet.add(progressName);
            context.lastProgressStep = context.currentStep;
        }

        if (context.currentStep - context.lastProgressStep > 1) {
            throw new Error(
                'no progress in last 1 steps (last: ' + progressName + ')',
            );
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
