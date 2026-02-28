/**
 * Citi scraper for Refreshmint.
 */

const CITI_ORIGINS = ['https://www.citi.com', 'https://online.citi.com'];
const LOGIN_URL = 'https://www.citi.com/login?nextRoute=dashboard';
const STATEMENTS_PAGE_NAME = 'pageName=StatementsAndDocumentServices';
const DASHBOARD_URL = 'https://online.citi.com/US/ag/dashboard';
const DASHBOARD_CREDIT_CARD_URL_PREFIX =
    'https://online.citi.com/US/ag/dashboard/credit-card';
const ACCOUNT_STATEMENTS_URL_PREFIX =
    'https://online.citi.com/US/nga/accstatement';
const REWARDS_DETAILS_URL_PREFIX =
    'https://online.citi.com/US/nga/reward/dashboard/costco/';
const DOWNLOAD_LIMIT = 2;

function utf8Bytes(text) {
    const encoded = encodeURIComponent(text);
    /** @type {number[]} */
    const bytes = [];
    for (let i = 0; i < encoded.length; i++) {
        const ch = encoded[i];
        if (ch === '%') {
            bytes.push(parseInt(encoded.slice(i + 1, i + 3), 16));
            i += 2;
        } else {
            bytes.push(ch.charCodeAt(0));
        }
    }
    return bytes;
}

/**
 * @param {string} dateText
 * @returns {string | null}
 */
function isoDateFromCitiText(dateText) {
    const parsed = new Date(dateText + ' 12:00:00 GMT-0500');
    if (Number.isNaN(parsed.getTime())) {
        return null;
    }
    return parsed.toISOString().slice(0, 10);
}

/**
 * @param {string[]} values
 * @returns {string}
 */
function toCsvLine(values) {
    return values
        .map((value) => '"' + String(value).replace(/"/g, '""') + '"')
        .join(',');
}

function todayIsoDate() {
    return new Date().toISOString().slice(0, 10);
}

/**
 * @param {string} amountText
 * @returns {string}
 */
function normalizeCurrencyAmount(amountText) {
    const raw = String(amountText || '').trim();
    if (!raw) {
        return '';
    }
    const negative = raw.includes('-') || /^\(.*\)$/.test(raw);
    const unsigned = raw
        .replace(/[()]/g, '')
        .replace(/[$,]/g, '')
        .replace(/-/g, '')
        .trim();
    if (!unsigned) {
        return '';
    }
    return negative ? `-${unsigned}` : unsigned;
}

/**
 * @param {string} accountName
 * @returns {string}
 */
function getCitiLabel(accountName) {
    return String(accountName || '')
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, '_')
        .replace(/^_+|_+$/g, '');
}

/**
 * @param {PageApi} page
 * @returns {Promise<{accountName: string, last4: string, label: string} | null>}
 */
async function getCurrentCitiAccountInfo(page) {
    const accountInfoJson = /** @type {string} */ (
        await page.evaluate(`(function() {
            const body = document.body
                ? document.body.innerText.replace(/\\u00a0/g, ' ')
                : '';
            const match = body.match(/([^\\n]+Card by Citi - (\\d{4}))/);
            if (!match) {
                return JSON.stringify(null);
            }
            return JSON.stringify({
                accountName: match[1].trim().replace(/\\s+/g, ' '),
                last4: match[2],
            });
        })()`)
    );
    const accountInfo = JSON.parse(accountInfoJson);
    if (accountInfo == null) {
        return null;
    }
    return {
        accountName: accountInfo.accountName,
        last4: accountInfo.last4,
        label: getCitiLabel(accountInfo.accountName),
    };
}

/**
 * @param {PageApi} page
 * @returns {Promise<{accountName: string, last4: string, label: string, statementClosingDate: string, currentBalance: string, availableCredit: string, creditLimit: string, paymentDueDate: string, lastStatementBalance: string, minimumPaymentDue: string, rewardsEarnedYtd: string} | null>}
 */
async function getDashboardAccountSnapshot(page) {
    const snapshotJson = /** @type {string} */ (
        await page.evaluate(`(function() {
            const body = document.body
                ? document.body.innerText.replace(/\\u00a0/g, ' ')
                : '';
            const accountMatch = body.match(/([^\\n]+Card by Citi - (\\d{4}))/);
            if (!accountMatch) {
                return JSON.stringify(null);
            }
            function matchValue(pattern) {
                const m = body.match(pattern);
                return m ? m[1].trim() : '';
            }
            return JSON.stringify({
                accountName: accountMatch[1].trim().replace(/\\s+/g, ' '),
                last4: accountMatch[2],
                statementClosingDate: matchValue(/Statement closing ([A-Za-z]{3} \\d{1,2}, \\d{4})/),
                currentBalance: matchValue(/Current Balance\\s+\\$([\\d,]+\\.\\d{2})/),
                availableCredit: matchValue(/Available Credit\\s+\\$([\\d,]+\\.\\d{2})/),
                creditLimit: matchValue(/Credit Limit:\\s+\\$([\\d,]+\\.\\d{2})/),
                paymentDueDate: matchValue(/Payment due on ([A-Za-z]{3} \\d{1,2}, \\d{4})/),
                lastStatementBalance: matchValue(/Last Statement Balance\\s+\\$([\\d,]+\\.\\d{2})/),
                minimumPaymentDue: matchValue(/Minimum Payment Due\\s+\\$([\\d,]+\\.\\d{2})/),
                rewardsEarnedYtd: matchValue(/\\$([\\d,]+(?:\\.\\d{2})?)\\s+Costco Cash Rewards\\s+Earned YTD/),
            });
        })()`)
    );
    const snapshot = JSON.parse(snapshotJson);
    if (snapshot == null) {
        return null;
    }
    return {
        accountName: snapshot.accountName,
        last4: snapshot.last4,
        label: getCitiLabel(snapshot.accountName),
        statementClosingDate: snapshot.statementClosingDate,
        currentBalance: snapshot.currentBalance,
        availableCredit: snapshot.availableCredit,
        creditLimit: snapshot.creditLimit,
        paymentDueDate: snapshot.paymentDueDate,
        lastStatementBalance: snapshot.lastStatementBalance,
        minimumPaymentDue: snapshot.minimumPaymentDue,
        rewardsEarnedYtd: snapshot.rewardsEarnedYtd,
    };
}

/**
 * @param {PageApi} page
 * @returns {Promise<string | null>}
 */
async function saveDashboardAccountSnapshot(page) {
    const snapshot = await getDashboardAccountSnapshot(page);
    if (snapshot == null) {
        refreshmint.log('Could not derive Citi dashboard account snapshot.');
        return null;
    }
    const existingDocs = JSON.parse(
        await refreshmint.listAccountDocuments({ label: snapshot.label }),
    );
    const summaryFilename = 'account-summary.json';
    const summaryExists = existingDocs.some((doc) =>
        String(doc.filename || '').endsWith(`-${summaryFilename}`),
    );
    if (!summaryExists) {
        const payload =
            JSON.stringify(
                {
                    scrapedAt: new Date().toISOString(),
                    accountName: snapshot.accountName,
                    last4: snapshot.last4,
                    statementClosingDate: snapshot.statementClosingDate,
                    currentBalance: normalizeCurrencyAmount(
                        snapshot.currentBalance,
                    ),
                    availableCredit: normalizeCurrencyAmount(
                        snapshot.availableCredit,
                    ),
                    creditLimit: normalizeCurrencyAmount(snapshot.creditLimit),
                    paymentDueDate: snapshot.paymentDueDate,
                    lastStatementBalance: normalizeCurrencyAmount(
                        snapshot.lastStatementBalance,
                    ),
                    minimumPaymentDue: normalizeCurrencyAmount(
                        snapshot.minimumPaymentDue,
                    ),
                    rewardsEarnedYtd: normalizeCurrencyAmount(
                        snapshot.rewardsEarnedYtd,
                    ),
                },
                null,
                2,
            ) + '\n';
        await refreshmint.saveResource(summaryFilename, utf8Bytes(payload), {
            coverageEndDate: todayIsoDate(),
            mimeType: 'application/json',
            label: snapshot.label,
            accountName: snapshot.accountName,
            accountLast4: snapshot.last4,
            currentBalance: normalizeCurrencyAmount(snapshot.currentBalance),
            availableCredit: normalizeCurrencyAmount(snapshot.availableCredit),
            creditLimit: normalizeCurrencyAmount(snapshot.creditLimit),
            paymentDueDate: snapshot.paymentDueDate,
            lastStatementBalance: normalizeCurrencyAmount(
                snapshot.lastStatementBalance,
            ),
            minimumPaymentDue: normalizeCurrencyAmount(
                snapshot.minimumPaymentDue,
            ),
            rewardsEarnedYtd: normalizeCurrencyAmount(
                snapshot.rewardsEarnedYtd,
            ),
        });
        refreshmint.log(
            `Saved Citi account summary for ${snapshot.accountName} to label ${snapshot.label}`,
        );
    } else {
        refreshmint.log(
            `Skipping existing Citi account summary for label ${snapshot.label}`,
        );
    }
    refreshmint.reportValue('citi_account_label', snapshot.label);
    refreshmint.reportValue(
        'citi_current_balance',
        normalizeCurrencyAmount(snapshot.currentBalance),
    );
    refreshmint.reportValue(
        'citi_rewards_earned_ytd',
        normalizeCurrencyAmount(snapshot.rewardsEarnedYtd),
    );
    return snapshot.label;
}

/**
 * Click the visible rewards details CTA from the dashboard.
 *
 * @param {PageApi} page
 * @returns {Promise<boolean>}
 */
async function openRewardsDetailsFromDashboard(page) {
    const clickedJson = /** @type {string} */ (
        await page.evaluate(`(function() {
            function isVisible(el) {
                return !!(
                    el &&
                    (el.offsetWidth || el.offsetHeight || el.getClientRects().length)
                );
            }

            const target = Array.from(document.querySelectorAll('button, a')).find(
                (el) =>
                    isVisible(/** @type {HTMLElement} */ (el)) &&
                    ((el.innerText || el.textContent || '').replace(/\\s+/g, ' ').trim() ===
                        'View Details'),
            );
            if (!target) {
                return JSON.stringify({ clicked: false });
            }
            target.click();
            return JSON.stringify({ clicked: true });
        })()`)
    );
    const clicked = JSON.parse(clickedJson);
    if (!clicked.clicked) {
        refreshmint.log(
            'Citi dashboard does not show a visible rewards details CTA.',
        );
        return false;
    }
    refreshmint.log('Opened Citi rewards details from dashboard.');
    await waitMs(page, 3000);
    return true;
}

/**
 * @param {PageApi} page
 * @returns {Promise<{accountName: string, last4: string, label: string, rewardsYear: string, earnedYearToDate: string, certificateStatus: string, certificateIssuedDate: string, certificateIssuedVia: string, certificateNumber: string, certificateAmount: string, categories: Record<string, string>} | null>}
 */
async function getRewardsSummary(page) {
    const rewardsJson = /** @type {string} */ (
        await page.evaluate(`(function() {
            const body = document.body
                ? document.body.innerText.replace(/\\u00a0/g, ' ')
                : '';
            const accountMatch = body.match(/(Costco Anywhere Visa[^\\n]*?Card by Citi-?(\\d{4}))/);
            if (!accountMatch) {
                return JSON.stringify(null);
            }

            function matchValue(pattern) {
                const m = body.match(pattern);
                return m ? m[1].trim() : '';
            }

            function normalizeKey(text) {
                return text
                    .toLowerCase()
                    .replace(/[%*]/g, '')
                    .replace(/[^a-z0-9]+/g, '_')
                    .replace(/^_+|_+$/g, '');
            }

            const categories = {};
            const categoryPattern =
                /(5%\\*?\\s*Gas at Costco|4%\\*?\\s*Other Eligible Gas & EV Charging|3%\\s*Restaurants|3%\\s*Eligible Travel|2%\\s*Costco and Costco\\.com|1%\\s*All Other Purchases)\\s*\\$([\\d,]+\\.\\d{2})/g;
            let match;
            while ((match = categoryPattern.exec(body)) !== null) {
                categories[normalizeKey(match[1])] = match[2];
            }

            return JSON.stringify({
                accountName: accountMatch[1].replace(/\\s+/g, ' ').replace(/ -?(\\d{4})$/, ' - $1').trim(),
                last4: accountMatch[2],
                rewardsYear: matchValue(/(\\d{4}) Costco Cash Rewards\\s+Earned Year to Date/),
                earnedYearToDate: matchValue(/\\$([\\d,]+\\.\\d{2})\\s+(?:\\d{4} Costco Cash Rewards\\s+)?Earned Year to Date/),
                certificateStatus: matchValue(/Certificate Status:\\s*([^\\n]+)/),
                certificateIssuedDate: matchValue(/On\\s+(\\d{2}\\/\\d{2}\\/\\d{4})\\s+via/),
                certificateIssuedVia: matchValue(/On\\s+\\d{2}\\/\\d{2}\\/\\d{4}\\s+via\\s+([^\\n]+)/),
                certificateNumber: matchValue(/Current Certificate Number:\\s*([^\\n]+)/),
                certificateAmount: matchValue(/\\$([\\d,]+\\.\\d{2})\\s+\\d{4} Credit Card Reward Certificate Amount/),
                categories,
            });
        })()`)
    );
    const rewards = JSON.parse(rewardsJson);
    if (rewards == null) {
        return null;
    }
    return {
        accountName: rewards.accountName,
        last4: rewards.last4,
        label: getCitiLabel(rewards.accountName),
        rewardsYear: rewards.rewardsYear,
        earnedYearToDate: rewards.earnedYearToDate,
        certificateStatus: rewards.certificateStatus,
        certificateIssuedDate: rewards.certificateIssuedDate,
        certificateIssuedVia: rewards.certificateIssuedVia,
        certificateNumber: rewards.certificateNumber,
        certificateAmount: rewards.certificateAmount,
        categories: rewards.categories || {},
    };
}

/**
 * @param {PageApi} page
 * @returns {Promise<string | null>}
 */
async function saveRewardsSummary(page) {
    const summary = await getRewardsSummary(page);
    if (summary == null) {
        refreshmint.log('Could not derive Citi rewards summary.');
        return null;
    }

    const rewardsYear = summary.rewardsYear || todayIsoDate().slice(0, 4);
    const filename = `rewards/${rewardsYear}-costco-rewards-summary.json`;
    const existingDocs = new Set(
        JSON.parse(
            await refreshmint.listAccountDocuments({ label: summary.label }),
        ).map((doc) => doc.filename),
    );
    if (!existingDocs.has(filename)) {
        const payload =
            JSON.stringify(
                {
                    scrapedAt: new Date().toISOString(),
                    accountName: summary.accountName,
                    last4: summary.last4,
                    rewardsYear,
                    earnedYearToDate: normalizeCurrencyAmount(
                        summary.earnedYearToDate,
                    ),
                    certificateStatus: summary.certificateStatus,
                    certificateIssuedDate: summary.certificateIssuedDate,
                    certificateIssuedVia: summary.certificateIssuedVia,
                    certificateNumber: summary.certificateNumber,
                    certificateAmount: normalizeCurrencyAmount(
                        summary.certificateAmount,
                    ),
                    categories: Object.fromEntries(
                        Object.entries(summary.categories).map(
                            ([key, value]) => [
                                key,
                                normalizeCurrencyAmount(value),
                            ],
                        ),
                    ),
                },
                null,
                2,
            ) + '\n';
        await refreshmint.saveResource(filename, utf8Bytes(payload), {
            coverageEndDate: todayIsoDate(),
            mimeType: 'application/json',
            label: summary.label,
            accountName: summary.accountName,
            accountLast4: summary.last4,
            rewardsYear,
            rewardsEarnedYtd: normalizeCurrencyAmount(summary.earnedYearToDate),
            rewardCertificateStatus: summary.certificateStatus,
            rewardCertificateIssuedDate: summary.certificateIssuedDate,
            rewardCertificateIssuedVia: summary.certificateIssuedVia,
            rewardCertificateNumber: summary.certificateNumber,
            rewardCertificateAmount: normalizeCurrencyAmount(
                summary.certificateAmount,
            ),
        });
        refreshmint.log(`Saved Citi rewards summary: ${filename}`);
    } else {
        refreshmint.log(`Skipping existing Citi rewards summary: ${filename}`);
    }

    refreshmint.reportValue(
        'citi_rewards_earned_ytd',
        normalizeCurrencyAmount(summary.earnedYearToDate),
    );
    refreshmint.reportValue(
        'citi_reward_certificate_amount',
        normalizeCurrencyAmount(summary.certificateAmount),
    );
    refreshmint.reportValue(
        'citi_reward_certificate_status',
        summary.certificateStatus,
    );
    return summary.label;
}

/**
 * Click the visible rewards CTA once and log what Citi exposes next.
 *
 * @param {PageApi} page
 * @returns {Promise<void>}
 */
/**
 * @param {string} optionText
 * @returns {{filename: string, coverageEndDate: string | null} | null}
 */
function activityFileInfoForOption(optionText) {
    const statementMatch = optionText.match(
        /^Statement closed ([A-Za-z]{3} \d{1,2}, \d{4})$/,
    );
    if (statementMatch) {
        const coverageEndDate = isoDateFromCitiText(statementMatch[1]);
        if (coverageEndDate == null) {
            return null;
        }
        return {
            filename: `activity/${coverageEndDate}-transactions.csv`,
            coverageEndDate,
        };
    }

    const lastYearMatch = optionText.match(/^Last year \((\d{4})\)$/);
    if (lastYearMatch) {
        const year = lastYearMatch[1];
        return {
            filename: `activity/${year}-last-year-transactions.csv`,
            coverageEndDate: `${year}-12-31`,
        };
    }

    if (optionText === 'Year to date') {
        const now = new Date();
        const year = String(now.getUTCFullYear());
        return {
            filename: `activity/${year}-year-to-date-transactions.csv`,
            coverageEndDate: now.toISOString().slice(0, 10),
        };
    }

    return null;
}

/**
 * @typedef {object} ScrapeContext
 * @property {PageApi} mainPage
 * @property {number} currentStep
 * @property {string[]} progressNames
 * @property {Set<string>} progressNamesSet
 * @property {number} lastProgressStep
 */

async function waitMs(page, ms) {
    try {
        await page.evaluate(`new Promise(r => setTimeout(r, ${ms}))`);
    } catch (e) {
        // Navigation can interrupt the execution context during sleeps.
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

async function logSnapshot(page, tag, track = 'state-loop') {
    try {
        const diff = await page.snapshot({ incremental: true, track });
        refreshmint.log(`${tag} snapshot: ${diff}`);
    } catch (e) {
        refreshmint.log(`${tag} snapshot failed: ${e}`);
    }
}

async function waitForBusy(page) {
    // UNTESTED: replace with Citi-specific busy indicator once observed.
    const spinnerSelector =
        '[aria-busy="true"], .loading, .spinner, [data-testid="loading-spinner"]';
    try {
        for (let i = 0; i < 20; i++) {
            const visible = await page.isVisible(spinnerSelector);
            if (!visible) {
                return;
            }
            refreshmint.log('Waiting for loading indicator to clear...');
            await waitMs(page, 500);
        }
    } catch (_e) {
        // Ignore errors if the page changes while polling.
    }
}

/**
 * @param {ScrapeContext} context
 * @returns {Promise<{progressName: string, done?: boolean}>}
 */
async function handleLogin(context) {
    const page = context.mainPage;
    const url = await page.url();
    refreshmint.log('State: login. URL: ' + url);

    await page.switchToMainFrame();
    await waitForBusy(page);
    await logSnapshot(page, 'login');

    const loginStateJson = /** @type {string} */ (
        await page.evaluate(`(function() {
            const bodyText = document.body ? document.body.innerText.toLowerCase() : '';
            const username = document.querySelector('#username');
            const password = document.querySelector('#citi-input2-0');
            const signInButton = document.querySelector('#signInBtn');
            const errorText = Array.from(
                document.querySelectorAll('[role="alert"], .alert, .error, .error-message'),
            )
                .map((el) => (el.innerText || el.textContent || '').trim())
                .filter(Boolean)
                .join(' | ');
            return JSON.stringify({
                title: document.title,
                hasUsername: !!username,
                hasPassword: !!password,
                hasSignInButton: !!signInButton,
                bodyHasSignOn:
                    bodyText.includes('sign on') || bodyText.includes('sign in'),
                errorText,
            });
        })()`)
    );
    const loginState = JSON.parse(loginStateJson);
    refreshmint.log('Login state: ' + loginStateJson);

    if (
        !loginState.hasUsername ||
        !loginState.hasPassword ||
        !loginState.hasSignInButton
    ) {
        refreshmint.log('Expected Citi login fields are not ready yet.');
        return { progressName: 'waiting for citi login fields' };
    }

    if (loginState.errorText) {
        throw new Error(
            'Citi login page shows an error: ' + loginState.errorText,
        );
    }

    // The top-level www.citi.com page hosts the real login form.
    await page.fill('#username', 'citi_username');
    await humanPace(page, 200, 400);
    await page.fill('#citi-input2-0', 'citi_password');
    await humanPace(page, 600, 1000);

    refreshmint.log('Submitting Citi sign-on form.');
    await page.click('#signInBtn');
    await waitMs(page, 4000);

    return { progressName: 'submitted citi login' };
}

/**
 * @param {ScrapeContext} context
 * @returns {Promise<{progressName: string, done?: boolean}>}
 */
async function handleMfa(context) {
    const page = context.mainPage;
    const url = await page.url();
    refreshmint.log('State: mfa. URL: ' + url);

    await page.switchToMainFrame();
    await waitForBusy(page);
    await logSnapshot(page, 'mfa');

    const mfaStateJson = /** @type {string} */ (
        await page.evaluate(`(function() {
            const bodyText = document.body ? document.body.innerText.toLowerCase() : '';
            const codeInput = document.querySelector(
                'input[name="otp"], input[name="code"], input[inputmode="numeric"], input[autocomplete="one-time-code"]',
            );
            const verifyButton = Array.from(document.querySelectorAll('button'))
                .find((el) =>
                    ['verify', 'continue', 'submit', 'sign on'].includes(
                        (el.innerText || '').trim().toLowerCase(),
                    ),
                );
            return JSON.stringify({
                title: document.title,
                hasCodeInput: !!codeInput,
                hasVerifyButton: !!verifyButton,
                bodySnippet: document.body ? document.body.innerText.slice(0, 1500) : '',
                bodyHasMfaText:
                    bodyText.includes('verification code') ||
                    bodyText.includes('security code') ||
                    bodyText.includes('one-time passcode') ||
                    bodyText.includes('one time passcode') ||
                    bodyText.includes('multi-factor'),
            });
        })()`)
    );
    refreshmint.log('MFA state: ' + mfaStateJson);
    return { progressName: 'inspected citi mfa', done: true };
}

/**
 * @param {ScrapeContext} context
 * @returns {Promise<{progressName: string, done?: boolean}>}
 */
async function handleLoggedIn(context) {
    const page = context.mainPage;
    const url = await page.url();
    refreshmint.log('State: logged-in. URL: ' + url);

    await page.switchToMainFrame();
    await waitForBusy(page);
    await logSnapshot(page, 'logged-in');

    const loggedInStateJson = /** @type {string} */ (
        await page.evaluate(`(function() {
            const accountLinks = Array.from(document.querySelectorAll('a'))
                .map((el) => (el.innerText || '').trim())
                .filter((text) => /\\.\\.\\.-\\d{4}$/.test(text) || /\\.\\.\\.\\d{4}$/.test(text))
                .slice(0, 10);
            return JSON.stringify({
                title: document.title,
                hasSignOff: !!document.querySelector('#signOffmainAnchor'),
                hasAccountsMenu: !!document.querySelector('#accountsmainAnchor0, #accountsMainLI'),
                hasReturnToAccount: Array.from(document.querySelectorAll('a, button')).some(
                    (el) =>
                        ((el.innerText || el.textContent || '').trim().toLowerCase() ===
                            'return to your account'),
                ),
                hasNotFoundText:
                    (document.body ? document.body.innerText.toLowerCase() : '').includes(
                        "looks like that information isn't here",
                    ),
                accountLinks,
                bodySnippet: document.body ? document.body.innerText.slice(0, 1200) : '',
            });
        })()`)
    );
    const loggedInState = JSON.parse(loggedInStateJson);
    refreshmint.log('Logged-in state: ' + loggedInStateJson);

    if (loggedInState.hasNotFoundText || loggedInState.hasReturnToAccount) {
        refreshmint.log(
            'Logged-in shell is on a missing page. Returning to dashboard.',
        );
        await page.goto(DASHBOARD_URL);
        await waitMs(page, 3000);
        return { progressName: 'returned to citi dashboard' };
    }

    if (
        url.startsWith(DASHBOARD_CREDIT_CARD_URL_PREFIX) &&
        !context.progressNamesSet.has('citi rewards pass complete')
    ) {
        await saveDashboardAccountSnapshot(page);
        const openedRewards = await openRewardsDetailsFromDashboard(page);
        if (openedRewards) {
            return { progressName: 'opened citi rewards page' };
        }
        return { progressName: 'citi rewards pass complete' };
    }

    if (
        url.startsWith(DASHBOARD_CREDIT_CARD_URL_PREFIX) &&
        !context.progressNamesSet.has('citi activity pass complete')
    ) {
        await saveDashboardAccountSnapshot(page);
        const activityResult = await scrapeDashboardActivityCsvs(page);
        if (activityResult != null) {
            return { progressName: 'citi activity pass complete' };
        }
    }

    if (!url.includes(STATEMENTS_PAGE_NAME)) {
        const navigationResultJson = /** @type {string} */ (
            await page.evaluate(`(function() {
                const statementLink = Array.from(document.querySelectorAll('a')).find(
                    (el) =>
                        (el.getAttribute('href') || '').includes(
                            'pageName=StatementsAndDocumentServices',
                        ),
                );
                if (statementLink) {
                    statementLink.click();
                    return JSON.stringify({ path: 'link-click' });
                }
                const viewStatements = Array.from(document.querySelectorAll('a, button')).find(
                    (el) => ((el.innerText || el.textContent || '').trim().toLowerCase() === 'view statements'),
                );
                if (viewStatements) {
                    viewStatements.click();
                    return JSON.stringify({ path: 'view-statements-click' });
                }
                return JSON.stringify({ path: 'none' });
            })()`)
        );
        refreshmint.log(
            'Statements navigation attempt: ' + navigationResultJson,
        );

        const navigationResult = JSON.parse(navigationResultJson);
        if (navigationResult.path === 'none') {
            throw new Error(
                'Could not find Citi statements navigation from the logged-in page',
            );
        }

        await waitMs(page, 4000);
        return { progressName: 'navigated toward citi statements page' };
    }

    return { progressName: 'logged in', done: true };
}

/**
 * @param {ScrapeContext} context
 * @returns {Promise<{progressName: string, done?: boolean}>}
 */
async function handleRewardsDetailsPage(context) {
    const page = context.mainPage;
    const url = await page.url();
    refreshmint.log('State: rewards-details-page. URL: ' + url);

    await page.switchToMainFrame();
    await waitForBusy(page);
    await waitMs(page, 3000);
    await logSnapshot(page, 'rewards-details-page');

    const pageStateJson = /** @type {string} */ (
        await page.evaluate(`(function() {
            return JSON.stringify({
                title: document.title,
                bodySnippet: document.body ? document.body.innerText.slice(0, 2000) : '',
            });
        })()`)
    );
    refreshmint.log('Citi rewards page state: ' + pageStateJson);

    await saveRewardsSummary(page);

    refreshmint.log('Returning to Citi dashboard after rewards capture.');
    await page.goto(DASHBOARD_URL);
    await waitMs(page, 3000);
    return { progressName: 'citi rewards pass complete' };
}

/**
 * @param {PageApi} page
 * @param {string} optionText
 */
async function selectActivityPeriod(page, optionText) {
    // The Citi mobile-layout combobox is visible in our current debug viewport.
    await page.click('#ums-timePeriodDropdown-mobile');
    await waitMs(page, 400);
    await page.evaluate(
        `(function(optionText) {
            const option = Array.from(
                document.querySelectorAll('#ums-timePeriodDropdown-mobile-listbox li[role="option"]'),
            ).find((el) =>
                ((el.innerText || el.textContent || '').trim().replace(/\\s+/g, ' ') === optionText),
            );
            if (!option) {
                throw new Error('activity period option not found: ' + optionText);
            }
            const target = option.querySelector('.cds-option2-item-container') || option;
            const rect = target.getBoundingClientRect();
            const eventInit = {
                bubbles: true,
                cancelable: true,
                composed: true,
                clientX: rect.left + rect.width / 2,
                clientY: rect.top + rect.height / 2,
            };
            for (const type of ['pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click']) {
                target.dispatchEvent(new MouseEvent(type, eventInit));
            }
        })(${JSON.stringify(optionText)})`,
    );
    await waitMs(page, 3000);

    const selectedText = await page.innerText('#ums-timePeriodDropdown-mobile');
    if (selectedText.trim() !== optionText) {
        throw new Error(
            'Citi activity period did not switch as expected. Wanted "' +
                optionText +
                '" but saw "' +
                selectedText.trim() +
                '"',
        );
    }
}

/**
 * @param {PageApi} page
 * @returns {Promise<{dateText: string, description: string, amount: string, note: string}[]>}
 */
async function extractVisibleActivityRows(page) {
    const rowsJson = /** @type {string} */ (
        await page.evaluate(`(function() {
            function text(el, selector) {
                if (!el) {
                    return '';
                }
                const node = selector ? el.querySelector(selector) : el;
                return node ? (node.innerText || node.textContent || '').trim().replace(/\\s+/g, ' ') : '';
            }
            return JSON.stringify(
                Array.from(document.querySelectorAll('#ums-transaction-tile .transaction-body.onyx_enhanced_layout'))
                    .map((row) => {
                        const description = text(row, '.description');
                        const topText = text(row, '.top');
                        const bottomText = text(row, '.bottom');
                        const note = text(
                            row,
                            '.transaction-chip, .chips-display, .chips-rewards-display',
                        );
                        const amountMatch = topText.match(/(-?\\$[\\d,]+\\.\\d{2})$/);
                        const dateMatch = bottomText.match(/([A-Za-z]{3} \\d{1,2}, \\d{4})/);
                        return {
                            description,
                            amount: amountMatch ? amountMatch[1] : '',
                            dateText: dateMatch ? dateMatch[1] : '',
                            note,
                        };
                    })
                    .filter((row) => row.description && row.amount && row.dateText),
            );
        })()`)
    );
    return JSON.parse(rowsJson);
}

/**
 * @param {PageApi} page
 * @returns {Promise<{progressName: string} | null>}
 */
async function scrapeDashboardActivityCsvs(page) {
    const accountInfo = await getCurrentCitiAccountInfo(page);
    if (accountInfo == null) {
        refreshmint.log('Could not derive Citi account label on dashboard.');
        return null;
    }
    const hasDropdown = await page.isVisible('#ums-timePeriodDropdown-mobile');
    if (!hasDropdown) {
        refreshmint.log(
            'Citi dashboard activity dropdown is not visible in this layout.',
        );
        return null;
    }

    const optionsJson = /** @type {string} */ (
        await page.evaluate(`(function() {
            return JSON.stringify(
                Array.from(
                    document.querySelectorAll('#ums-timePeriodDropdown-mobile-listbox li[role="option"]'),
                ).map((el) => ((el.innerText || el.textContent || '').trim().replace(/\\s+/g, ' '))),
            );
        })()`)
    );
    /** @type {string[]} */
    const options = JSON.parse(optionsJson);
    const statementPeriods = options.filter((text) =>
        /^Statement closed [A-Za-z]{3} \d{1,2}, \d{4}$/.test(text),
    );
    const specialPeriods = options.filter(
        (text) => /^Last year \(\d{4}\)$/.test(text) || text === 'Year to date',
    );
    const targetPeriods = [...statementPeriods, ...specialPeriods];
    if (targetPeriods.length === 0) {
        refreshmint.log(
            'No Citi statement-backed activity periods were found.',
        );
        return null;
    }

    const existingDocs = new Set(
        JSON.parse(
            await refreshmint.listAccountDocuments({
                label: accountInfo.label,
            }),
        ).map((doc) => doc.filename),
    );
    let downloadedCount = 0;

    for (const optionText of targetPeriods) {
        const fileInfo = activityFileInfoForOption(optionText);
        if (fileInfo == null) {
            refreshmint.log(
                'Could not derive Citi activity file info: ' + optionText,
            );
            continue;
        }
        const { filename, coverageEndDate } = fileInfo;
        if (existingDocs.has(filename)) {
            refreshmint.log('Skipping existing Citi activity CSV: ' + filename);
            continue;
        }

        refreshmint.log('Selecting Citi activity period: ' + optionText);
        await selectActivityPeriod(page, optionText);
        await waitForBusy(page);
        const rows = await extractVisibleActivityRows(page);
        refreshmint.log(
            'Extracted ' +
                rows.length +
                ' Citi activity rows for ' +
                optionText,
        );

        const csvLines = [
            'date,description,amount,note,period',
            ...rows.map((row) =>
                toCsvLine([
                    row.dateText,
                    row.description,
                    row.amount,
                    row.note,
                    optionText,
                ]),
            ),
        ];
        const csv = csvLines.join('\n') + '\n';
        await refreshmint.saveResource(filename, utf8Bytes(csv), {
            coverageEndDate,
            mimeType: 'text/csv',
            label: accountInfo.label,
            accountName: accountInfo.accountName,
            accountLast4: accountInfo.last4,
            period: optionText,
        });
        refreshmint.log('Saved Citi activity CSV: ' + filename);
        existingDocs.add(filename);
        downloadedCount++;
        await humanPace(page, 800, 1400);
    }

    if (downloadedCount > 0) {
        return {
            progressName: `downloaded ${downloadedCount} citi activity csvs`,
        };
    }

    return null;
}

/**
 * @param {ScrapeContext} context
 * @returns {Promise<{progressName: string, done?: boolean}>}
 */
async function handleStatementsPage(context) {
    const page = context.mainPage;
    const url = await page.url();
    refreshmint.log('State: statements-page. URL: ' + url);

    await page.switchToMainFrame();
    await waitForBusy(page);
    await logSnapshot(page, 'statements-page');

    const statementsStateJson = /** @type {string} */ (
        await page.evaluate(`(function() {
            const controls = Array.from(document.querySelectorAll('a, button'))
                .map((el) => ({
                    tag: el.tagName.toLowerCase(),
                    id: el.id || null,
                    text: (el.innerText || el.textContent || '')
                        .trim()
                        .replace(/\\s+/g, ' ')
                        .slice(0, 200),
                    aria: el.getAttribute('aria-label'),
                    href: el.getAttribute('href'),
                    hidden: !(el.offsetWidth || el.offsetHeight || el.getClientRects().length),
                }))
                .filter((item) => item.text || item.aria || item.id);
            const statementsLink = Array.from(document.querySelectorAll('a')).find(
                (el) => ((el.innerText || el.textContent || '').trim() === 'Statements'),
            );
            return JSON.stringify({
                title: document.title,
                hasStatementsLink: !!statementsLink,
                statementish: controls.filter((item) => {
                    const hay = (
                        item.text +
                        ' ' +
                        (item.aria || '') +
                        ' ' +
                        (item.id || '')
                    ).toLowerCase();
                    return (
                        hay.includes('statement') ||
                        hay.includes('document') ||
                        hay.includes('download') ||
                        hay.includes('pdf')
                    );
                }),
                bodySnippet: document.body ? document.body.innerText.slice(0, 2000) : '',
            });
        })()`)
    );
    const statementsState = JSON.parse(statementsStateJson);
    refreshmint.log('Statements page state: ' + statementsStateJson);

    if (statementsState.hasStatementsLink) {
        refreshmint.log(
            'Opening Citi Statements link from servicing landing page.',
        );
        await page.evaluate(`(function() {
            const link = Array.from(document.querySelectorAll('a')).find(
                (el) => ((el.innerText || el.textContent || '').trim() === 'Statements'),
            );
            if (!link) {
                throw new Error('Statements link not found');
            }
            link.click();
        })()`);
        await waitMs(page, 4000);
        return { progressName: 'opened citi statements detail page' };
    }

    return { progressName: 'inspected citi statements page', done: true };
}

/**
 * @param {ScrapeContext} context
 * @returns {Promise<{progressName: string, done?: boolean}>}
 */
async function handleAccountStatementsPage(context) {
    const page = context.mainPage;
    const url = await page.url();
    refreshmint.log('State: account-statements-page. URL: ' + url);

    if (!context.progressNamesSet.has('citi activity pass complete')) {
        refreshmint.log(
            'Returning to Citi dashboard before statements so activity and rewards capture run first.',
        );
        await page.goto(DASHBOARD_URL);
        await waitMs(page, 3000);
        return { progressName: 'returned to citi dashboard before statements' };
    }

    await page.switchToMainFrame();
    await waitForBusy(page);
    await waitMs(page, 3000);
    await logSnapshot(page, 'account-statements-page');

    const framesJson = await page.frames();
    refreshmint.log('Account statements frames: ' + framesJson);

    const pageStateJson = /** @type {string} */ (
        await page.evaluate(`(function() {
            return JSON.stringify({
                title: document.title,
                bodySnippet: document.body ? document.body.innerText.slice(0, 1500) : '',
                iframeIds: Array.from(document.querySelectorAll('iframe')).map((el) => ({
                    id: el.id || null,
                    name: el.getAttribute('name'),
                    src: el.getAttribute('src'),
                })),
            });
        })()`)
    );
    refreshmint.log('Account statements page state: ' + pageStateJson);

    const downloadTargetsJson = /** @type {string} */ (
        await page.evaluate(`(function() {
            function isVisible(el) {
                return !!(el && (el.offsetWidth || el.offsetHeight || el.getClientRects().length));
            }
            function findStatementContainerText(el) {
                let node = el;
                for (let i = 0; i < 6 && node; i++) {
                    const text = (node.innerText || node.textContent || '')
                        .trim()
                        .replace(/\\s+/g, ' ');
                    if (text.toLowerCase().includes('statement ending on')) {
                        return text;
                    }
                    node = node.parentElement;
                }
                return '';
            }
            return JSON.stringify(
                Array.from(document.querySelectorAll('a, button'))
                    .filter((el) => isVisible(el))
                    .map((el) => ({
                        text: (el.innerText || el.textContent || '').trim(),
                        rowText: findStatementContainerText(el),
                    }))
                    .filter((item) => item.text === 'Download' && item.rowText),
            );
        })()`)
    );
    refreshmint.log(
        'Account statement download targets: ' + downloadTargetsJson,
    );

    /** @type {{text: string, rowText: string}[]} */
    const downloadTargets = JSON.parse(downloadTargetsJson);
    if (downloadTargets.length === 0) {
        return {
            progressName: 'inspected citi account statements frame shell',
            done: true,
        };
    }

    const accountInfo = await getCurrentCitiAccountInfo(page);
    const accountLabel = accountInfo == null ? null : accountInfo.label;
    if (accountInfo != null) {
        refreshmint.reportValue('citi_account_label', accountInfo.label);
    }

    /**
     * @param {string} rowText
     * @returns {{coverageEndDate: string | null, filename: string | null}}
     */
    function deriveStatementFile(rowText) {
        const dateMatch = rowText.match(
            /statement ending on ([A-Za-z]+ \d{1,2}, \d{4})/i,
        );
        if (!dateMatch) {
            return { coverageEndDate: null, filename: null };
        }
        const parsed = new Date(dateMatch[1] + ' 12:00:00 GMT-0500');
        if (Number.isNaN(parsed.getTime())) {
            return { coverageEndDate: null, filename: null };
        }
        const coverageEndDate = parsed.toISOString().slice(0, 10);
        return {
            coverageEndDate,
            filename: `statements/${coverageEndDate}-statement.pdf`,
        };
    }

    const existingDocs = new Set(
        JSON.parse(
            await refreshmint.listAccountDocuments(
                accountLabel == null ? undefined : { label: accountLabel },
            ),
        ).map((doc) => doc.filename),
    );
    let downloadedCount = 0;
    let skippedExistingCount = 0;

    for (const target of downloadTargets) {
        if (DOWNLOAD_LIMIT > 0 && downloadedCount >= DOWNLOAD_LIMIT) {
            refreshmint.log('Reached Citi download limit: ' + DOWNLOAD_LIMIT);
            break;
        }

        const { coverageEndDate, filename } = deriveStatementFile(
            target.rowText,
        );
        if (filename && existingDocs.has(filename)) {
            skippedExistingCount++;
            refreshmint.log('Skipping existing Citi statement: ' + filename);
            continue;
        }

        refreshmint.log('Downloading Citi statement row: ' + target.rowText);
        const downloadPromise = page.waitForDownload(15000);
        await page.evaluate(
            `(function(targetRowText) {
                function isVisible(el) {
                    return !!(el && (el.offsetWidth || el.offsetHeight || el.getClientRects().length));
                }
                function findStatementContainerText(el) {
                    let node = el;
                    for (let i = 0; i < 6 && node; i++) {
                        const text = (node.innerText || node.textContent || '')
                            .trim()
                            .replace(/\\s+/g, ' ');
                        if (text.toLowerCase().includes('statement ending on')) {
                            return text;
                        }
                        node = node.parentElement;
                    }
                    return '';
                }
                const target = Array.from(document.querySelectorAll('a, button'))
                    .filter((el) => isVisible(el))
                    .find((el) => {
                        const text = (el.innerText || el.textContent || '').trim();
                        return text === 'Download' && findStatementContainerText(el) === targetRowText;
                    });
                if (!target) {
                    throw new Error('visible statement download control not found for row: ' + targetRowText);
                }
                target.click();
            })(${JSON.stringify(target.rowText)})`,
        );
        const download = await downloadPromise;
        const outputFilename =
            filename || `statements/${download.suggestedFilename}`;
        const options = coverageEndDate
            ? {
                  coverageEndDate,
                  label: accountLabel == null ? undefined : accountLabel,
                  accountName:
                      accountInfo == null ? undefined : accountInfo.accountName,
                  accountLast4:
                      accountInfo == null ? undefined : accountInfo.last4,
              }
            : undefined;
        await refreshmint.saveDownloadedResource(
            download.path,
            outputFilename,
            options,
        );
        refreshmint.log('Saved Citi statement: ' + outputFilename);
        existingDocs.add(outputFilename);
        downloadedCount++;
        await humanPace(page, 800, 1400);
    }

    if (downloadedCount === 0 && skippedExistingCount > 0) {
        return {
            progressName: 'citi statements already downloaded',
            done: true,
        };
    }

    return {
        progressName: `downloaded ${downloadedCount} citi statements`,
        done: true,
    };
}

async function main() {
    refreshmint.log('Citi scraper starting');
    const pages = await browser.pages();
    const mainPage = pages[0];
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
        refreshmint.log(`Step ${context.currentStep}: URL=${url}`);

        /** @type {{progressName: string, done?: boolean}} */
        let stepReturn;

        if (url === 'about:blank') {
            refreshmint.log('Navigating to Citi landing page...');
            await context.mainPage.goto(LOGIN_URL);
            stepReturn = { progressName: 'navigated to citi landing page' };
        } else if (url.startsWith(REWARDS_DETAILS_URL_PREFIX)) {
            stepReturn = await handleRewardsDetailsPage(context);
        } else if (url.startsWith(ACCOUNT_STATEMENTS_URL_PREFIX)) {
            stepReturn = await handleAccountStatementsPage(context);
        } else if (url.includes(STATEMENTS_PAGE_NAME)) {
            stepReturn = await handleStatementsPage(context);
        } else if (
            CITI_ORIGINS.some((origin) => url.startsWith(origin + '/'))
        ) {
            // UNTESTED: refine these state checks once we have real Citi snapshots.
            const stateJson = /** @type {string} */ (
                await context.mainPage.evaluate(`(function() {
                    const bodyText = document.body ? document.body.innerText.toLowerCase() : '';
                    return JSON.stringify({
                        hasUsername: !!document.querySelector('#username'),
                        hasPassword: !!document.querySelector('#citi-input2-0'),
                        hasSignOff: !!document.querySelector('#signOffmainAnchor'),
                        hasAccountsMenu:
                            !!document.querySelector('#accountsmainAnchor0, #accountsMainLI'),
                        hasOtpField: !!document.querySelector(
                            'input[name="otp"], input[name="code"], input[inputmode="numeric"]',
                        ),
                        hasMfaText:
                            bodyText.includes('verification code') ||
                            bodyText.includes('security code') ||
                            bodyText.includes('one-time passcode') ||
                            bodyText.includes('multi-factor'),
                        bodyHasInactivityHome:
                            bodyText.includes('we are sorry, our system is currently unavailable') ||
                            bodyText.includes('inactivity') ||
                            bodyText.includes('sign on and continue where you left off'),
                    });
                })()`)
            );
            const state = JSON.parse(stateJson);

            if (state.hasSignOff || state.hasAccountsMenu) {
                stepReturn = await handleLoggedIn(context);
            } else if (state.hasUsername || state.hasPassword) {
                stepReturn = await handleLogin(context);
            } else if (state.hasOtpField || state.hasMfaText) {
                stepReturn = await handleMfa(context);
            } else if (
                url.startsWith('https://www.citi.com/') ||
                state.bodyHasInactivityHome
            ) {
                refreshmint.log(
                    'Citi public-site shell did not match a known state. Returning to explicit login URL.',
                );
                await context.mainPage.goto(LOGIN_URL);
                stepReturn = {
                    progressName: 'returned to explicit citi login url',
                };
            } else {
                await logSnapshot(context.mainPage, 'unknown citi state');
                throw new Error(
                    'Unable to classify Citi page state at URL ' + url,
                );
            }
        } else {
            refreshmint.log(
                'Unexpected origin. Returning to Citi landing page.',
            );
            await context.mainPage.goto(LOGIN_URL);
            stepReturn = { progressName: 'returned to citi landing page' };
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
            refreshmint.log('Citi stub complete');
            break;
        }

        await humanPace(context.mainPage, 800, 1400);
    }
}

main().catch((err) => {
    refreshmint.log(`Fatal error: ${err.message}`);
    if (err.stack) {
        refreshmint.log(err.stack);
    }
});
