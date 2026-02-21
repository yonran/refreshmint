# Scraper Authoring

This document covers running scrapers, scraper runtime behavior, and scraper JS APIs.

For extension structure and manifest details, see `docs/extension.md`.

## Run a scraper

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin app -- \
  scrape \
  --ledger /path/to/ledger.refreshmint \
  --account Assets:Checking \
  --extension my-extension
```

`--extension` is optional when account config already has `extension`.

If your script uses `refreshmint.prompt(message)`, supply overrides:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin app -- \
  scrape \
  --ledger /path/to/ledger.refreshmint \
  --account Assets:Checking \
  --extension my-extension \
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

- Scrapers will fail often. Make them fast to debug and retry
    - The script should have a state machine that keeps working (for loop) until an error or the end state is achieved
    - Scrapers should define a function for each state (e.g. URL or page) and then the main code should switch on the URL or other identifying information on the page. This allows us to edit the script and re-run it without starting from the original logged out state.
    - The state machine should also keep track of progress being made (e.g. a set of distinct pages viewed or output identifiers such as last month downloaded) and iterations since last progress. If there are too many steps with no real progress, then throw an error.
    - Log what state you are in.
    - Add checks to make sure you are on the page that you expect to be.
    - When there is an exception or something unexpected, log context before re-throwing so that we can see what went wrong quickly.
- Domain checks must compare the URL prefix to the full origin (for example `https://secure.bankofamerica.com/`), not a substring match.
- Prefer `const url = await page.url(); if (!url.startsWith("https://example.com/")) { ... }` over checks like `url.includes("example.com")`.
- When adding a new code path/branch, add a brief `UNTESTED` comment until that exact branch is exercised in a real run; remove or update the comment after verification.
- For any code path, uniquely log the branch that we take so that you will see whether it is actually being executed. If it is an inner loop, remove the logging after you have tested it if the logging is too verbose.
- At each page handler/state, add a short comment describing expected page conditions and enforce them with explicit assertions (URL/selector checks) before taking actions.
- Add comments for non-obvious actions describing what outcome each action is trying to accomplish (not just what selector is clicked).
- Pace automation actions (especially login, navigation, and repeated downloads) with short delays so behavior is less bot-like and less likely to trigger anti-automation defenses.
- Add a reusable snapshot logger for state loops. Prefer `await page.snapshot({ incremental: true, track: 'state-loop' })` so logs show only page changes from the previous checkpoint.

## Recommended debug-first flow

Start a debug session:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin app -- \
  debug start \
  --ledger /path/to/ledger.refreshmint \
  --account Assets:Checking \
  --extension my-extension
```

Execute scripts against the live session:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin app -- \
  debug exec \
  --socket /path/to/debug.sock \
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
| `await page.tabs()`                                  | Return open tabs as JSON (`index`, `targetId`, `url`, `current`).                                                                                                |
| `await page.selectTab(index)`                        | Switch the active `page` handle to a tab index and return its URL.                                                                                               |
| `await page.waitForPopup(timeoutMs?)`                | Wait for popup tab (prefers `window.open`/opener match), switch `page`, and return popup summary JSON.                                                           |
| `await page.waitForEvent('popup', timeoutMs?)`       | Playwright-style alias for `waitForPopup` that returns popup summary JSON.                                                                                       |
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

`waitForPopup` / `waitForEvent('popup')` return a JSON string, not a page handle.

Use the race-safe pattern (start waiting before triggering the popup):

```js
const popupPromise = page.waitForEvent('popup', 10000);
await page.click('#open-popup');
const popup = JSON.parse(await popupPromise);
// popup: { index, targetId, url, current }
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
