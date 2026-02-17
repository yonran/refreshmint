# Writing Scrapers

This document covers:

- extension layout
- how to load and run a scraper
- the JavaScript API available to scraper scripts

## Extension layout

A scraper extension is a directory with at least:

```text
my-extension/
  manifest.json
  driver.mjs
```

`manifest.json` must include a `name` field:

```json
{
    "name": "my-extension"
}
```

`driver.mjs` is the script Refreshmint executes.

When loaded into a ledger, the extension lives at:

```text
<ledger>.refreshmint/extensions/<name>/
```

Scraper output files are written under:

```text
<ledger>.refreshmint/extensions/<name>/output/
```

## Load an extension

You can load from a directory or zip:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin app -- \
  extension load /path/to/my-extension --ledger /path/to/ledger.refreshmint
```

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin app -- \
  extension load /path/to/my-extension.zip --ledger /path/to/ledger.refreshmint
```

Use `--replace` to overwrite an existing extension with the same manifest name.

## Run a scraper

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin app -- \
  scrape \
  --ledger /path/to/ledger.refreshmint \
  --account Assets:Checking \
  --extension my-extension
```

The same flow can be run from the GUI Scraping tab.

Site-specific notes can live under `knowledge/sites/<site>/README.md` (example: `knowledge/sites/clipper-card/README.md`).

## Driver runtime model

- `driver.mjs` is executed inside a QuickJS sandbox.
- Top-level `await` works (the runtime wraps your script in an async function).
- Two globals are available:
    - `page` for browser automation
    - `refreshmint` for output/logging helpers

Errors thrown from your script fail the scrape run.

## Recommended development flow (debug-first)

Do not start by writing a full `driver.mjs` and running it end-to-end.

Use a debug session and build incrementally:

1. Start a debug session

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin app -- \
  debug start \
  --ledger /path/to/ledger.refreshmint \
  --account Assets:Checking \
  --extension my-extension
```

2. Run tiny scripts with one action each

```js
// step-1.mjs
await page.goto('https://example.com/login');
refreshmint.log(`URL: ${await page.url()}`);
```

```js
// step-2.mjs
await page.waitForSelector('form');
refreshmint.log(await page.innerHTML('form'));
```

```js
// step-3.mjs
await page.fill('#username', 'bank_username');
await page.fill('#password', 'bank_password');
refreshmint.log('filled credentials');
```

3. Verify each step before moving forward

- after fill: check `inputValue`, `isVisible`, `isEnabled`
- after submit: check URL with `waitForURL` and inspect page text
- after data loads: use `waitForLoadState('networkidle')` or `waitForResponse`

4. Combine successful steps into final `driver.mjs`

This is faster to debug and avoids opaque failures in long scripts.

## Script example (final combined driver)

```js
refreshmint.log('starting scrape');

await page.goto('https://example.com/login');
await page.waitForSelector('#username');

await page.fill('#username', 'my_username');
await page.fill('#password', 'bank_password'); // secret-name substitution supported
await page.click('button[type="submit"]');
await page.waitForURL('https://example.com/dashboard*');

const snapshot = await page.evaluate(
    'JSON.stringify({ title: document.title, url: location.href })',
);
await refreshmint.saveResource(
    'session/snapshot.json',
    Array.from(new TextEncoder().encode(snapshot)),
);
refreshmint.reportValue('status', 'ok');
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
| `await page.waitForDownload()`                       | Configure download behavior and return download info object.        |

`page.waitForDownload()` currently returns:

- `path`: download directory path
- `suggestedFilename`: currently empty string

Network response capture (`waitForResponse`, `networkRequests`, `responsesReceived`) is backed by the browser debug protocol (`Network.responseReceived`), not DOM monkeypatching.

For frame APIs, `frameRef` can be any of:

- frame id
- frame name
- frame URL (full match or substring)

### `refreshmint`

| Method                                           | Description                                                                 |
| ------------------------------------------------ | --------------------------------------------------------------------------- |
| `await refreshmint.saveResource(filename, data)` | Write bytes to extension output dir. Parent dirs are created automatically. |
| `refreshmint.reportValue(key, value)`            | Print key/value status line.                                                |
| `refreshmint.log(message)`                       | Log message to stderr.                                                      |
| `refreshmint.prompt(message)`                    | Prompt and read one line from stdin.                                        |

For `saveResource`, `data` should be bytes (`number[]` is supported).

## Secrets and `page.fill`

`page.fill(selector, value)` has secret substitution behavior:

- Refreshmint gets the current page domain (host from URL).
- If `value` matches a stored secret **name** for that domain/account, the secret value is pulled from keychain and injected.
- Otherwise, `value` is used literally.
- There is no warning when a name does not match; the literal input is used.

This lets scripts refer to symbolic names instead of embedding credentials.

Check configured secrets via:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin app -- \
  secret list --account Assets:Checking
```

Manage secrets via:

- CLI: `secret add|reenter|remove|list`
- GUI: Scraping tab, Account secrets section

## Debug workflow (LLM/manual control)

Debug sessions run the same script API but keep the browser open.

Start:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin app -- \
  debug start \
  --ledger /path/to/ledger.refreshmint \
  --account Assets:Checking \
  --extension my-extension
```

Execute script against live session:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin app -- \
  debug exec \
  --socket /path/to/debug.sock \
  --script script.mjs
```

Stop:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin app -- \
  debug stop \
  --socket /path/to/debug.sock
```

The GUI Scraping tab also has debug start/stop and socket copy controls.
