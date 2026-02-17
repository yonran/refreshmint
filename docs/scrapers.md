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

## Driver runtime model

- `driver.mjs` is executed inside a QuickJS sandbox.
- Top-level `await` works (the runtime wraps your script in an async function).
- Two globals are available:
    - `page` for browser automation
    - `refreshmint` for output/logging helpers

Errors thrown from your script fail the scrape run.

## Script example

```js
refreshmint.log('starting scrape');

await page.goto('https://example.com/login');
await page.waitForSelector('#username');

await page.fill('#username', 'my_username');
await page.fill('#password', 'bank_password'); // secret name support; see below
await page.click('button[type="submit"]');
await page.waitForNavigation();

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

| Method                                 | Description                                                         |
| -------------------------------------- | ------------------------------------------------------------------- |
| `await page.goto(url)`                 | Navigate to a URL.                                                  |
| `await page.url()`                     | Return current page URL as a string.                                |
| `await page.reload()`                  | Reload current page.                                                |
| `await page.waitForSelector(selector)` | Wait for a CSS selector to appear.                                  |
| `await page.waitForNavigation()`       | Wait for navigation to complete.                                    |
| `await page.click(selector)`           | Click first element matching selector.                              |
| `await page.type(selector, text)`      | Click and type text into element.                                   |
| `await page.fill(selector, value)`     | Set input value and dispatch `input`/`change` events.               |
| `await page.evaluate(expression)`      | Evaluate JS in browser context. Returns a stringified/debug result. |
| `await page.screenshot()`              | Capture screenshot and return PNG as base64 string.                 |
| `await page.waitForDownload()`         | Configure download behavior and return download info object.        |

`page.waitForDownload()` currently returns:

- `path`: download directory path
- `suggestedFilename`: currently empty string

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

This lets scripts refer to symbolic names instead of embedding credentials.

Secrets are managed via:

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
