/**
 * Provident Credit Union scraper for Refreshmint.
 * Downloads all available statements and account activity CSVs from Provident Credit Union.
 */

/**
 * @typedef {object} ScrapeContext
 * @property {PageApi} mainPage
 * @property {number} currentStep
 * @property {string[]} progressNames
 * @property {Set<string>} progressNamesSet
 * @property {number} lastProgressStep
 * @property {string[]} pendingAccounts
 * @property {Set<string>} completedAccounts
 * @property {boolean} accountsDone
 * @property {boolean} statementsDone
 *
 * @typedef {object} StepReturn
 * @property {string} progressName
 * @property {boolean} [done]
 */

const BASE_URL = 'https://accountmanager.providentcu.org';
const SIGN_IN_URL = `${BASE_URL}/ProvidentOnlineBanking/SignIn.aspx`;
const STATEMENTS_URL = `${BASE_URL}/ProvidentOnlineBanking/statements.aspx`;
const SUMMARY_URL = `${BASE_URL}/ProvidentOnlineBanking/Accounts/AccountSummary.aspx`;

// Configuration for efficient scraping and debugging
const DOWNLOAD_LIMIT = null; // Set to number to limit downloads per run
const SKIP_BEFORE_DATE = null; // Format: YYYY-MM-DD e.g. "2026-01-01"

async function waitMs(page, ms) {
    await page.evaluate(`new Promise(r => setTimeout(r, ${ms}))`);
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

async function waitForBusy(page) {
    await page.evaluate(`(async () => {
        const start = Date.now();
        while (Date.now() - start < 30000) {
            const busy = document.getElementById('busy-div');
            if (!busy) return;
            const style = window.getComputedStyle(busy);
            if (style.display === 'none' || style.visibility === 'hidden' || style.opacity === '0') return;
            await new Promise(r => setTimeout(r, 500));
        }
    })()`);
}

async function humanPace(page, minMs, maxMs) {
    const delta = maxMs - minMs;
    const ms = minMs + Math.floor(Math.random() * (delta + 1));
    await waitMs(page, ms);
}

/**
 * @param {ScrapeContext} context
 * @returns {Promise<StepReturn>}
 */
async function handleLogin(context) {
    const page = context.mainPage;
    refreshmint.log('State: Login Page');

    // Dismiss browser compatibility warning if present.
    const hasContinue = await page.evaluate(`(function() {
            const buttons = Array.from(document.querySelectorAll('button, input[type="submit"], input[type="button"]'));
            const continueBtn = buttons.find(b => (b.textContent || b.value || "").includes("Continue"));
            if (!continueBtn) return false;
            const style = window.getComputedStyle(continueBtn);
            return style.display !== 'none' && style.visibility !== 'hidden';
        })()`);
    if (assertBoolean(hasContinue)) {
        refreshmint.log('Dismissing browser warning...');
        await page.evaluate(`(function() {
            const buttons = Array.from(document.querySelectorAll('button, input[type="submit"], input[type="button"]'));
            const continueBtn = buttons.find(b => (b.textContent || b.value || "").includes("Continue"));
            if (continueBtn) continueBtn.click();
        })()`);
        await waitMs(page, 2000);
    }

    refreshmint.log('Filling credentials...');
    await page.type(
        '#M_layout_content_PCDZ_MMCA7G7_ctl00_webInputForm_txtLoginName',
        'providentcu_username',
    );
    await humanPace(page, 200, 500);
    await page.type(
        '#M_layout_content_PCDZ_MMCA7G7_ctl00_webInputForm_txtPassword',
        'providentcu_password',
    );

    await humanPace(page, 1000, 2000);

    refreshmint.log('Clicking Sign On');
    await page.click(
        '#M_layout_content_PCDZ_MMCA7G7_ctl00_webInputForm_cmdContinue',
    );

    await waitMs(page, 3000);

    return { progressName: 'login submitted' };
}

/**
 * @param {ScrapeContext} context
 * @returns {Promise<StepReturn>}
 */
async function handleMfa(context) {
    const page = context.mainPage;
    refreshmint.log('State: MFA required');

    const code = await refreshmint.prompt('Enter MFA code:');
    await page.fill('input[name*="txtCode"], input[id*="txtCode"]', code);
    await page.click(
        'input[type="submit"][value="Continue"], #M_layout_content_PCDZ_MMCA7G7_ctl00_webInputForm_cmdContinue',
    );

    await waitMs(page, 3000);
    return { progressName: 'mfa submitted' };
}

/**
 * @param {ScrapeContext} context
 * @returns {Promise<StepReturn>}
 */
async function handleAccountSummary(context) {
    const page = context.mainPage;
    refreshmint.log('State: Account Summary');

    // Discover accounts if not already done
    if (
        !context.accountsDone &&
        context.pendingAccounts.length === 0 &&
        context.completedAccounts.size === 0
    ) {
        refreshmint.log('Discovering accounts...');
        await waitMs(page, 2000);
        const accountsJson = /** @type {string} */ (
            await page.evaluate(`(function() {
                const all = Array.from(document.querySelectorAll('button, a, .account-nickname'));
                const matches = all.filter(el => /x\\d{4}/.test(el.textContent));
                return JSON.stringify(matches.map(el => el.textContent.trim()));
            })()`)
        );
        context.pendingAccounts = [...new Set(JSON.parse(accountsJson))];
        refreshmint.log(
            `Discovered ${context.pendingAccounts.length} accounts: ${context.pendingAccounts.join(', ')}`,
        );
        if (context.pendingAccounts.length === 0) {
            const bodyText = /** @type {string} */ (
                await page.evaluate('document.body.innerText')
            );
            refreshmint.log(
                'WARNING: No accounts discovered on summary page. Body text length: ' +
                    bodyText.length,
            );
            context.accountsDone = true; // Nothing to do
        }
    }

    if (context.pendingAccounts.length > 0) {
        const account = context.pendingAccounts[0];
        refreshmint.log(`Navigating to account activity: ${account}`);
        await waitForBusy(page);
        const clicked = await page.evaluate(`(function(acc) {
                const all = Array.from(document.querySelectorAll('button, a, span'));
                const btn = all.find(b => b.textContent.includes(acc));
                if (btn) {
                    btn.click();
                    return true;
                }
                return false;
            })("${account}")`);
        if (assertBoolean(clicked)) {
            await waitMs(page, 3000);
            return { progressName: `navigating to activity: ${account}` };
        } else {
            refreshmint.log(`Account button not found for ${account}`);
            context.completedAccounts.add(context.pendingAccounts.shift());
            return { progressName: `skipped account ${account}` };
        }
    }

    context.accountsDone = true;

    if (!context.statementsDone) {
        refreshmint.log(
            'All accounts activity downloaded. Navigating to Statements & Notices...',
        );
        const clicked = await page.evaluate(`(function() {
                const links = Array.from(document.querySelectorAll('a'));
                const link = links.find(a => (a.textContent || "").includes("Statements & Notices"));
                if (link) {
                    link.click();
                    return true;
                }
                return false;
            })()`);

        if (assertBoolean(clicked)) {
            await waitMs(page, 3000);
            return { progressName: 'navigating to statements' };
        } else {
            refreshmint.log(
                'Statements & Notices link not found, falling back to goto',
            );
            await page.goto(STATEMENTS_URL);
            await waitMs(page, 3000);
            return { progressName: 'navigated to statements' };
        }
    }

    return { progressName: 'all tasks complete', done: true };
}

function getLabel(accountMatch) {
    // e.g. "Super Reward Checking x6590" -> "super_reward_checking_6590"
    return accountMatch
        .toLowerCase()
        .replace(/x(\d{4})/, '$1')
        .replace(/[^a-z0-9]+/g, '_')
        .replace(/^_+|_+$/g, '');
}

/**
 * @param {ScrapeContext} context
 * @returns {Promise<StepReturn>}
 */
async function handleAccountActivity(context) {
    const page = context.mainPage;
    const accountMatch = context.pendingAccounts[0];
    const label = getLabel(accountMatch);
    refreshmint.log(
        `State: Account Activity for ${accountMatch} (label: ${label})`,
    );

    await waitForBusy(page);
    await page.waitForSelector('button.icon-download', undefined);
    await page.click('button.icon-download');
    await waitMs(page, 1000);

    refreshmint.log('  Clicking Spreadsheet button...');
    const clickedSpreadsheet = await page.evaluate(`(function() {
            const buttons = Array.from(document.querySelectorAll('button'));
            const btn = buttons.find(b => b.textContent.trim() === "Spreadsheet");
            if (btn) {
                btn.click();
                return true;
            }
            return false;
        })()`);

    if (assertBoolean(clickedSpreadsheet)) {
        try {
            refreshmint.log('  Waiting for CSV download...');
            const dl = await page.waitForDownload(15000);
            const date = new Date().toISOString().split('T')[0];
            const sanitizedAcc = accountMatch.replace(/[^a-zA-Z0-9]/g, '_');
            const filename = `activity-${sanitizedAcc}-${date}.csv`;

            await refreshmint.saveDownloadedResource(
                dl.path,
                'transactions/' + filename,
                {
                    mimeType: 'text/csv',
                    label,
                },
            );
            refreshmint.log(
                `  Downloaded and saved: ${filename} to label ${label}`,
            );
        } catch (e) {
            refreshmint.log(
                `  Failed to download CSV for ${accountMatch}: ${String(e)}`,
            );
        }
    } else {
        refreshmint.log('  Spreadsheet button not found!');
    }

    context.completedAccounts.add(context.pendingAccounts.shift());
    refreshmint.log('Navigating back to Account Summary...');
    await page.goto(SUMMARY_URL);
    await waitMs(page, 2000);

    return { progressName: `completed activity: ${accountMatch}` };
}

function makeFilename(accountNumber, name, date) {
    const [mm, dd, yyyy] = date.split('/');
    const acct = accountNumber.replace(/\*/g, '');
    const safeName = name.replace(/\s+/g, '-');
    return `${acct}-${safeName}-${yyyy}-${mm}-${dd}.pdf`;
}

/**
 * @param {ScrapeContext} context
 * @returns {Promise<StepReturn>}
 */
async function handleStatements(context) {
    const page = context.mainPage;
    refreshmint.log('State: Statements Page');

    if (!context.accountsDone) {
        refreshmint.log('Accounts not yet done, navigating to Summary first');
        await page.goto(SUMMARY_URL);
        await waitMs(page, 2000);
        return { progressName: 'navigating to summary from statements' };
    }

    await waitForBusy(page);
    await page.waitForSelector('h2', undefined);

    const linksJson = /** @type {string} */ (
        await page.evaluate(`(function() {
            const links = Array.from(document.querySelectorAll('table a[aria-haspopup="true"][aria-owns]'));
            return JSON.stringify(links.map(link => ({
                dialogId: link.getAttribute('aria-owns'),
                name: link.closest('tr').cells[0].textContent.trim(),
                accountNumber: link.closest('tr').cells[2].textContent.trim()
            })));
        })()`)
    );

    const sections = JSON.parse(linksJson);
    refreshmint.log(`Found ${sections.length} statement sections`);

    const existingDocsJson = await refreshmint.listAccountDocuments();
    const existingDocs = JSON.parse(existingDocsJson || '[]');
    const existingFilenames = new Set(existingDocs.map((d) => d.filename));

    let totalDownloaded = 0;

    for (const section of sections) {
        const label = getLabel(
            `${section.name} x${section.accountNumber.slice(-4)}`,
        );
        refreshmint.log(
            `Processing section: ${section.name} ${section.accountNumber} (label: ${label})`,
        );

        await page.evaluate(`(function(dialogId) {
            const dialog = document.getElementById(dialogId);
            if (dialog) {
                dialog.style.display = 'block';
                dialog.style.visibility = 'visible';
            }
        })("${section.dialogId}")`);
        await waitMs(page, 500);

        const rowsJson = /** @type {string} */ (
            await page.evaluate(`(function(dialogId) {
                const dialog = document.getElementById(dialogId);
                if (!dialog) return "[]";
                const rows = Array.from(dialog.querySelectorAll('tbody tr'));
                return JSON.stringify(rows.map((row, index) => ({
                    index,
                    name: row.cells[0].textContent.trim(),
                    accountNumber: row.cells[2].textContent.trim(),
                    date: row.cells[3].textContent.trim()
                })));
            })("${section.dialogId}")`)
        );

        const rows = JSON.parse(rowsJson);
        refreshmint.log(`  Found ${rows.length} statements in dialog`);

        for (const row of rows) {
            const filename = makeFilename(
                row.accountNumber,
                row.name,
                row.date,
            );
            if (existingFilenames.has(filename)) continue;

            const [mm, dd, yyyy] = row.date.split('/');
            const coverageEndDate = `${yyyy}-${mm}-${dd}`;

            if (
                SKIP_BEFORE_DATE != null &&
                coverageEndDate < SKIP_BEFORE_DATE
            ) {
                // refreshmint.log(`  Skipping (before ${SKIP_BEFORE_DATE}): ${filename}`);
                continue;
            }

            if (DOWNLOAD_LIMIT != null && totalDownloaded >= DOWNLOAD_LIMIT) {
                refreshmint.log(
                    `  Download limit (${DOWNLOAD_LIMIT}) reached.`,
                );
                break;
            }

            refreshmint.log(`  Downloading ${filename}...`);

            await page.evaluate(`(function(dialogId, index, fn) {
                const dialog = document.getElementById(dialogId);
                const tr = dialog.querySelectorAll('tbody tr')[index];
                const link = tr.querySelector('a');
                if (link) {
                    link.setAttribute('download', fn);
                    link.removeAttribute('target');
                }
            })("${section.dialogId}", ${row.index}, "${filename}")`);

            const downloadPromise = page.waitForDownload(10000);

            const clicked = await page.evaluate(`(function(dialogId, index) {
                    const dialog = document.getElementById(dialogId);
                    const tr = dialog.querySelectorAll('tbody tr')[index];
                    const link = tr.querySelector('a');
                    if (link) {
                        link.click();
                        return true;
                    }
                    return false;
                })("${section.dialogId}", ${row.index})`);

            if (assertBoolean(clicked)) {
                try {
                    const download = await downloadPromise;
                    await refreshmint.saveDownloadedResource(
                        download.path,
                        'statements/' + filename,
                        {
                            coverageEndDate,
                            mimeType: 'application/pdf',
                            label,
                        },
                    );
                    refreshmint.log(
                        `  Downloaded and saved: ${filename} to label ${label}`,
                    );
                    totalDownloaded++;
                } catch (e) {
                    refreshmint.log(
                        `  Failed to download ${filename}: ${String(e)}`,
                    );
                }
            }
            await humanPace(page, 500, 1000);
        }

        await page.evaluate(`(function(dialogId) {
            const dialog = document.getElementById(dialogId);
            if (dialog) dialog.style.display = 'none';
        })("${section.dialogId}")`);
        await waitMs(page, 500);
    }

    refreshmint.log(`Finished downloading ${totalDownloaded} statements.`);
    context.statementsDone = true;

    refreshmint.log('Navigating back to Account Summary...');
    await page.goto(SUMMARY_URL);
    await waitMs(page, 2000);

    return { progressName: 'statements downloaded' };
}

async function main() {
    refreshmint.log('Provident scraper starting');
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
        pendingAccounts: [],
        completedAccounts: new Set(),
        accountsDone: false,
        statementsDone: false,
    };

    while (true) {
        context.currentStep++;
        const url = await context.mainPage.url();
        const urlWithoutFragment = url.split('#', 2)[0];
        const fragment = url.split('#', 2)[1] || '';

        refreshmint.log(
            `Step ${context.currentStep}: URL=${urlWithoutFragment}#${fragment}`,
        );

        /** @type {StepReturn} */
        let stepReturn;

        if (urlWithoutFragment.includes('SignIn.aspx')) {
            stepReturn = await handleLogin(context);
        } else if (
            urlWithoutFragment.includes('Mfa.aspx') ||
            urlWithoutFragment.includes('SecurityChallenge')
        ) {
            stepReturn = await handleMfa(context);
        } else if (
            urlWithoutFragment.includes('AccountSummary.aspx') &&
            fragment.startsWith('Accounts/') &&
            context.pendingAccounts.length > 0
        ) {
            stepReturn = await handleAccountActivity(context);
        } else if (
            urlWithoutFragment.includes('AccountSummary.aspx') ||
            urlWithoutFragment.includes('Home.aspx')
        ) {
            stepReturn = await handleAccountSummary(context);
        } else if (urlWithoutFragment.includes('statements.aspx')) {
            stepReturn = await handleStatements(context);
        } else if (
            urlWithoutFragment === 'about:blank' ||
            urlWithoutFragment === 'chrome://new-tab-page/'
        ) {
            refreshmint.log('Navigating to login page...');
            await context.mainPage.goto(SIGN_IN_URL);
            stepReturn = { progressName: 'navigating to login' };
        } else {
            refreshmint.log(`Unexpected URL: ${url}`);
            await context.mainPage.goto(SIGN_IN_URL);
            stepReturn = { progressName: 'lost, navigating to login' };
        }

        const progressName = stepReturn.progressName;
        context.progressNames.push(progressName);
        if (!context.progressNamesSet.has(progressName)) {
            context.progressNamesSet.add(progressName);
            context.lastProgressStep = context.currentStep;
        }

        if (context.currentStep - context.lastProgressStep > 30) {
            throw new Error('no progress in last 30 steps');
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
