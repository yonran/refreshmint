/**
 * target-circle-card scraper for Refreshmint.
 *
 * Scrapes statements and transaction-history activity exports from
 * https://mytargetcirclecard.target.com/ (Target's RedCard / Circle Card
 * credit card portal, powered by TD Bank).
 *
 * Verified selectors and page notes live in README.md next to this file;
 * keep that file in sync with any selector changes made here.
 */

const ORIGIN = 'https://mytargetcirclecard.target.com';
const LOGIN_URL = `${ORIGIN}/`;
const AUTH_URL_PREFIX = `${ORIGIN}/ecs/auth/`;
const MFA_URL_PREFIX = `${ORIGIN}/ecs/auth/multi-factor-auth`;
const HOME_URL = `${ORIGIN}/home`;
const STATEMENTS_URL = `${ORIGIN}/statements`;
const TRANSACTION_HISTORY_URL = `${ORIGIN}/account/transaction-history`;

// Set > 0 during development to only fetch N items per run.
/** @type {number} */
const DOWNLOAD_LIMIT = 0;

/**
 * @typedef {object} ScrapeContext
 * @property {PageApi} mainPage
 * @property {number} currentStep
 * @property {string[]} progressNames
 * @property {Set<string>} progressNamesSet
 * @property {number} lastProgressStep
 * @property {boolean} statementsDone
 * @property {boolean} activityDone
 */

/**
 * @typedef {object} MfaRadioChoice
 * @property {string} id
 * @property {string} text
 */

/**
 * @typedef {object} StatementRow
 * @property {string} closeDateText
 * @property {string | null} linkId
 */

/**
 * @param {PageApi} page
 * @param {number} ms
 */
async function waitMs(page, ms) {
    await page.evaluate(`new Promise(r => setTimeout(r, ${ms}))`);
}

/**
 * @param {PageApi} page
 * @param {number} minMs
 * @param {number} maxMs
 */
async function humanPace(page, minMs, maxMs) {
    const delta = maxMs - minMs;
    const ms = minMs + Math.floor(Math.random() * (delta + 1));
    await waitMs(page, ms);
}

/**
 * @param {PageApi} page
 * @param {string} label
 */
async function logStateSnapshot(page, label) {
    const snapshot = await page.snapshot({
        incremental: true,
        track: 'state-loop',
    });
    refreshmint.log(`${label}: ${snapshot}`);
}

/**
 * Runs `script` in the page and JSON-parses its (string) return value into an
 * array. Returns `[]` if the page returned nothing usable.
 *
 * @param {PageApi} page
 * @param {string} script
 * @returns {Promise<unknown[]>}
 */
async function evaluateJsonArray(page, script) {
    const raw = await page.evaluate(script);
    if (typeof raw !== 'string' || raw.trim() === '') {
        return [];
    }
    /** @type {unknown} */
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? /** @type {unknown[]} */ (parsed) : [];
}

/**
 * @param {PageApi} page
 * @param {string} selector
 * @param {string} value
 */
async function setSelectValue(page, selector, value) {
    await page.evaluate(`(function() {
        const el = document.querySelector(${JSON.stringify(selector)});
        if (!el) throw new Error(${JSON.stringify(`select not found: ${selector}`)});
        el.value = ${JSON.stringify(value)};
        el.dispatchEvent(new Event('input', { bubbles: true }));
        el.dispatchEvent(new Event('change', { bubbles: true }));
    })()`);
}

/**
 * @returns {Promise<Set<string>>}
 */
async function existingDocumentFilenames() {
    const docsJson = await refreshmint.listAccountDocuments();
    /** @type {unknown} */
    const parsed = JSON.parse(docsJson === '' ? '[]' : docsJson);
    const docs = Array.isArray(parsed) ? /** @type {unknown[]} */ (parsed) : [];
    /** @type {Set<string>} */
    const filenames = new Set();
    for (const item of docs) {
        if (item == null || typeof item !== 'object') {
            continue;
        }
        const doc = /** @type {{filename?: unknown}} */ (item);
        if (typeof doc.filename === 'string') {
            filenames.add(doc.filename);
        }
    }
    return filenames;
}

/**
 * Expected page conditions:
 * - URL is under `/ecs/auth/` (not the MFA sub-path).
 * - Username and password fields are both present on one page (unlike some
 *   sites this portal does not split login into separate email/password steps).
 *
 * @param {ScrapeContext} context
 * @returns {Promise<{progressName: string}>}
 */
async function handleLogin(context) {
    const page = context.mainPage;
    refreshmint.log('State: Login Page');

    const usernameVisible = await page.locator('input#username').isVisible();
    if (!usernameVisible) {
        await logStateSnapshot(page, 'target-circle-card login snapshot');
        refreshmint.log(
            'target-circle-card login branch: waiting for login fields',
        );
        return { progressName: 'waiting for login fields' };
    }

    const currentUsername = await page.inputValue('input#username');
    const currentPassword = await page.inputValue('input#password');
    if (currentUsername.trim() === '' || currentPassword === '') {
        refreshmint.log('target-circle-card login branch: filling credentials');
        // page.type() fires CDP key events that React/framework event handlers
        // pick up; secret substitution resolves these literal values from the
        // keychain per manifest.json `secrets.mytargetcirclecard.target.com`.
        if (currentUsername.trim() === '') {
            await page.type('input#username', 'target_circle_card_username');
            await humanPace(page, 300, 700);
        }
        if (currentPassword === '') {
            // UNTESTED after the 2026-09-05 failure artifact showed fill()
            // leaving this React-controlled field empty.
            await page.type('input#password', 'target_circle_card_password');
        }
        await humanPace(page, 400, 900);
        if ((await page.inputValue('input#password')) === '') {
            throw new Error('Target Circle Card password field remained empty');
        }
        await page.locator('button#login').click();
        try {
            await waitMs(page, 4000);
        } catch (_e) {
            // page navigated away — login submit succeeded
        }
        return { progressName: 'submitted login credentials' };
    }

    refreshmint.log('target-circle-card login branch: waiting after submit');
    return { progressName: 'waiting after login submit' };
}

/**
 * Expected page conditions:
 * - URL is under `/ecs/auth/multi-factor-auth`.
 * - Either a method-selection screen (visible radio choices), a code-entry
 *   screen (text/tel/passcode input), or a transient loading state with
 *   neither.
 *
 * @param {ScrapeContext} context
 * @returns {Promise<{progressName: string}>}
 */
async function handleMfa(context) {
    const page = context.mainPage;
    refreshmint.log('State: MFA');
    await logStateSnapshot(page, 'target-circle-card mfa snapshot');

    /** @type {MfaRadioChoice[]} */
    const radios = /** @type {MfaRadioChoice[]} */ (
        await evaluateJsonArray(
            page,
            `(function() {
                const radios = Array.from(document.querySelectorAll('input[type="radio"]'));
                return JSON.stringify(radios.map(function(radio) {
                    const label = radio.closest('label');
                    const text = label
                        ? label.textContent
                        : (document.querySelector('label[for="' + radio.id + '"]') || {}).textContent;
                    return { id: radio.id, text: (text || '').replace(/\\s+/g, ' ').trim() };
                }).filter(function(r) { return r.text !== ''; }));
            })()`,
        )
    );

    if (radios.length > 0) {
        refreshmint.log(
            'target-circle-card mfa branch: method-selection screen',
        );
        // refreshmint.promptChoice() is documented in docs/scraper.md but is not
        // yet implemented in the Rust runtime (only refreshmint.prompt() exists
        // as of this writing), so list the visible choices in a free-text prompt
        // instead and match the reply back to the option whose text starts with it.
        const choices = radios.map((r) => r.text);
        const reply = await refreshmint.prompt(
            `Select MFA delivery method (type the exact text): ${choices.join(' | ')}`,
        );
        const chosen =
            radios.find((r) => r.text === reply.trim()) ??
            radios.find((r) => r.text.startsWith(reply.trim())) ??
            radios[0];
        await page.locator(`#${chosen.id}`).click();
        await humanPace(page, 300, 600);
        await page.getByRole('button', { name: 'Continue' }).first().click();
        await waitMs(page, 1500);
        return { progressName: 'selected mfa method' };
    }

    const codeInputSelector =
        'input[type="tel"], input[type="text"][name*="code" i], input[type="password"][name*="code" i], input#passcode';
    const codeInputVisible = await page
        .locator(codeInputSelector)
        .first()
        .isVisible();
    if (codeInputVisible) {
        refreshmint.log('target-circle-card mfa branch: code-entry screen');
        const code = await refreshmint.prompt(
            'Enter Target Circle Card MFA code',
        );
        await page.locator(codeInputSelector).first().fill(code);
        await humanPace(page, 300, 600);
        await page.getByRole('button', { name: 'Submit' }).first().click();
        await waitMs(page, 2000);
        return { progressName: 'submitted mfa code' };
    }

    refreshmint.log('target-circle-card mfa branch: transient/loading state');
    return { progressName: 'waiting for mfa screen' };
}

/**
 * Expected page conditions:
 * - User is authenticated; URL is `/home`.
 * - A blocking financial-info modal may be present and must be closed first.
 *
 * @param {ScrapeContext} context
 * @returns {Promise<{progressName: string, done?: boolean}>}
 */
async function handleHome(context) {
    const page = context.mainPage;
    refreshmint.log('State: Authenticated Home');

    const modalCloseVisible = await page
        .locator('button#close-btn-modal')
        .isVisible();
    if (modalCloseVisible) {
        refreshmint.log('target-circle-card home branch: closing info modal');
        await page.locator('button#close-btn-modal').click();
        await waitMs(page, 800);
        return { progressName: 'closed home modal' };
    }

    if (!context.statementsDone) {
        refreshmint.log(
            'target-circle-card home branch: navigating to statements',
        );
        await page.goto(STATEMENTS_URL, { waitUntil: 'load', timeout: 30000 });
        return { progressName: 'navigate to statements' };
    }

    if (!context.activityDone) {
        refreshmint.log(
            'target-circle-card home branch: navigating to transaction history',
        );
        await page.goto(TRANSACTION_HISTORY_URL, {
            waitUntil: 'load',
            timeout: 30000,
        });
        return { progressName: 'navigate to transaction history' };
    }

    refreshmint.log('target-circle-card home branch: all subflows complete');
    return { progressName: 'home complete', done: true };
}

/**
 * @param {PageApi} page
 * @returns {Promise<string[]>}
 */
async function discoverStatementYearIds(page) {
    const ids = await evaluateJsonArray(
        page,
        `(function() {
            const ids = [];
            const candidates = Array.from(
                document.querySelectorAll('[role="tab"], button, a'),
            );
            for (const el of candidates) {
                if (/^(19|20)\\d{2}$/.test(el.id || '')) {
                    ids.push(el.id);
                }
            }
            return JSON.stringify(Array.from(new Set(ids)));
        })()`,
    );
    return ids.filter((id) => typeof id === 'string');
}

/**
 * @param {PageApi} page
 * @returns {Promise<StatementRow[]>}
 */
async function discoverStatementRows(page) {
    const rows = /** @type {StatementRow[]} */ (
        await evaluateJsonArray(
            page,
            `(function() {
                const dateRe = /^\\d{2}-\\d{2}-\\d{4}$/;
                const seen = new Set();
                const rows = [];
                for (const el of Array.from(document.querySelectorAll('body *'))) {
                    const text = (el.textContent || '').trim();
                    if (!dateRe.test(text) || el.children.length > 0) {
                        continue;
                    }
                    const container =
                        el.closest('tr') || el.closest('li') || el.parentElement;
                    if (!container) {
                        continue;
                    }
                    const link = Array.from(
                        container.querySelectorAll('a, button'),
                    ).find(function (a) {
                        return /download pdf/i.test((a.textContent || '').trim());
                    });
                    if (!link) {
                        continue;
                    }
                    const key = text + '|' + (link.id || link.getAttribute('href') || '');
                    if (seen.has(key)) {
                        continue;
                    }
                    seen.add(key);
                    rows.push({ closeDateText: text, linkId: link.id || null });
                }
                return JSON.stringify(rows);
            })()`,
        )
    );
    return rows;
}

/**
 * @param {string} text
 * @returns {string | null}
 */
function statementCloseDateToIso(text) {
    const match = text.match(/^(\d{2})-(\d{2})-(\d{4})$/);
    if (!match) {
        return null;
    }
    return `${match[3]}-${match[1]}-${match[2]}`;
}

/**
 * Expected page conditions:
 * - URL is `/statements`.
 * - Year tabs are exposed as DOM ids like `2026`, `2025`, `2024`.
 * - Each statement row shows a close-date and a `Download pdf` control.
 *
 * @param {ScrapeContext} context
 * @returns {Promise<{progressName: string}>}
 */
async function handleStatements(context) {
    const page = context.mainPage;
    refreshmint.log('State: Statements');
    await logStateSnapshot(page, 'target-circle-card statements snapshot');

    const existing = await existingDocumentFilenames();
    const yearIds = await discoverStatementYearIds(page);
    refreshmint.log(
        `target-circle-card statements: found year tabs ${JSON.stringify(yearIds)}`,
    );

    let downloaded = 0;
    let progressed = false;
    /** @type {(string | null)[]} */
    const yearsToVisit = yearIds.length > 0 ? yearIds : [null];

    for (const yearId of yearsToVisit) {
        if (DOWNLOAD_LIMIT > 0 && downloaded >= DOWNLOAD_LIMIT) {
            break;
        }
        if (yearId != null) {
            refreshmint.log(
                `target-circle-card statements: opening year tab ${yearId}`,
            );
            await page.locator(`#${yearId}`).click();
            await waitMs(page, 1200);
            await page.waitForLoadState('networkidle', undefined);
        }

        const rows = await discoverStatementRows(page);
        for (const row of rows) {
            if (DOWNLOAD_LIMIT > 0 && downloaded >= DOWNLOAD_LIMIT) {
                break;
            }
            const closeDate = statementCloseDateToIso(row.closeDateText);
            const filename = `statements/statement-${closeDate ?? row.closeDateText}.pdf`;
            if (existing.has(filename) || row.linkId == null) {
                continue;
            }

            refreshmint.log(
                `target-circle-card statements: downloading ${filename}`,
            );
            const downloadPromise = page.waitForDownload(30000);
            await page.locator(`#${row.linkId}`).click();
            const download = await downloadPromise;
            await refreshmint.saveDownloadedResource(download.path, filename, {
                coverageEndDate: closeDate ?? undefined,
                mimeType: 'application/pdf',
            });
            existing.add(filename);
            downloaded++;
            progressed = true;
            await humanPace(page, 500, 900);
        }
    }

    if (progressed) {
        return { progressName: `downloaded ${downloaded} statement(s)` };
    }

    context.statementsDone = true;
    refreshmint.log(
        'target-circle-card statements: no new statements to download',
    );
    await page.goto(HOME_URL, { waitUntil: 'load', timeout: 30000 });
    return { progressName: 'statements complete' };
}

/**
 * @param {PageApi} page
 * @returns {Promise<string[]>}
 */
async function discoverStatementPeriodValues(page) {
    const values = await evaluateJsonArray(
        page,
        `(function() {
            const select = document.querySelector('select#security_q');
            if (!select) return JSON.stringify([]);
            return JSON.stringify(
                Array.from(select.options).map(function (o) {
                    return o.value;
                }),
            );
        })()`,
    );
    return values.filter((value) => typeof value === 'string');
}

/**
 * Downloads CSV and OFX activity exports for one already-selected statement
 * period via the "Download transactions" modal. QBO/QFX are intentionally
 * skipped as redundant with OFX (see README.md "Activity Export Comparison").
 *
 * @param {PageApi} page
 * @param {string} period
 * @param {Set<string>} existing
 * @returns {Promise<boolean>} whether any file was downloaded
 */
async function downloadActivityExports(page, period, existing) {
    const formats = [
        { value: 'CSV', ext: 'csv' },
        { value: 'OFX', ext: 'ofx' },
    ];
    let downloaded = false;

    for (const format of formats) {
        const filename = `activity/${period}.${format.ext}`;
        if (existing.has(filename)) {
            continue;
        }

        refreshmint.log(
            `target-circle-card activity: opening download modal for ${period} (${format.value})`,
        );
        await page
            .getByRole('button', { name: 'Download transactions' })
            .first()
            .click();
        await waitMs(page, 800);
        await setSelectValue(page, 'select#user', format.value);
        await humanPace(page, 300, 600);

        const downloadPromise = page.waitForDownload(30000);
        await page.getByRole('button', { name: 'Download' }).first().click();
        const download = await downloadPromise;
        await refreshmint.saveDownloadedResource(download.path, filename, {
            mimeType: format.ext === 'csv' ? 'text/csv' : 'application/x-ofx',
        });
        existing.add(filename);
        downloaded = true;
        // The modal closes itself after each successful download.
        await humanPace(page, 500, 900);
    }

    return downloaded;
}

/**
 * Expected page conditions:
 * - URL is `/account/transaction-history`.
 * - `select#security_q` lists statement periods; changing it swaps the
 *   visible transaction table.
 *
 * @param {ScrapeContext} context
 * @returns {Promise<{progressName: string}>}
 */
async function handleTransactionHistory(context) {
    const page = context.mainPage;
    refreshmint.log('State: Transaction History');
    await logStateSnapshot(
        page,
        'target-circle-card transaction history snapshot',
    );

    const existing = await existingDocumentFilenames();
    const periods = await discoverStatementPeriodValues(page);
    refreshmint.log(
        `target-circle-card activity: found periods ${JSON.stringify(periods)}`,
    );

    let downloaded = 0;
    for (const period of periods) {
        if (DOWNLOAD_LIMIT > 0 && downloaded >= DOWNLOAD_LIMIT) {
            break;
        }
        const csvFilename = `activity/${period}.csv`;
        const ofxFilename = `activity/${period}.ofx`;
        if (existing.has(csvFilename) && existing.has(ofxFilename)) {
            continue;
        }

        refreshmint.log(
            `target-circle-card activity: selecting period ${period}`,
        );
        await setSelectValue(page, 'select#security_q', period);
        await waitMs(page, 1200);
        await page.waitForLoadState('networkidle', undefined);

        const gotAny = await downloadActivityExports(page, period, existing);
        if (gotAny) {
            downloaded++;
            return {
                progressName: `downloaded activity exports for ${period}`,
            };
        }
    }

    if (downloaded === 0) {
        context.activityDone = true;
        refreshmint.log(
            'target-circle-card activity: no new activity exports to download',
        );
        await page.goto(HOME_URL, { waitUntil: 'load', timeout: 30000 });
        return { progressName: 'activity complete' };
    }

    return {
        progressName: `downloaded ${downloaded} activity export period(s)`,
    };
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
        statementsDone: false,
        activityDone: false,
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
        } else if (url.startsWith(MFA_URL_PREFIX)) {
            stepReturn = await handleMfa(context);
        } else if (url.startsWith(AUTH_URL_PREFIX)) {
            stepReturn = await handleLogin(context);
        } else if (url.startsWith(HOME_URL)) {
            stepReturn = await handleHome(context);
        } else if (url.startsWith(STATEMENTS_URL)) {
            stepReturn = await handleStatements(context);
        } else if (url.startsWith(TRANSACTION_HISTORY_URL)) {
            stepReturn = await handleTransactionHistory(context);
        } else {
            refreshmint.log(
                `Unexpected URL: ${url}; navigating to ${HOME_URL}`,
            );
            await context.mainPage.goto(HOME_URL, {
                waitUntil: 'load',
                timeout: 30000,
            });
            stepReturn = {
                progressName: 'navigate to home from unexpected url',
            };
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

// Fail loud: await main() so any thrown error rejects the top-level promise and
// the scrape is recorded as failed. A top-level `.catch` that only logs resolves
// the promise, making every scrape report success even when nothing was captured.
await main();
