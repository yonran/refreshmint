# Chase Activity Download - Site Knowledge

## URL

Download dialog accessed from any account's activity page via the "Download account activity" button (download icon in the toolbar above the transactions table).

Direct URL pattern: `https://secure.chase.com/web/auth/dashboard#/dashboard/accountDetails/downloadAccountTransactions/index;params={acctType},{subType},{acctId}`

Examples:

- Savings: `params=DDA,SAV,375901131`
- Credit card: `params=CARD,BAC,335666184`

## How to Access

1. From the dashboard, click an account name (e.g. "CHASE SAVINGS (...6870)").
2. On the account activity page, find the toolbar above the transactions table with icons for: Search, Collapse rows, Print, **Download**.
3. Click the download icon button (`button "Download account activity"`). Test ID: `quick-action-download-activity-tooltip-button`.
4. A modal dialog opens with dropdowns for Account, File type, and Activity.

## Download Dialog Structure

### Account Dropdown

Lists all accounts. Can switch between accounts without leaving the dialog:

- CHASE SAVINGS (...6870)
- Freedom (...8354)
- Amazon Visa (...2074)

**Gotcha:** After completing a download and clicking "Download other activity", the account dropdown resets to savings regardless of which account was just downloaded.

### File Type Dropdown

- **Spreadsheet (Excel, CSV)** — default
- Quicken Web Connect (QFX)
- Quicken or Microsoft Money (QIF)
- QuickBooks Web Connect (QBO)

### Activity Dropdown

- **Current display, including filters** — default; downloads only what's currently shown in the table
- **All transactions** — downloads all available history (~2 years observed, back to Feb 2024 from Feb 2026)
- **Choose a date range** — allows custom start/end dates

### Warning for Credit Cards

When a credit card account is selected, a warning appears: "If you have more than 1,000 transactions to display, you'll need to create more than one report. Use the date range to narrow your results."

### Download/Cancel Buttons

- "Cancel" closes the dialog
- "Download" triggers the file download

### Success Page

After download, shows "Success" message with two buttons:

- "Download other activity" — returns to the download form (resets account to savings)
- "Go back to accounts" — returns to the account dashboard

## Format Comparison

### Savings (DDA) — Field availability by format

| Field                  | CSV                                            | QFX/QBO (OFX)                             | QIF                    |
| ---------------------- | ---------------------------------------------- | ----------------------------------------- | ---------------------- |
| Posting Date           | `Posting Date` (MM/DD/YYYY)                    | `DTPOSTED` (YYYYMMDD)                     | `D` (MM/DD/YYYY)       |
| Description            | `Description` (full text)                      | `NAME` (truncated) + `MEMO` (overflow)    | `P` (payee, truncated) |
| Amount                 | `Amount`                                       | `TRNAMT`                                  | `T`                    |
| Debit/Credit indicator | `Details` (CREDIT/DEBIT/DSLIP)                 | `TRNTYPE` (CREDIT/DEBIT)                  | —                      |
| Transaction subtype    | `Type` (MISC_CREDIT, FEE_TRANSACTION, DEPOSIT) | —                                         | —                      |
| Running balance        | `Balance` (per row)                            | `LEDGERBAL`/`AVAILBAL` (end-of-file only) | —                      |
| Check or Slip #        | `Check or Slip #` (usually empty)              | —                                         | `N` (always "N/A")     |
| Transaction ID         | —                                              | `FITID` (e.g. `202601260`)                | —                      |
| Routing number         | —                                              | `BANKID` (e.g. `322271627`)               | —                      |
| Full account number    | —                                              | `ACCTID` (e.g. `2915916870`)              | —                      |
| Account type           | —                                              | `ACCTTYPE` (SAVINGS)                      | `!Type:Bank`           |

### Credit Card — Field availability by format

| Field            | CSV                                 | QFX/QBO (OFX)                    | QIF              |
| ---------------- | ----------------------------------- | -------------------------------- | ---------------- |
| Transaction Date | `Transaction Date` (MM/DD/YYYY)     | —                                | —                |
| Post Date        | `Post Date` (MM/DD/YYYY)            | `DTPOSTED` (YYYYMMDD)            | `D` (MM/DD/YYYY) |
| Description      | `Description`                       | `NAME`                           | `P` (payee)      |
| Category         | `Category` (Chase's categorization) | —                                | —                |
| Type             | `Type` (Sale/Payment/Return)        | `TRNTYPE` (DEBIT/CREDIT)         | —                |
| Amount           | `Amount`                            | `TRNAMT`                         | `T`              |
| Memo             | `Memo` (usually empty)              | —                                | —                |
| Transaction ID   | —                                   | `FITID` (long unique string)     | —                |
| Account ID       | —                                   | `ACCTID` (e.g. `335666184-8354`) | —                |

### Key findings

- **CSV is the richest format overall.** It is the only format with Transaction Date (as distinct from Post Date), Category, per-row running balance (savings), and transaction subtype.
- **QFX/QBO add only FITID** (transaction ID for dedup), routing number, and full account number — all static metadata except FITID.
- **QFX and QBO are nearly identical** OFX/SGML. The only difference observed is `INTU.BID` (10898 in QFX vs 2430 in QBO).
- **QIF is the worst format.** It loses transaction IDs, account metadata, and memo fields. Chase's QIF export has a bug: "INTEREST PAYMENT" is rendered as "PINTEREST PAYMENT".
- **DTPOSTED = Post Date**, not Transaction Date. Verified across multiple credit card transactions where the two dates differ by 1-3 days.
- **No single format has everything.** CSV lacks FITID; QFX/QBO lack Transaction Date and Category. Downloading both CSV + QFX gives complete coverage.
- **Recommendation:** Download CSV for primary data, QFX for FITID-based deduplication.

### Description handling across formats

For long descriptions like "SAFE DEPOSIT BOX 741157 816825-0 ANNUAL RENT":

- **CSV:** Full text in one `Description` field
- **QFX/QBO:** Split across `NAME` ("SAFE DEPOSIT BOX 741157 816825-0") and `MEMO` ("ANNUAL RENT")
- **QIF:** Truncated to payee field only ("SAFE DEPOSIT BOX 741157 816825-0")

## CSV Format — Savings (DDA)

**Header:** `Details,Posting Date,Description,Amount,Type,Balance,Check or Slip #`

| Field           | Description                                  |
| --------------- | -------------------------------------------- |
| Details         | `CREDIT`, `DEBIT`, or `DSLIP` (deposit slip) |
| Posting Date    | `MM/DD/YYYY`                                 |
| Description     | Transaction description                      |
| Amount          | Positive for credits, negative for debits    |
| Type            | `MISC_CREDIT`, `FEE_TRANSACTION`, `DEPOSIT`  |
| Balance         | Running balance after transaction            |
| Check or Slip # | Usually empty                                |

Example:

```
CREDIT,01/26/2026,"INTEREST PAYMENT",0.01,MISC_CREDIT,1015.29,,
DEBIT,10/31/2025,"SAFE DEPOSIT BOX 741157 816825-0 ANNUAL RENT",-84.00,FEE_TRANSACTION,1015.26,,
```

## CSV Format — Credit Cards

**Header:** `Transaction Date,Post Date,Description,Category,Type,Amount,Memo`

| Field            | Description                                                                                                      |
| ---------------- | ---------------------------------------------------------------------------------------------------------------- |
| Transaction Date | `MM/DD/YYYY` — when the transaction occurred                                                                     |
| Post Date        | `MM/DD/YYYY` — when it posted to the account                                                                     |
| Description      | Merchant/transaction description                                                                                 |
| Category         | Chase's categorization: `Groceries`, `Shopping`, `Food & Drink`, `Health & Wellness`, `Home`, empty for payments |
| Type             | `Sale`, `Payment`, `Return`                                                                                      |
| Amount           | Negative for purchases, positive for payments/returns                                                            |
| Memo             | Usually empty                                                                                                    |

Example:

```
01/13/2026,01/15/2026,STARBUCKS STORE 03255,Food & Drink,Sale,-4.36,
12/11/2025,12/11/2025,AUTOMATIC PAYMENT - THANK,,Payment,188.54,
10/11/2025,10/12/2025,PAYPAL *HOME DEPOT,Home,Return,31.42,
```

## QFX/QBO Format (OFX/SGML)

Both QFX and QBO use the same OFX 1.02 SGML format. Structure:

```
OFXHEADER:100
DATA:OFXSGML
VERSION:102
...
<OFX>
  <SIGNONMSGSRSV1>...</SIGNONMSGSRSV1>
  <BANKMSGSRSV1>        ← savings
    <STMTTRNRS>
      <STMTRS>
        <BANKACCTFROM>
          <BANKID>322271627
          <ACCTID>2915916870
          <ACCTTYPE>SAVINGS
        </BANKACCTFROM>
        <BANKTRANLIST>
          <STMTTRN>
            <TRNTYPE>CREDIT
            <DTPOSTED>20260126120000[0:GMT]
            <TRNAMT>0.01
            <FITID>202601260
            <NAME>INTEREST PAYMENT
          </STMTTRN>
          ...
        </BANKTRANLIST>
        <LEDGERBAL><BALAMT>1015.29 ...
        <AVAILBAL><BALAMT>1015.29 ...
  <CREDITCARDMSGSRSV1>  ← credit cards
    <CCSTMTTRNRS>
      <CCSTMTRS>
        <CCACCTFROM>
          <ACCTID>335666184-8354
        </CCACCTFROM>
        <BANKTRANLIST>
          <STMTTRN>
            <TRNTYPE>DEBIT
            <DTPOSTED>20260115120000[0:GMT]
            <TRNAMT>-4.36
            <FITID>2026011524510726014103339367319
            <NAME>STARBUCKS STORE 03255
          </STMTTRN>
          ...
```

### FITID patterns

- **Savings:** Short date-based, e.g. `202601260` (YYYYMMDD + sequence)
- **Credit cards:** Long unique string, e.g. `2026011524510726014103339367319` (date prefix + account + sequence)
- **Payments:** Prefixed with `GEN`, e.g. `GEN20251211AUTOMATIC_PAYME00000`

## Downloaded File Naming

- **Savings:** `Chase{last4}_Activity_{downloadDate}.{ext}` (e.g. `Chase6870_Activity_20260202.CSV`)
- **Credit cards:** `Chase{last4}_Activity{startDate}_{endDate}_{downloadDate}.{ext}` (e.g. `Chase8354_Activity20240202_20260202_20260202.CSV`)

Extensions: `.CSV`, `.QFX`, `.QIF`, `.QBO`

Note: Playwright MCP renames files on save, replacing underscores with hyphens.

**Bug:** Amazon Visa QBO download was served with a `.CSV` extension instead of `.QBO`. The file contents were correct QBO/OFX format despite the wrong extension.

## What Worked

### Accessing the download dialog

```js
// From account activity page, click the download icon
await page.getByTestId('quick-action-download-activity-tooltip-button').click();
```

### Selecting options and downloading

```js
// Switch account
const acctBtn = page.getByRole('button', { name: /^Account,/ });
await acctBtn.click();
await page.getByRole('option', { name: 'Freedom (...8354)' }).click();

// File type is CSV by default; change if needed:
// const fileBtn = page.getByRole('button', { name: /^File type,/ });
// await fileBtn.click();
// await page.getByRole('option', { name: 'Spreadsheet (Excel, CSV)' }).click();

// Select "All transactions"
const activityBtn = page.getByRole('button', { name: /^Activity,/ });
await activityBtn.click();
await page.getByRole('option', { name: 'All transactions' }).click();

// Download
const [download] = await Promise.all([
    page.waitForEvent('download', { timeout: 15000 }),
    page.getByRole('button', { name: 'Download', exact: true }).click(),
]);
console.log(download.suggestedFilename());

// Click "Download other activity" to download next account
await page.getByRole('button', { name: 'Download other activity' }).click();
```

### Bulk download all formats for all accounts

```js
const accounts = [
    'CHASE SAVINGS (...6870)',
    'Freedom (...8354)',
    'Amazon Visa (...2074)',
];
const fileTypes = ['Spreadsheet (Excel, CSV)', 'Quicken Web Connect (QFX)'];

for (const fileType of fileTypes) {
    for (const account of accounts) {
        const acctBtn = page.getByRole('button', { name: /^Account,/ });
        await acctBtn.click();
        await page.getByRole('option', { name: account }).click();
        await page.waitForTimeout(500);

        const fileBtn = page.getByRole('button', { name: /^File type,/ });
        await fileBtn.click();
        await page.getByRole('option', { name: fileType }).click();
        await page.waitForTimeout(500);

        const activityBtn = page.getByRole('button', { name: /^Activity,/ });
        await activityBtn.click();
        await page.getByRole('option', { name: 'All transactions' }).click();
        await page.waitForTimeout(500);

        const [download] = await Promise.all([
            page.waitForEvent('download', { timeout: 15000 }),
            page.getByRole('button', { name: 'Download', exact: true }).click(),
        ]);
        console.log(download.suggestedFilename());

        await page
            .getByRole('button', { name: 'Download other activity' })
            .waitFor({ timeout: 10000 });
        await page
            .getByRole('button', { name: 'Download other activity' })
            .click();
        await page.waitForTimeout(1000);
    }
}
```

### Navigating to download dialog from dashboard

```js
await page.getByRole('button', { name: 'CHASE SAVINGS (...6870)' }).click();
await page.getByRole('heading', { name: /CHASE SAVINGS/ }).waitFor();
await page.getByTestId('quick-action-download-activity-tooltip-button').click();
```

## What Was Confusing / Failed

### 1. Account resets after each download

After clicking "Download other activity", the account dropdown resets to CHASE SAVINGS regardless of which account was previously selected. You must re-select the desired account each time.

### 2. "All transactions" covers ~2 years, not all history

The "All transactions" option returned data back to Feb 2024 (from Feb 2026). For older data, you may need to use "Choose a date range" or download statements instead.

### 3. Credit card CSV has two date columns; QFX only has Post Date

Credit cards provide both Transaction Date and Post Date in CSV. QFX/QBO/QIF only have DTPOSTED which equals the Post Date. The original Transaction Date is lost in all non-CSV formats.

### 4. Category field is CSV credit-card only

Savings CSV has no category. Credit card CSV includes Chase's own categorization (Groceries, Shopping, Food & Drink, etc.). Payments have an empty category. No other format includes categories.

### 5. HTML entities in descriptions

Some descriptions contain HTML entities, e.g. `GYROM &amp; KEBABS` instead of `GYROM & KEBABS`. These are not properly decoded in the CSV export.

### 6. QIF "PINTEREST PAYMENT" bug

Chase's QIF export renders "INTEREST PAYMENT" as "PINTEREST PAYMENT" — a corruption/truncation bug. The QIF format is strictly worse than all others.

### 7. Amazon Visa QBO served with wrong extension

The QBO download for Amazon Visa (...2074) was served with a `.CSV` filename extension despite containing valid OFX/SGML content. A Chase server-side bug.

## Merging CSV + QFX + PDF

When merging transaction data across formats, keep these in mind:

### HTML entity decoding

CSV and QFX descriptions contain HTML entities (e.g. `&amp;`, `&lt;`). PDF text is clean (plain `&`). Decode HTML entities in CSV/QFX descriptions before comparing against PDF text for matching.

### QFX MEMO field exists for all account types

Don't assume MEMO is empty for credit cards. Long descriptions are split across `NAME` and `MEMO` fields in QFX for both savings and credit card accounts. Always concatenate `NAME + " " + MEMO` (when MEMO is present) to reconstruct the full description.

### Matching strategy

- **CSV ↔ QFX:** Match by `(post_date, amount)` within the same account. When multiple transactions share the same date+amount, use normalized description as a tiebreaker.
- **PDF enrichment:** Match by `(transaction_date, amount)` for credit cards, `(post_date, amount)` for savings. Credit card PDFs show the purchase date (transaction date), not the post date — this is a critical distinction. See `statements-download.md` for more PDF parsing details.
