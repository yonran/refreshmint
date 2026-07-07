# TODO

## Chip-action safety residuals (accepted from the 2026-07-07 feedback-safety batch)

- Undo-of-categorize is a blind overwrite: runUndoPlan recategorizes the row back
  to its captured old account without checking whether anything mutated that GL
  posting in the ~10s undo window. An out-of-band edit landing before the user
  clicks Undo is silently overwritten. Accepted for now (narrow window, single
  user); a compare-and-swap on the posting's current account would close it.
- Stale Active TransferLink after undoing a Pipeline-created transfer post: undo
  reverts the GL block but does not retract the TransferLink resolution the
  Pipeline post created, leaving an Active link with no backing transfer.
- Surrogate-pair truncation nit in mergeTransferChipLabel: the 40-char slice can
  split an astral-plane grapheme (emoji), emitting a lone surrogate before the
  ellipsis. Cosmetic; switch to a code-point-aware slice if it ever bites.
- Per-account "Post All" label ambiguity when two logins share a label: the
  button names only the label, so two logins with the same account label render
  indistinguishable buttons. Qualify with the login name when labels collide.

## Scraping UX (deferred from the 2026-07-07 review-fixes batch)

- Scrape-output buffering while ScrapeTab is unmounted: the live console listener
  is torn down when the tab unmounts, so driver lines emitted while the user is
  on another tab are lost. The running/status state now survives the unmount, but
  buffering the output stream (e.g. at the App level, or via a backend replay)
  would let the console catch up on remount.
- Evidence / artifact click-through silent error paths: several artifact and
  evidence actions swallow errors into a status string or nothing; surface them
  as a toast instead.
- `resolve_artifact_dir` `..` over-rejection: the guard rejects any path
  containing `..` as a substring, which would also reject a legitimately-named
  component that merely contains `..`. Tighten to reject only true parent
  (`ParentDir`) components once such names are actually needed.

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

- ✅ User-defined rules (regex on description/payee → account mapping) —
  `ResolutionKind::CategoryRule` (see "Automation actually running" below).
- ✅ Rules applied automatically during pipeline review before ML suggestions —
  rules post directly / get policy `Auto`; ML stays `Review`.
- Rule management UI beyond the existing Saved-decisions enable/disable table
  (dedicated create/edit/prioritize/delete screen) — not yet built.
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

### Correctness (do first) — DONE 2026-07-02

The whole "do first" batch shipped as one commit per fix (branch: `main`):

- ✅ Extraction now holds the per-login lock across its read→dedup→write in
  both the GUI (`run_login_account_extraction_blocking`) and CLI
  (`run_account_extract`) paths (`4a22da5`). Note: `migration.rs` still writes
  `account.journal` unlocked — deliberately deferred (one-shot maintenance
  path; see below).
- ✅ `merge_gl_transfer` guardrails: rejects multi-source blocks, split txns,
  blocker-protected txns, and non-cancelling amount pairs (`335f0e5`). The
  amount guard uses the existing f64 + 0.005-epsilon convention (kept
  deliberately; a decimal refactor is a separate cross-cutting item).
- ✅ Blocker check added to `sync_gl_transaction` (`d298665`); bank-leg
  (Assets:/Liabilities:) guard added to `apply_recategorizations`, mirroring
  the frontend rule with reciprocal cross-links (`307cddd`). Per the audit
  decision recategorize is a leg-guard only — counterpart edits on reconciled
  txns stay legal (no blocker check there).
- ✅ Per-leg (`posted_postings`) amount drift is now surfaced as a new
  `PostedLegAmountDrift` import anomaly in the dedup update arms (`474db42`).
  The amount is still updated (bank data is truth) and sync still cannot
  process posting-indexed sources — but the staleness is now visible instead of
  silent. A deeper fix (teach sync/drift about posting-indexed sources) is
  still open.
- ✅ Legacy mutation paths deleted: `post_entry`/`unpost_entry` and their
  wrappers/handlers/TS stubs are gone; `post_transfer` kept for the CLI but its
  Tauri command + the ScrapeTab "Transfer posting" form removed (`2b71f8e`).
- ✅ One-liners: finalize staged scrape downloads unconditionally, sharing
  `combine_run_and_finalize` with `debug exec` (`0b0ed7d`); skip
  `Expenses:Unknown` in `build_training_examples` (`26ea961`); block glued
  `-f`/`-o` short-flag forms in `report.rs` validation (`a1738e3`); raise a
  `CoverageInfoMissing` anomaly when a document has no coverage info
  (`2c3c523`).

Stability fixes surfaced by the batch (also `main`):

- ✅ `login_config::acquire_lock_file` retries briefly on transient EWOULDBLOCK
  (`f371e07`, reverted and reinstated with the real root cause in `642e35b`:
  fork/posix_spawn duplicate open lock fds into children until exec, so a
  subprocess spawned by any thread briefly keeps a just-released flock alive —
  verified experimentally; see docs/locking.md "Acquisition Semantics");
  `secret::test_login` disambiguated with an atomic counter (`2b7c8f0`).

Review follow-ups (2026-07-02 four-agent review of the batch; all on `main`):

- ✅ Orphaned `postTransfer` TS stub deleted (`f3f37ca`); merge now also
  requires same-commodity, parseable amounts (`f464f5d`); per-leg drift
  anomaly compares every leg, not just the primary (`50558cb`); extraction
  validates the login exists before locking, so a typo'd login no longer
  creates a phantom `logins/<name>/` dir (`206b9c4`); doc/comment debris and
  the `setSessionMetadata` coverage-ordering constraint documented
  (`eb7bdb6`).

Review follow-ups (2026-07-03 four-agent review of the automation-multiplier
batch; all on `main`):

- ✅ HIGH: `RecategorizeGl` now targets the posting that IS `Expenses:Unknown`
  rather than blindly the last posting, fixing an empirically reproduced
  policy-loop infinite loop + split corruption on manual txns with Unknown in a
  non-last position (`24f2dd1`).
- ✅ CLI `post-all`'s policy pass now drains entry-bound Auto proposals
  (`PostCategory`/`LinkTransfer`) by scoping per `(login, label)` plus a GL pass,
  instead of the dead `{login: Some, label: None}` scope (`9f6f549`).
- ✅ `automation-utils.ts` dedup key no longer embeds a raw NUL byte, so git
  treats the file as text again (`0151787`).
- ✅ Bulk recategorize refreshes + prunes even when standing-rule creation fails
  (was silently stale); rule-creation failures now surface in a banner
  (`3e8b433`).
- ✅ Hardening: payee boilerplate-prefix stripping honors a word boundary
  (`CHECKCARDIO GYM` no longer becomes `IO GYM`); GL scoped-rule load errors
  propagate instead of silently dropping all rules; `amountMin`/`amountMax`
  validated (numeric, ordered) at rule creation (`0c33cfd`).
- ✅ Frontend polish: batch "Accept N suggestions" has an in-flight guard +
  error surface; Pipeline one-click "Always" confirms against the NORMALIZED
  payee before saving the standing rule (`495c713`).
- ✅ Rule proposals for a possible transfer leg (asymmetric transfer-uniqueness)
  are downgraded from Auto to Review so a human decides, preventing an
  auto-expense that would strand the other side's `MergeGlTransfer` (`cb4d8a8`).

Still-open architectural items from the June review (NOT in this batch):
recategorize/merge log no operation (ops log remains write-only;
`docs/operation-log-redesign.md` unimplemented), no undo/redo, positional
`file:row:col` evidence refs go stale on row reorder, `Expenses:Unknown`
doubles as account and sentinel. Plus follow-ups spun off from this batch:
✅ `migration.rs` now locks the ledger for the whole `migrate_ledger`
(`ad86372`); still open: sync/drift ignorance of posting-indexed sources;
f64→decimal for money comparisons.

### Automation actually running (the labor multiplier)

Shipped in the automation-multiplier batch (2026-07-02):

- ✅ Wire `apply_automation_policy` into the posting paths (auto-ETL, both Post
  All buttons, CLI `post-all`). It ran nowhere before; now every batch-post runs
  the policy loop once and drains Auto proposals. (`cef99c3`)
- ✅ Make category resolutions predicate-based (merchant/amount rules) instead of
  entry-id-bound single-use — new `ResolutionKind::CategoryRule` with a
  regex/normalized-payee/amount predicate (`a12f076`), surfaced through
  suggestions (`0a1c8f9`) and direct-posted at post time (`bc4ddb6`); one-click
  "Always: <account>" and a bulk "Create rule" checkbox (`12b491c`).
- ✅ Emit recategorize proposals from ML suggestions in `gl_proposals` — a rule
  match is Auto, the ML `suggested` (previously dropped) is Review. (`147cb25`)
- ✅ "Accept all N suggestions" bulk action in Transactions. (`b83e82c`)
- ✅ Payee normalization (strip store numbers, city suffixes, "PURCHASE
  AUTHORIZED" boilerplate) shared by the tokenizer and the rules layer —
  `payee_normalize` module (`2bf30e2`), tokenizer normalization (`5ac28cd`).

Still open:

- Decay or drop the compiled-in seed examples once real history exists
  (`COSTCO → Expenses:Shopping` is hardcoded and never decays); confidence
  tiering with an auto-apply threshold instead of the flat 0.5 abstain.

### Transfers

- ✅ Near-miss candidates: 2+ in-window matches now populate
  `transferCandidates` (date-proximity order) on both suggestion levels instead
  of silently returning None; the same commit stops same-login-account GL pairs
  (refund/charge) matching as self-transfers (`a4f0420`).
- ✅ Transactions-tab Link Transfer modal ranked: empty search pre-filters to
  cancelling amounts within ±14 days sorted by date proximity; the `amt:`
  search prefix is advertised in the placeholder; near-miss rows get a
  non-destructive "N possible" chip that opens the modal (`cb06cef`).
- ✅ Not-a-transfer negative memory: new `not-transfer-link` resolution kind +
  `TransferPolicy` filter inside both matchers and the proposal arms, mutually
  exclusive with `transfer-link` (`7ecb793`); unposting a merged transfer
  auto-records it, killing the re-post loop (`386a7f5`); explicit "Not a
  transfer" actions in Pipeline and Transactions (`8957566`).
- ✅ Fee-tolerant merge/post/sync: an explicit fee account writes both real
  legs plus a fee posting with all amounts explicit, balancing by construction;
  sync preserves and recomputes the fee leg (`c08d18d`). `TransferSplit`
  remains planned vocabulary — fee merges do not use it.
- ✅ One-click Unmerge on transfer rows: `unpost_gl_transaction` resolves the
  source entry server-side from the GL txn id; Transactions context menu gains
  "Unmerge transfer" with confirm + error banner (`8957566`).
- ✅ Transfer resolutions recorded from the auto-ETL and CLI post-all paths
  (idempotent via fingerprint dedup) (`9deea61`).
- ✅ Configurable `transferDateWindowDays` and `extraTransferPatterns` in
  `refreshmint.json`; the cancel epsilon is a shared named constant
  (`3aac617`).
- Same-journal unpost clobbers the other leg's ref (pre-existing, verified at
  `900caac~1`). Unposting a transfer whose two legs live in the SAME account
  journal restores the other leg's cleared `posted` ref from a stale snapshot:
  `post.rs` reads the triggering journal (`unpost_login_account_entry`, ~:426)
  before `write_other_sides` clears the other side, then writes that stale copy
  back over it, leaving a dangling ref. Fix by re-reading (or merging) the
  triggering journal after the other-side writes when the paths coincide.

### Reports

Done in the 2026-07-07 Reports batch (starting `ec8bfa5`):

- Period presets (This month / Last month / YTD / Last 12 months) and canned
  one-click reports (Spending by category, Income vs expense, Net worth trend,
  Budget vs. actual) in ReportsTab — frontend-only over `run_hledger_report`.
  (`798e96c`, `9bd1e2f`, budget canned report in this batch)
- Data-quality banner: "N transactions ($X) still Expenses:Unknown in this
  period" with a jump-to-categorize link. (`9575968`)
- Budgets via hledger periodic transactions and `balance --budget`, reading a
  user-owned `budget.journal` next to `general.journal`. (`317b07b` plus the
  budget canned report/checkbox in this batch; see docs/budgets.md)

Still deferred:

- Saved report configs (named, reloadable) + an Overview/dashboard landing
  panel (also covers the "Net Worth Dashboard" item above).
- CSS overhaul of ReportsTab (the batch left the ~18 undefined helper classes
  as-is; see the CSS-debt item below).

Deferred from the 2026-07-07 Reports review-fixes batch:

- Expenses:Unknown banner session persistence: the banner is intentionally
  component-local (recomputed after each run, cleared on session adopt), so it
  vanishes on tab switch until the next run. Persisting it in the session is a
  possible future refinement.
- Auto-run when a new ledger opens while Reports is already mounted: today the
  auto-run is mount-only, so opening a ledger without leaving the tab does not
  re-trigger the default report.
- Comma-decimal commodity styles in `summarizeUnknownRegister` (it strips `$`
  and `,` as a thousands separator; a `,`-decimal locale would misparse — only
  theoretical for the $-denominated ledgers in use).

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

- ~~Delete the ScrapeTab legacy pipeline (~450 lines: documents table + Run
  extraction, posting queue, Use as A/B transfer form, unpost-by-typed-ID) —
  superseded by the Pipeline tab; keep the Tauri commands for the CLI. Also
  delete Pipeline's "GL Rows" sub-tab (renders the same `TransactionsTable` as
  the Transactions tab).~~ Done: legacy pipeline `a9eb96e`, GL Rows sub-tab
  `e193498`.
- ~~Split ScrapeTab into a slim run-and-observe screen and a
  Settings/Connections screen (login CRUD, extensions, GL mappings, secrets,
  migrations).~~ Done: Settings shell (Extensions/Migrations/Secrets/Developer)
  `6e08353`, Connections + slim Scrape `ea09f82`. Still TODO: put developer
  tools (debug socket, Load unpacked) behind a dev-mode pref — they currently
  live in a "Developer tools" disclosure on the Settings tab.
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
  tab; ~~Pipeline default sub-tab should be the review queue, not Evidence~~
  (done `3f497c5`); drop the redundant account dropdown (the status table
  already selects).
- ~~Collapse the permanent "Ledger workspace" onboarding header to a one-line
  title bar once a ledger is open; clear the stale `openStatus` line.~~ Done
  `45ea73f`.
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
- Reports: `type="date"` inputs (`798e96c`); plain-language option labels with
  the hledger flag as tooltip and a distinctly-styled command selector
  (`a7467ed`); default report on first open (`41ee2d5`); stale-result hint when
  options change after a run (`9573610`). [done in the 2026-07-07 Reports batch]
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
