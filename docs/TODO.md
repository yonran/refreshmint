# TODO

## Receipts And Retail Attachments

### Product model

We currently treat retail receipts as attachment-only evidence. Future work needs
to decide whether receipts should remain evidence-only or evolve into one of
these richer models:

- Receipt-enriched bank transactions
- Receipt-created expense transactions
- Explicit purchase/payment/fulfillment/refund event modeling

### Date semantics

If receipts become more than attachments, define and preserve separate meanings
for:

- Purchase date
- Fulfillment date
- Payment or capture date
- Refund date
- Statement or posting date

Future implementations should decide which of these dates drives ledger entries
under each product mode.

### Matching policy

We need a clear matching policy between receipt artifacts and imported bank/card
transactions, including:

- allowed date drift
- allowed amount drift
- split shipments and split captures
- refunds and partial refunds
- double-count prevention when both receipts and bank imports exist

### Extraction shape

Future receipt extraction needs decisions on:

- whether receipts should ever create transactions directly
- whether extraction should operate at purchase level or item level
- whether taxes, discounts, shipping, and fees should become explicit fields or
  postings
- whether receipt images remain evidence-only attachments

### Stable metadata contract

The phase-1 attachment metadata contract should stay additive. Current stable
keys for receipt-style attachments are:

- `attachmentKey`
- `attachmentType`
- `purchaseDate`
- `sourceKind`
- `attachmentPart`
- provider-specific IDs such as `targetOrderId`

Future changes should add metadata keys rather than renaming or repurposing
these.

### Returns and refunds

We still need a product decision for:

- whether return receipts are separate attachment groups
- whether refunds should be modeled as separate events
- how returned items should link back to original evidence

### Online vs in-store normalization

Retail providers often expose different levels of detail for online and
in-store purchases. We need to decide whether all retail receipt sources should
normalize to one shared schema or preserve source-specific differences where the
data is materially different.

## Features Missing vs. Comparable Apps

Compared to apps like YNAB, Monarch Money, Copilot, Mint, Empower, and Firefly III.

### Budgets

- Period budgets (monthly/quarterly/annual spending limits per category)
- Zero-based / envelope budgeting mode
- Budget-vs-actual report view in the UI (beyond raw hledger `balance`)
- Rollover support (carry unspent amounts to next period)
- Overspending warnings

### Auto-Categorization Rules

- User-defined rules (regex on description/payee → account mapping)
- Rules applied automatically during pipeline review before ML suggestions
- Rule management UI (create, edit, prioritize, delete rules)
- This complements the existing ML suggestions which require manual confirmation

### Net Worth Dashboard

- Overview page showing total assets, liabilities, and net worth
- Net worth trend chart over time
- Account balance summary across all logins

### Savings Goals

- Define a goal (target account, target amount, target date)
- Track current balance / progress toward the goal
- Link a goal to a specific hledger account or tag

### Recurring Transaction Detection

- Detect repeating transactions (bills, subscriptions) by amount+payee pattern
- Surface upcoming expected transactions
- Alert when a recurring charge is missed or amount changes

### Transaction Import (CSV / OFX)

- Import transactions from downloaded CSV or OFX/QFX files
- Useful for banks and institutions that cannot be scraped
- Map CSV columns to hledger fields via a configurable profile

### Export

- Export filtered transaction list to CSV
- Export hledger reports to PDF or CSV for sharing/tax preparation

### Split Transactions

- Split a single bank entry into multiple GL postings with different accounts and amounts
- Useful for mixed-purpose transactions (e.g., Amazon order with groceries + electronics)

### Alerts and Notifications

- Large transaction alert (over a configurable threshold)
- Overspending alert (category exceeds budget)
- Low account balance warning
- Unusual spending pattern detection

### Tax Tagging

- Mark individual transactions as tax-deductible (with category: home office, medical, charitable, etc.)
- Annual tax summary report grouping tagged transactions
- Capital gains tracking for investment accounts (long-term vs. short-term)

## Major Version Cleanups

### Remove legacy secret fallback

- Remove `ENABLE_LEGACY_SECRET_FALLBACK` and legacy keychain resolution in
  `src-tauri/src/scrape/js_api.rs`.
- Remove no-longer-needed legacy keychain helper APIs after cutover.
- Remove `migrate_login_secrets` once the migration window closes.
- Remove legacy-credentials migration UI affordances.
- Add release notes for the breaking change (legacy secret format unsupported).
