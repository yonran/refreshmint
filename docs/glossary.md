# Glossary

Canonical vocabulary for refreshmint UI strings, docs, and commit messages.
Derived from [bookkeeping-state-model.md](./bookkeeping-state-model.md) "Canonical
Terms" plus a survey of the current UI. When a user-facing string must name one of
these concepts, use the term here rather than a synonym.

## Core objects

| Term               | Meaning                                                                                                                        | Not to be confused with |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------ | ----------------------- |
| **entry**          | An account-journal line (a source event scraped/imported from a bank or card).                                                 | transaction             |
| **transaction**    | A GL record in `general.journal`.                                                                                              | entry                   |
| **row**            | An evidence row (a document/statement line shown as supporting evidence).                                                      | entry, transaction      |
| **proposal**       | A machine suggestion (`AutomationProposal`) awaiting a human decision.                                                         | saved decision          |
| **saved decision** | A persisted `Resolution`. Always surface as "saved decision" or the specific decision label — never the raw word "resolution". | proposal                |
| **bank account**   | A scraped source account (a login/label pair).                                                                                 | GL account              |

## Bank vs GL status

- **bank pending** / **bank posted** — the status a transaction has at the bank
  (as scraped). Rendered as chips describing the upstream source state.
- **GL posted** / **GL unposted** — whether a source entry has been materialized
  into `general.journal`. This is refreshmint's own posting state, independent of
  the bank's status.

## Verbs (destructive vs benign)

These are deliberately distinct; do not use them interchangeably.

| Verb                    | Meaning                                                                                                                                       |
| ----------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| **Mark reviewed**       | Persist that an anomaly has been reviewed. (Replaces the old "Dismiss" on anomalies — that word wrongly implied the anomaly was thrown away.) |
| **Dismiss**             | Hide a transient banner only. No persisted state changes.                                                                                     |
| **Retire source entry** | Permanently remove a pending source entry that will never post. Labeled "Retire source entry" in full.                                        |
| **Delete**              | Destroy a stored record.                                                                                                                      |
| **Remove**              | Detach a mapping/link without destroying the underlying records.                                                                              |

## Tab names (2026-07-07 decisions)

The nav tab ids in `src/store.ts` (`ActiveTab`) are stable literals validated on
load; only labels and order changed.

| `ActiveTab` id | Old label   | New label             |
| -------------- | ----------- | --------------------- |
| `pipeline`     | Pipeline    | **Review**            |
| `bookkeeping`  | Bookkeeping | **Reconcile & Close** |

Order (left to right): Accounts, Transactions, [Recategorize], Review, Reports,
Reconcile & Close, Scraping, Preferences, Settings.

The Pipeline tab's "Account Rows" sub-tab is relabeled **Account Entries** (its
state id is unchanged).

## Enum labels

Human-readable labels for backend enum unions live in `src/enum-labels.ts`
(`enumLabel(table, value)` / `enumDescription(table, value)`). That module is the
single source of truth for how proposal kinds, policy decisions, reversibility,
resolution kinds/statuses, anomaly kinds, link kinds, period-close statuses, and
extract/post skip reasons appear in the UI. Do not hand-write these strings in
components; import the label tables instead.

## Deferred

Renames that are intentionally out of scope for the terminology batch:

- **Identifier / type / command renames.** The TypeScript union values
  (`'transfer-link'`, `'soft-closed'`, etc.), Rust enum variants, and Tauri
  command names keep their current wire spellings. Only their _labels_ changed.
- **Full dark mode.** Removed rather than fixed (the app is light-only); see
  [terminology-css-batch] CSS phase.
- **Bookkeeping pickers.** Adjustment GL-txn-id and TypedRef Left/Right selection
  still require typing ids by hand; a real picker (modal + search) is deferred.
  </content>
  </invoke>
