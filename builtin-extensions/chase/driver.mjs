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
 * @property {Set<string>} downloadedAccounts
 * @property {number} loginFailures
 * @property {boolean} loginAttempted
 * @property {boolean} otpSubmitted
 * @property {boolean} activityDone
 * @property {boolean} statementsDone
 */

async function waitMs(page, ms) {
    try {
        await page.evaluate(`new Promise(r => setTimeout(r, ${ms}))`);
    } catch (e) {
        // If the page navigates during the sleep, the execution context is destroyed
        // and evaluate throws. We can safely ignore this for a sleep function.
        refreshmint.log(
            'waitMs interrupted (likely by navigation): ' +
                /** @type {Error} */ (e).message,
        );
    }
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

async function waitForBusy(page) {
    const spinnerSelector = '#logon-spin';
    try {
        for (let i = 0; i < 20; i++) {
            const visible = await page.isVisible(spinnerSelector);
            if (!visible) break;
            refreshmint.log('Waiting for logon-spin to finish...');
            await waitMs(page, 500);
        }
    } catch (_e) {
        // Ignore errors if element disappears
    }
}

function getLabel(accountName) {
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

    await page.switchToMainFrame();
    await waitForBusy(page);

    // 1. Check for login error
    const loginErrorText = await page.evaluate(`(function() {
        const el = document.querySelector('#logon-error-header, #logon-error-accessible-text, .logon-error');
        // Only return text if the element is actually visible
        if (el && el.offsetHeight > 0 && el.offsetWidth > 0) {
            return el.innerText;
        }
        return '';
    })()`);

    if (loginErrorText || url.includes('/logon/error')) {
        if (!loginErrorText) {
            const errorContextJson = /** @type {string} */ (
                await page.evaluate(`(function() {
                const h1 = document.querySelector('h1');
                const h2 = document.querySelector('h2');
                return JSON.stringify({
                    h1: h1 ? h1.innerText.trim() : null,
                    h2: h2 ? h2.innerText.trim() : null,
                    bodySnippet: document.body ? document.body.innerText.substring(0, 500).replace(/\\n/g, ' ') : null
                });
            })()`)
            );
            const errorContext = JSON.parse(errorContextJson);
            refreshmint.log(
                'Detected login error: Unknown error. Context: ' +
                    JSON.stringify(errorContext, null, 2),
            );
        } else {
            refreshmint.log('Detected login error: ' + loginErrorText);
        }

        if (url.includes('/logon/error')) {
            refreshmint.log('Navigating back to home to retry...');
            await page.goto(BASE_URL);
            await waitMs(page, 3000);
            return { progressName: 'retry from error' };
        } else {
            refreshmint.log(
                'Inline error detected. Proceeding to retry credentials from secrets...',
            );
            // Wait a moment for any animations, then continue to the credential filling logic
            await waitMs(page, 1000);
        }
    }

    // 2. Check for login fields in main DOM or iframe
    const userSelector =
        '#userId-input-field-input, #userId-input, input[name="userId"]';
    const passSelector =
        '#password-input-field-input, #password-input, input[name="password"]';

    let targetFrame = null;
    const isUserVisibleMain = await page.locator(userSelector).isVisible();

    if (!isUserVisibleMain) {
        const framesJson = await page.frames();
        const frames = JSON.parse(framesJson);
        // Be more selective about logon frames
        const logonFrame = frames.find(
            (f) =>
                f.id === 'logonbox' ||
                f.name === 'logonbox' ||
                (f.url &&
                    f.url.includes('logonbox') &&
                    !f.url.includes('doubleclick') &&
                    !f.url.includes('google')),
        );
        if (logonFrame) {
            targetFrame = logonFrame.id || logonFrame.name || logonFrame.url;
            refreshmint.log(`Switching to login iframe: ${targetFrame}`);
            try {
                await page.switchToFrame(targetFrame);
            } catch (e) {
                refreshmint.log(
                    `Failed to switch to frame ${targetFrame}: ${e}`,
                );
                targetFrame = null;
            }
        }
    }

    if (await page.locator(userSelector).isVisible()) {
        refreshmint.log('Filling login fields from secrets...');
        await page.click(userSelector);
        await page.type(userSelector, 'chase_username');
        await humanPace(page, 1000, 2000);

        await page.click(passSelector);
        await page.type(passSelector, 'chase_password');
        await humanPace(page, 1000, 2000);

        refreshmint.log('Clicking Sign in button...');
        await page.click('#signin-button');

        if (targetFrame) await page.switchToMainFrame();

        await waitMs(page, 5000);
        return { progressName: 'login submitted' };
    }

    if (!url.includes('/logon/')) {
        let hasSignIn = false;
        try {
            const hasSignInResult = await page.evaluate(
                `!!Array.from(document.querySelectorAll('a, button')).find(el => el.textContent.trim().toLowerCase() === 'sign in')`,
            );
            hasSignIn = assertBoolean(hasSignInResult);
        } catch (_e) {
            // Ignore if page navigated while checking
        }

        if (hasSignIn) {
            refreshmint.log('Found "Sign in" button. Clicking...');
            try {
                await page.evaluate(`(function() {
                    const btn = Array.from(document.querySelectorAll('a, button')).find(el => el.textContent.trim().toLowerCase() === 'sign in');
                    if (btn) btn.click();
                })()`);
            } catch (_e) {
                // Ignore navigation errors
            }
            await waitMs(page, 3000);
            return { progressName: 'clicked sign in' };
        }
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
    const url = await page.url();
    const [_path, fragment] = url.split('#', 2);
    const urlFragment = fragment || '';

    refreshmint.log(
        'Intent: Handle MFA identity confirmation. Fragment: ' + urlFragment,
    );
    await page.switchToMainFrame();

    // 0. Verify page state and check for errors
    const pageInfoJson = /** @type {string} */ (
        await page.evaluate(`(function() {
        const header = document.querySelector('h1, h2, #header');
        const headerText = header ? header.innerText.trim() : '';
        const bodyText = document.body.innerText;
        return JSON.stringify({
            header: headerText,
            isRateLimited: bodyText.includes('maximum number of codes') || bodyText.includes('too many requests'),
            isError: bodyText.includes('something went wrong') || location.href.includes('caas=error')
        });
    })()`)
    );
    const pageInfo = JSON.parse(pageInfoJson);

    if (pageInfo.isRateLimited) {
        throw new Error('MFA Rate Limited: ' + pageInfo.header);
    }

    if (pageInfo.isError) {
        refreshmint.log('MFA error detected. Manual intervention required.');
        return { progressName: 'waiting for manual mfa recovery' };
    }

    // Only proceed if we see the confirmation header or specific MFA elements
    if (
        !pageInfo.header.toLowerCase().includes('confirm') &&
        !urlFragment.includes('confirmIdentity')
    ) {
        refreshmint.log(
            'MFA header not found. Waiting... (found: ' + pageInfo.header + ')',
        );
        return { progressName: 'waiting for mfa header' };
    }

    await logSnapshot(page, 'MFA wait');

    // 1. OTP entry field (Most specific state - check FIRST)
    const otpInput = page
        .locator(
            '#otp-code-input, input[name="otpCode"], #otpInput, input[name="otp-input"]',
        )
        .first();
    if (urlFragment.includes('verifyOTP') || (await otpInput.isVisible())) {
        // Check for inline errors indicating the previous code failed
        const hasInlineError = await page.evaluate(`(function() {
            const findInShadow = (root) => {
                const all = Array.from(root.querySelectorAll('*'));
                for (const el of all) {
                    if (el.tagName.includes('ALERT') && (el.innerText || el.textContent).toLowerCase().includes('code')) return true;
                    if ((el.className || '').toString().toLowerCase().includes('error') && (el.innerText || el.textContent).toLowerCase().includes('code')) return true;
                    const sr = el.shadowRoot || el.openOrClosedShadowRoot;
                    if (sr && findInShadow(sr)) return true;
                }
                return false;
            };
            return findInShadow(document);
        })()`);

        if (hasInlineError) {
            throw new Error(
                'MFA code rejected. An inline error is present on the OTP page. Halting to prevent retry loop.',
            );
        }

        if (context.otpSubmitted) {
            throw new Error(
                'MFA code already submitted but the page did not navigate or show an error. Halting to prevent infinite loop of the same code.',
            );
        }

        const code = await refreshmint.prompt('Enter MFA code:');

        refreshmint.log('Filling MFA code via trusted fill...');
        await otpInput.fill(code);
        await humanPace(page, 1000, 2000);

        const submitBtn = page
            .getByRole('button', { name: /Submit|Next|Continue/i })
            .first();
        if (await submitBtn.isVisible()) {
            const btnHtml =
                (await submitBtn.getAttribute('id')) +
                ' | ' +
                (await submitBtn.innerText());
            refreshmint.log(
                'OTP submit button is visible. Element info: ' + btnHtml,
            );
            refreshmint.log('Clicking via getByRole...');
            await submitBtn.click();
            await waitMs(page, 5000);
            return { progressName: 'mfa code submitted' };
        }

        refreshmint.log('Failed to locate OTP submit button.');
        return { progressName: 'mfa code submit failed' };
    }

    // 2. Mobile number confirmation screen (Next button without OTP input)
    // Check for specific text indicating we are confirming where to send the code
    const isConfirmationScreen = await page.evaluate(`(function() {
        const container = document.querySelector('main, #challenge-options');
        if (!container) return false;
        return container.textContent.toLowerCase().includes('use this code to confirm your identity');
    })()`);

    if (assertBoolean(isConfirmationScreen)) {
        const nextBtn = page
            .getByRole('button', { name: 'Next', exact: false })
            .first();
        if (await nextBtn.isVisible()) {
            refreshmint.log(
                'Detected mobile confirmation screen with specific text. Pacing and clicking Next...',
            );
            await humanPace(page, 1000, 3000);
            await nextBtn.click();
            await waitMs(page, 5000);
            return { progressName: 'mfa send sms clicked' };
        }
    }

    // 3. Method selection (links containing "Get a ")
    const linksJson = await page.evaluate(`(function() {
        const findInShadow = (root) => {
            let found = [];
            const labels = Array.from(root.querySelectorAll('a, mds-list-item, label'));
            found.push(...labels.map(l => l.innerText.trim()));
            
            const hosts = Array.from(root.querySelectorAll('*')).filter(el => el.shadowRoot || el.openOrClosedShadowRoot);
            for (const host of hosts) {
                found.push(...findInShadow(host.shadowRoot || host.openOrClosedShadowRoot));
            }
            return found;
        };
        const all = findInShadow(document).filter(t => t.includes('Get a '));
        return JSON.stringify(all);
    })()`);
    const methods = JSON.parse(/** @type {string} */ (linksJson));

    if (methods.length > 0) {
        refreshmint.log('Discovered MFA methods: ' + methods.join(', '));
        const choice = await refreshmint.prompt(
            'Select MFA method: ' + methods.join(' | '),
        );

        const target =
            methods.find((m) =>
                m.toLowerCase().includes(choice.toLowerCase()),
            ) || methods[0];

        refreshmint.log(`Selecting MFA method via getByRole: ${target}`);
        const mfaLocator = page.getByRole('link', {
            name: target,
            exact: false,
        });
        if (await mfaLocator.isVisible()) {
            await mfaLocator.click();
            await waitMs(page, 5000);
            return { progressName: 'mfa method selected' };
        }
        return { progressName: 'mfa method selection failed' };
    }

    return { progressName: 'waiting for mfa state' };
}

/**
 * @param {ScrapeContext} context
 * @returns {Promise<object>}
 */
async function identifyAccountsOnDashboard(context) {
    const page = context.mainPage;
    refreshmint.log('Intent: Identify accounts on dashboard');
    await page.switchToMainFrame();

    refreshmint.log('Searching for accounts in dashboard DOM...');
    const accountsJson = /** @type {string} */ (
        await page.evaluate(`(function() {
        const all = Array.from(document.querySelectorAll('button, a, span, h3'));
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
        return {
            progressName: 'dashboard (accounts discovered)',
            success: true,
        };
    }

    refreshmint.log('No accounts discovered yet.');
    return { progressName: 'dashboard (waiting for accounts)', success: false };
}

/**
 * @param {ScrapeContext} context
 * @returns {Promise<object>}
 */
async function handleAccountNavigation(context) {
    const page = context.mainPage;
    const account = context.accounts.find(
        (a) => !context.downloadedAccounts.has(a.name),
    );
    if (!account) {
        context.activityDone = true;
        return { progressName: 'all accounts navigated' };
    }

    refreshmint.log(`Navigating to account details: ${account.name}`);
    const clicked = await page.evaluate(`(function(name) {
        const els = Array.from(document.querySelectorAll('button, a'));
        const el = els.find(e => e.textContent.includes(name));
        if (el) {
            el.scrollIntoView();
            el.click();
            return true;
        }
        return false;
    })("${account.name}")`);

    if (clicked) {
        await waitMs(page, 5000);
        return { progressName: `navigating to ${account.name}` };
    }

    refreshmint.log(`Could not find click target for account: ${account.name}`);
    return { progressName: `failed to navigate to ${account.name}` };
}

/**
 * @param {ScrapeContext} context
 * @returns {Promise<object>}
 */
async function handleAccountDetails(context) {
    const page = context.mainPage;
    refreshmint.log('Intent: Find download button on account details page');
    await page.switchToMainFrame();

    const downloadBtn = page.locator(
        '[data-testid="quick-action-download-activity-tooltip-button"]',
    );
    if (await downloadBtn.isVisible()) {
        refreshmint.log('Found download button. Clicking...');
        await humanPace(page, 1000, 2000);
        await downloadBtn.click();
        await waitMs(page, 3000);
        return { progressName: 'clicked download icon' };
    }

    const altDownloadBtn = page.locator('.icon-download-transactions');
    if (await altDownloadBtn.isVisible()) {
        refreshmint.log('Found alternative download icon. Clicking...');
        await humanPace(page, 1000, 2000);
        await altDownloadBtn.click();
        await waitMs(page, 3000);
        return { progressName: 'clicked download icon (alt)' };
    }

    refreshmint.log('Download button not found. Waiting...');
    await logSnapshot(page, 'Account details wait');
    return { progressName: 'waiting for download button' };
}

/**
 * @param {ScrapeContext} context
 * @returns {Promise<object>}
 */
async function handleDownload(context) {
    const page = context.mainPage;
    refreshmint.log('Intent: Interact with download dialog');
    await page.switchToMainFrame();

    const nextAccount = context.accounts.find(
        (a) => !context.downloadedAccounts.has(a.name),
    );
    if (!nextAccount) {
        refreshmint.log('All accounts downloaded. Returning to dashboard...');
        await page.evaluate(`(function() {
            const btn = Array.from(document.querySelectorAll('button')).find(el => el.textContent.includes('Go back to accounts'));
            if (btn) { btn.click(); return true; }
            return false;
        })()`);
        await waitMs(page, 3000);
        context.activityDone = true;
        return { progressName: 'downloading complete', done: true };
    }

    refreshmint.log(`Preparing download for: ${nextAccount.name}`);
    await humanPace(page, 1000, 2000);

    // 1. Select account in dropdown
    const accountDropdown = page.evaluate(`(function(name) {
        const btn = Array.from(document.querySelectorAll('button')).find(el => el.innerText.includes('Account,'));
        if (btn) { btn.click(); return true; }
        return false;
    })()`);

    if (await accountDropdown) {
        await waitMs(page, 1000);
        const selected = await page.evaluate(`(function(name) {
            const options = Array.from(document.querySelectorAll('[role="option"], li, button'));
            const opt = options.find(o => o.innerText.includes(name));
            if (opt) { opt.click(); return true; }
            return false;
        })("${nextAccount.name}")`);
        if (selected) await waitMs(page, 1000);
    }

    // 2. Select File Type: Spreadsheet (Excel, CSV)
    const fileTypeDropdown = await page.evaluate(`(function() {
        const btn = Array.from(document.querySelectorAll('button')).find(el => el.innerText.includes('File type,'));
        if (btn) { btn.click(); return true; }
        return false;
    })()`);
    if (fileTypeDropdown) {
        await waitMs(page, 1000);
        await page.evaluate(`(function() {
            const opt = Array.from(document.querySelectorAll('[role="option"], li, button')).find(o => o.innerText.includes('Spreadsheet (Excel, CSV)'));
            if (opt) opt.click();
        })()`);
        await waitMs(page, 1000);
    }

    // 3. Select Activity: All transactions
    const activityDropdown = await page.evaluate(`(function() {
        const btn = Array.from(document.querySelectorAll('button')).find(el => el.innerText.includes('Activity,'));
        if (btn) { btn.click(); return true; }
        return false;
    })()`);
    if (activityDropdown) {
        await waitMs(page, 1000);
        await page.evaluate(`(function() {
            const opt = Array.from(document.querySelectorAll('[role="option"], li, button')).find(o => o.innerText.includes('All transactions'));
            if (opt) opt.click();
        })()`);
        await waitMs(page, 1000);
    }

    // 4. Click Download
    refreshmint.log('Clicking Download button...');
    await humanPace(page, 1000, 2000);
    const downloadPromise = page.waitForDownload(30000);
    const clickedDownload = await page.evaluate(`(function() {
        const btn = Array.from(document.querySelectorAll('button')).find(el => el.textContent === 'Download');
        if (btn) { btn.click(); return true; }
        return false;
    })()`);

    if (clickedDownload) {
        try {
            const download = await downloadPromise;
            refreshmint.log(`Download finished: ${download.suggestedFilename}`);
            await refreshmint.saveDownloadedResource(
                download.path,
                download.suggestedFilename,
                {
                    label: nextAccount.label,
                },
            );
            context.downloadedAccounts.add(nextAccount.name);
            await page.evaluate(`(function() {
                const btn = Array.from(document.querySelectorAll('button')).find(el => el.textContent.includes('Download other activity'));
                if (btn) btn.click();
            })()`);
            await waitMs(page, 3000);
            return { progressName: `downloaded ${nextAccount.name}` };
        } catch (e) {
            refreshmint.log(`Download failed for ${nextAccount.name}: ${e}`);
        }
    }

    return { progressName: `retrying download for ${nextAccount.name}` };
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
        downloadedAccounts: new Set(),
        loginFailures: 0,
        loginAttempted: false,
        otpSubmitted: false,
        activityDone: false,
        statementsDone: false,
    };

    while (true) {
        context.currentStep++;
        let url = 'unknown';
        try {
            url = await context.mainPage.url();
        } catch (e) {
            refreshmint.log('Transient error getting URL: ' + e);
            throw e;
        }

        refreshmint.log(`Step ${context.currentStep}: URL=${url}`);

        const [urlBeforeFragment, fragment] = url.split('#', 2);
        const urlFragment = fragment || '';

        // Get page context for better routing
        const pageStatusJson = /** @type {string} */ (
            await context.mainPage.evaluate(`(function() {
            const h1 = document.querySelector('h1');
            const title = document.title;
            const body = document.body ? document.body.innerText : '';
            return JSON.stringify({
                h1: h1 ? h1.innerText.trim() : '',
                title: title,
                isLogin: body.includes('Sign in') || body.includes('User ID'),
                isMfa: body.includes('confirm your identity') || body.includes('one-time code'),
                isDashboard: body.includes('Accounts') && (body.includes('Credit Cards') || body.includes('Checking'))
            });
        })()`)
        );
        const pageStatus = JSON.parse(pageStatusJson);
        const header = pageStatus.h1.toLowerCase();
        const title = pageStatus.title.toLowerCase();
        refreshmint.log(
            'Page status: H1="' +
                pageStatus.h1 +
                '" Title="' +
                pageStatus.title +
                '"',
        );

        let stepReturn;
        try {
            if (urlBeforeFragment.startsWith('https://secure.chase.com/')) {
                if (
                    header.includes('confirm') ||
                    title.includes('identity') ||
                    urlFragment.includes('step=confirmIdentity')
                ) {
                    stepReturn = await handleMfa(context);
                } else if (
                    header.includes('download') ||
                    urlFragment.includes('downloadAccountTransactions')
                ) {
                    stepReturn = await handleDownload(context);
                } else if (
                    header.includes('account details') ||
                    urlFragment.includes('accountDetails')
                ) {
                    stepReturn = await handleAccountDetails(context);
                } else if (
                    header.includes('accounts') ||
                    title.includes('accounts') ||
                    urlFragment.includes('/dashboard')
                ) {
                    const dashboardInfo =
                        await identifyAccountsOnDashboard(context);
                    if (dashboardInfo.success) {
                        if (!context.activityDone) {
                            stepReturn = await handleAccountNavigation(context);
                        } else {
                            stepReturn = {
                                progressName: 'dashboard (activity done)',
                                done: true,
                            };
                        }
                    } else {
                        stepReturn = await handleLogin(context);
                    }
                } else {
                    stepReturn = await handleLogin(context);
                }
            } else if (url.includes('chase.com')) {
                stepReturn = await handleLogin(context);
            } else {
                refreshmint.log('Not on Chase. Navigating to homepage...');
                await context.mainPage.goto(BASE_URL);
                await context.mainPage.waitForLoadState('load', undefined);
                stepReturn = { progressName: 'navigating to home' };
            }
        } catch (e) {
            refreshmint.log(`Error in step ${context.currentStep}: ${e}`);
            throw e;
        }

        const progressName = stepReturn.progressName;
        context.progressNames.push(progressName);
        if (!context.progressNamesSet.has(progressName)) {
            context.progressNamesSet.add(progressName);
            context.lastProgressStep = context.currentStep;
        }

        if (context.currentStep - context.lastProgressStep > 3) {
            throw new Error(
                'no progress in last 3 steps (last: ' + progressName + ')',
            );
        }
        if (stepReturn.done) {
            refreshmint.log('Scraping complete');
            break;
        }
        await humanPace(context.mainPage, 3000, 5000);
    }
}

main().catch((err) => {
    refreshmint.log(`Fatal error: ${err.message}`);
    if (err.stack) refreshmint.log(err.stack);
});
