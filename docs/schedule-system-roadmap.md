# Schedule System Roadmap

Last updated: April 1, 2026

## Summary

Do not build the schedule system yet. Build it only after better reconciliation support exists.

The bookkeeping vocabulary and state separation that this roadmap depends on is
documented in [bookkeeping-state-model.md](./bookkeeping-state-model.md).

The schedule system should be general to both expense and income recognition, not expense-only. The intended v1 scope is:

- planning-only smoothing
- prepaid expense amortization
- accrued expense recognition
- deferred income recognition
- accrued income recognition

The intended v1 explicitly excludes day-based service-period allocation, variable formulas, automatic true-up logic, and full AR/AP workflows.

## Why This Waits

Schedules, accruals, deferrals, and settlements depend on stronger bookkeeping primitives than refreshmint currently has.

### Must exist before schedules

1. Better reconciliation support
   We need a first-class reconciliation workflow for imported bank and credit activity, plus clearer cleared/uncleared state management. Schedule-generated entries must not be confused with real cash activity.

2. Better settlement and linking primitives
   We need explicit linkage between imported bank/card transactions, bills/invoices/documents, and GL adjusting entries. Accrued and deferred balances are not safe to automate without this.

3. Stronger journal provenance
   Every generated entry should record source kind, source id, and related schedule/occurrence id so users can audit and reverse generated activity safely.

4. Basic close and adjustment ergonomics
   Schedule posting is recurring adjusting-entry generation. The app should have a cleaner month-end review workflow first.

### Nice to have before schedules

- Better document linking from GL rows
- Better reporting on cleared vs uncleared vs adjusting activity
- Better visibility into open obligations, receivables, and payables

### Can wait until after schedules

- Full budgeting system
- Invoice-native AR/AP subledger
- Day-prorated utility allocation
- Variable recurrence formulas

## Product Positioning

If this roadmap is implemented well, refreshmint would likely be:

- Better than YNAB, Monarch, and Simplifi for explicit ledger control, evidence-linked bookkeeping, and accountant-style adjustments
- Potentially competitive with parts of Wave and FreshBooks for solo-business bookkeeping, but narrower on invoicing, payments, and operational workflows
- Worse than QuickBooks Online and Xero on full small-business breadth, accountant collaboration, reconciliation maturity, AP/AR completeness, and broader business operations

This is a good roadmap only if the product thesis remains:

`refreshmint is the best app for turning messy real-world financial artifacts into reviewable, evidence-backed books`

It is not the right roadmap if the goal is to become primarily:

- a consumer budgeting app
- a dashboard-first cash-planning app
- or a full QuickBooks/Xero replacement in the near term

## Core Model

### `Schedule`

A persistent business object describing recognition or planning over time.

- `id: string`
  Stable identifier.

- `name: string`
  User-facing label.

- `kind: 'planning' | 'prepaid_expense' | 'accrued_expense' | 'deferred_income' | 'accrued_income'`
  Defines the accounting pattern.
    - `planning`: no journal entries
    - `prepaid_expense`: recognize asset first, expense later
    - `accrued_expense`: recognize expense first, liability until settlement
    - `deferred_income`: recognize liability first, income later
    - `accrued_income`: recognize income first, asset until settlement

- `status: 'draft' | 'active' | 'completed' | 'canceled'`
    - `draft`: does not generate due occurrences
    - `active`: generates occurrences
    - `completed`: finished normally
    - `canceled`: stopped early

- `description: string | null`
  Optional note.

- `currency: string`
  Commodity/code used by generated entries.

- `nominalAmount: Decimal`
  Fixed periodic or allocable amount basis.

- `totalAmount: Decimal | null`
  Total finite amount to allocate over the schedule when applicable.

- `allocationMethod: 'straight_line'`
  v1 only. Even allocation by occurrence count, not by covered days.

- `cadence: 'monthly' | 'quarterly' | 'yearly'`
  Recognition/review frequency, not service coverage length.

    Important clarification:
    - A utility can still use `monthly` cadence if the accounting policy is “review or accrue monthly,” even if actual bills cover 57 to 62 days.
    - If the user does not want monthly recognition and only wants to book actual irregular bills, that is not a strong v1 schedule use case.

- `startDate: LocalDate`
  Inclusive business-effective start date. Format `YYYY-MM-DD`, no time or timezone.

- `endDate: LocalDate | null`
  Inclusive business-effective end date. Same type.

- `occurrenceCount: integer | null`
  Optional finite-count alternative to `endDate`.

- `anchorKind: 'none' | 'manual' | 'from_transaction' | 'from_document'`
  How the schedule originated.

- `anchorRef: string | null`
  Stable link to the originating object.

- `recognitionAccount: string | null`
  P&L account.
    - expense account for expense kinds
    - income account for income kinds

- `deferralAccount: string | null`
  Balance-sheet account used before or after recognition.
    - prepaid asset
    - accrued liability
    - deferred revenue liability
    - accrued receivable asset

- `cashAccountHint: string | null`
  Optional hint for later settlement matching.

- `reversalPolicy: 'none' | 'reverse_next_period'`
  Only relevant for accrual kinds. See reversal section below.

- `evidenceRefs: string[]`
  Related source docs or transaction refs.

- `createdAt: Timestamp`
- `updatedAt: Timestamp`
  UTC RFC3339 system timestamps, not accounting dates.

### `ScheduleOccurrence`

A persistent derived recognition unit.

- `id: string`
- `scheduleId: string`

- `sequence: integer`
  Stable order number. Recommend 1-based for UI.

- `periodStart: LocalDate`
  Inclusive logical recognition period start.

- `periodEnd: LocalDate`
  Inclusive logical recognition period end.

- `postingDate: LocalDate`
  Default accounting date for the generated recognition entry.

    This is separate from `periodStart` and `periodEnd` because:
    - recognition often happens on the last day of the period
    - the covered period is usually a range, not a single accounting date
    - the generated adjusting entry date is not necessarily the source transaction date

    Default v1 behavior:
    - normal recognition occurrence: `postingDate = periodEnd`
    - reversal entry, if used: first day of the next cadence period

    `postingDate` should not generally equal the imported bill/payment date unless the chosen workflow explicitly says so.

- `amount: Decimal`
  Recognition amount for the occurrence.

- `state: 'pending' | 'proposed' | 'posted' | 'skipped' | 'settled'`
    - `pending`: generated but not yet surfaced
    - `proposed`: ready for review
    - `posted`: recognition entry posted
    - `skipped`: intentionally not posted
    - `settled`: linked to downstream bill/payment resolution where meaningful

- `postedGlTxnId: string | null`
  Posted recognition transaction id.

- `reversalGlTxnId: string | null`
  Posted reversal transaction id, if any.

- `settlementRef: string | null`
  Linked real-world bill/payment/import object.

- `notes: string | null`

## Date and Time Semantics

Use two distinct types:

- `LocalDate`
  `YYYY-MM-DD`, no time, no timezone.
  Use for all accounting and schedule dates:
    - `startDate`
    - `endDate`
    - `periodStart`
    - `periodEnd`
    - `postingDate`

- `Timestamp`
  UTC RFC3339 instant.
  Use only for system metadata:
    - `createdAt`
    - `updatedAt`

Reason:

- accounting periods are date-based, not instant-based
- using timestamps for journal dates creates timezone bugs without adding value
- this matches the rest of refreshmint’s current ledger semantics better

## Reversal and Settlement Policy

### What reversal means

A reversal is an automatically generated journal entry in the next accounting period that undoes a prior accrual estimate.

Example:

March 31 estimate:

```text
2026-03-31 Utility accrual for March
    Expenses:Utilities                300
    Liabilities:Accrued:Utilities    -300
```

If `reversalPolicy = reverse_next_period`, then on April 1:

```text
2026-04-01 Reverse March utility accrual
    Liabilities:Accrued:Utilities     300
    Expenses:Utilities               -300
```

Then when the real April bill arrives for `$320`, it can be booked normally:

```text
2026-04-18 Utility bill
    Expenses:Utilities                320
    Accounts Payable                 -320
```

Net effect:

- March shows the estimated March expense
- April shows only the variance between estimate and actual

### Recommendation for refreshmint

Support both reversal policies, but default to `none` in v1.

Reason:

- refreshmint is evidence- and import-driven
- explicit settlement is easier to understand than automatic reversal chains
- reversal is correct accounting, but it adds another generated transaction that users must mentally net against the eventual bill

### Preferred v1 accrual workflow

Use settlement-first, not reversal-first, as the primary UX:

1. Post accrual estimate
2. When real bill/payment arrives, settle against the deferral account explicitly
3. Do not silently create duplicate expense

## Mid-Month and Irregular Bills in v1

If a utility bill arrives mid-month or covers 57 to 62 days, v1 does **not** try to model exact covered-day economics.

Allowed v1 handling:

- `monthly` cadence still means “review or accrue monthly,” not “vendor bills every month exactly”
- actual bill/payment is attached as settlement evidence
- user either:
    - accrues monthly using estimated values, then settles when bill arrives
    - or does not use schedules for that vendor yet and books actual bills directly

Implication:

- irregular utility billing is **not** a strong v1 schedule use case unless the user wants coarse monthly accruals
- exact service-period accounting must wait for a later phase with day-based allocation and service-period fields

## User-Facing Workflows

### Workflow A: Planning smoothing

- no journal entries
- affects planning/forecast outputs only

### Workflow B: Prepaid expense amortization

- usually created from a real payment
- generates period-end recognition entries from prepaid asset to expense

### Workflow C: Accrued expense recognition

- manual estimate-based recurring accruals
- later settled against real bills/payments

### Workflow D: Deferred income recognition

- cash/obligation recognized first as liability
- later recognized into revenue

### Workflow E: Accrued income recognition

- income recognized before invoicing/collection
- later settled through receivable/cash workflow

## Explicitly Out of Scope for v1

These are intentionally not part of this plan:

- `allocationMethod = prorate_by_day`
- `allocationMethod = business_days`
- `allocationMethod = custom_weights`
- `allocationMethod = usage_based`
- variable amount formulas
- automatic estimate-vs-actual true-up
- partial settlement math across many periods
- invoice-native AR/AP subledger
- service-period-aware utility allocation
- background auto-posting of generated entries
- full household budgeting system
- automatic schedule creation from irregular bills

## Reasonable Later Steps

### Phase 2

- add service-period-aware inputs
- add `prorate_by_day`
- support utility and telecom bills with irregular covered periods
- add estimate-vs-actual variance assistance

### Phase 3

- add invoice/bill-native workflows
- richer receivable/payable settlement
- add project/class/location allocation
- support deferred revenue from retainers or contract schedules
- add planning dashboards and schedule-aware forecasts

### Phase 4

- variable/usage-based schedules
- automatic variance postings
- optional auto-post for trusted workflows

## Test and Acceptance Targets

- `LocalDate` fields round-trip without timezone drift
- cadence expansion produces correct monthly/quarterly/yearly occurrences
- `postingDate` remains distinct from source transaction dates
- prepaid, deferred income, accrued expense, and accrued income generate correct journal templates
- reversal entries are generated only when policy requires them
- settlement-first accrual flow avoids duplicate recognition
- planning schedules never create journal entries
- posted occurrences are not silently regenerated after edits
- unsupported allocation methods are rejected explicitly
- irregular utility bills can be linked as evidence/settlement, but are not split by day in v1

## Chosen Defaults

- Reconciliation improvements are required before implementation begins.
- Settlement/linking improvements are required before implementation begins.
- The schedule system is general to expense and income recognition.
- Accounting dates use `LocalDate`; metadata uses UTC timestamps.
- v1 default `postingDate = periodEnd`.
- v1 default `reversalPolicy = none`.
- v1 default `allocationMethod = straight_line`.
- v1 explicitly excludes day-based service-period allocation.
- v1 is competitive primarily as an evidence-native, ledger-first bookkeeping tool, not a QuickBooks/Xero replacement.
