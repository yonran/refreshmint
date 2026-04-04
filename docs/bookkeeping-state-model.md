# Bookkeeping State Model

Last updated: April 3, 2026

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

## Relationship To Schedules

Schedules should build on this state model later.

In particular:

- schedule-generated entries should be `generated`
- settlement of accruals/deferrals should use explicit settlement links
- period-end schedule reviews should respect reconciliation and soft-close state

See [schedule-system-roadmap.md](./schedule-system-roadmap.md)
for the deferred schedule roadmap built on top of this bookkeeping foundation.
