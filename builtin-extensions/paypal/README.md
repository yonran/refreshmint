# PayPal Scraper Knowledge Base

## Login Flow

1. Navigate to `https://www.paypal.com/signin`
2. Fill email: `textbox "Email or mobile number"` (role selector)
3. Click `button "Next"` (role selector)
4. Fill password: `textbox "Password"` (role selector)
5. Click `button "Log In"` (role selector)
6. 2FA challenge: authenticator app TOTP code
    - 6 individual spinbutton fields: `spinbutton "1-6"` through `spinbutton "6-6"` (role selectors)
    - "Remember this device" checkbox is checked by default
    - Click `button "Submit"` (role selector)
7. Lands on `https://www.paypal.com/myaccount/summary`

### Storage State

Save to `.playwright-mcp/paypal/storageState.json` after login and before closing. "Remember this device" checkbox helps reduce future MFA prompts.

## Monthly Statements (Reg E Official Statements)

### Overview

PayPal provides official Regulation E periodic statements as monthly PDFs at `/myaccount/statements/monthly`. These are the legally mandated account statements containing:

- Statement period (e.g., Dec 1, 2025 - Dec 31, 2025)
- Each transfer: date, description, currency, amount, fees, total
- Beginning and ending balances
- Error resolution notice (phone: 402-938-3614, P.O. Box 45950, Omaha, NE 68145-0950)
- Two sections: "PayPal account statement" and "PayPal Balance account statement"

### Availability

- **History:** 3 years back
- **Format:** PDF only (individual months or full-year ZIP)
- **Years available as of Feb 2026:** 2023, 2024, 2025

### Navigation

1. Navigate to `https://www.paypal.com/myaccount/statements/`
2. Click `button "All transactions"` (testId: `statements_side_nav_test_id`)
3. Lands on `/myaccount/statements/monthly`

### Downloading

Each year has a collapsible section with individual month Download buttons and a "Download all" button.

```js
// Expand a year (2025 is expanded by default)
await page.getByRole('button', { name: '2024' }).click();

// Download all months for a year as a ZIP
const [download] = await Promise.all([
    page.waitForEvent('download', { timeout: 15000 }),
    page
        .getByRole('region', { name: '2025' })
        .getByRole('button', { name: 'Download all' })
        .click(),
]);
// Suggested filename: "statement-2025.zip"
await download.saveAs('path/to/statements-2025.zip');

// Or download a single month
const [dlMonth] = await Promise.all([
    page.waitForEvent('download', { timeout: 15000 }),
    page
        .getByRole('region', { name: '2025' })
        .getByRole('button', { name: 'Download' })
        .first()
        .click(),
]);
// Single month PDF
```

### ZIP Contents

Each yearly ZIP contains 12 monthly PDFs:

```
statement-Jan-2025.pdf
statement-Feb-2025.pdf
...
statement-Dec-2025.pdf
```

### PDF Structure (3 pages for Dec 2025)

- **Page 1:** PayPal Account Activity — transaction table (Date, Description, Currency, Amount, Fees, Total) with transaction IDs
- **Page 2:** Error resolution notice (Reg E required disclosure)
- **Page 3:** PayPal Balance Account — balance summary (beginning/ending), fees summary, and Balance account activity

### Gotchas

- The "Download all" button downloads a ZIP, not individual PDFs
- The suggested filename is `statement-{year}.zip` — the `saveAs` path prepends "statement-" so watch for double-naming (e.g., `statement-statement-2025.zip`)
- 2023 section may only show December if the account wasn't active earlier — need to expand to verify

## Activity Reports (Data Export)

### Overview

PayPal's Reports system at `/reports/dlog` allows downloading CSV activity reports. These are **data exports** (not official Reg E statements) but contain far more fields.

- **Max range per report:** 12 months
- **Max history:** 7 years back from today
- **Max reports stored:** 12 at a time
- **Formats available:** CSV, TAB, PDF, Quickbooks (IIF), Quicken (QIF)
- **Transaction type options:** All transactions, Completed payments, Balance affecting

### Navigation Path

1. Activity page (`/myaccount/activities/`) -> `button "download statement"` -> Statements & Taxes page
2. Click `button "Custom"` (testId: `custom_side_nav_test_id`) -> Reports page (`/reports/`)
3. Click "Activity report" under Activities tab -> `/reports/dlog`

Or navigate directly to `https://www.paypal.com/reports/dlog`.

### Customizable Fields (Important!)

Click `button "Customize report fields"` (testId: `linkButtonRow` > `linkButton`) to open a dialog with field groups. **Default CSV has only 41 columns. With all fields enabled, you get 83 columns.**

Field groups (with default checked status):

- **Default fields** (always included, cannot be removed): Date, Time, Time zone, Type, Status, Currency, Gross, Fee, Net, To email, Transaction ID, Reference transaction ID, Name, From email, Receipt ID
- **Transaction details** (partially checked by default): Sales tax, Invoice number, Balance, Subject, Note, Balance Impact are checked. **Unchecked by default:** Payment source, Card type, Transaction event code, Payment tracking ID, Bank reference ID, Transaction buyer country code, Tip, Discount, Seller ID, Risk Filter, Credit Transactional Fee, Credit Promotional Fee, Credit Term, Credit Offer Type, Original Invoice ID, Payment Source Subtype, Decline Code, Fastlane Checkout Transaction, Reward Points
- **Buyer details** (unchecked): CounterParty Status
- **Shipping details** (checked by default): Shipping Address, Address Status, Address Line 1/2, Town/City, State, Zip, Country, Contact Phone Number
- **Auction details** (unchecked): Auction Site, Buyer ID, Item URL, Closing Date, Escrow Id
- **Cart details** (unchecked): Item Title, Item ID, Option 1/2 Name/Value, Shipping and Handling Amount, Insurance Amount, Item Details, Coupons, Special Offers, Loyalty Card Number
- **Funding details** (unchecked): Buyer Wallet
- **Risk details** (unchecked): Authorization Review Status, Protection Eligibility
- **Payflow Details** (unchecked): Comment 1, Comment 2, Invoice Number (duplicate), PO Number, Customer Reference Number, Payflow Transaction ID (PNREF), Campaign Fee, Campaign Name, Campaign Discount, Campaign Discount Currency
- **Include shopping cart details as line items** (unchecked): When enabled, adds "Shopping Cart Item" sub-rows under each transaction breaking out individual items

To enable all fields at once via DOM:

```js
const dialog = document.querySelector('[role="dialog"]');
const checkboxes = dialog.querySelectorAll('input[type="checkbox"]');
for (const cb of checkboxes) {
    if (!cb.checked) cb.click();
}
```

**Note:** You must expand all collapsed groups (click all chevron-down icons) before the sub-field checkboxes are in the DOM. The field customization is saved server-side (`POST /reports/apis/common/ql` with action `createTemplate`) and persists across sessions.

### All-Fields CSV Columns (83 columns)

```
Date, Time, TimeZone, Name, Type, Status, Currency, Gross, Fee, Net,
From Email Address, To Email Address, Transaction ID, CounterParty Status,
Shipping Address, Address Status, Item Title, Item ID,
Shipping and Handling Amount, Insurance Amount, Sales Tax,
Option 1 Name, Option 1 Value, Option 2 Name, Option 2 Value,
Auction Site, Buyer ID, Item URL, Closing Date, Escrow Id,
Reference Txn ID, Invoice Number, Custom Number, Quantity, Receipt ID,
Balance, Address Line 1, Address Line 2/District/Neighborhood, Town/City,
State/Province/Region/County/Territory/Prefecture/Republic,
Zip/Postal Code, Country, Contact Phone Number, Subject, Note,
Payment Source, Card Type, Transaction Event Code, Payment Tracking ID,
Bank Reference ID, Transaction Buyer Country Code, Item Details, Coupons,
Special Offers, Loyalty Card Number, Authorization Review Status,
Protection Eligibility, Country Code, Balance Impact, Buyer Wallet,
Comment 1, Comment 2, Invoice Number, PO Number,
Customer Reference Number, Payflow Transaction ID (PNREF), Tip, Discount,
Seller ID, Risk Filter, Credit Transactional Fee, Credit Promotional Fee,
Credit Term, Credit Offer Type, Original Invoice ID, Payment Source Subtype,
Campaign Fee, Campaign Name, Campaign Discount, Campaign Discount Currency,
Decline Code, Fastlane Checkout Transaction, Reward Points
```

### Key Extra Fields in All-Fields Mode

- **Payment Source:** e.g., `PayPal`, `PayPal [4569]` (shows which card)
- **Transaction Event Code:** e.g., T0003 (payment), T0700 (card deposit), T1300 (auth), T6000 (cart item)
- **Shopping Cart Items:** When enabled, adds ~2x more rows — each line item in a purchase becomes its own "Shopping Cart Item" row
- **Authorization Review Status / Protection Eligibility:** Buyer protection codes

### All-Fields vs Default Comparison (2025-2026 period)

| Metric    | Default (41 cols) | All fields (83 cols) |
| --------- | ----------------- | -------------------- |
| Columns   | 41                | 83                   |
| Rows      | 99                | 243                  |
| File size | 47KB              | 128KB                |

The row increase comes from shopping cart line item sub-rows.

### Creating a Report (UI Selectors)

```js
// 1. Select transaction type
await page.getByTestId('TransactionType').click();
await page.getByRole('option', { name: 'All transactions' }).click();

// 2. Set date range
await page.getByRole('textbox', { name: 'Date range' }).click();
const fromInput = page.getByTestId('startInputBox');
await fromInput.click({ clickCount: 3 });
await fromInput.fill('2/10/2024'); // MM/DD/YYYY format
await page.keyboard.press('Tab');
const toInput = page.getByTestId('endInputBox');
await toInput.click({ clickCount: 3 });
await toInput.fill('2/9/2025');
await page.keyboard.press('Escape'); // Close calendar overlay

// 3. Format defaults to CSV (change via testId 'fileFormatDropdown' if needed)

// 4. Create report
await page.getByTestId('ActivityCreateReport').click();

// 5. Wait for report to be ready (polls automatically, usually ~10-30s)
// Status goes: Submitted -> In progress -> Download

// 6. Download
const [download] = await Promise.all([
    page.waitForEvent('download', { timeout: 15000 }),
    page.getByRole('button', { name: 'Download' }).first().click(),
]);
await download.saveAs('path/to/file.csv');
```

### Batch Report Creation

Reports can be created in a loop. Use 3s delay between submissions:

```js
const periods = [
    { from: '2/10/2025', to: '2/9/2026' },
    { from: '2/10/2024', to: '2/9/2025' },
    { from: '2/10/2023', to: '2/9/2024' },
    // ... up to 7 years back
];

for (const period of periods) {
    // set transaction type, date range, create report (see above)
    await page.waitForTimeout(3000);
}
// Wait ~30s for all to finish, then download in a loop
```

### API Endpoints (Behind the UI)

All use browser session cookies for auth.

- **Create report:** `POST /reports/apis/common/ql`
- **List activity reports:** `POST /reports/apis/dlog/ql`
- **Check report status / download:** `POST /reports/apis/common/ql`
- **Save field template:** `POST /reports/apis/common/ql` (action: `createTemplate`)
- **Get favorites:** `GET /reports/apis/favourites/get`

### Date Format

- CSV dates: `MM/DD/YYYY` (e.g., `02/20/2025`)
- CSV times: `HH:MM:SS` with timezone column (PST/PDT)

**Balance Impact values:** Credit, Debit, Memo

**Type values observed:** PreApproved Payment Bill User Payment, General Card Deposit, General Authorization, Mobile Payment, Express Checkout Payment, Website Payment, Payment Reversal, Shopping Cart Item, etc.

## Other Report Types (Not Yet Explored)

The Reports page (`/reports/`) has additional report types under different tabs:

### Payments tab

- **Balance report** — Detailed information about balances, transactions, fees and refunds
- **Statements - monthly and custom** — Consolidated view with categorized record types for reconciliation
- **Transaction details report** — Complete details of each transaction including fees and taxes

### Risks tab

- (Not explored)

### Tax tab

- (Not explored)

These may contain data not available in the Activity report.

## Activity Page (Individual Transactions)

URL: `https://www.paypal.com/myaccount/activities/`

Default shows last 90 days. URL params control filtering:

```
?free_text_search=&start_date=2025-11-11&end_date=2026-02-09&type=&status=&currency=
```

### Transaction Detail View

Click any transaction to expand inline. Shows:

- Paid with (card name, last 4, card statement descriptor e.g. "PAYPAL \*QUESTDIAGNO")
- Transaction ID
- Seller info (full business name)
- Invoice ID
- Order summary (purchase amount, shipping, tax, total)
- Actions: Split Payment, Request Refund, Report a problem

### Activity Widget API

`GET /myaccount/activities/widget?activity_context_data=...`

## Gotchas & Notes

1. **12-month limit per report** - Must create multiple reports for full history. Use non-overlapping 12-month windows (e.g., Feb 10 2024 - Feb 9 2025).
2. **Calendar overlay blocks To field** - When setting From date, a calendar appears that covers the To input. Press Escape to dismiss it before clicking the To field.
3. **Reports auto-poll** - The page automatically polls `POST /reports/apis/common/ql` every ~3-10s to check report status. No need to manually refresh.
4. **Date range validation is exact** - 13 months returns "Range is invalid". Keep to exactly 12 months.
5. **Empty periods return header-only CSV** - Periods with no activity still return a valid CSV with just the header row (641 bytes).
6. **"Since last download" preset** - Available as a quick option, tracks where you left off.
7. **File suggested name is always "Download.CSV"** - Rename after saving to avoid overwrites.
8. **Reports stored up to 12** - Old reports get pushed out when creating new ones.
9. **BOM in CSV** - Files start with UTF-8 BOM (`\xEF\xBB\xBF`).
10. **Customize fields is persistent** - Once you save the all-fields template, future reports will use it automatically.
11. **Session can expire during long operations** - When creating/downloading many reports, the session may time out. Save storage state periodically.
12. **Monthly statement ZIP naming** - The suggested download filename is `statement-{year}.zip`. Be careful with `saveAs` to avoid double-naming.

## What Didn't Work / Not Explored Yet

- Direct API scraping (haven't reverse-engineered the report creation/download API request/response payloads)
- Invoices page (not explored)
- Subscription/recurring payment details
- Dispute/resolution center data
- Tax documents
- Balance report / Transaction details report (different report types on /reports/)
- Monthly statement download timed out on "Download all" for 2024/2023 — may need to download individual months instead, or the session had expired
