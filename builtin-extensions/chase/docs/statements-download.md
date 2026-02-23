# Chase Statements Download - Site Knowledge

## URL

`https://secure.chase.com/web/auth/dashboard#/dashboard/documents/myDocs/index;mode=documents`

Navigated from the dashboard via the "Statements & documents" link in the Overview nav.

## Authentication Flow

1. Load storage state from `.playwright-mcp/chase/storageState.json` if it exists (to restore device-recognition cookies and potentially skip MFA).
2. Navigate to `https://secure.chase.com/web/auth/dashboard`
3. **Two login form variants observed:**
    - `secure.chase.com` login: form is inside an iframe (`iframe[title="logon"]`). MCP snapshot shows iframe elements with `f1e*` refs.
    - `www.chase.com` homepage (after session expiry redirect): form is directly in the page DOM, no iframe.
4. Fill username and password textboxes, click "Sign in".
5. MFA: Chase may present "Confirm Your Identity" with options: "Get a text" or "Get a call".
    - After selecting a method and clicking "Next", a one-time code is sent. Enter it and click "Next" again.
    - **MFA is sometimes skipped** on repeat logins (device is remembered). On second login after session timeout, went straight to dashboard with no MFA.
6. On success, redirected to the dashboard.
7. **Session timeout:** Chase signs you out after ~15 minutes of inactivity, redirecting to `www.chase.com` with "We've signed you out of your account." banner.
8. **Storage state:** Save browser storage state (cookies + localStorage) to `.playwright-mcp/chase/storageState.json` after login via `page.context().storageState({ path })`. Load it next time with `browser.newContext({ storageState: path })` to potentially skip MFA. The device-remembering cookies may help avoid MFA on subsequent sessions even after the login session itself expires.

## Statements Page Structure

### Year Selector

- A dropdown button shows the current year (e.g. `"2026"`).
- Clicking it opens a listbox with options for each available year (2019-2026 observed).
- Selecting a year reloads all account tables below.

### Account Sections

Three expandable account sections, each with a heading button:

- **CHASE SAVINGS (...6870)**
- **FREEDOM (...8354)**
- **AMAZON VISA (...2074)**

Each section has a toggle button (`aria-expanded`). When collapsed, the table rows are **not in the DOM**. You must expand each section to access its download links.

### Table Structure

Each expanded account section contains a `<table>` with rows for each statement. Tables are indexed sequentially within a year view:

| Account                 | Table Index       |
| ----------------------- | ----------------- |
| CHASE SAVINGS (...6870) | `accountsTable-0` |
| FREEDOM (...8354)       | `accountsTable-1` |
| AMAZON VISA (...2074)   | `accountsTable-2` |

Row IDs follow the pattern: `accountsTable-{tableIdx}-row{rowIdx}`

### Download Link Types

**Two different download mechanisms were observed:**

#### 1. Savings Account - Dropdown Menu Pattern

The savings account uses a dropdown menu for downloads:

- Trigger: `#header-accountsTable-0-row{rowIdx}-cell3-downloadDocumentDropdown`
- After clicking the trigger, a dropdown appears with:
    - "Save as PDF": `#item-0-{rowIdx}-downloadPDFOption`
    - "Save as accessible PDF": second list item
- You must click the trigger first, wait, then click the PDF option.
- The `header-` prefix ID only exists for table index 0 (savings).

#### 2. Credit Card Accounts - Direct Download Links

Freedom and Amazon Visa use simpler direct download links:

- Download link: `#accountsTable-{tableIdx}-row{rowIdx}-cell3-requestThisDocumentAnchor-download`
- View link: `#accountsTable-{tableIdx}-row{rowIdx}-cell3-requestThisDocumentAnchor-pdf`
- Clicking the download link directly triggers a file download (no dropdown).

### Downloaded File Naming

Chase names downloaded PDFs as: `{YYYYMMDD}-statements-{last4}-.pdf`

- Example: `20251222-statements-6870-.pdf`
- The date is the statement date, last4 is the account suffix.

## What Worked

### Discovering table structure

```js
// Find all download rows across all visible account tables
const rows = await page
    .locator('tr')
    .filter({ hasText: 'Saves document' })
    .all();
for (const row of rows) {
    const rowId = await row.getAttribute('id');
    const table = row.locator('xpath=ancestor::table');
    const caption = await table.locator('caption').textContent();
    // yields: { rowId: "accountsTable-1-row0", caption: "Statements FREEDOM (...8354)" }
}
```

### Bulk downloading savings (dropdown pattern)

```js
for (let row = 0; row < rowCount; row++) {
    const dropdown = page.locator(
        `#header-accountsTable-0-row${row}-cell3-downloadDocumentDropdown`,
    );
    await dropdown.scrollIntoViewIfNeeded();
    await dropdown.click({ force: true });
    await page.waitForTimeout(500);
    const pdfLink = page.locator(`#item-0-${row}-downloadPDFOption`);
    const [download] = await Promise.all([
        page.waitForEvent('download', { timeout: 15000 }),
        pdfLink.click({ force: true }),
    ]);
    results.push(download.suggestedFilename());
    await page.waitForTimeout(1000);
}
```

### Bulk downloading credit cards (direct link pattern)

```js
for (let row = 0; row < rowCount; row++) {
    const link = page.locator(
        `#accountsTable-{tableIdx}-row${row}-cell3-requestThisDocumentAnchor-download`,
    );
    await link.scrollIntoViewIfNeeded();
    const [download] = await Promise.all([
        page.waitForEvent('download', { timeout: 15000 }),
        link.click({ force: true }),
    ]);
    results.push(download.suggestedFilename());
    await page.waitForTimeout(1500);
}
```

## What Was Confusing / Failed

### 1. `require()` is not available in `browser_run_code`

`require('fs')` and `require('path')` throw `ReferenceError: require is not defined`. The eval context is browser-side, not Node.

### 2. `setTimeout` not directly available

`new Promise(r => setTimeout(r, 1000))` threw `ReferenceError: setTimeout is not defined` in the eval context. Use `page.waitForTimeout(ms)` instead.

### 3. Searching for dropdown IDs missed credit card tables

Searching for `[id^="header-accountsTable"]` only found savings (table 0) elements because the `header-` prefixed download dropdown ID pattern only exists for the savings account. Credit cards use `requestThisDocumentAnchor-download` instead. The generic search `[id*="downloadDocumentDropdown"]` also missed them.

### 4. Sticky header intercepts clicks

When scrolling to elements, the Chase sticky header (`#header-outer-container`) can intercept click events. Using `{ force: true }` on clicks bypasses this.

### 5. Open dropdowns block subsequent clicks

If a save dropdown is already open from a previous interaction, clicking another dropdown can fail because the open one intercepts pointer events. Press Escape first or use `{ force: true }`.

### 6. `page.waitForEvent('download')` fails on dropdown triggers

Wrapping the dropdown trigger click with `waitForEvent('download')` times out because the trigger only opens a menu — it doesn't initiate a download. The download only starts when "Save as PDF" is clicked.

### 7. Don't inject local paths into the page DOM

Attempted to use Chrome DevTools Protocol (`Browser.setDownloadBehavior`) to set a download directory, but this would expose local filesystem paths to the page's JavaScript context. Instead, download to the default MCP location and move files afterwards with shell commands.

## Approach for Remaining Years

To download all statements for years 2024-2019:

1. Click the year selector button, select the target year.
2. Wait for content to reload.
3. Expand each account section (click buttons with `aria-expanded="false"` matching account names).
4. Discover rows using `tr` filter or the `requestThisDocumentAnchor-download` / `downloadDocumentDropdown` ID patterns.
5. Download using the appropriate pattern (dropdown for savings, direct for credit cards).
6. Move all downloaded PDFs from `.playwright-mcp/` to `.playwright-mcp/chase/`.

## PDF Statement Parsing (pdf2json)

### Suppressing pdf2json warnings

pdf2json emits noisy Type3 font warnings via `console.log` (NOT `console.warn`). It captures a reference to `console.log` at module load time via `.bind()`. To suppress these warnings, you must:

1. Patch `console.log` to filter out Type3/fake worker messages **before** importing pdf2json.
2. Use dynamic `await import('pdf2json')` — ESM static imports are hoisted before any module-level code, so a static import defeats the patch.

```typescript
const _origLog = console.log;
console.log = (...a: unknown[]) => {
    if (typeof a[0] === 'string' && /Type3 font|fake worker/i.test(a[0]))
        return;
    _origLog.apply(console, a);
};
const { default: PDFParser } = await import('pdf2json');
```

### Date columns differ by account type

- **Credit card PDFs** show the **transaction date** (purchase date), NOT the post date. This is the same date as the CSV `Transaction Date` column.
- **Savings PDFs** show the **post date**.
- This distinction is critical when matching PDF transactions to CSV/QFX data. Use `transaction_date` for credit card matching and `post_date` for savings matching.

### PDF amount formats

- **Savings debits:** shown as `- 70.00` (space between minus sign and number). Parse by detecting the separated minus.
- **Credit card purchases:** shown as positive numbers (e.g. `4.36`). The sign convention is opposite from CSV (where purchases are negative).

### Statement availability and gaps

- Chase provides statements back to **2019** (as of Feb 2026).
- **Credit card statements are only generated for months with activity.** Missing months in the statement list are normal — they mean no transactions occurred that billing period. Don't treat gaps as missing downloads.
- **Payment-only months don't generate statements.** Chase does not generate statement PDFs for billing periods with only payments and no purchases. If an AUTOMATIC PAYMENT was made but no purchases occurred that month, no statement will exist. These payments appear in CSV/QFX exports but cannot be PDF-enriched since there's no PDF to extract data from. This is expected Chase platform behavior, not a missing download.

### Text extraction

- Use pdf2json's page data: sort text items by `(y, x)` coordinates to reconstruct reading order.
- Decode text with `decodeURIComponent(textItem.R[0].T)`, but wrap in try/catch — some text contains malformed percent-encoded sequences.
- Transaction lines appear on page 2+ under an "ACCOUNT ACTIVITY" header section.
- **Amazon order numbers** appear on a separate line following the transaction: `Order Number       111-5133196-0444250`

### Overlapping billing periods

Multiple statement PDFs may contain the same transaction (billing periods overlap at boundaries). When enriching merged data with PDF info, track whether a transaction was already enriched to avoid double-counting in statistics.
