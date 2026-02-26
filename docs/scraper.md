# Scraper Authoring

This document covers running scrapers, scraper runtime behavior, and scraper JS APIs.

For extension structure and manifest details, see `docs/extension.md`.

## Run a scraper

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin app -- \
  scrape \
  --ledger /path/to/ledger.refreshmint \
  --account Assets:Checking
```

The extension is read from the login config. The command fails if none is configured.

If your script uses `refreshmint.prompt(message)`, supply overrides:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin app -- \
  scrape \
  --ledger /path/to/ledger.refreshmint \
  --account Assets:Checking \
  --prompt "OTP=123456" \
  --prompt "Security answer=blue"
```

CLI runs fail with an explicit error when a required prompt override is missing.

## Runtime model

- `driver.mjs` runs in a QuickJS sandbox
- top-level `await` is supported
- globals available:
    - `page` for browser automation
    - `refreshmint` for logging/resources/prompt helpers

Errors thrown from your script fail the scrape run.

## Best practices

- Whenever you scrape a new page, try doing it once, with plenty of incremental snapshot observations, before you are confident that you can script it in the general case.
- Scrapers will fail often. Make them fast to debug and retry
    - The script should have a state machine that keeps working (for loop) until an error or the end state is achieved
    - Scrapers should define a function for each state (e.g. URL or page) and then the main code should switch on the URL or other identifying information on the page. This allows us to edit the script and re-run it without starting from the original logged out state.
    - The state machine should also keep track of progress being made (e.g. a set of distinct pages viewed or output identifiers such as last month downloaded) and iterations since last progress. If there are too many steps with no real progress, then throw an error.
    - Log what state you are in.
    - Add checks to make sure you are on the page that you expect to be.
    - When there is an exception or something unexpected, log context before re-throwing so that we can see what went wrong quickly.
    - Before doing something slow (sleeping, or navigating), log what you are about to do.
- **Be careful and observant during development to avoid state machine thrashing.** Don't blindly add catch-alls or overly broad fallbacks when something fails. If a selector isn't found, it's better to log a snapshot and explicitly throw/exit early during development so you can inspect the exact page state, rather than letting the script guess and click the wrong things in an infinite loop.
- Domain checks must compare the URL prefix to the full origin (for example `https://secure.bankofamerica.com/`), not a substring match.
- Prefer `const url = await page.url(); if (!url.startsWith("https://example.com/")) { ... }` over checks like `url.includes("example.com")`.
- When adding a new code path/branch, add a brief `UNTESTED` comment until that exact branch is exercised in a real run; remove or update the comment after verification.
- For any code path, uniquely log the branch that we take so that you will see whether it is actually being executed. If it is an inner loop, remove the logging after you have tested it if the logging is too verbose.
- At each page handler/state, add a short comment describing expected page conditions and enforce them with explicit assertions (URL/selector checks) before taking actions.
- Add comments for non-obvious actions describing what outcome each action is trying to accomplish (not just what selector is clicked).
- Pace automation actions (especially login, navigation, and repeated downloads) with short delays so behavior is less bot-like and less likely to trigger anti-automation defenses.
- Add a reusable snapshot logger for state loops. Prefer `await page.snapshot({ incremental: true, track: 'state-loop' })` so logs show only page changes from the previous checkpoint.
- **Fail Fast During Development:** Do not artificially inflate "no progress" timeouts or add broad `try/catch` blocks just to bypass errors. If the scraper reaches an unexpected state or a selector fails, it should crash immediately so you can inspect the exact failure point.
- **State Identification Robustness:**
    - **Use Headers, Not Just URLs:** If the URL is ambiguous (e.g. SPA, multiple forms that use the same URL), then check other indicators such as title, or heading to determine what page you are on.
    - **Avoid Broad ORs:** Do not chain loose `url.includes()` or `text.includes()` conditions with `||` to patch broken state checks, as this drastically increases false positives. Use single, highly specific strings derived from actual page snapshots.
- **Beware Overlapping Branches:** Beware if multiple branches in the main loop match a page. Where possible, make the `if` branches uniquely identify the page so that you can start the script at any point and it will reliably jump into the correct state handler.
- **Prompting:** When it is time to prompt for user input (e.g., an MFA code), actually prompt the CLI user. Do not hardcode or submit dummy data (like `123456`) to live production systems just to test filling logic, as this can trigger fraud alerts or rate limits.
- **Prioritize Trusted Interactions:** You SHOULD generally use native Playwright-style APIs (`Locator.click()`, `Locator.fill()`, `ElementHandle.click()`) for interacting with elements, as these perform OS-level trusted actions. Do not default to synthetic JavaScript events (e.g., `el.click()` inside `page.evaluate()`), as security-conscious sites routinely ignore them.
- **Provide all arguments to JS APIs:** The QuickJS runtime requires every non-optional argument to be explicitly passed. If a method like `waitForLoadState(state, timeoutMs)` is called, even if you want the default timeout, pass `undefined`: `await page.waitForLoadState('networkidle', undefined)`.
- **Use standard CSS selectors:** Underlying engine does not support Playwright-specific selectors like `:has-text()`. Use standard CSS or `page.evaluate()` to find elements by text.
- **Handle "Busy" states:** Banking sites often use global loading overlays (e.g. `div#busy-div`). Implement a `waitForBusy` helper to ensure the page is interactive before clicking.
- **Prefer `evaluate` for tricky clicks:** If `page.click()` fails due to visibility or pointer-event interception, use `page.evaluate('document.querySelector(selector).click()')`.
- **Robust account discovery:** Search for account patterns (e.g., `x\d{4}`) across all relevant tags (`button`, `a`, `span`) to build a pending account list.

## State management and optimization

Avoid downloading everything every time by tracking progress and checking existing files.

- **Deduplicate downloads:** Use `await refreshmint.listAccountDocuments()` to get a list of already saved files. Compare filenames before initiating a download.
- **Filter by date:** Implement a "since" date check (e.g., `skip before 2026-01-01`).
- **Debugging limits:** Use a `DOWNLOAD_LIMIT` variable during development to only fetch 1-2 items per run.

Example of optimized loop:

```js
const DOWNLOAD_LIMIT = 2; // Set to 0 for no limit
const SKIP_BEFORE_DATE = '2026-01-01';

async function handleStatements(context) {
    // ... discovery ...
    const existing = new Set(
        JSON.parse(await refreshmint.listAccountDocuments()).map(
            (d) => d.filename,
        ),
    );
    let downloaded = 0;

    for (const row of rows) {
        if (DOWNLOAD_LIMIT > 0 && downloaded >= DOWNLOAD_LIMIT) break;
        if (row.date < SKIP_BEFORE_DATE) continue;
        if (existing.has(row.filename)) continue;

        // ... download ...
        downloaded++;
    }
}
```

Template for new scrapers:

```js
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
 * @param {ScrapeContext} context
 * @returns {Promise<StepReturn>}
 */
async function scrapeLoginPage(context) {
    refreshmint.log(`login snapshot: ${await context.mainPage.snapshot()}`);
    return {
        progressName: 'snapshot login page',
        done: true,
    };
}

async function main() {
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
        const urlWithoutFragment = url.split('#', 2)[0];
        /** @type {StepReturn} */
        let stepReturn;
        if (
            urlWithoutFragment ===
            'https://accountmanager.providentcu.org/ProvidentOnlineBanking/SignIn.aspx'
        ) {
            stepReturn = await scrapeLoginPage(context);
        } else {
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
            break;
        }
    }
}
main().catch((e) => {
    refreshmint.log(e);
});
```

## Recommended debug-first flow

Start a debug session:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin app -- \
  debug start \
  --ledger /path/to/ledger.refreshmint \
  --login my-bankofamerica \
  --socket ~/Library/Caches/refreshmint/debug/debug.sock
```

Execute scripts against the live session:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin app -- \
  debug exec \
  --socket ~/Library/Caches/refreshmint/debug/debug.sock \
  --script script.mjs \
  --prompt "OTP=123456"
```

`debug exec` streams `refreshmint.log(...)` and `refreshmint.reportValue(...)` output to its own stderr/stdout.
`debug start` remains focused on hosting the browser/session and startup diagnostics.
If the `debug exec` client disconnects before completion, the server cancels the in-flight script.
This is useful when a run is hung or stuck in a loop: you can disconnect to stop it early, edit the script, and immediately try again.

When `debug exec` finishes (success or failure), any resources staged via `refreshmint.saveResource(...)` are finalized into `accounts/<account>/documents/` using the same evidence pipeline used by `scrape`.
If both the driver and finalization fail, `debug exec` reports both failures in the returned error so partial-output persistence issues are visible immediately.

Stop:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin app -- \
  debug stop \
  --socket /path/to/debug.sock
```

## JavaScript API

### `page`

All methods are async and should be awaited.

| Method                                               | Description                                                                                                                                                      |
| ---------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `page.locator(selector)`                             | Create a `Locator` for reusable element interactions with strictness checks and auto-waiting.                                                                    |
| `await page.goto(url)`                               | Navigate to a URL.                                                                                                                                               |
| `await page.url()`                                   | Return current page URL as a string.                                                                                                                             |
| `await page.reload()`                                | Reload current page.                                                                                                                                             |
| `await page.waitForSelector(selector, timeoutMs?)`   | Wait for a CSS selector to appear, with descriptive timeout errors.                                                                                              |
| `await page.waitForNavigation(timeoutMs?)`           | Wait for URL change from the current page.                                                                                                                       |
| `await page.waitForURL(pattern, timeoutMs?)`         | Wait for current URL to match a pattern (`*` wildcard).                                                                                                          |
| `await page.waitForLoadState(state?, timeoutMs?)`    | Wait for `load`, `domcontentloaded`, or `networkidle`.                                                                                                           |
| `await page.waitForResponse(urlPattern, timeoutMs?)` | Wait for captured network response URL pattern.                                                                                                                  |
| `await page.networkRequests()`                       | Return captured network responses as JSON.                                                                                                                       |
| `await page.responsesReceived()`                     | Alias of `networkRequests()` (Playwright-style naming).                                                                                                          |
| `await page.clearNetworkRequests()`                  | Clear captured network responses.                                                                                                                                |
| `await page.waitForPopup(timeoutMs?)`                | Wait for a popup opened by this page and return the popup `Page` handle.                                                                                         |
| `await page.waitForEvent('popup', timeoutMs?)`       | Playwright-style alias for `waitForPopup` that returns a popup `Page` handle.                                                                                    |
| `await page.click(selector)`                         | Click first element matching selector.                                                                                                                           |
| `await page.type(selector, text)`                    | Click and type text into element.                                                                                                                                |
| `await page.fill(selector, value)`                   | Set input value and dispatch `input`/`change` events.                                                                                                            |
| `await page.innerHTML(selector)`                     | Return `innerHTML` for an element.                                                                                                                               |
| `await page.innerText(selector)`                     | Return visible text for an element.                                                                                                                              |
| `await page.textContent(selector)`                   | Return `textContent` for an element.                                                                                                                             |
| `await page.getAttribute(selector, name)`            | Return attribute value (empty string if missing).                                                                                                                |
| `await page.inputValue(selector)`                    | Return current input value.                                                                                                                                      |
| `await page.isVisible(selector)`                     | Return whether element is visible.                                                                                                                               |
| `await page.isEnabled(selector)`                     | Return whether element is enabled.                                                                                                                               |
| `await page.evaluate(expression)`                    | Evaluate JS in browser context. Returns unwrapped string/JSON text.                                                                                              |
| `await page.frameEvaluate(frameRef, expression)`     | Evaluate JS inside a specific frame execution context.                                                                                                           |
| `await page.frameFill(frameRef, selector, value)`    | Fill an input inside a specific frame execution context.                                                                                                         |
| `await page.snapshot(options?)`                      | Return ARIA-oriented interactive-element snapshot JSON. With `{ incremental: true, track?: string }`, returns only changes from previous snapshot in that track. |
| `await page.setDialogHandler(mode, promptText?)`     | Handle JS dialogs (`accept`, `dismiss`, `none`).                                                                                                                 |
| `await page.lastDialog()`                            | Return most recent intercepted dialog event as JSON.                                                                                                             |
| `await page.setPopupHandler(mode)`                   | Handle `window.open` popups (`ignore` preserves native behavior, `same_tab` redirects current tab).                                                              |
| `await page.popupEvents()`                           | Return captured popup events as JSON.                                                                                                                            |
| `await page.screenshot()`                            | Capture screenshot and return PNG as base64 string.                                                                                                              |
| `await page.waitForDownload(timeoutMs?)`             | Wait for next completed download and return its file info.                                                                                                       |

For frame APIs, `frameRef` can be frame id, frame name, or frame URL (full match or substring).

`page` is target-stable: one `Page` handle maps to one tab/window for the full run.

### `Locator`

Locators provide reusable element finding logic with strictness (fails if multiple elements match) and auto-waiting.

| Method                                       | Description                                                                                    |
| -------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| `locator.locator(selector)`                  | Create a sub-locator scoped to this locator.                                                   |
| `locator.first()`                            | Filter to the first matching element.                                                          |
| `locator.last()`                             | Filter to the last matching element.                                                           |
| `locator.nth(index)`                         | Filter to the element at the 0-based index.                                                    |
| `await locator.count()`                      | Return number of matching elements.                                                            |
| `await locator.click(options?)`              | Click the element. `options` can be `{ timeout?: number }` or a timeout number.                |
| `await locator.fill(value, options?)`        | Fill the element with `value`. `options` can be `{ timeout?: number }` or a timeout number.    |
| `await locator.innerText(options?)`          | Return visible text.                                                                           |
| `await locator.textContent(options?)`        | Return text content.                                                                           |
| `await locator.getAttribute(name, options?)` | Return attribute value.                                                                        |
| `await locator.inputValue(options?)`         | Return current input value.                                                                    |
| `await locator.isVisible()`                  | Return whether element is visible.                                                             |
| `await locator.isEnabled()`                  | Return whether element is enabled.                                                             |
| `await locator.wait_for(options?)`           | Wait for state (`attached`, `detached`, `visible`, `hidden`). Default: `{ state: 'visible' }`. |

### `browser`

| Method                                    | Description                                                  |
| ----------------------------------------- | ------------------------------------------------------------ |
| `await browser.pages()`                   | Return all open pages as `Page[]`.                           |
| `await browser.waitForEvent('page', ms?)` | Wait for any newly opened page and return its `Page` handle. |

Use the race-safe pattern (start waiting before triggering the popup):

```js
const popupPromise = page.waitForEvent('popup', 10000);
await page.click('#open-popup');
const popup = await popupPromise;
await popup.waitForLoadState('domcontentloaded');
```

Suggested debug helper for state machines:

```js
async function logSnapshot(tag, track = 'state-loop') {
    const diff = await page.snapshot({ incremental: true, track });
    refreshmint.log(`${tag} snapshot: ${diff}`);
}
```

### `refreshmint`

| Method                                                                | Description                                                                  |
| --------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| `await refreshmint.saveResource(filename, data, options?)`            | Write bytes to extension output dir and stage for account-doc finalization.  |
| `await refreshmint.saveDownloadedResource(path, filename?, options?)` | Read a completed local download file and stage it as a resource.             |
| `await refreshmint.listAccountDocuments()`                            | Return JSON list of existing account documents (with optional sidecar info). |
| `await refreshmint.setSessionMetadata(metadata)`                      | Set optional sidecar metadata (`dateRangeStart`, `dateRangeEnd`).            |
| `refreshmint.reportValue(key, value)`                                 | Print key/value status line.                                                 |
| `refreshmint.log(message)`                                            | Log message to stderr.                                                       |
| `refreshmint.prompt(message)`                                         | Ask for a value. CLI runs require `--prompt "MESSAGE=VALUE"`.                |

For `saveResource`, `data` should be bytes (`number[]` is supported). `options` may include `coverageEndDate`, `originalUrl`, and `mimeType`.

## Secrets and `page.fill`

`page.fill(selector, value)` performs secret substitution:

- if `value` is declared in manifest `secrets.<domain>` for current top-level page domain, it is resolved from keychain
- if declared only for a different domain, `fill`/`frameFill` throws
- if keychain secret exists but is not declared for current domain, `fill`/`frameFill` throws
- otherwise `value` is treated literally

Check secrets:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin app -- \
  secret list --account Assets:Checking
```
