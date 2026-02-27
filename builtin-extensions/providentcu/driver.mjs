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

function inspect(value) {
    if (value instanceof Error) {
        if (typeof value.stack === 'string' && value.stack.length > 0) {
            return value.stack;
        }
        return `${value.name || 'Error'}: ${value.message || ''}`;
    }
    try {
        return JSON.stringify(value);
    } catch {
        return String(value);
    }
}

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

async function navigateToSignIn(page) {
    await page.goto(SIGN_IN_URL, {
        waitUntil: 'domcontentloaded',
        timeout: 90000,
    });
    await page.waitForSelector('input[id$="txtLoginName"]', 90000);
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
    const raw = String(accountMatch || '');
    const compact = raw.replace(/\s+/g, ' ').trim();
    const last4Match = compact.match(/x\s*(\d{4})/i);
    const last4 = last4Match == null ? null : last4Match[1];

    const withoutAvailability = compact.replace(/available.*$/i, ' ');
    const withoutAmounts = withoutAvailability.replace(
        /\$[\d,]+(?:\.\d{2})?/g,
        ' ',
    );
    const withoutAccountNumber = withoutAmounts.replace(
        /x\s*\d{4}[a-z]*/gi,
        ' ',
    );
    const normalizedName = withoutAccountNumber
        .replace(/[-–—]+/g, ' ')
        .replace(/\s+/g, ' ')
        .trim();

    const base = slugify(normalizedName || compact || 'account');
    if (last4 == null) {
        return base;
    }
    return `${base}_${last4}`;
}

function slugify(value) {
    return value
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, '_')
        .replace(/^_+|_+$/g, '');
}

function makeActivityFilename(
    accountKey,
    dateRange,
    coverageStartDate,
    coverageEndDate,
) {
    const safeAccount = slugify(accountKey);
    const safeRange = slugify(dateRange);
    const safeStart = coverageStartDate || 'unknown_start';
    const safeEnd = coverageEndDate || 'unknown_end';
    return `activity-${safeAccount}-${safeRange}-${safeStart}-to-${safeEnd}.csv`;
}

function hasSavedDocument(existingFilenames, coverageEndDate, filename) {
    if (
        coverageEndDate == null ||
        coverageEndDate.trim() === '' ||
        filename.trim() === ''
    ) {
        return false;
    }
    const datedName = `${coverageEndDate}-${filename}`;
    if (existingFilenames.has(datedName)) {
        return true;
    }

    const dot = filename.lastIndexOf('.');
    const stem = dot >= 0 ? filename.slice(0, dot) : filename;
    const ext = dot >= 0 ? filename.slice(dot) : '';
    const collisionPrefix = `${coverageEndDate}-${stem}-`;

    for (const existing of existingFilenames) {
        if (
            existing.startsWith(collisionPrefix) &&
            existing.endsWith(ext) &&
            existing.length > collisionPrefix.length + ext.length
        ) {
            const suffix = existing.slice(
                collisionPrefix.length,
                existing.length - ext.length,
            );
            if (/^\d+$/.test(suffix)) {
                return true;
            }
        }
    }

    return false;
}

async function getHistoryDateRangeOptions(page) {
    for (let attempt = 0; attempt < 10; attempt++) {
        const optionsJson = /** @type {string} */ (
            await page.evaluate(`(function() {
                const host = document.querySelector('.IDS-Banking-Retail-Web-React-TransactionHistoryModule');
                if (!host) return "[]";

                const searchBtn = Array.from(host.querySelectorAll('button')).find(btn =>
                    btn.classList.contains('icon-search') && (btn.textContent || '').trim() === 'Search'
                );
                if (!searchBtn) return "[]";
                if (searchBtn.getAttribute('aria-expanded') !== 'true') {
                    searchBtn.click();
                }

                const section = host.querySelector('.expandable-container-section');
                if (!section) return "[]";
                const dateGroup = Array.from(section.querySelectorAll('.field-group')).find(group =>
                    /Date Range/i.test(group.textContent || '')
                );
                if (!dateGroup) return "[]";
                const dateButton = dateGroup.querySelector('button[role="combobox"]');
                if (!dateButton) return "[]";

                if (dateButton.getAttribute('aria-expanded') !== 'true') {
                    dateButton.click();
                }

                const optionButtons = Array.from(dateGroup.querySelectorAll('button[role="option"]'));
                const options = optionButtons
                    .map(btn => (btn.textContent || '').trim())
                    .filter(Boolean);

                if (dateButton.getAttribute('aria-expanded') === 'true') {
                    dateButton.click();
                }

                return JSON.stringify([...new Set(options)]);
            })()`)
        );
        const parsed = JSON.parse(optionsJson);
        if (Array.isArray(parsed) && parsed.length > 0) {
            return parsed;
        }
        await waitMs(page, 300);
    }
    return [];
}

async function setHistoryDateRange(page, dateRange) {
    for (let attempt = 0; attempt < 10; attempt++) {
        const resultJson = /** @type {string} */ (
            await page.evaluate(`(function(targetRange) {
                const host = document.querySelector('.IDS-Banking-Retail-Web-React-TransactionHistoryModule');
                if (!host) return JSON.stringify({ ok: false, reason: 'history module missing' });

                const searchBtn = Array.from(host.querySelectorAll('button')).find(btn =>
                    btn.classList.contains('icon-search') && (btn.textContent || '').trim() === 'Search'
                );
                if (!searchBtn) return JSON.stringify({ ok: false, reason: 'search button missing' });
                if (searchBtn.getAttribute('aria-expanded') !== 'true') {
                    searchBtn.click();
                }

                const section = host.querySelector('.expandable-container-section');
                if (!section) return JSON.stringify({ ok: false, reason: 'search section missing' });

                const dateGroup = Array.from(section.querySelectorAll('.field-group')).find(group =>
                    /Date Range/i.test(group.textContent || '')
                );
                if (!dateGroup) return JSON.stringify({ ok: false, reason: 'date range group missing' });

                const dateButton = dateGroup.querySelector('button[role="combobox"]');
                if (!dateButton) return JSON.stringify({ ok: false, reason: 'date range button missing' });

                if (dateButton.getAttribute('aria-expanded') !== 'true') {
                    dateButton.click();
                }

                const optionButtons = Array.from(dateGroup.querySelectorAll('button[role="option"]'));
                const targetOption = optionButtons.find(btn => (btn.textContent || '').trim() === targetRange);
                if (!targetOption) {
                    if (dateButton.getAttribute('aria-expanded') === 'true') {
                        dateButton.click();
                    }
                    return JSON.stringify({ ok: false, reason: 'target date range missing' });
                }
                targetOption.click();

                const searchSubmit = Array.from(section.querySelectorAll('button')).find(btn =>
                    btn.classList.contains('btn-primary') && (btn.textContent || '').trim() === 'Search'
                );
                if (!searchSubmit) {
                    return JSON.stringify({ ok: false, reason: 'search submit missing' });
                }
                searchSubmit.click();
                return JSON.stringify({ ok: true });
            })(${JSON.stringify(dateRange)})`)
        );
        const result = JSON.parse(resultJson);
        if (result.ok) {
            return true;
        }
        await waitMs(page, 300);
    }
    return false;
}

async function getHistoryCoverage(page) {
    const infoJson = /** @type {string} */ (
        await page.evaluate(`(function() {
            const host = document.querySelector('.IDS-Banking-Retail-Web-React-TransactionHistoryModule');
            if (!host) return JSON.stringify({
                criteria: '',
                rowCount: 0,
                coverageStartDate: null,
                coverageEndDate: null
            });

            const toIso = function(usDate) {
                const m = usDate.match(/^(\\d{2})\\/(\\d{2})\\/(\\d{4})$/);
                if (!m) return null;
                return m[3] + '-' + m[1] + '-' + m[2];
            };

            const rows = Array.from(host.querySelectorAll('tbody tr.item-row'));
            const isoDates = [];
            for (const row of rows) {
                const dateCell = row.cells[1];
                if (!dateCell) continue;
                const matches = (dateCell.textContent || '').match(/\\d{2}\\/\\d{2}\\/\\d{4}/g) || [];
                for (const m of matches) {
                    const iso = toIso(m);
                    if (iso) isoDates.push(iso);
                }
            }
            isoDates.sort();

            return JSON.stringify({
                criteria: (host.querySelector('.search-criteria-text')?.textContent || '').trim(),
                rowCount: rows.length,
                coverageStartDate: isoDates.length > 0 ? isoDates[0] : null,
                coverageEndDate: isoDates.length > 0 ? isoDates[isoDates.length - 1] : null
            });
        })()`)
    );
    return JSON.parse(infoJson);
}

async function downloadHistoryCsv(page) {
    const clickedDownload = await page.evaluate(`(function() {
        const host = document.querySelector('.IDS-Banking-Retail-Web-React-TransactionHistoryModule');
        if (!host) return false;
        const btn = host.querySelector('.module-actions-container button.icon-download');
        if (!btn) return false;
        btn.click();
        return true;
    })()`);
    if (!assertBoolean(clickedDownload)) {
        throw new Error('history download button not found');
    }

    await waitMs(page, 300);
    const downloadPromise = page.waitForDownload(15000);
    const clickedSpreadsheet = await page.evaluate(`(function() {
        const host = document.querySelector('.IDS-Banking-Retail-Web-React-TransactionHistoryModule');
        if (!host) return false;
        const buttons = Array.from(host.querySelectorAll('button'));
        const btn = buttons.find(b => (b.textContent || '').trim() === 'Spreadsheet');
        if (!btn) return false;
        btn.click();
        return true;
    })()`);
    if (!assertBoolean(clickedSpreadsheet)) {
        throw new Error('spreadsheet option not found');
    }
    return downloadPromise;
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
    const existingDocsJson = await refreshmint.listAccountDocuments({ label });
    const existingDocs = JSON.parse(existingDocsJson || '[]');
    const existingFilenames = new Set(existingDocs.map((d) => d.filename));

    const availableDateRanges = await getHistoryDateRangeOptions(page);
    const preferredDateRanges = [
        'Last 30 Days',
        'Last 60 Days',
        'Last 90 Days',
        'Last 120 Days',
        'All',
    ];
    const targetDateRanges = preferredDateRanges.filter((range) =>
        availableDateRanges.includes(range),
    );

    if (targetDateRanges.length === 0) {
        refreshmint.log(
            `  No supported date ranges found for ${accountMatch}; available=${inspect(availableDateRanges)}`,
        );
    }

    const activityStats = {
        attempted: targetDateRanges.length,
        downloaded: 0,
        skippedExisting: 0,
        skippedNoRows: 0,
        setRangeFailed: 0,
        downloadFailed: 0,
    };

    for (const dateRange of targetDateRanges) {
        refreshmint.log(`  Processing activity range: ${dateRange}`);
        const setRangeOk = await setHistoryDateRange(page, dateRange);
        if (!setRangeOk) {
            activityStats.setRangeFailed++;
            refreshmint.log(`  Failed to set range: ${dateRange}`);
            continue;
        }

        await waitForBusy(page);
        await waitMs(page, 1000);

        const coverage = await getHistoryCoverage(page);
        const filename = makeActivityFilename(
            label,
            dateRange,
            coverage.coverageStartDate,
            coverage.coverageEndDate,
        );

        if (coverage.rowCount === 0) {
            activityStats.skippedNoRows++;
            refreshmint.log(
                `  No rows for range ${dateRange}; skipping export.`,
            );
            continue;
        }

        if (
            hasSavedDocument(
                existingFilenames,
                coverage.coverageEndDate,
                filename,
            )
        ) {
            activityStats.skippedExisting++;
            refreshmint.log(
                `  CSV already exists for range ${dateRange}: ${filename}`,
            );
            continue;
        }

        try {
            refreshmint.log(
                `  Downloading CSV for range ${dateRange} (criteria: ${coverage.criteria})...`,
            );
            const dl = await downloadHistoryCsv(page);
            await refreshmint.saveDownloadedResource(dl.path, filename, {
                mimeType: 'text/csv',
                label,
                coverageStartDate: coverage.coverageStartDate,
                coverageEndDate: coverage.coverageEndDate,
            });
            activityStats.downloaded++;
            if (coverage.coverageEndDate != null) {
                existingFilenames.add(
                    `${coverage.coverageEndDate}-${filename}`,
                );
            }
            refreshmint.log(
                `  Downloaded and saved: ${filename} to label ${label}`,
            );
        } catch (e) {
            activityStats.downloadFailed++;
            refreshmint.log(
                `  Failed CSV download for range ${dateRange}: ${String(e)}`,
            );
        }
    }

    refreshmint.log(
        `  Activity summary (${label}): attempted=${activityStats.attempted}, downloaded=${activityStats.downloaded}, existing=${activityStats.skippedExisting}, noRows=${activityStats.skippedNoRows}, setRangeFailed=${activityStats.setRangeFailed}, downloadFailed=${activityStats.downloadFailed}`,
    );

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

    let totalDownloaded = 0;

    for (const section of sections) {
        const sectionAccountLast4Match = String(
            section.accountNumber || '',
        ).match(/(\d{4})$/);
        const sectionAccountLast4 =
            sectionAccountLast4Match == null
                ? null
                : sectionAccountLast4Match[1];
        const label = getLabel(
            `${section.name} x${section.accountNumber.slice(-4)}`,
        );
        refreshmint.log(
            `Processing section: ${section.name} ${section.accountNumber} (label: ${label})`,
        );

        const sectionExistingDocsJson = await refreshmint.listAccountDocuments({
            label,
        });
        const sectionExistingDocs = JSON.parse(sectionExistingDocsJson || '[]');
        const existingFilenames = new Set(
            sectionExistingDocs.map((d) => d.filename),
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

        const sectionStats = {
            totalRows: rows.length,
            downloaded: 0,
            skippedExisting: 0,
            skippedBeforeDate: 0,
            skippedLimit: 0,
            clickFailed: 0,
            downloadFailed: 0,
            invalidRow: 0,
        };
        let loggedAccountMismatch = false;
        let limitReached = false;

        for (const row of rows) {
            let accountNumberForFilename = String(
                row.accountNumber || '',
            ).trim();
            const rowAccountLast4Match =
                accountNumberForFilename.match(/(\d{4})$/);
            const rowAccountLast4 =
                rowAccountLast4Match == null ? null : rowAccountLast4Match[1];
            if (
                sectionAccountLast4 != null &&
                rowAccountLast4 !== sectionAccountLast4
            ) {
                if (!loggedAccountMismatch) {
                    refreshmint.log(
                        `  Row account mismatch in ${label}; expected *${sectionAccountLast4}, saw ${accountNumberForFilename || '(missing)'}. Falling back to section account number for filenames.`,
                    );
                    loggedAccountMismatch = true;
                }
                accountNumberForFilename = section.accountNumber;
            } else if (
                rowAccountLast4 == null &&
                section.accountNumber != null
            ) {
                accountNumberForFilename = section.accountNumber;
            }

            const rowDate = String(row.date || '').trim();
            const dateMatch = rowDate.match(/^(\d{2})\/(\d{2})\/(\d{4})$/);
            if (dateMatch == null) {
                sectionStats.invalidRow++;
                refreshmint.log(
                    `  Invalid statement row date for ${label}; row=${inspect(row)}`,
                );
                continue;
            }

            const filename = makeFilename(
                accountNumberForFilename,
                row.name,
                rowDate,
            );
            const [, mm, dd, yyyy] = dateMatch;
            const coverageEndDate = `${yyyy}-${mm}-${dd}`;
            if (
                hasSavedDocument(existingFilenames, coverageEndDate, filename)
            ) {
                sectionStats.skippedExisting++;
                continue;
            }

            if (
                SKIP_BEFORE_DATE != null &&
                coverageEndDate < SKIP_BEFORE_DATE
            ) {
                sectionStats.skippedBeforeDate++;
                continue;
            }

            if (DOWNLOAD_LIMIT != null && totalDownloaded >= DOWNLOAD_LIMIT) {
                refreshmint.log(
                    `  Download limit (${DOWNLOAD_LIMIT}) reached.`,
                );
                sectionStats.skippedLimit++;
                limitReached = true;
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
                        filename,
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
                    sectionStats.downloaded++;
                    existingFilenames.add(`${coverageEndDate}-${filename}`);
                } catch (e) {
                    sectionStats.downloadFailed++;
                    refreshmint.log(
                        `  Failed to download ${filename}: ${String(e)}`,
                    );
                }
            } else {
                sectionStats.clickFailed++;
                refreshmint.log(
                    `  Failed to click statement link: ${filename}`,
                );
            }
            await humanPace(page, 500, 1000);
        }

        await page.evaluate(`(function(dialogId) {
            const dialog = document.getElementById(dialogId);
            if (dialog) dialog.style.display = 'none';
        })("${section.dialogId}")`);
        await waitMs(page, 500);

        refreshmint.log(
            `  Section summary (${label}): rows=${sectionStats.totalRows}, downloaded=${sectionStats.downloaded}, existing=${sectionStats.skippedExisting}, beforeDate=${sectionStats.skippedBeforeDate}, limit=${sectionStats.skippedLimit}, invalid=${sectionStats.invalidRow}, clickFailed=${sectionStats.clickFailed}, downloadFailed=${sectionStats.downloadFailed}`,
        );

        if (
            sectionStats.totalRows > 0 &&
            sectionStats.downloaded === 0 &&
            sectionStats.skippedExisting === sectionStats.totalRows
        ) {
            refreshmint.log(
                `  Section ${label}: all rows already exist; no new statements downloaded.`,
            );
        }

        if (
            sectionStats.totalRows > 0 &&
            sectionStats.downloaded === 0 &&
            sectionStats.skippedExisting !== sectionStats.totalRows &&
            !limitReached
        ) {
            throw new Error(
                `statement section anomaly (${label}): rows=${sectionStats.totalRows}, downloaded=0, existing=${sectionStats.skippedExisting}, beforeDate=${sectionStats.skippedBeforeDate}, invalid=${sectionStats.invalidRow}, clickFailed=${sectionStats.clickFailed}, downloadFailed=${sectionStats.downloadFailed}`,
            );
        }
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
            await navigateToSignIn(context.mainPage);
            stepReturn = { progressName: 'navigating to login' };
        } else {
            refreshmint.log(`Unexpected URL: ${url}`);
            await navigateToSignIn(context.mainPage);
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

async function run() {
    try {
        await main();
    } catch (e) {
        refreshmint.log(`Fatal error: ${inspect(e)}`);
        try {
            const pages = await browser.pages();
            if (pages.length > 0) {
                const p = pages[0];
                refreshmint.log(`URL at failure: ${await p.url()}`);
                const snapshot = await p.snapshot();
                refreshmint.log(`Snapshot at failure: ${snapshot}`);
            }
        } catch (innerE) {
            refreshmint.log(
                `Failed to capture error snapshot: ${inspect(innerE)}`,
            );
        }
        throw e;
    }
}

run().catch(() => {});
