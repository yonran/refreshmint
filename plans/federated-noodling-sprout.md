# Plan: Account-Level Evidence and Reconciliation Pipeline

## Context

Currently, refreshmint extensions scrape raw data into a flat `output/` directory, but there is no pipeline from raw evidence → structured transactions → account journal → general journal. This plan adds:

- Organized scrape sessions with evidence provenance
- Per-account hledger journals (usually single-sided with `Equity:Unreconciled` counterpart, but pass-through transactions like Venmo bank-to-bank produce two unreconciled postings)
- A separate extraction step (raw → structured) that can be re-run without re-scraping
- Fuzzy dedup/merge for pending→finalized transactions
- Account→General reconciliation (auto or manual)
- Heuristic transfer detection for inter-account payments

---

## Design Principles: Provenance and Undo

The pipeline must support **removing a bad scrape** and **undoing reconciliation decisions** without corrupting the ledger.

### Provenance, not derivation

The account journal is ideally a function of (evidence documents + manual decisions), and the general journal is ideally a function of (account journals + reconciliation decisions). We store the journals as primary working state, but **also persist structured operations logs** (`operations.jsonl`) so that journals can be re-derived when algorithms are upgraded.

### Operations log

Each account and the GL have their own append-only operations log, since accounts are computed independently from each other:

- `accounts/<account-name>/operations.jsonl` — per-account human decisions (manual adds, dedup overrides, scrape removals)
- `operations.jsonl` (ledger root) — GL-level decisions (reconciliation, transfer matching)

```jsonl
# accounts/chase-checking/operations.jsonl
{"type":"entry-created","entryId":"txn-abc","evidence":["2024-02-17-transactions.csv:12:1"],"date":"2024-02-15","amount":"-21.32","tags":[["bankId","FIT123"]],"timestamp":"..."}
{"type":"manual-add","entryId":"txn-xyz","transaction":{...},"timestamp":"..."}
{"type":"dedup-override","action":"force-match","entryId":"txn-abc","proposedTxn":{...},"timestamp":"..."}
{"type":"dedup-override","action":"prevent-match","entryId":"txn-abc","proposedTxn":{...},"timestamp":"..."}
{"type":"remove-scrape","scrapeSessionId":"20240219-090000","timestamp":"..."}

# operations.jsonl (GL)
{"type":"reconcile","account":"chase-checking","entryId":"txn-abc123","counterpartAccount":"Expenses:Food","timestamp":"..."}
{"type":"transfer-match","entries":[{"account":"chase-checking","entryId":"txn-abc"},{"account":"amex-gold","entryId":"txn-def"}],"timestamp":"..."}
{"type":"undo-reconcile","account":"chase-checking","entryId":"txn-abc123","timestamp":"..."}
```

The logs are append-only: undos are recorded as new entries (e.g., `undo-reconcile`), not deletions.

**ID stability for re-derivation**: every entry creation (whether from extraction or manual add) is recorded as an `entry-created` op with the assigned UUID plus content fingerprint (evidence refs, date, amount, bankId). During re-derivation, when a new entry would be created, the system matches against unmatched `entry-created` ops by content fingerprint to recover the historical UUID. Matching rules:

- Match by `(evidence refs)` first (exact document + row), then by `(bankId)`, then by `(date, amount)` as fallback
- Each `entry-created` op is consumed at most once (one-time consumption prevents multiple re-derived entries from claiming the same UUID)
- When multiple `entry-created` ops match a single re-derived entry, prefer the one with the closest evidence ref match, then earliest timestamp as tiebreaker
- Unmatched `entry-created` ops after re-derivation are flagged as discrepancies (entries that existed before but not after the algorithm change)

Algorithmic operations (extraction, auto-dedup) are **not** in the log — they are re-runnable from the evidence documents using the current algorithm version. But entry ID assignments **are** logged (as `entry-created`) so IDs survive re-derivation.

### Re-derivation

When algorithms change, re-derivation works per-account then for the GL:

1. Run current extraction algorithm on all evidence documents for the account → proposed transactions
2. Run current dedup algorithm (cross-document only) → auto-matched groups
3. Apply decisions from `accounts/<name>/operations.jsonl` (manual adds, dedup overrides)
4. → produces account journal; compare against current → flag discrepancies
5. Apply reconciliation decisions from root `operations.jsonl`
6. → produces general journal; compare against current → flag discrepancies

### Provenance tags

Every journal entry also records inline provenance for readability and fast lookup:

- **Account journal entries**: `; evidence: 2024-02-17-transactions.csv:12:1` (CSV) or `; evidence: 2024-01-31-statement.pdf#page=3&viewrect=12%2C45%2C200%2C10` (PDF)
- **General journal entries**: `; source: accounts/chase-checking:txn-abc123` and `; generated-by: refreshmint-reconcile`

This enables:

- **Fast undo** without full re-derivation: follow provenance backward to identify and remove entries
- **Audit**: full chain from GL entry → account journal entry → evidence document
- **Discrepancy detection**: compare re-derived output against current journals

### Undo mechanics

- **Remove bad scrape**: identify documents from that scrape (via `scrapeSessionId` in `<doc>-info.json`); for each account journal entry referencing those documents:
    - **Sole evidence**: entry's evidence comes only from removed documents → delete the entry; cascade to any GL entries that reference it (unreconcile them, appending `undo-reconcile` to root operations.jsonl for each cascaded GL entry); append `remove-scrape` to per-account operations.jsonl
    - **Mixed evidence**: entry has evidence from both removed and kept documents → remove the evidence links from the removed documents; re-evaluate the entry by re-extracting from the remaining documents' evidence to determine correct status/amount; flag for review if the remaining evidence produces a different result
- **Undo reconciliation**: remove the GL entry; clear the `; reconciled:` tag on the account journal entry; append `undo-reconcile` to root operations.jsonl
- **Manual adds**: delete the account journal entry; append an undo entry to per-account operations.jsonl

### Evidence preservation

Removing a scrape or undoing a reconciliation does **not** delete evidence files. Documents remain in `accounts/<name>/documents/` for audit. Only journal entries are affected.

---

## Component Overview

### 1. Scrape evidence organization

**Goal:** Flat per-account `documents/` directory with per-document metadata sidecars, so all statements and downloads for an account are browsable in one place.

- Change output path from `extensions/<name>/output/` to `accounts/<account-name>/documents/`
    - `account-name` comes from `manifest.json` (a new required field `"account"`)
    - Extension code (driver.mjs, extract.mjs, etc.) stays in `extensions/<name>/`
- Filenames start with the **last date covered** by that document (statement date, or scrape date for current-activity snapshots): `<YYYY-MM-DD>-<original-filename>` (e.g., `2024-02-17-transactions.csv`, `2024-01-31-statement.pdf`). On collision, add an incrementing suffix (`2024-02-17-transactions-2.csv`). Scrape timestamp is not in the filename — it's in the `-info.json` sidecar's `scrapedAt` field.
- Each document gets a sidecar `<filename>-info.json`. The sidecar is written in its **final** form when the file is moved from staging to `documents/` (after extraction determines the coverage end date):

    ```json
    {
        "mimeType": "text/csv",
        "originalUrl": "https://...",
        "scrapedAt": "2024-02-17T12:00:00Z",
        "extensionName": "chase-checking-driver",
        "accountName": "chase-checking",
        "scrapeSessionId": "20240217-120000",
        "coverageEndDate": "2024-02-17",
        "dateRangeStart": "2024-01-01",
        "dateRangeEnd": "2024-02-17"
    }
    ```

    - `scrapeSessionId` groups documents from the same scrape run (for undo-by-scrape)
    - `dateRangeStart`/`dateRangeEnd` come from `refreshmint.setSessionMetadata()` if the driver called it
    - `coverageEndDate` is set by the driver via `saveResource({ coverageEndDate })`, or discovered by the extractor, or falls back to scrape date

- New JS API: `refreshmint.setSessionMetadata({ dateRangeStart, dateRangeEnd })` — stored in memory during the scrape; written into each `<doc>-info.json`
- `saveResource(filename, data, { coverageEndDate? })`: during scraping, files are saved to a staging area within the scrape session. If `coverageEndDate` is provided by the driver, the final filename will use it. Otherwise, the file waits for extraction to determine the coverage date. After extraction runs (which may discover the date by parsing content), files are moved to their final location at `accounts/<account-name>/documents/<coverageEndDate>-<filename>` (with incrementing suffix on collision) and the corresponding `-info.json` sidecar is written. If neither driver nor extractor provides a coverage date, the scrape date is used as fallback.

**Files to modify:**

- `src-tauri/src/scrape/js_api.rs` — add `setSessionMetadata` call; update `saveResource` to stage files and write final date-prefixed (coverage end date) filenames + sidecars after extraction
- `src-tauri/src/scrape/mod.rs` — resolve account name from manifest; generate scrape session ID
- `docs/scrapers.md` — document new API and path structure

---

### 2. Per-account journal

**Goal:** Each scrape account has its own `account.journal` in standard hledger format.

- Location: `accounts/<account-name>/account.journal`
- **Primary store**: entries are appended/updated incrementally (not regenerated). Provenance tags on each entry enable undo and discrepancy detection.
- **Usually single-sided**: one posting to the real account, counterpart `Equity:Unreconciled:<LedgerAccountName>`
- **Pass-through case** (e.g. Venmo bank-to-bank, where the Venmo balance is unchanged): two postings are both unreconciled — one to the bank account (`Assets:Checking`) and one to the expense/payee side (`Equity:Unreconciled:Venmo-Expense`). Both sides are subsequently reconciled separately.
- Transaction tags store evidence links: `; evidence: 2024-02-17-transactions.csv:12:1` (CSV) or `; evidence: 2024-01-31-statement.pdf#page=3&viewrect=12%2C45%2C200%2C10` (PDF with standard open parameters). Multiple evidence tags accumulate when multiple scrapes confirm the same transaction.
- Pending/finalized: hledger `!`/`*` status
- Multiple evidence links accumulate on one entry when multiple scrapes confirm the same transaction
- Each entry gets a random `; id: <uuid>` tag assigned at creation, used for reconciliation back-links, operations log references, and undo (not derived from content — see §4)
- `; extracted-by: <extension-name>:<algorithm-version>` tag enables discrepancy detection on algorithm upgrade
- Git-tracked (same repo as general.journal)

**New file:**

- `src-tauri/src/account_journal.rs` — read/write/update account journal entries, format evidence/id/provenance tags

---

### 3. Field extraction layer (separate step)

**Goal:** Decouple scraping (evidence capture) from extraction (structured transaction list).

Extension can provide either or both:

- **`extract.mjs`**: Script run in the same QuickJS sandbox; reads raw session files (HTML, CSV, PDF text) and calls a new API `refreshmint.reportExtractedTransaction(txn)`. The schema mirrors `hledger.rs::Transaction` fields (minus `tsourcepos`, which is reserved for position within the hledger journal file and gets set when the entry is written to `account.journal`):

    ```json
    {
        "tdate": "2024-02-15",
        "tstatus": "Cleared", // "Unmarked" | "Pending" | "Cleared"
        "tdescription": "SHELL OIL 12345",
        "tcomment": "",
        "ttags": [
            // HledgerTag = [name, value] pairs
            ["evidence", "2024-02-17-transactions.csv:12:1"], // REQUIRED: document:line:col
            ["bankId", "FIT123"], // optional: stable bank ID (OFX fitid, etc.) — used for dedup
            ["memo", "Purchase at Shell"] // optional
        ],
        "tpostings": [
            // optional; omit for default single-sided posting
            {
                "paccount": "Assets:Checking",
                "pamount": [{ "acommodity": "USD", "aquantity": "-21.32" }]
            },
            {
                "paccount": "Equity:Unreconciled:Venmo-Expense",
                "pamount": [{ "acommodity": "USD", "aquantity": "21.32" }]
            }
        ]
    }
    ```

    **Validation** (throws on violation):
    - Must have at least one `evidence` tag; the document name portion must match the input document being extracted
    - Evidence tag format varies by document type:
        - **CSV**: `<document>:<line>:<col>` — line is 1-indexed data row (extractor handles header offset), col is column index
        - **PDF**: `<document>#page=<pagenum>&viewrect=<left>%2C<top>%2C<wd>%2C<ht>` — standard PDF open parameters (per Adobe Acrobat spec); commas percent-encoded as `%2C` since hledger tags cannot contain literal commas; coordinates in PDF page units from pdf2json
    - When `tpostings` is omitted, the app generates the standard single-sided posting + `Equity:Unreconciled` counterpart. When present (e.g. Venmo pass-through), the explicit postings are used verbatim.
    - `bankId` is a tag (not a top-level field) — consistent with hledger's tag model. The dedup engine reads it from `ttags`.

- **`account.rules`**: Standard hledger CSV rules file (used via `hledger import`), for simple CSV-only cases. The rules file can name an `id-field` column to use as `bankId` when present in the CSV.

Extraction runs per-document (or per-scrape-session grouping of documents). By default, extraction runs automatically after scraping completes. This is configurable per-account via a `"autoExtract": true|false` field in the account's manifest (defaults to `true`). When auto-extract is off, the user triggers extraction manually from the UI.

**Output**: extraction produces proposed transactions that go through the dedup engine (§4) and are then committed to `account.journal` as new or updated entries. There is no intermediate `extracted.json` file — the account journal is the primary store, and each entry's provenance tags record what produced it.

**New API in `js_api.rs`:**

- `refreshmint.reportExtractedTransaction(txn)` — accumulates structured rows for the session
- `refreshmint.readSessionFile(relPath)` — read a raw evidence file from the current session

**New file:**

- `src-tauri/src/extract.rs` — orchestrate extraction: run `extract.mjs` or `account.rules`, collect proposed rows, pass to dedup engine, update account journal

**Manifest additions** (`manifest.json`):

```json
{
    "extract": "extract.mjs", // optional extraction script
    "rules": "account.rules", // optional hledger CSV rules
    "idField": "TransactionId", // optional: CSV column name to emit as bankId tag in ttags
    "autoExtract": true // default true; set false to require manual extraction trigger
}
```

---

### 4. Cross-document dedup / account journal updater

**Goal:** When extraction produces proposed transactions from a document, match them against existing account journal entries (from _other_ documents) to avoid duplicates and handle pending→finalized transitions.

**Critical constraint — no within-document merging**: transactions that appear as separate rows within the same document are **never** merged with each other. Two $50 Amazon charges on the same day in the same CSV are two distinct transactions. Dedup only operates across documents — matching a proposed transaction from document B against existing entries that came from document A.

**Same-document re-extraction idempotency**: when extraction is run on a document that was already extracted, proposed transactions carry the same evidence tags (e.g., `2024-02-17-transactions.csv:12:1`). If an existing entry already has that exact evidence tag, the proposed transaction is matched to it (update if changed, skip if identical) rather than creating a duplicate. This is "same-evidence" matching, distinct from cross-document fuzzy matching.

Algorithm per proposed transaction:

1. **Same-evidence match**: if an existing entry already references the same evidence (same document + row) → update in place if content changed; skip if identical
2. **Exact match by bankId tag** (when present in `ttags`, across other documents): if an entry with the same `bankId` tag exists → update it (add evidence link, update status if more finalized, update amount if finalized differs from pending)
3. **Fuzzy match** (no bankId, across other documents): find entries within ±1 day, same amount, similar description (after normalization) → treat as same transaction; add evidence link, update status
4. **Pending→finalized**: pending (`!`) entry within ±N days (e.g., 7), amount within tolerance (e.g., ±$5 or ±20%) → update to finalized amount, change to `*`, append evidence link
5. **Statement coverage check**: if document metadata covers a date range and no match found for a pending entry within that range → tag entry with `; no-final-transaction: covered by documents/...`
6. **New transaction**: no match → append new entry to account journal with a random `; id: <uuid>`, log `entry-created` op
7. **Ambiguous** (multiple fuzzy candidates): flag for human review; do not auto-merge

**Entry IDs**: each account journal entry gets a random UUID (`; id: <uuid>`) assigned when it is first created. This ID is stored in the journal and used by GL back-links and operations.jsonl references. It is **not** derived from content — derivation would be fragile since amounts differ between pending/finalized and descriptions may be truncated differently across sources. When entries are merged (e.g., pending→finalized), the earlier entry's UUID is preserved. Each creation is logged as `entry-created` in operations.jsonl with the UUID + content fingerprint so IDs can be recovered during re-derivation.

**New file:**

- `src-tauri/src/dedup.rs` — fuzzy match logic, account journal update operations

---

### 5. Account→General reconciliation

**Goal:** Map account journal entries (with `Equity:Unreconciled` counterpart) to real double-entry general journal postings.

Two modes:

- **Auto**: Use a rules file or pattern matching to assign counterpart account
- **Manual**: UI lists unreconciled entries; user assigns counterpart per entry

On reconciliation of an account journal entry:

- For **single-posting entries** (the common case): a GL transaction is appended with the real counterpart account (e.g., `Expenses:Food`), tagged `; generated-by: refreshmint-reconcile` and `; source: accounts/chase-checking:txn-abc123`
- For **multi-posting entries** (e.g., Venmo pass-through): each unreconciled posting is reconciled independently. The GL entry's `; source:` tag includes the posting index: `; source: accounts/chase-checking:txn-abc123:posting:1`. The account journal entry gets a per-posting reconciled tag: `; reconciled-posting-1: general.journal:<gl-txn-id>`
- The account journal entry gets a forward-link tag `; reconciled: general.journal:<gl-txn-id>` (single-posting) or `; reconciled-posting-N: general.journal:<gl-txn-id>` (multi-posting) and remains otherwise unchanged
- **Undo**: remove the GL entry (identified by `generated-by` + `source` tags); clear the corresponding `reconciled:` or `reconciled-posting-N:` tag on the account journal entry
- **Post-reconciliation changes**: if a reconciled account journal entry later changes (e.g., pending→finalized amount update), the linked GL entry is flagged `; stale: amount-changed` for user review rather than auto-updated. The user can then re-reconcile or confirm the change.

**Transfer detection** (heuristic):

- Description parser flags probable inter-account transfers: "TRANSFER TO/FROM", "PAYMENT THANK YOU", "AUTOPAY", "VENMO", "ZELLE", patterns with amounts
- Flagged entries go to a cross-account reconciliation queue instead of simple single-account assignment
- Cross-account reconciliation: show two flagged entries side-by-side (e.g., checking outflow + CC inflow), user confirms match → both entries reconciled with each other as counterpart

**New files:**

- `src-tauri/src/reconcile.rs` — reconciliation logic
- `src-tauri/src/transfer_detector.rs` — heuristic description parser
- `src-tauri/src/operations.rs` — append to / read from `operations.jsonl` (both per-account and GL-level); used by reconcile, dedup, and undo operations

---

## Data layout

```
ledger.refreshmint/
  operations.jsonl                                   # GL-level decisions: reconciliation, transfer matching (append-only)
  general.journal                                    # main double-entry ledger
                                                     #   pipeline entries: ; generated-by: refreshmint-reconcile
  extensions/
    chase-checking-driver/
      manifest.json                                  # { "account": "chase-checking", "extract": "extract.mjs", ... }
      driver.mjs                                     # scraping automation (code)
      extract.mjs                                    # (optional) raw → structured converter
      account.rules                                  # (optional) hledger CSV rules
  accounts/
    chase-checking/
      operations.jsonl                               # per-account decisions: manual adds, dedup overrides (append-only)
      account.journal                                # primary store; entries have provenance tags
      documents/
        2024-02-17-transactions.csv              # raw evidence (immutable); date = last date covered
        2024-02-17-transactions.csv-info.json    # { mimeType, originalUrl, scrapedAt, scrapeSessionId, ... }
        2024-02-17-page.html
        2024-02-17-page.html-info.json
        2024-01-31-statement.pdf                 # statement date prefix
        2024-01-31-statement.pdf-info.json
        2024-02-17-transactions-2.csv            # collision → incrementing suffix
        2024-02-17-transactions-2.csv-info.json
```

---

## New Tauri commands (frontend API)

| Command                                                                   | Purpose                                                                                           |
| ------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| `listDocuments(accountName)`                                              | List evidence documents with info metadata, grouped by scrape session                             |
| `removeScrape(accountName, scrapeSessionId)`                              | Remove/recompute entries from this scrape's documents; cascade to GL; log `remove-scrape` op      |
| `runExtraction(accountName, documentNames[])`                             | Run extract.mjs or rules on specified documents → dedup → update account journal                  |
| `getAccountJournal(accountName)`                                          | Return account journal entries for UI                                                             |
| `getUnreconciled(accountName)`                                            | Return entries needing reconciliation                                                             |
| `reconcileEntry(accountName, entryId, counterpartAccount, postingIndex?)` | Assign counterpart for a posting; for multi-posting entries, specify which posting to reconcile   |
| `unreconcileEntry(accountName, entryId, postingIndex?)`                   | Remove GL entry; clear `reconciled:` tag; log `undo-reconcile` op                                 |
| `reconcileTransfer(acct1, entry1, acct2, entry2)`                         | Match two entries as inter-account transfer                                                       |
| `checkDiscrepancies(accountName)`                                         | Re-run extraction on existing documents, compare with current account journal, report differences |

**Atomicity**: commands that mutate journal files + operations.jsonl must be atomic. Since this is a single-user desktop app, use a per-account file lock (and a root-level lock for GL operations) to prevent concurrent mutations. Write operations should write to a temp file and atomically rename to prevent partial writes.

---

## Frontend additions

- **Extraction panel** (within Scraping tab): list documents per account, "Run Extraction" button, show extracted rows before committing to account journal
- **Account Journal view**: per-account view of all scraped transactions with evidence links, status badges (pending/cleared/reconciled)
- **Reconciliation queue**: list of unreconciled + flagged transfer entries; inline counterpart assignment; side-by-side transfer matching

---

## Verification

1. Scrape a test extension → verify documents saved to `accounts/<name>/documents/` with date-prefixed filenames and `-info.json` sidecars
2. Run extraction on documents → verify new entries appended to `account.journal` with `; evidence:` and `; id:` tags
3. Run extraction again on same documents → verify no duplicate entries (dedup correctly matches existing entries); evidence links may be updated
4. Scrape overlapping date range → run extraction → verify dedup merges duplicate transactions (combined evidence links on single entry)
5. Pending then finalized: scrape pending transactions, then scrape finalized → verify single account journal entry updated to `*` with both evidence links
6. Remove bad scrape via `removeScrape` → verify account journal entries that came only from that scrape's documents are removed; evidence document files remain on disk
7. Reconcile an entry → verify GL entry created with `; generated-by: refreshmint-reconcile` and `; source:` tags; account journal entry has `; reconciled:` forward-link
8. Undo reconciliation → verify GL entry removed; account journal `; reconciled:` tag cleared; no orphaned state
9. Scrape checking + CC with matching transfer amounts → verify transfer detector flags both; reconcile as transfer → verify GL has correct inter-account posting
10. Upgrade extraction algorithm → run `checkDiscrepancies` → verify differences between new extraction output and existing account journal entries are reported
11. Full re-derivation: from evidence documents + `operations.jsonl`, reproduce account journal and GL; compare against current state to verify consistency
