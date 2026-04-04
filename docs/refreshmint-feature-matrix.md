# Refreshmint Feature Matrix

Based on the current repo state on March 31, 2026.

## Legend

- `EX` = explicit, first-class feature/module
- `PA` = partial / limited / workflow exists but not fully generalized
- `E` = easy user workflow
- `M` = moderate workflow
- `H` = advanced / multi-step workflow

Cell format: `Support·Ease — short note with repo pointers`

## Core app and ledger shell

| Feature / Task                       | Support·Ease | Notes                                                                                                                                            |
| ------------------------------------ | ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| Create a new `.refreshmint` ledger   | EX·E         | File menu + dialog-backed ledger creation via `new_ledger`; see `src/App.tsx`, `src-tauri/src/lib.rs`.                                           |
| Open existing ledger files           | EX·E         | Open dialog supports `.refreshmint` ledgers, including macOS package handling; see `src/App.tsx`.                                                |
| Recent ledgers + startup reopen      | EX·E         | Recent list is persisted in store and auto-opened on startup; see `src/store.ts`, `src/App.tsx`.                                                 |
| Persist last active tab              | EX·E         | Accounts / Transactions / Pipeline / Reports / Preferences / Scraping tab is restored across launches; see `src/store.ts`, `src/App.tsx`.        |
| Accounts overview                    | EX·E         | Accounts tab shows account names, balances, and unposted extraction counts; see `src/App.tsx`.                                                   |
| Global GL mapping conflict detection | EX·M         | Dedicated conflict panel detects duplicate login-label-to-GL mappings and offers load/ignore actions; see `src/App.tsx`.                         |
| Transaction-display preference       | EX·E         | Preference to collapse obvious posting amounts in two-posting asset/liability transactions; see `src/App.tsx`, `src/tabs/TransactionsTable.tsx`. |
| Auto-scrape preferences              | EX·E         | User can enable/disable auto-scrape and set stale interval in hours; see `src/App.tsx`.                                                          |
| Auto-scrape queue and status banner  | EX·M         | Stale logins are queued, scraped one at a time, and surface progress/errors in-app; see `src/App.tsx`.                                           |
| Auto ETL after auto-scrape           | EX·M         | Auto mode chains scrape -> extract -> post, including transfer-aware posting and aggregated errors; see `src/App.tsx`.                           |

## Scraping, logins, and credentials

| Feature / Task                               | Support·Ease | Notes                                                                                                                                                   |
| -------------------------------------------- | ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Load scraper extensions from `.zip`          | EX·E         | Scraping tab supports runtime-ready zip imports; see `src/tabs/ScrapeTab.tsx`, `docs/extension.md`.                                                     |
| Load scraper extensions from directory       | EX·E         | Runtime-ready directories can be loaded directly; see `src/tabs/ScrapeTab.tsx`, `docs/extension.md`.                                                    |
| Load unpacked extension source path          | EX·M         | UI supports binding an unpacked extension directory path directly; see `src/tabs/ScrapeTab.tsx`, `docs/extension.md`.                                   |
| Built-in bundled extensions                  | EX·E         | Built-ins include `bankofamerica`, `chase`, `citi`, `paypal`, `providentcu`, and `target`; see `docs/extension.md`.                                     |
| Create / select / delete logins              | EX·E         | Login management supports creating a login namespace, selecting it, and deleting it when clean; see `src/tabs/ScrapeTab.tsx`, `src-tauri/src/lib.rs`.   |
| Bind extension per login                     | EX·E         | Each login can save a default extension used for scrape and extract; see `src/tabs/ScrapeTab.tsx`, `docs/extension.md`.                                 |
| Login-label -> GL account mapping            | EX·M         | Labels can be added, edited, ignored, or removed; mappings feed the ETL/posting pipeline; see `src/tabs/ScrapeTab.tsx`.                                 |
| Repair mislabeled / legacy login labels      | EX·M         | Dedicated repair action migrates alias/default buckets into normalized login-label storage; see `src/tabs/ScrapeTab.tsx`, `src-tauri/src/migration.rs`. |
| Per-login scrape run                         | EX·E         | Scraping tab runs the same scraper pipeline as CLI for the selected login; see `src/tabs/ScrapeTab.tsx`, `docs/scraper.md`.                             |
| Scrape all logins                            | EX·E         | UI can queue all logins for scrape+extract processing; see `src/tabs/ScrapeTab.tsx`, `src/App.tsx`.                                                     |
| Scrape log history                           | EX·E         | Each login has a newest-first scrape log view backed by `scrape-log.jsonl`; see `src/tabs/ScrapeTab.tsx`, `docs/scraper.md`.                            |
| Headed scrape debug session                  | EX·M         | Start/stop a persistent debug browser session and copy its socket for CLI/LLM tooling; see `README.md`, `src/tabs/ScrapeTab.tsx`.                       |
| In-app scraper prompt modal                  | EX·M         | Rust scrape drivers can request user input such as MFA codes through a modal prompt; see `src/App.tsx`, `docs/scraper.md`.                              |
| Login secret sync against extension manifest | EX·M         | Required domains are compared against manifest declarations and surfaced as missing/extras; see `src/tabs/ScrapeTab.tsx`, `src/tauri-commands.ts`.      |
| Username/password storage per domain         | EX·M         | Save combined credentials, username-only, or password-only for a login/domain pair; see `src/tabs/ScrapeTab.tsx`, `src/tauri-commands.ts`.              |
| Edit/remove stored domain secrets            | EX·E         | Existing secret entries can be refreshed, edited, or deleted from the login secrets panel; see `src/tabs/ScrapeTab.tsx`.                                |
| Legacy secret migration                      | EX·M         | Old keychain entries can be migrated into the per-domain secret model; see `src/tabs/ScrapeTab.tsx`, `src/tauri-commands.ts`.                           |
| Legacy ledger migration                      | EX·M         | App detects old `accounts/` layout and offers an in-app migration workflow; see `src/tabs/ScrapeTab.tsx`, `src-tauri/src/migration.rs`.                 |

## ETL and posting pipeline

| Feature / Task                                   | Support·Ease | Notes                                                                                                                                                          |
| ------------------------------------------------ | ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Ledger-wide Extract All readiness dashboard      | EX·M         | Pipeline tab computes eligible accounts, skip reasons, inspect failures, document counts, and lock state; see `src/tabs/PipelineTab.tsx`.                      |
| Ledger-wide Post All readiness dashboard         | EX·M         | Same dashboard tracks GL lock, missing mappings, unposted counts, and per-account posting eligibility; see `src/tabs/PipelineTab.tsx`.                         |
| Lock-aware pipeline operations                   | EX·M         | Selected login locks and general-journal locks are watched live and block unsafe actions; see `src/tabs/PipelineTab.tsx`, `src/tauri-commands.ts`.             |
| Evidence document inventory                      | EX·E         | Evidence tab lists documents with MIME type, coverage end date, scrape time, and scrape session id; see `src/tabs/PipelineTab.tsx`, `docs/extractor.md`.       |
| Evidence document preview                        | PA·M         | In-app viewing exists for images/text-like documents; PDFs are listed but not previewed inline here; see `src/tabs/PipelineTab.tsx`.                           |
| CSV raw-row inspection                           | EX·E         | Evidence Rows tab loads CSV rows, numbers them, and highlights rows already referenced as evidence; see `src/tabs/PipelineTab.tsx`.                            |
| Document-level extraction / re-extraction        | EX·M         | A selected document can be extracted directly into account rows from the Evidence Rows view; see `src/tabs/PipelineTab.tsx`.                                   |
| Multi-document extraction from Scraping tab      | EX·M         | Scraping tab lets you select documents or default to all documents for a login-label account and run extraction; see `src/tabs/ScrapeTab.tsx`.                 |
| Account-row journal inspection                   | EX·E         | Account Rows tab shows extracted entries before or after posting; see `src/tabs/PipelineTab.tsx`.                                                              |
| Suggested GL account seed for login labels       | EX·M         | Pipeline can suggest/save a GL account for an unmapped login label; see `src/tabs/PipelineTab.tsx`.                                                            |
| Category suggestions for extracted rows          | EX·M         | Category engine suggests counterpart accounts and detects amount/status drift plus transfer matches; see `src/tabs/PipelineTab.tsx`, `src/tauri-commands.ts`.  |
| Post one extracted row to GL                     | EX·E         | Individual unposted account-row entries can be posted into the general journal; see `src/tabs/PipelineTab.tsx`.                                                |
| Post selected extracted rows                     | EX·M         | Checkbox-driven partial posting is supported for the selected account; see `src/tabs/PipelineTab.tsx`.                                                         |
| Post all extracted rows for one account          | EX·E         | Selected account can post all unposted entries in one action; see `src/tabs/PipelineTab.tsx`.                                                                  |
| Split transaction posting                        | EX·M         | One extracted row can be posted to multiple counterpart accounts via a split modal; see `src/tabs/PipelineTab.tsx`.                                            |
| Transfer linking between unposted extracted rows | EX·M         | Dedicated modal searches other unposted account rows and posts paired transfers; see `src/tabs/PipelineTab.tsx`.                                               |
| Sync posted entry back to GL changes             | EX·M         | When status/amount diverges, a posted extracted row can be synced back to the GL transaction; see `src/tabs/PipelineTab.tsx`.                                  |
| Ledger-wide Extract All                          | EX·M         | Runs extraction across every eligible unlocked login-label account and reports successes/failures/locks/new entries; see `src/tabs/PipelineTab.tsx`.           |
| Ledger-wide Post All                             | EX·M         | Runs posting across every eligible unlocked login-label account, including transfer-aware posting; see `src/tabs/PipelineTab.tsx`.                             |
| GL rows cross-navigation                         | EX·E         | GL Rows subtab reuses the transaction table and can jump into the Transactions tab focused on a GL transaction; see `src/tabs/PipelineTab.tsx`, `src/App.tsx`. |

## Transactions and general ledger tools

| Feature / Task                                    | Support·Ease | Notes                                                                                                                                                                         |
| ------------------------------------------------- | ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Transaction search with query syntax              | EX·E         | Transactions tab supports debounced `hledger`-style query search; see `src/tabs/TransactionsTab.tsx`, `src-tauri/src/lib.rs`.                                                 |
| Search autocomplete for accounts/query tokens     | EX·E         | Search bar offers completions using current token and account list; see `src/tabs/TransactionsTab.tsx`, `src/search-utils.ts`.                                                |
| Unposted-only filter                              | EX·E         | One-click filter narrows the table to entries still touching the staging account family `Equity:Staging:*` (with legacy `Equity:Unreconciled:*` compatibility).               |
| Manual transaction entry (form mode)              | EX·E         | Structured form supports date, description, comment, and arbitrary postings; see `src/tabs/TransactionsTab.tsx`.                                                              |
| Manual transaction entry (raw text mode)          | EX·M         | Users can paste raw hledger transaction text and add it directly; see `src/tabs/TransactionsTab.tsx`.                                                                         |
| Live hledger validation of drafts                 | EX·E         | Form and raw modes both validate drafts asynchronously before submission; see `src/tabs/TransactionsTab.tsx`.                                                                 |
| Account autocomplete in posting editor            | EX·E         | Posting-account fields autocomplete and warn on account-type changes; see `src/tabs/TransactionsTable.tsx`.                                                                   |
| Transaction table with postings and evidence      | EX·E         | Table shows postings, balances, comments, evidence refs, and per-posting actions; see `src/tabs/TransactionsTable.tsx`.                                                       |
| Image attachment lightbox                         | EX·E         | Evidence refs ending in `#attachment` for image files open in a modal lightbox; see `src/tabs/TransactionsTable.tsx`.                                                         |
| Inline recategorization of GL postings            | EX·E         | Non-balance-sheet postings can be edited inline, especially uncategorized `Expenses:Unknown` rows; see `src/tabs/TransactionsTable.tsx`.                                      |
| ML/category suggestions for uncategorized GL rows | EX·M         | Transactions tab loads GL-side category suggestions and surfaces quick-apply actions; see `src/tabs/TransactionsTab.tsx`, `src/tauri-commands.ts`.                            |
| Transfer-merge suggestions for GL rows            | EX·M         | Candidate transfer counterpart transactions can be merged directly from the table; see `src/tabs/TransactionsTable.tsx`, `src/tabs/TransactionsTab.tsx`.                      |
| Similar-transaction grouping                      | EX·M         | Uncategorized rows are grouped by description + balancing account to seed bulk categorization flows; see `src/tabs/TransactionsTable.tsx`.                                    |
| Dedicated recategorize workspace tabs             | EX·M         | Similar-transaction actions open a separate recategorize tab with its own query, selection state, and destination account; see `src/App.tsx`, `src/tabs/TransactionsTab.tsx`. |
| Bulk recategorize with confirmation               | EX·M         | Bulk account replacement can be confirmed when multiple source accounts are involved; see `src/tabs/TransactionsTab.tsx`, `src/tabs/TransactionsTable.tsx`.                   |
| Context-menu search shortcuts                     | EX·E         | Table context actions can add useful search terms directly from transactions/postings; see `src/tabs/TransactionsTable.tsx`.                                                  |

## Reports and analytics

| Feature / Task                    | Support·Ease | Notes                                                                                                                                          |
| --------------------------------- | ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| Built-in report builder           | EX·E         | Reports tab wraps `hledger` commands in a first-class GUI; see `src/tabs/ReportsTab.tsx`, `src-tauri/src/lib.rs`.                              |
| Balance-family reports            | EX·E         | Supports `balance`, `balancesheet`, `balancesheetequity`, `cashflow`, and `incomestatement`; see `src/tabs/ReportsTab.tsx`.                    |
| Register/activity/stats reports   | EX·E         | Supports `register`, `aregister`, `activity`, and `stats`; see `src/tabs/ReportsTab.tsx`.                                                      |
| Period controls                   | EX·E         | Begin/end date plus daily/weekly/monthly/quarterly/yearly interval buttons; see `src/tabs/ReportsTab.tsx`.                                     |
| Query autocomplete in reports     | EX·E         | The report query field uses the same token-aware autocomplete model as transactions; see `src/tabs/ReportsTab.tsx`.                            |
| Report filter options             | EX·M         | Cleared/pending/unmarked/real-only/show-empty/depth controls are exposed in the GUI; see `src/tabs/ReportsTab.tsx`.                            |
| Valuation options                 | EX·M         | Cost basis, market value, and exchange-commodity options are supported; see `src/tabs/ReportsTab.tsx`.                                         |
| Balance/register advanced options | EX·M         | Accumulation, average, totals, summary, sort, percent, invert, transpose, drop, and related flags are surfaced; see `src/tabs/ReportsTab.tsx`. |
| Tabular report results            | EX·E         | Structured report output renders as tables, with text-mode output for activity/stats; see `src/tabs/ReportsTab.tsx`.                           |
| Inline charts                     | EX·E         | Interval balance reports and register output can render charts in-app; see `src/tabs/ReportsTab.tsx`, `src/tabs/ReportChart.tsx`.              |

## Extension platform and CLI/developer features

| Feature / Task                                              | Support·Ease | Notes                                                                                                                                                    |
| ----------------------------------------------------------- | ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| CLI parity for scrape/extract/debug flows                   | EX·M         | README/docs cover `scrape`, `account extract`, `debug start`, `debug exec`, and `debug stop`; see `README.md`, `docs/scraper.md`, `docs/extractor.md`.   |
| JS or rules-based extraction                                | EX·M         | Extensions can use either hledger CSV rules or a JS extractor, but exactly one must be declared; see `docs/extractor.md`, `docs/extension.md`.           |
| TypeScript-based extensions                                 | EX·M         | Drivers/extractors may be `.ts`/`.mts` with erasable-syntax runtime stripping; see `docs/extension.md`, `docs/scraper.md`, `docs/extractor.md`.          |
| Extension manifest for secrets/extract/id field/autoExtract | EX·M         | Manifest declares driver, extractor/rules, secret roles, source id field, and extraction preference; see `docs/extension.md`.                            |
| Scraper runtime APIs                                        | EX·H         | Runtime exposes browser automation, prompt/log/report helpers, network capture, and staged resource saving; see `docs/scraper.md`.                       |
| Incremental scraper debugging loop                          | EX·H         | `debug start` hosts the browser while repeated `debug exec` calls iterate on scripts without restarting the session; see `README.md`, `docs/scraper.md`. |
| Network inspection for scraper authors                      | EX·H         | Scraper API exposes request/response waiters, request logs, headers, timing, and response bodies; see `docs/scraper.md`.                                 |
| Staged resources -> finalized account documents             | EX·H         | `refreshmint.saveResource(...)` feeds the same evidence pipeline used by regular scrapes; see `docs/scraper.md`, `docs/extractor.md`.                    |
| Document metadata sidecars                                  | EX·M         | Finalized documents get `*-info.json` metadata with scrape session, coverage dates, source URL, and mime type; see `docs/extractor.md`.                  |
| Dedup-aware extraction pipeline                             | EX·M         | Extraction validates evidence refs, dedups against existing account journals, and writes updated journal entries; see `docs/extractor.md`.               |
