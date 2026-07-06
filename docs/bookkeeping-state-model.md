# Bookkeeping State Model

Last updated: April 30, 2026

## Summary

This document standardizes the bookkeeping terms that refreshmint uses before
building first-class schedules, accrual settlement, and statement close flows.

The core rule is:

- hledger status stays responsible for transaction-level `pending` / `cleared`
- statement reconciliation is tracked separately in persisted reconciliation sessions
- linking and settlement are explicit relationships, not overloaded status flags
- closing a period is separate from both clearing and reconciliation

## Canonical Terms

### Imported

A transaction or document came from scrape, extract, sync, or another external source.

### Posted

A source account-journal entry has been materialized into `general.journal`.

Current implementation detail:

- source journals store `posted:` or `posted-posting-N:` references that point to the generated GL transaction
- this is not the same thing as statement reconciliation

### Pending

hledger `!`.

Meaning in refreshmint:

- the transaction is not yet treated as cleared/final cash activity

### Cleared

hledger `*`.

Meaning in refreshmint:

- the transaction is eligible to match a bank or card statement

Important:

- `cleared` does **not** mean `reconciled`

### Reconciled

A GL transaction is included in a finalized statement reconciliation session.

This is tracked in persisted bookkeeping state, not only in hledger status.

### Linked

Two bookkeeping objects are explicitly related.

Examples:

- GL transaction to document
- imported source entry to document
- GL transaction to another bookkeeping artifact

### Settled

A link resolves an open balance-sheet position such as an accrual, deferral, receivable, or payable.

Important:

- every settlement is a link
- not every link is a settlement

### Soft-closed

An accounting period has been reviewed and should warn/gate later edits, but is not hard-locked.

### Generated

A transaction was created by refreshmint bookkeeping logic rather than imported as a cash-side source event.

## Legacy Terms and How To Read Them

### `Equity:Staging:*`

This is the canonical staging counterpart account used during extraction and pre-post review.

It does **not** mean statement reconciliation.

Interpret it as:

- imported / extracted
- not fully posted to intended GL accounts yet

Legacy ledgers may still contain `Equity:Unreconciled:*` until migrated. Read it as the same staging concept.

### `unpostedCount`

Count of source-journal entries that still have unposted portions.

Meaning:

- count of source-journal entries that still have unposted portions

It does **not** mean “not bank-reconciled.”

### `posted`

In source journals, `posted` means “linked to a GL transaction.”

It does not imply:

- cleared
- reconciled
- settled
- period closed

## State Layers

These layers are intentionally separate.

1. Source state
   Imported / extracted / deduplicated in account journals.

2. GL materialization state
   Posted or unposted relative to `general.journal`.

3. Transaction bookkeeping state
   hledger `pending` / `cleared` markers.

4. Statement reconciliation state
   Draft / finalized / reopened reconciliation sessions.

5. Link and settlement state
   Explicit bookkeeping links, with settlement links reserved for resolving open balances.

6. Period close state
   Draft / soft-closed / reopened accounting periods.

## Typical State Progression

Cash-side example:

1. imported source entry appears in an account journal
2. source entry is posted into `general.journal`
3. GL transaction may remain unmarked, pending, or cleared
4. cleared GL transactions are gathered into a reconciliation session
5. session is finalized, making those transactions reconciled
6. month can be soft-closed after reconciliation and adjustment review

Accrual-side example:

1. generated GL adjustment is posted
2. it may be linked to supporting evidence
3. later imported bill/payment is linked as a settlement
4. the period may then be soft-closed

## Persisted Objects

Ledger-local bookkeeping objects live under:

```text
<ledger>.refreshmint/bookkeeping/
```

Mutable JSON objects:

- `bookkeeping/reconciliation-sessions/<session-id>.json`
- `bookkeeping/links/<link-id>.json`
- `bookkeeping/period-closes/<YYYY-MM>.json`

Current source-of-truth split:

- hledger status markers live in `general.journal`
- source posting refs live in account journals
- reconciliation membership, links, and close state live in `bookkeeping/`

## Ledger Configuration (`refreshmint.json`)

`<ledger>.refreshmint/refreshmint.json` stores ledger-wide configuration
(`ledger::RefreshmintConfig` in `src-tauri/src/ledger.rs`). Fields:

- `version` (string, required): app version that created the ledger.
- `transferDateWindowDays` (number, optional, default 3): the ± day window the
  transfer matchers use when pairing opposite-amount entries or GL
  transactions.
- `extraTransferPatterns` (array of strings, optional, default empty):
  additional case-insensitive substring patterns that mark a description as a
  probable transfer, on top of the built-in list in
  `src-tauri/src/transfer_detector.rs`.

## Row-Level Factual Propositions

This section defines what each durable row or object asserts. The distinction
matters because refreshmint can regenerate derived rows, but it must preserve
observed facts and explicit user choices.

### Source Account Journals

File pattern:

```text
logins/<login>/accounts/<label>/account.journal
accounts/<account>/account.journal
```

Each hledger transaction block asserts one imported source event for that
source account.

- Header date/status/description: the institution or importer reported this
  source event with that date, lifecycle marker, and description.
- Posting lines: the source event changed the source account by the posted
  amount or amounts.
- `id` tag: this is the stable refreshmint identity for that source event.
- `evidence` tags: the event was derived from those scrape rows, document
  rows, or other external locators.
- Extracted tags such as `bankId`, `amount`, `fitId`, or merchant fields:
  the importer observed those source-system fields.
- `posted` tag: the whole source event has been materialized into the named
  `general.journal` transaction.
- `posted-posting-N` tag: posting `N` of a split source event has been
  materialized into the named `general.journal` transaction.

Pending source entries assert that the institution currently reports a
provisional event. Cleared source entries assert that the institution reports a
finalized event. If a finalized source entry later disappears from a covered
export, refreshmint treats that as an import anomaly instead of silently
deleting it.

### General Journal

File pattern:

```text
general.journal
```

Each hledger transaction block asserts one accounting transaction in the GL.

- Header date/status/payee/narration: the accounting transaction has that
  date, lifecycle marker, and description in the ledger.
- Posting lines: the accounting transaction affects those GL accounts by those
  amounts.
- `id` tag: this is the stable refreshmint identity for that GL transaction.
- `source` tags: this GL transaction was generated from, synced from, or
  manually linked to those source entries.
- `generated-by: refreshmint-post`: refreshmint generated the transaction from
  source posting automation, so operations such as sync, undo, and transfer
  merge may treat it as app-managed.

`general.journal` is the current accounting surface. Rows in account journals,
operations logs, resolutions, links, and reconciliation files explain where
that surface came from and what actions are allowed to modify it.

### Account Operations Logs

File patterns:

```text
logins/<login>/accounts/<label>/operations.jsonl
accounts/<account>/operations.jsonl
```

Each JSONL row asserts that a source-account operation happened at
`timestamp`. These rows are an audit trail and replay aid; the current source
state still comes from `account.journal`.

- `entry-created`: refreshmint created or re-derived the source entry with the
  given stable `entryId`, evidence, date, amount, and tags.
- `manual-add`: a user manually created the source entry.
- `dedup-override`: a user or repair flow forced or prevented a proposed
  source dedup match.
- `remove-scrape`: a scrape session's imported effects were removed.
- `entry-retired`: refreshmint intentionally retired a provisional source
  entry for the stated reason.

### GL Operations Log

File pattern:

```text
operations.jsonl
```

Each JSONL row asserts that a GL-level operation happened at `timestamp`.

- `post`: a source entry, or one posting of a source entry, was posted to a
  counterpart GL account.
- `post-split`: a source entry was posted across multiple counterpart GL
  accounts.
- `transfer-match`: two or more source entries were matched as one
  inter-account transfer.
- `undo-post`: a previous post operation was undone for that source entry or
  posting.
- `sync-transaction`: an app-managed GL transaction was updated in place to
  match the listed source snapshots.
- `import-duplicate-repair`: refreshmint removed a duplicate import artifact,
  preserving the kept and removed source/GL identities for audit.

These rows say what refreshmint did. They do not by themselves prove that a
bank reported a transaction; that proposition comes from source journal rows
and their evidence.

### Automation Resolutions

File pattern:

```text
bookkeeping/resolutions/<resolution-id>.json
```

Each JSON object asserts an active or disabled durable preference, constraint,
or manual conclusion. Resolutions are inputs to proposal generation; proposals
can be regenerated from current ledger state and active resolutions.

- `category`: the subject source entry should post to the account or parts in
  `parts`.
- `category-rule`: a predicate-based standing rule (no subject entry). The
  `predicate` field matches entries by `descriptionRegex` (case-insensitive,
  against the raw description), `normalizedPayee` (exact match against
  `payee_normalize::normalize_payee(description)`), and optional
  `amountMin`/`amountMax` bounds; every set field must match. `subjectRefs` is
  either empty (global) or one `login-entry` scope ref without an `entryId`
  (restricts the rule to one bank account). Matching unposted entries post
  directly to the single account part; matching `Expenses:Unknown` GL rows get
  Auto `recategorize-gl` proposals. Newest `updatedAt` wins when several rules
  match. See `automation::matching_rule_account`.
- `posting-split`: the subject source entry should be split across the
  accounts and amounts in `parts`.
- `transfer-link`: the subject source entries should be treated as one
  transfer.
- `transfer-split`: planned vocabulary for representing a transfer by the
  parts in `parts`; proposal generation and application are not implemented
  yet.
- `same-source`: the subject refs represent the same external source event.
- `not-same-source`: the subject refs must not be deduplicated or merged
  together.
- `ignore-source`: automation should not act on the subject source entry.
- `pending-retired`: the subject pending source entry was intentionally
  retired.
- `reversal-link`: the subject refs form a reversal relationship.

`status: disabled` preserves the historical user choice while removing it from
future automation decisions.

For source deduplication, these relationship resolutions are actionable when
they pair a `login-entry` ref with an `evidence-row` ref such as
`statement.csv:12:1`. A `same-source` row forces the incoming extracted row for
that evidence ref to merge into the named source entry; a `not-same-source` row
prevents heuristic matching between them. Older resolutions may encode the same
evidence locator as a `document` ref; dedup reads those for compatibility, but
new decisions should use `evidence-row`.

### Automation Proposals

Automation proposals are returned by commands and are not durable rows. A
proposal asserts only that, given the current source journals, GL, anomalies,
and active resolutions, refreshmint currently recommends or blocks an action.

- `reasons`: the facts, rules, model suggestions, or resolutions that led to
  the proposal.
- `blockers`: conditions that must be resolved before the proposal can be
  applied.
- `policyDecision`: whether refreshmint may auto-apply, should ask for review,
  must block, or should skip.
- `reversible`: whether the resulting operation can be undone automatically,
  conditionally, or not safely.

Because proposals are derived, they should be explained and reviewable, but the
user-facing steering state belongs in resolutions.

### Import Anomalies

File pattern:

```text
bookkeeping/import-anomalies/<anomaly-id>.json
```

Each JSON object asserts that refreshmint found an import inconsistency that
needs review.

- `finalized-missing-from-covered-export`: a finalized source or GL event was
  missing from an export that claims to cover the event's date.
- `unsafe-pending-retirement`: a pending source entry disappeared, but
  refreshmint could not prove that retiring it is safe.
- `duplicate-import-repair-skipped`: refreshmint detected a duplicate import
  candidate but skipped automatic repair. For ambiguous dedup, one anomaly is
  created for each candidate source entry, and `evidence` names the incoming
  extracted row that needs a `same-source` or `not-same-source` decision.
- `safeToRetire`: refreshmint's current safety conclusion for retiring the
  source entry.
- `safetyReasons`: the concrete checks behind that conclusion.
- `status: reviewed`: a user or automation reviewed the anomaly; it does not
  necessarily mean the source fact was true.

An anomaly row does not assert that the bank was wrong. It asserts that the
ledger and available evidence disagree in a way that should not be hidden.

### Bookkeeping Links

File pattern:

```text
bookkeeping/links/<link-id>.json
```

Each JSON object asserts an explicit relationship between two typed refs.

- `evidence-link`: one object is supported by or derived from the other.
- `settlement-link`: the relationship resolves an open balance-sheet position
  such as an accrual, deferral, payable, or receivable.
- `source-link`: the two objects share source provenance.
- `amount`: optional amount scope for the relationship.

Links are not categories and do not replace postings. They assert
relationships between already identifiable objects.

### Reconciliation Sessions

File pattern:

```text
bookkeeping/reconciliation-sessions/<session-id>.json
```

Each JSON object asserts a statement-reconciliation session for one GL account.

- Statement fields: the user entered or imported the statement period and
  balance facts.
- `reconciledTxnIds`: those GL transaction ids are included in the session.
- `status: draft`: the session is editable review state.
- `status: finalized`: the listed transactions are reconciled to the statement
  as of the session's finalization.
- `status: reopened`: a previously finalized reconciliation was reopened.

Reconciliation membership is intentionally separate from hledger cleared
status.

### Period Closes

File pattern:

```text
bookkeeping/period-closes/<YYYY-MM>.json
```

Each JSON object asserts review state for an accounting period.

- `draft`: close work exists but the period is not protected.
- `soft-closed`: the period was reviewed and later edits should warn or gate.
- `reopened`: a previously soft-closed period was reopened.
- `reconciliationSessionIds`: reconciliations considered in the close.
- `adjustmentTxnIds`: GL adjustments considered in the close.

Period closes are review/protection state, not bank-cleared or reconciled
markers.

### Scrape And Extract Logs

File patterns:

```text
logins/<login>/scrape-log.jsonl
logins/<login>/extract-log.jsonl
```

Each JSONL row asserts that an external data-gathering run happened.

- Scrape log row: a scrape was attempted for the login, with success or error
  state and manual or automatic source.
- Extract log row: document extraction was attempted, with discovered document
  results, console output, or errors.

These logs are operational evidence. They do not by themselves create source
transactions; source transactions are created by account-journal rows.

## Relationship To Schedules

Schedules should build on this state model later.

In particular:

- schedule-generated entries should be `generated`
- settlement of accruals/deferrals should use explicit settlement links
- period-end schedule reviews should respect reconciliation and soft-close state

See [schedule-system-roadmap.md](./schedule-system-roadmap.md)
for the deferred schedule roadmap built on top of this bookkeeping foundation.
