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

## 2026-07-02 Audit Findings

From a multi-agent review of soundness, transfers, categorization, reporting,
scraping UX, and the GUI. Ranked within each subsection.

### Correctness (do first)

- Extraction's read-modify-write of `account.journal` takes no login lock
  (`lib.rs` `run_login_account_extraction_blocking`, CLI extract path). A
  concurrent post/unpost (GUI or `account post-all` in another process) can be
  silently reverted, leaving a dangling `posted:` ref that
  `consistency::recover_ledger` cannot auto-repair. Violates the documented
  `fs_atomic` contract ("callers must serialize writes to the same path").
- `merge_gl_transfer` guardrails: reject multi-source blocks (currently takes
  only the first `; source:` tag, corrupting the other entry's `posted:` ref),
  reject split txns, check `gl_txn_removal_blockers`, require opposite signs.
  It also performs no amount validation, so fee-differing pairs merge with the
  fee silently vanishing (leg 2 is forced to exact negation).
- Blocker checks on `recategorize_gl_transactions` and `sync_gl_transaction`:
  both can rewrite reconciled/soft-closed txns; recategorize can rewrite any
  posting index including the bank-account leg.
- Per-leg (`posted_postings`) posts are outside the drift/sync safety net:
  dedup can mutate their amount, drift detection only checks `entry.posted`,
  and sync errors on posting-indexed sources — GL goes permanently stale with
  no signal.
- Remove or guard the legacy ScrapeTab post paths (`post_entry`,
  `unpost_entry`, `post_transfer`): no locks, no git commit, and legacy
  `unpost_entry` has no blocker check.
- One-liners: finalize staged scrape downloads on failure (a run that fails on
  statement 12 of 12 currently discards the other 11; `debug exec` already
  finalizes on failure); skip `Expenses:Unknown` in
  `build_training_examples` (the login-level classifier currently learns
  "Unknown" as a category); block the glued `-fPATH` form in `report.rs` arg
  validation; raise an anomaly when coverage info is missing instead of
  silently disabling the disappearance safety net.
- Still-open architectural items from the June review: recategorize/merge log
  no operation (ops log remains write-only; `docs/operation-log-redesign.md`
  unimplemented), no undo/redo, positional `file:row:col` evidence refs go
  stale on row reorder, `Expenses:Unknown` doubles as account and sentinel.

### Automation actually running (the labor multiplier)

- Wire `apply_automation_policy` into the posting paths (auto-ETL, both Post
  All buttons, CLI `post-all`). It is exposed as a Tauri command but called
  from no UI component — every proposal requires a manual per-row Apply click.
- Make category resolutions predicate-based (merchant/amount rules) instead of
  entry-id-bound single-use; offer one-click "always do this for <payee>" when
  the user recategorizes. (See "Auto-Categorization Rules" above — this is the
  same item; the resolution storage/fingerprint machinery in `automation.rs`
  can host it.)
- Emit recategorize proposals from ML suggestions in `gl_proposals` (currently
  `suggestion.suggested` is dropped; only `transfer_match` is consumed).
- "Accept all N suggestions" bulk action in Transactions (per-row suggestions
  and bulk-bar tallies are already computed).
- Payee normalization (strip store numbers, city suffixes, "PURCHASE
  AUTHORIZED" boilerplate) shared by the tokenizer and the rules layer.
- Decay or drop the compiled-in seed examples once real history exists
  (`COSTCO → Expenses:Shopping` is hardcoded and never decays); confidence
  tiering with an auto-apply threshold instead of the flat 0.5 abstain.

### Transfers

- Show near-miss candidates instead of exactly-one-or-nothing: 2+ candidates
  in the window currently return None with no indication a match existed.
- Pre-filter and rank the Transactions-tab Link Transfer modal by opposite
  amount/date proximity (`transfer_candidate_score` already exists); it
  currently lists all posted history unsorted, and its `amt:` search prefix is
  undocumented.
- Not-a-transfer negative memory: unposting a false-positive auto-transfer
  does not stop it being re-detected and re-posted on the next Post All (and
  the Pipeline path saved a transfer-link resolution asserting the wrong pair).
- Fee-tolerant merge writing both real legs plus an explicit fee posting
  (gives `TransferSplit` its implementation).
- One-click Unmerge/Unpost on transfer rows (backend `unpost` is already
  transfer-aware; the only GUI unpost is typing an entry ID in ScrapeTab).
- Record transfer resolutions from the auto-ETL and CLI paths too (only the
  Pipeline UI path saves one today).
- Configurable date window and user-extensible transfer description patterns
  (both are hardcoded: ±3 days, uppercase substring list).

### Reports (plumbing exists; presentation missing)

- Period presets (This month / Last month / YTD / Last 12 months) and canned
  one-click reports (Spending by category, Income vs expense, Net worth trend)
  in ReportsTab — frontend-only over the existing `run_hledger_report`.
- Saved report configs + an Overview/dashboard landing panel (also covers the
  "Net Worth Dashboard" item above).
- Data-quality banner: "N transactions ($X) still Expenses:Unknown in this
  period" with a jump-to-categorize link.
- Budgets via hledger periodic transactions and `balance --budget` (cheapest
  credible answer to the Budgets section above).

### Scraping UX

- Live log pane: wire the existing `debug_output_sink` into
  `run_scrape_for_login` and forward frames as Tauri events (driver logs
  currently go to stderr the GUI user never sees).
- Real cancel for an in-flight scrape (the banner "Skip" only clears the
  queue); mirror `cancel_exec_task` from the debug server.
- Failure artifacts: on driver error, capture URL + `page.screenshot()` + log
  tail before closing the browser; link from the scrape-log table.
- MFA prompt robustness: `rx.recv()` has no timeout, so an unattended
  auto-scrape that hits MFA hangs forever holding the login lock; add timeout,
  OS notification, and a way to re-open a dismissed prompt.
- GL row → evidence click-through: a resolver command from a GL txn's
  `source:` comment to `{login, label, entryId, document, row}`; make
  Transactions-tab evidence chips navigate to the Pipeline evidence-row viewer
  (every hop already exists as data).
- Per-login scrape console (last success from `scrape-log.jsonl` instead of
  `localStorage`, lock state, run/cancel); longer-term a backend scheduler so
  scraping doesn't depend on the app sitting open.
- Per-account (label) scrape granularity; resumable runs by unifying normal
  scrapes with the debug-server session model.

### GUI cleanup

- Delete the ScrapeTab legacy pipeline (~450 lines: documents table + Run
  extraction, posting queue, Use as A/B transfer form, unpost-by-typed-ID) —
  superseded by the Pipeline tab; keep the Tauri commands for the CLI. Also
  delete Pipeline's "GL Rows" sub-tab (renders the same `TransactionsTable` as
  the Transactions tab).
- Split ScrapeTab into a slim run-and-observe screen and a
  Settings/Connections screen (login CRUD, extensions, GL mappings, secrets,
  migrations); developer tools (debug socket, Load unpacked) behind a dev-mode
  pref.
- Surface silent failures: recategorize / merge transfer / bulk recategorize
  swallow errors into `console.error` with no busy state — the most-used
  actions in the app are the only silent ones. One shared status/banner
  pattern instead of ~10 per-tab status lines (ScrapeTab alone has five, with
  post/unpost feedback at the very bottom of the page; `pipelineStatus`
  renders twice).
- Label destructive one-click chips ("Merge as transfer: …" instead of a bare
  ↔ chip; suggestions commit on click) and add confirm-or-undo; shared Modal
  component with Escape-to-close (no modal closes on Escape today); the MFA
  overlay blocks the entire app from a background timer.
- Make "Post" name its destination ("Record N as Uncategorized"); rename one
  of the two same-label different-scope "Post All (N)" buttons on the Pipeline
  tab; Pipeline default sub-tab should be the review queue, not Evidence; drop
  the redundant account dropdown (the status table already selects).
- Collapse the permanent "Ledger workspace" onboarding header to a one-line
  title bar once a ledger is open; clear the stale `openStatus` line.
- Ledger-wide "needs review" inbox (proposals + anomalies + unposted across
  all accounts) instead of one-account-at-a-time Pipeline review; hoist
  `refreshPipelineLoginAccountData` out of the per-entry Post All loop.
- Transactions: filter preset pills (Needs review / Unposted / Transfers /
  All) compiling to hledger queries, raw query as Advanced; move manual entry
  behind an "+ Add" button; visible similar-group/bulk affordances instead of
  right-click-only; name Recategorize tabs by content instead of an internal
  counter (or make them a drawer).
- Terminology pass with a glossary: entry vs transaction vs row; one word for
  proposal/decision/resolution; Retire/Remove/Delete/Dismiss by semantics;
  "Bank account" label vs "Select a source..." placeholder; "posted" vs "bank
  posted" chips; humanize raw enum values and UUIDs shown in tables
  (`missing-extractor`, `duplicate-import-repair-skipped`, policy/reversible
  columns); rename/reorder tabs for humans (Bookkeeping is slot #2 but is the
  rarest surface).
- CSS debt (user-visible): ~18 referenced classes are never defined (ReportsTab
  is effectively unstyled; `secondary-button` falls back to the Vite default
  button); strip Vite template CSS from `index.css` (global purple-hover
  button, dark `:root`) and finish or remove the half-implemented dark mode;
  introduce `:root` tokens; unify the four chip systems and five same-yellow
  meanings; one account-picker widget (`AccountInput`) everywhere; dedupe the
  twice-implemented AccountsTable / lightbox / bulk-confirm modal /
  formatTotal / autocomplete logic into shared modules.
- Bookkeeping: replace raw-UUID textareas and Left/Right ref inputs with
  pickers; humanize enum options ("Settlement" not "settlement-link"); label
  the tab as advanced ("Reconcile & Close").
- Reports: `type="date"` inputs (other tabs already use them); plain-language
  option labels with the hledger flag as tooltip; command selector styled
  distinctly from the top-level nav tabs; default report on first open; stale-
  result hint when options change after a run.
- Accounts tab as a real home screen: rename the misleading "Extraction"
  column, add net-worth total, per-login last-synced freshness, an
  Expenses:Unknown count chip, and a per-login "Sync now" button.

## Provident CU

Should go to Payment Center (https://accountmanager.providentcu.org/ProvidentOnlineBanking/billpay/Payment-Center.aspx?#/), look through Payment Activity, click each row's Search icon, scrape each field including Check Number, Send Date, Estimated Delivery Date, Payee.

## Major Version Cleanups

### Remove legacy secret fallback

- Remove `ENABLE_LEGACY_SECRET_FALLBACK` and legacy keychain resolution in
  `src-tauri/src/scrape/js_api.rs`.
- Remove no-longer-needed legacy keychain helper APIs after cutover.
- Remove `migrate_login_secrets` once the migration window closes.
- Remove legacy-credentials migration UI affordances.
- Add release notes for the breaking change (legacy secret format unsupported).
