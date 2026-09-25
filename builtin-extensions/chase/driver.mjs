/**
 * Chase scraper for Refreshmint.
 */

const BASE_URL = 'https://www.chase.com';
const DASHBOARD_URL =
    'https://secure.chase.com/web/auth/dashboard#/dashboard/overview';

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
            context.loginAttempted = false;
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
        const logonFrame = await findLogonFrame(page);
        if (logonFrame !== null) {
            targetFrame = logonFrame;
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
        // The rejection banner renders *inside* the `logonbox` iframe
        // (`#logon-error-accessible-text`, observed 2026-09-15: "Important:
        // We can't find that username and password. Try again."), so the
        // main-frame check in step 1 never sees it. Once we've submitted and
        // Chase rejected the credentials, stop: re-submitting the same
        // Keychain values on every auto-scrape risks a lockout, and the
        // generic "no progress" error hid the real cause for weeks.
        const frameErrorText = /** @type {string} */ (
            await page.evaluate(`(function() {
            const el = document.querySelector('#logon-error-accessible-text, #logon-error-header, .logon-error');
            if (el && el.offsetHeight > 0 && el.offsetWidth > 0) {
                return el.innerText.trim();
            }
            return '';
        })()`)
        );
        if (frameErrorText !== '' && context.loginAttempted) {
            if (targetFrame !== null) await page.switchToMainFrame();
            throw new Error(
                'Login rejected by Chase: ' +
                    frameErrorText +
                    ' -- check the chase_username/chase_password Keychain entries; this is not a scraper selector bug.',
            );
        }
        if (frameErrorText !== '') {
            // Stale banner from an earlier session; the fields are empty and
            // we have not submitted anything yet in this run.
            refreshmint.log(
                'Login iframe shows a pre-existing error banner: ' +
                    frameErrorText,
            );
        }

        if (context.loginAttempted) {
            // Already submitted once, no error banner was detected above,
            // and yet the login fields are still visible -- Chase's
            // post-submit redirect/device-check is just slow (well over the
            // 5s wait below). Re-filling and re-clicking Sign In here would
            // double-submit the form on every subsequent iteration, which
            // is what produced "no progress in last 3 steps (last: login
            // submitted)" -- the progress tracker treats every iteration's
            // identical 'login submitted' as no progress. Wait instead.
            refreshmint.log(
                'Login already submitted; waiting for navigation instead of resubmitting...',
            );
            await waitMs(page, 5000);
            return { progressName: 'login submitted' };
        }

        refreshmint.log('Filling login fields from secrets...');
        await page.click(userSelector);
        await page.type(userSelector, 'chase_username');
        await humanPace(page, 1000, 2000);

        await page.click(passSelector);
        await page.type(passSelector, 'chase_password');
        await humanPace(page, 1000, 2000);

        refreshmint.log('Clicking Sign in button...');
        await page.click('#signin-button');
        context.loginAttempted = true;

        if (targetFrame) await page.switchToMainFrame();

        await waitMs(page, 5000);
        return { progressName: 'login submitted' };
    }

    if (!url.includes('/logon/')) {
        // The homepage header "Sign in" link (`a[data-pt-name="hd_fs_sign-in"]`,
        // href https://secure.chase.com) must get a *trusted* click. A
        // synthetic `el.click()` from page.evaluate() lands on
        // /digital/resources/privacy-security/security/system-requirements
        // every time (observed 2026-09-09 .. 2026-09-15), whereas a real
        // pointer click via the locator navigates to
        // https://secure.chase.com/web/auth/dashboard#/dashboard/overview,
        // which hosts the `#logonbox` login iframe (verified 2026-09-15 in a
        // debug session).
        const headerSignIn = page.locator('a[data-pt-name="hd_fs_sign-in"]');
        if ((await headerSignIn.count()) > 0) {
            refreshmint.log(
                'Found header "Sign in" link. Clicking (trusted)...',
            );
            await headerSignIn.click();
            await waitMs(page, 3000);
            return { progressName: 'clicked sign in' };
        }

        // Fallback when the header link's analytics attribute is gone. The
        // link renders as `<a>Sign in<span class="visually-hidden">Opens
        // overlay</span></a>`, so its textContent is "Sign inOpens overlay"
        // -- not an exact "sign in" match. Meanwhile a hidden login-flyout
        // submit button (`#signin-button`, only revealed after the overlay
        // opens) has textContent that *does* exactly equal "Sign in". An
        // exact-match search finds that hidden button first; clicking it
        // submits the login form with empty fields. Require the element to be
        // visible and match by prefix so the real header link wins.
        const findSignInLink = `Array.from(document.querySelectorAll('a, button')).find(el => {
                        const visible = !!(el.offsetWidth || el.offsetHeight || el.getClientRects().length);
                        if (!visible) return false;
                        const text = el.textContent.trim().toLowerCase();
                        return text.startsWith('sign in');
                    })`;

        let hasSignIn = false;
        try {
            const hasSignInResult = await page.evaluate(`!!${findSignInLink}`);
            hasSignIn = assertBoolean(hasSignInResult);
        } catch (_e) {
            // Ignore if page navigated while checking
        }

        if (hasSignIn) {
            refreshmint.log(
                'Found "Sign in" button by text (fallback). Clicking...',
            );
            try {
                await page.evaluate(`(function() {
                    const btn = ${findSignInLink};
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
 * Dismiss Chase's Qualtrics "We'd love to hear what you think of our new
 * site" survey modal (seen 2026-09-25 on a download form), which intercepts
 * every click. Only clicks "Cancel" inside that modal: the download form has
 * its own Cancel button.
 * @param {PageApi} page
 * @returns {Promise<boolean>} whether a survey was dismissed
 */
async function dismissSurvey(page) {
    return assertBoolean(
        await page.evaluate(`(function() {
        const modal = Array.from(document.querySelectorAll('mds-dialog-modal')).find(
            (m) => m.getClientRects().length > 0 && /survey/i.test(m.innerText || ''),
        );
        if (!modal) return false;
        const find = (root) => {
            for (const b of Array.from(root.querySelectorAll('button'))) {
                if ((b.innerText || '').trim().startsWith('Cancel')) return b;
            }
            for (const h of Array.from(root.querySelectorAll('*')).filter((e) => e.shadowRoot)) {
                const found = find(h.shadowRoot);
                if (found) return found;
            }
            return null;
        };
        const cancel = find(modal) || (modal.shadowRoot && find(modal.shadowRoot));
        if (!cancel) return false;
        cancel.click();
        return true;
    })()`),
    );
}

/**
 * Frame ref (for `page.switchToFrame`) of Chase's `logonbox` sign-in iframe,
 * or null when the page has none.
 * @param {PageApi} page
 * @returns {Promise<string | null>}
 */
async function findLogonFrame(page) {
    const parsedFrames = /** @type {unknown} */ (
        JSON.parse(await page.frames())
    );
    const frames =
        /** @type {Array<{id?: string, name?: string, url?: string}>} */ (
            Array.isArray(parsedFrames) ? parsedFrames : []
        );
    // Be selective: ad/analytics frames can carry "logonbox" in their URL.
    const logonFrame = frames.find(
        (f) =>
            f.id === 'logonbox' ||
            f.name === 'logonbox' ||
            (f.url !== undefined &&
                f.url.includes('logonbox') &&
                !f.url.includes('doubleclick') &&
                !f.url.includes('google')),
    );
    if (logonFrame === undefined) return null;
    for (const ref of [logonFrame.id, logonFrame.name, logonFrame.url]) {
        if (ref !== undefined && ref !== '') return ref;
    }
    return null;
}

/**
 * Whether the current frame shows a visible MFA "Confirm Your Identity"
 * screen. The visibility check keeps a hidden, stale sign-in iframe from
 * looking like an MFA prompt after login has moved on.
 * @param {PageApi} page
 * @returns {Promise<boolean>}
 */
async function currentFrameShowsMfa(page) {
    return assertBoolean(
        await page.evaluate(`(function() {
        const h1 = document.querySelector('h1');
        if (!h1 || h1.getClientRects().length === 0) return false;
        const body = document.body ? document.body.innerText.toLowerCase() : '';
        return h1.innerText.toLowerCase().includes('confirm') ||
            body.includes('confirm your identity') ||
            body.includes('one-time code');
    })()`),
    );
}

/**
 * Since at least 2026-09-25 Chase renders the MFA screens inside the
 * `logonbox` iframe on the dashboard URL, while the main frame is only a
 * "loading" shell (verified against a retained failed-scrape session).
 * Returns that frame's ref when it holds the MFA screen, else null (the MFA
 * screen, if any, is in the main frame).
 * @param {PageApi} page
 * @returns {Promise<string | null>}
 */
async function findMfaFrame(page) {
    await page.switchToMainFrame();
    if (await currentFrameShowsMfa(page)) return null;
    const logonFrame = await findLogonFrame(page);
    if (logonFrame === null) return null;
    await page.switchToFrame(logonFrame);
    try {
        return (await currentFrameShowsMfa(page)) ? logonFrame : null;
    } finally {
        await page.switchToMainFrame();
    }
}

/**
 * Handles the MFA screen in whichever frame is current; the main loop
 * selects the sign-in iframe first when `findMfaFrame` finds it there.
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

    // 0. Verify page state and check for errors
    const pageInfoJson = /** @type {string} */ (
        await page.evaluate(`(function() {
        // Keep heading precedence aligned with the main-loop page-status probe
        // below. querySelector('h1, h2, #header') uses document order and can
        // select Chase's earlier, empty #header instead of the visible h1.
        const header = document.querySelector('h1') || document.querySelector('h2') || document.querySelector('#header');
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

        // `#next-content` is the OTP screen's "Next" (an mds-button host,
        // verified 2026-09-25). The role/name fallback alone matched the code
        // field's Show/Hide toggle first, so it toggled visibility forever
        // instead of submitting.
        const nextContent = page.locator('#next-content');
        const submitBtn = (await nextContent.isVisible())
            ? nextContent
            : page
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
            refreshmint.log('Clicking OTP submit...');
            await submitBtn.click();
            context.otpSubmitted = true;
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
    const parsedLinks = /** @type {unknown} */ (
        JSON.parse(/** @type {string} */ (linksJson))
    );
    const methods = Array.isArray(parsedLinks)
        ? parsedLinks.map((m) => String(m))
        : [];

    if (methods.length > 0) {
        refreshmint.log('Discovered MFA methods: ' + methods.join(', '));
        // Present the discovered delivery methods as a dropdown so the user
        // picks an exact option instead of typing a substring to match.
        const choice = await refreshmint.promptChoice(
            'Select MFA method:',
            methods,
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

    // Open the download form by URL: an account with no activity in the
    // current view (e.g. a card with nothing since its last statement)
    // disables the download button, but the form itself can still export
    // all transactions. Verified 2026-09-25: #/dashboard/summary/<id>/<type>
    // maps to #/dashboard/transactions/downloads/<id>/<type>, preselected.
    const url = await page.url();
    const summaryMarker = '#/dashboard/summary/';
    const summaryIndex = url.indexOf(summaryMarker);
    if (summaryIndex !== -1) {
        const accountPath = url.slice(summaryIndex + summaryMarker.length);
        refreshmint.log(`Opening download form for ${accountPath}...`);
        await humanPace(page, 1000, 2000);
        await page.goto(
            url.slice(0, summaryIndex) +
                '#/dashboard/transactions/downloads/' +
                accountPath,
        );
        await waitMs(page, 5000);
        return { progressName: `opened download form for ${accountPath}` };
    }

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

    const pending = context.accounts.filter(
        (a) => !context.downloadedAccounts.has(a.name),
    );
    if (pending.length === 0) {
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

    // The form's selects are mds-select web components (buttons in shadow
    // DOM, verified 2026-09-25), which the old document.querySelectorAll
    // ('button') search never saw -- so it silently downloaded the
    // preselected account once per account. Opening the form from an
    // account's own page preselects that account, so use it, and go back
    // for the next account instead of driving the account dropdown.
    const accountSelect = page.locator(
        '#select-account_options_id-selector-no-label',
    );
    const selectedAccountText = (await accountSelect.isVisible())
        ? await accountSelect.innerText()
        : '';
    const account = pending.find(
        (a) => a.last4 !== '' && selectedAccountText.includes(a.last4),
    );
    if (account === undefined) {
        refreshmint.log(
            `Download form is for "${selectedAccountText}", not a pending account; returning to dashboard.`,
        );
        await page.goto(DASHBOARD_URL);
        await waitMs(page, 5000);
        return { progressName: `back to dashboard for ${pending[0].name}` };
    }

    refreshmint.log(`Preparing download for: ${account.name}`);
    await humanPace(page, 1000, 2000);

    const fileTypeSelect = page.locator(
        '#select-showing-file-select-selector-no-label',
    );
    const fileType = await fileTypeSelect.innerText();
    if (!fileType.includes('CSV')) {
        // UNTESTED: CSV has been the default every time so far.
        refreshmint.log(`File type is "${fileType}"; choosing CSV...`);
        await fileTypeSelect.click();
        await waitMs(page, 1000);
        await page
            .getByRole('option', { name: /Spreadsheet \(Excel, CSV\)/ })
            .first()
            .click();
        await waitMs(page, 1000);
    }

    const activitySelect = page.locator(
        '#select-showing-activity-select-selector-no-label',
    );
    if (!(await activitySelect.innerText()).includes('All transactions')) {
        await activitySelect.click();
        await waitMs(page, 1000);
        // Option ids differ by account type (savings: ...-last24monthsoption;
        // cards: a JSON date range), so pick the option by its `label`
        // attribute (its rendered text is empty until the list is laid out).
        const optionSelector =
            '#showing-activity-select-selector-no-label mds-select-option';
        const allIndex = /** @type {number} */ (
            await page.evaluate(`Array.from(document.querySelectorAll(${JSON.stringify(optionSelector)}))
                .findIndex((o) => o.getAttribute('label') === 'All transactions')`)
        );
        if (allIndex !== -1) {
            await page.locator(optionSelector).nth(allIndex).click();
        }
        await waitMs(page, 1000);
    }
    const activity = await activitySelect.innerText();
    const fileTypeNow = await fileTypeSelect.innerText();
    if (
        !activity.includes('All transactions') ||
        !fileTypeNow.includes('CSV')
    ) {
        refreshmint.log(
            `Download options did not stick (activity "${activity}", file type "${fileTypeNow}").`,
        );
        return {
            progressName: `retrying download options for ${account.name}`,
        };
    }

    refreshmint.log('Clicking Download button...');
    await humanPace(page, 1000, 2000);
    const downloadPromise = page.waitForDownload(30000);
    await page.locator('#downloadButton').click();
    try {
        const download = await downloadPromise;
        // Chase names the file with a bare UUID; extraction only parses
        // documents named *.csv (or with a CSV MIME type).
        const filename = `Chase${account.last4}_Activity.csv`;
        refreshmint.log(
            `Download finished: ${download.suggestedFilename} -> ${filename}`,
        );
        await refreshmint.saveDownloadedResource(download.path, filename, {
            label: account.label,
        });
        context.downloadedAccounts.add(account.name);
    } catch (e) {
        refreshmint.log(`Download failed for ${account.name}: ${String(e)}`);
        return { progressName: `retrying download for ${account.name}` };
    }
    await page.goto(DASHBOARD_URL);
    await waitMs(page, 5000);
    return { progressName: `downloaded ${account.name}` };
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

        // Handlers may leave a sub-frame selected (handleLogin returns from
        // inside the sign-in iframe while waiting), so reset before probing;
        // otherwise routing depends on whichever frame the last step used.
        await context.mainPage.switchToMainFrame();
        if (await dismissSurvey(context.mainPage)) {
            refreshmint.log('Dismissed Chase feedback survey popup.');
            await waitMs(context.mainPage, 1500);
        }
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
        const parsedPageStatus = /** @type {unknown} */ (
            JSON.parse(pageStatusJson)
        );
        const pageStatus =
            /** @type {{h1: string, title: string, isLogin: boolean, isMfa: boolean, isDashboard: boolean}} */ (
                parsedPageStatus
            );
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
                const mfaFrame = await findMfaFrame(context.mainPage);
                if (
                    mfaFrame !== null ||
                    header.includes('confirm') ||
                    title.includes('identity') ||
                    // UNTESTED after the 2026-09-05 retained artifact. This
                    // body marker keeps transient MFA renders from falling
                    // through to the dashboard/login handler.
                    pageStatus.isMfa ||
                    urlFragment.includes('step=confirmIdentity')
                ) {
                    if (mfaFrame !== null) {
                        // Verified 2026-09-25 end to end: method picker,
                        // mobile-number confirmation, and OTP entry all
                        // render inside this frame.
                        refreshmint.log(
                            `MFA screen is inside the sign-in iframe: ${mfaFrame}`,
                        );
                        await context.mainPage.switchToFrame(mfaFrame);
                    }
                    // No need to switch back afterwards: the loop resets
                    // to the main frame before its next probe.
                    stepReturn = await handleMfa(context);
                } else if (
                    header.includes('download') ||
                    urlFragment.includes('downloadAccountTransactions') ||
                    // Moved to #/dashboard/transactions/downloads/<id>/<type>
                    // (seen 2026-09-25) with no h1; must win over '/dashboard'.
                    urlFragment.includes('/transactions/downloads/')
                ) {
                    stepReturn = await handleDownload(context);
                } else if (
                    header.includes('account details') ||
                    urlFragment.includes('accountDetails') ||
                    // Account pages moved to #/dashboard/summary/<id>/<type>
                    // (seen 2026-09-25) with no h1; this must win over the
                    // generic '/dashboard' branch below.
                    urlFragment.includes('/dashboard/summary/')
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

// Fail loud: await main() so any thrown error rejects the top-level promise and
// the scrape is recorded as failed. A top-level `.catch` that only logs resolves
// the promise, making every scrape report success even when nothing was captured.
await main();
