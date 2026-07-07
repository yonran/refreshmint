# Budgets

Refreshmint supports budget-vs-actual reporting through hledger's periodic
transactions and the `balance --budget` report. Budgets live in a **user-owned**
`budget.journal` file that you create and edit by hand.

## Where budgets live

Put `budget.journal` **next to `general.journal`** in your ledger folder:

```
<ledger>/
  general.journal      <- app-owned; created and rewritten by refreshmint
  budget.journal       <- user-owned; you create and edit this
  ...
```

Do **not** add an `include budget.journal` directive inside `general.journal`.
`general.journal` is app-owned: the migration step `ensure_journal_has_ids`
backfills `; id: <uuid>` onto any block header that lacks one, and it would
corrupt a bare `include` line. To keep the two concerns separate, refreshmint
never touches `budget.journal` and passes it to hledger as a second `-f` only
when you ask for a budget report. It merges the two journals for that one report
and writes to neither.

## Writing budget goals

Budget goals are hledger [periodic transactions][periodic] — lines that begin
with `~` followed by a period expression. For example, a monthly budget:

```journal
~ monthly
    Expenses:Groceries      $600.00
    Expenses:Dining          $200.00
    Expenses:Transport       $150.00
    Assets:Checking
```

The unbalanced last posting (`Assets:Checking`) absorbs the total, exactly like
a normal transaction. You can add several periodic transactions with different
periods (`~ weekly`, `~ every 3 months`, `~ 2026`), and mix accounts freely.

[periodic]: https://hledger.org/hledger.html#periodic-transactions

## Running a budget report

- Click the **Budget vs. actual** quick report, which runs a monthly
  `balance --budget` over `Expenses` for the current month, or
- Turn on the **Budget (--budget)** checkbox in a balance-family report's
  options and run it with whatever period/interval/query you like.

hledger renders each cell as `actual [goal]`, so you can see where you are over
or under budget for each account and period.

## Notes

- `budget.journal` is only ever **read**, and only by budget reports. The app
  never writes to it, so it is safe to keep under your own version control or
  edit while refreshmint is running.
- If you request a budget report and no `budget.journal` exists, refreshmint
  returns a friendly error telling you to create one.
