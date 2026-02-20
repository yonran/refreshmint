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

Stop:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin app -- \
  debug stop \
  --socket /path/to/debug.sock
```

## JavaScript API

### `page`

All methods are async and should be awaited.

| Method                                               | Description                                                         |
| ---------------------------------------------------- | ------------------------------------------------------------------- |
| `await page.goto(url)`                               | Navigate to a URL.                                                  |
| `await page.url()`                                   | Return current page URL as a string.                                |
| `await page.reload()`                                | Reload current page.                                                |
| `await page.waitForSelector(selector, timeoutMs?)`   | Wait for a CSS selector to appear, with descriptive timeout errors. |
| `await page.waitForNavigation(timeoutMs?)`           | Wait for URL change from the current page.                          |
| `await page.waitForURL(pattern, timeoutMs?)`         | Wait for current URL to match a pattern (`*` wildcard).             |
| `await page.waitForLoadState(state?, timeoutMs?)`    | Wait for `load`, `domcontentloaded`, or `networkidle`.              |
| `await page.waitForResponse(urlPattern, timeoutMs?)` | Wait for captured network response URL pattern.                     |
| `await page.networkRequests()`                       | Return captured network responses as JSON.                          |
| `await page.responsesReceived()`                     | Alias of `networkRequests()` (Playwright-style naming).             |
| `await page.clearNetworkRequests()`                  | Clear captured network responses.                                   |
| `await page.click(selector)`                         | Click first element matching selector.                              |
| `await page.type(selector, text)`                    | Click and type text into element.                                   |
| `await page.fill(selector, value)`                   | Set input value and dispatch `input`/`change` events.               |
| `await page.innerHTML(selector)`                     | Return `innerHTML` for an element.                                  |
| `await page.innerText(selector)`                     | Return visible text for an element.                                 |
| `await page.textContent(selector)`                   | Return `textContent` for an element.                                |
| `await page.getAttribute(selector, name)`            | Return attribute value (empty string if missing).                   |
| `await page.inputValue(selector)`                    | Return current input value.                                         |
| `await page.isVisible(selector)`                     | Return whether element is visible.                                  |
| `await page.isEnabled(selector)`                     | Return whether element is enabled.                                  |
| `await page.evaluate(expression)`                    | Evaluate JS in browser context. Returns unwrapped string/JSON text. |
| `await page.frameEvaluate(frameRef, expression)`     | Evaluate JS inside a specific frame execution context.              |
| `await page.frameFill(frameRef, selector, value)`    | Fill an input inside a specific frame execution context.            |
| `await page.snapshot()`                              | Return an accessibility-like snapshot JSON of interactive elements. |
| `await page.setDialogHandler(mode, promptText?)`     | Handle JS dialogs (`accept`, `dismiss`, `none`).                    |
| `await page.lastDialog()`                            | Return most recent intercepted dialog event as JSON.                |
| `await page.setPopupHandler(mode)`                   | Handle popups (`ignore` or `same_tab`).                             |
| `await page.popupEvents()`                           | Return captured popup events as JSON.                               |
| `await page.screenshot()`                            | Capture screenshot and return PNG as base64 string.                 |
| `await page.waitForDownload(timeoutMs?)`             | Wait for next completed download and return its file info.          |

For frame APIs, `frameRef` can be frame id, frame name, or frame URL (full match or substring).

### `refreshmint`

| Method                                                     | Description                                                                 |
| ---------------------------------------------------------- | --------------------------------------------------------------------------- |
| `await refreshmint.saveResource(filename, data, options?)` | Write bytes to extension output dir and stage for account-doc finalization. |
| `await refreshmint.setSessionMetadata(metadata)`           | Set optional sidecar metadata (`dateRangeStart`, `dateRangeEnd`).           |
| `refreshmint.reportValue(key, value)`                      | Print key/value status line.                                                |
| `refreshmint.log(message)`                                 | Log message to stderr.                                                      |
| `refreshmint.prompt(message)`                              | Ask for a value. CLI runs require `--prompt "MESSAGE=VALUE"`.               |

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
