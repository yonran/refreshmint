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
const STATEMENT_DOWNLOAD_LINK_SELECTOR = 'a.statement-download-link';
const ACTIVITY_MODAL_SELECTOR = '.r_modal_container.download_popup';
const TRANSACTION_HISTORY_URL = `${ORIGIN}/account/transaction-history`;

// Set > 0 during development to only fetch N items per run.
/** @type {number} */
// `--option downloadLimit=N` (debug exec) caps downloads per subflow; 0 = no limit.
const DOWNLOAD_LIMIT =
    Number(refreshmint.getOptions()['downloadLimit'] ?? 0) || 0;

/**
 * @typedef {object} ScrapeContext
 * @property {PageApi} mainPage
 * @property {number} currentStep
 * @property {string[]} progressNames
 * @property {Set<string>} progressNamesSet
 * @property {number} lastProgressStep
 * @property {boolean} statementsDone
 * @property {boolean} activityDone
 * @property {Set<string> | null} existingDocuments stored document names, see `knownDocuments`
 */

/**
 * @typedef {object} MfaRadioChoice
 * @property {string} id
 * @property {string} text the radio's accessible name (`aria-label` when
 *   present, else its label text). Shown to the user as the prompt choice and
 *   used verbatim for `getByRole('radio', { name })`, so the two can't drift.
 */

/**
 * @typedef {object} StatementPeriod
 * @property {string} value ISO date; today's date for the open statement
 * @property {string} label `Current Statement` or `MM-DD-YYYY`
 */

/**
 * @typedef {object} StatementRow
 * @property {string} closeDateText
 * @property {number} linkIndex
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
 * Stored document names for dedupe, loaded once per run and extended as
 * downloads are staged. `listAccountDocuments()` only reports finalized
 * documents, not resources staged earlier in this run, so re-reading it on
 * every step re-downloaded the same file each step (8 copies of one OFX on
 * 2026-09-17) until the progress guard tripped.
 *
 * @param {ScrapeContext} context
 * @returns {Promise<Set<string>>}
 */
async function knownDocuments(context) {
    if (context.existingDocuments == null) {
        context.existingDocuments = await existingDocumentFilenames();
    }
    return context.existingDocuments;
}

/**
 * `saveDownloadedResource(path, original, {coverageEndDate})` finalizes as
 * `{coverageEndDate}-{original}` (see `date_prefixed_filename` in
 * `src-tauri/src/scrape.rs`), and `listAccountDocuments()` reports that
 * stored name. Dedupe must compare against the stored form, not `original`.
 *
 * @param {string} coverageEndDate
 * @param {string} original
 * @returns {string}
 */
function storedDocumentName(coverageEndDate, original) {
    return `${coverageEndDate}-${original}`;
}

/**
 * Poll until `script` (evaluated in the page) returns true, or `timeoutMs`
 * elapses. Returns whether the condition was met.
 *
 * @param {PageApi} page
 * @param {string} script
 * @param {number} timeoutMs
 * @returns {Promise<boolean>}
 */
async function waitForPageCondition(page, script, timeoutMs) {
    const deadline = Date.now() + timeoutMs;
    for (;;) {
        if ((await page.evaluate(script)) === true) {
            return true;
        }
        if (Date.now() >= deadline) {
            return false;
        }
        await waitMs(page, 500);
    }
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

const CODE_INPUT_SELECTOR =
    'input[type="tel"], input[type="text"][name*="code" i], input[type="password"][name*="code" i], input#passcode';

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
                    const visibleText = (text || '').replace(/\\s+/g, ' ').trim();
                    const ariaLabel = (radio.getAttribute('aria-label') || '').replace(/\\s+/g, ' ').trim();
                    return { id: radio.id, text: ariaLabel || visibleText };
                }).filter(function(r) { return r.text !== ''; }));
            })()`,
        )
    );

    if (radios.length > 0) {
        refreshmint.log(
            'target-circle-card mfa branch: method-selection screen',
        );
        const choices = radios.map((r) => r.text);
        const reply = await refreshmint.promptChoice(
            'Select MFA delivery method:',
            choices,
        );
        // refreshmint.promptChoice() blocks until the user responds, which can
        // take real time. If the page re-renders in that window (React et al.
        // commonly regenerate element ids on re-render), the ids captured in
        // `radios` above go stale. getByRole re-resolves live against the
        // page's accessible name (labels included), so it isn't affected by
        // that, and its click is a trusted CDP mouse click that scrolls the
        // element into view first -- unlike a raw evaluate()-driven
        // getElementById().click(), which previously threw on a stale id and,
        // even when the id matched, produced an untrusted synthetic click
        // that some sites' React state doesn't treat as a real selection.
        //
        // Two things about these radios (verified 2026-09-15 in a debug
        // session):
        // - Their accessible name comes from `aria-label` ("Email address
        //   starting with Y@GMAIL.COM"), which differs from the visible
        //   label text ("EmailY*******@GMAIL.COM"); `radios[].text` is the
        //   accessible name so the prompt reply maps back exactly.
        // - The `<input type="radio">` itself is parked offscreen
        //   (`position:absolute; left:-9901px`), so a trusted click on it
        //   fails with "Element is outside of the viewport". The visible,
        //   clickable control is its `<label for=...>`, so re-resolve the
        //   radio's current id by accessible name and click that label.
        const chosenId = /** @type {string} */ (
            await page.evaluate(
                `(function() {
                    const wanted = ${JSON.stringify(reply)};
                    const radio = Array.from(document.querySelectorAll('input[type="radio"]')).find(function(r) {
                        return (r.getAttribute('aria-label') || '').replace(/\\s+/g, ' ').trim() === wanted;
                    });
                    return radio ? radio.id : '';
                })()`,
            )
        );
        if (!chosenId) {
            throw new Error('MFA method radio not found for reply: ' + reply);
        }
        await page.locator('label[for="' + chosenId + '"]').click();
        await humanPace(page, 300, 600);
        await ensureRememberDevice(page);
        await page.getByRole('button', { name: 'Continue' }).first().click();
        // The code-entry screen can take several seconds to replace the
        // method list. With a fixed 1.5s wait the next iteration saw the
        // radios still present and re-selected the method, which sends the
        // user a second passcode (observed 2026-09-15). Poll for the code
        // input instead.
        for (let i = 0; i < 10; i++) {
            await waitMs(page, 1000);
            if (await page.locator(CODE_INPUT_SELECTOR).first().isVisible()) {
                break;
            }
        }
        return { progressName: 'selected mfa method' };
    }

    const codeInputSelector = CODE_INPUT_SELECTOR;
    const codeInputVisible = await page
        .locator(codeInputSelector)
        .first()
        .isVisible();
    if (codeInputVisible) {
        refreshmint.log('target-circle-card mfa branch: code-entry screen');
        const code = await refreshmint.prompt(
            'Enter Target Circle Card MFA code',
        );
        // Same React-controlled-input quirk as the login fields above:
        // fill() sets the value but the framework never sees an input event,
        // so the Continue button stays disabled (verified 2026-09-17 in the
        // retained failed-scrape session). type() sends real key events.
        await page.type(codeInputSelector, code);
        await humanPace(page, 300, 600);
        await ensureRememberDevice(page);
        // The code-entry screen's button is labelled "Continue", not
        // "Submit" (2026-09-17); it is disabled until the input has a value.
        await page.getByRole('button', { name: 'Continue' }).first().click();
        await waitMs(page, 2000);
        return { progressName: 'submitted mfa code' };
    }

    refreshmint.log('target-circle-card mfa branch: transient/loading state');
    return { progressName: 'waiting for mfa screen' };
}

/**
 * Tick the "Do you want to remember this device?" checkbox (`#rememberMe`)
 * when it is present and unchecked. Without it Target asks for a one-time
 * passcode on every login, so unattended auto-scrapes always stalled on the
 * MFA prompt (every run 2026-06-26 .. 2026-09-16). Like the method radios,
 * the input itself is offscreen and its `<label for>` is the clickable
 * control; the checkbox appears on both the method-selection and the
 * code-entry screens, so call this before each Continue/Submit.
 *
 * @param {PageApi} page
 */
async function ensureRememberDevice(page) {
    const state = /** @type {string} */ (
        await page.evaluate(
            `(function() {
                const box = document.getElementById('rememberMe');
                if (!box) return 'absent';
                return box.checked ? 'checked' : 'unchecked';
            })()`,
        )
    );
    if (state !== 'unchecked') {
        refreshmint.log(
            'target-circle-card remember-device checkbox: ' + state,
        );
        return;
    }
    // The checkbox input is hidden behind a styled span, and its <label>
    // wraps it rather than pointing at it with `for` (verified 2026-09-17:
    // `<label class="checkbox_label rememberMe_label"><input id="rememberMe">
    // <span class="checkmark"></span><span>Do you want to remember this
    // device?</span></label>`), so a `label[for="rememberMe"]` locator never
    // matches and times out. Click the wrapping label instead.
    await page.locator('label:has(> #rememberMe)').click();
    const after = /** @type {string} */ (
        await page.evaluate(
            `(function() {
                const box = document.getElementById('rememberMe');
                return box && box.checked ? 'checked' : 'unchecked';
            })()`,
        )
    );
    refreshmint.log(
        'target-circle-card remember-device checkbox: clicked label, now ' +
            after,
    );
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
                document.querySelectorAll('[role="tab"], [role="button"], button, a'),
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
    // Each row is `<tr><td>09-02-2026</td>...<td><a class="statement-download-link"
    // href="#">Download pdf</a></td>...</tr>` (verified 2026-09-17). The
    // links carry no id, so a row is addressed by its index among
    // `STATEMENT_DOWNLOAD_LINK_SELECTOR` matches in document order, which is
    // what `handleStatements` clicks via `locator(...).nth(index)`.
    const rows = /** @type {StatementRow[]} */ (
        await evaluateJsonArray(
            page,
            `(function() {
                const dateRe = /^\\d{2}-\\d{2}-\\d{4}$/;
                const links = Array.from(
                    document.querySelectorAll(${JSON.stringify(STATEMENT_DOWNLOAD_LINK_SELECTOR)}),
                );
                const rows = [];
                links.forEach(function (link, index) {
                    const tr = link.closest('tr');
                    if (!tr) return;
                    const dateCell = Array.from(tr.querySelectorAll('td')).find(function (td) {
                        return dateRe.test((td.textContent || '').trim());
                    });
                    if (!dateCell) return;
                    rows.push({
                        closeDateText: (dateCell.textContent || '').trim(),
                        linkIndex: index,
                    });
                });
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
 * - Year tabs are `<div id="2026" role="button" class="years">` (verified
 *   2026-09-17; they are not `<button>`/`<a>`), rendered a few seconds after
 *   navigation along with the statements table.
 * - Each statement row shows a close-date and a `Download pdf` control.
 *
 * @param {ScrapeContext} context
 * @returns {Promise<{progressName: string}>}
 */
async function handleStatements(context) {
    const page = context.mainPage;
    refreshmint.log('State: Statements');
    await logStateSnapshot(page, 'target-circle-card statements snapshot');

    // The year tabs and table render asynchronously after navigation; without
    // this wait discovery ran against an empty shell and reported no tabs
    // and no rows (every run through 2026-09-17).
    const tableReady = await waitForPageCondition(
        page,
        `document.querySelectorAll(${JSON.stringify(STATEMENT_DOWNLOAD_LINK_SELECTOR)}).length > 0`,
        15000,
    );
    if (!tableReady) {
        refreshmint.log(
            'target-circle-card statements: table did not render within 15s',
        );
    }

    const existing = await knownDocuments(context);
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
            // `#2026` is not a valid CSS selector (ids can't start with a
            // digit unescaped), so the old `#${yearId}` locator never matched.
            await page.locator(`[id="${yearId}"]`).click();
            await waitMs(page, 1200);
            await page.waitForLoadState('networkidle', undefined);
        }

        const rows = await discoverStatementRows(page);
        refreshmint.log(
            `target-circle-card statements: ${rows.length} row(s) in ${yearId ?? 'default'} tab`,
        );
        for (const row of rows) {
            if (DOWNLOAD_LIMIT > 0 && downloaded >= DOWNLOAD_LIMIT) {
                break;
            }
            const closeDate = statementCloseDateToIso(row.closeDateText);
            if (closeDate == null) {
                continue;
            }
            // Matches the pre-existing on-disk naming
            // (`2026-03-03-statement-2026-03-03.pdf`).
            const original = `statement-${closeDate}.pdf`;
            if (existing.has(storedDocumentName(closeDate, original))) {
                continue;
            }

            refreshmint.log(
                `target-circle-card statements: downloading ${original}`,
            );
            const downloadPromise = page.waitForDownload(30000);
            await page
                .locator(STATEMENT_DOWNLOAD_LINK_SELECTOR)
                .nth(row.linkIndex)
                .click();
            const download = await downloadPromise;
            await refreshmint.saveDownloadedResource(download.path, original, {
                coverageEndDate: closeDate,
                mimeType: 'application/pdf',
            });
            existing.add(storedDocumentName(closeDate, original));
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
 * @returns {Promise<StatementPeriod[]>}
 */
async function discoverStatementPeriods(page) {
    // `select#security_q` (name=statementDates) options are
    // `<option value="">Select Statement</option>`,
    // `<option value="2026-09-17">Current Statement</option>` (value is
    // today's date and changes daily), then one ISO-valued option per closed
    // statement labelled `MM-DD-YYYY` (verified 2026-09-17).
    const periods = /** @type {StatementPeriod[]} */ (
        await evaluateJsonArray(
            page,
            `(function() {
                const select = document.querySelector('select#security_q');
                if (!select) return JSON.stringify([]);
                return JSON.stringify(
                    Array.from(select.options)
                        .filter(function (o) { return o.value !== ''; })
                        .map(function (o) {
                            return { value: o.value, label: (o.textContent || '').trim() };
                        }),
                );
            })()`,
        )
    );
    return periods;
}

/**
 * Original (pre-date-prefix) filename for one activity export, matching the
 * pre-existing on-disk naming (`2026-03-03-transactions-2026-03-03.csv`,
 * `2026-03-26-transactions-current-statement.csv`).
 *
 * @param {StatementPeriod} period
 * @param {string} ext
 * @returns {string}
 */
function activityExportOriginalName(period, ext) {
    const slug = /current statement/i.test(period.label)
        ? 'current-statement'
        : period.value;
    return `transactions-${slug}.${ext}`;
}

/**
 * Downloads CSV and OFX activity exports for one already-selected statement
 * period via the "Download transactions" modal. QBO/QFX are intentionally
 * skipped as redundant with OFX (see README.md "Activity Export Comparison").
 *
 * @param {PageApi} page
 * @param {StatementPeriod} period
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
        const original = activityExportOriginalName(period, format.ext);
        const stored = storedDocumentName(period.value, original);
        if (existing.has(stored)) {
            continue;
        }

        refreshmint.log(
            `target-circle-card activity: opening download modal for ${period.label} (${format.value})`,
        );
        // Modal markup (verified 2026-09-17): the trigger is
        // `<a role="button" class="download_btn popup_click">Download
        // transactions</a>`; it shows `.r_modal_container.download_popup`
        // (display:block) with a `.modal_bg` overlay, `select#user`, and a
        // plain `<a download="Transactions.CSV" href="blob:..."
        // class="downbtn">Download</a>` whose `download` attribute follows the
        // selected format. Clicking the trigger while the modal is already up
        // fails with "div intercepts pointer events", so only click it when
        // the modal is hidden.
        const modalOpenScript = `(function() {
            const modal = document.querySelector(${JSON.stringify(ACTIVITY_MODAL_SELECTOR)});
            return !!modal && getComputedStyle(modal).display !== 'none';
        })()`;
        if ((await page.evaluate(modalOpenScript)) !== true) {
            await page.locator('a.download_btn.popup_click').first().click();
            if (!(await waitForPageCondition(page, modalOpenScript, 10000))) {
                throw new Error(
                    'Target Circle Card download-transactions modal did not open',
                );
            }
        }
        await setSelectValue(page, 'select#user', format.value);
        // For CSV the link is `<a download="Transactions.CSV" href="blob:...">`
        // and the attribute lags the select change; for OFX (and QBO/QFX)
        // it is `<a href="#" role="button">` with no download attribute and
        // the file is produced on click. Wait for whichever shape applies.
        const linkReady = await waitForPageCondition(
            page,
            `(function() {
                const link = document.querySelector(${JSON.stringify(ACTIVITY_MODAL_SELECTOR + ' a.downbtn')});
                if (!link) return false;
                const name = link.getAttribute('download');
                return name == null || /\\.${format.ext}$/i.test(name);
            })()`,
            10000,
        );
        if (!linkReady) {
            throw new Error(
                `Target Circle Card download link did not switch to ${format.value}`,
            );
        }
        await humanPace(page, 300, 600);

        const downloadPromise = page.waitForDownload(30000);
        await page
            .locator(ACTIVITY_MODAL_SELECTOR + ' a.downbtn')
            .first()
            .click();
        const download = await downloadPromise;
        await refreshmint.saveDownloadedResource(download.path, original, {
            coverageEndDate: period.value,
            mimeType: format.ext === 'csv' ? 'text/csv' : 'application/x-ofx',
            // Extra scalar options land in the sidecar's `metadata` map.
            period: period.label,
        });
        existing.add(stored);
        downloaded = true;
        // The modal closes itself after each successful download; if it
        // lingers, dismiss it so the next trigger click isn't intercepted.
        await humanPace(page, 500, 900);
        if ((await page.evaluate(modalOpenScript)) === true) {
            await page
                .locator(ACTIVITY_MODAL_SELECTOR + ' a.modal-close')
                .first()
                .click();
            await waitMs(page, 500);
        }
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

    // Like the statements table, the period select is populated a few
    // seconds after navigation; discovery without this wait found nothing.
    // The placeholder and "Current Statement" options appear first and the
    // closed periods are appended a moment later, so wait for at least one
    // closed period (3+ options) rather than for the select to merely exist.
    const selectReady = await waitForPageCondition(
        page,
        `(document.querySelector('select#security_q') || { options: [] }).options.length > 2`,
        15000,
    );
    if (!selectReady) {
        refreshmint.log(
            'target-circle-card activity: period select did not populate within 15s',
        );
    }

    const existing = await knownDocuments(context);
    const periods = await discoverStatementPeriods(page);
    refreshmint.log(
        `target-circle-card activity: found periods ${JSON.stringify(periods.map((p) => p.value))}`,
    );

    let downloaded = 0;
    for (const period of periods) {
        if (DOWNLOAD_LIMIT > 0 && downloaded >= DOWNLOAD_LIMIT) {
            break;
        }
        const haveAll = ['csv', 'ofx'].every((ext) =>
            existing.has(
                storedDocumentName(
                    period.value,
                    activityExportOriginalName(period, ext),
                ),
            ),
        );
        if (haveAll) {
            continue;
        }

        refreshmint.log(
            `target-circle-card activity: selecting period ${period.label}`,
        );
        await setSelectValue(page, 'select#security_q', period.value);
        await waitMs(page, 1200);
        await page.waitForLoadState('networkidle', undefined);

        const gotAny = await downloadActivityExports(page, period, existing);
        if (gotAny) {
            downloaded++;
            return {
                progressName: `downloaded activity exports for ${period.label}`,
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
        existingDocuments: null,
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
