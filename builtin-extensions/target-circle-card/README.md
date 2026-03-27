# Target Circle Card Scraper Notes

Verified against live Target Circle Card sessions while developing the
`target-circle-card` Refreshmint extension.

## Overview

This website is more complex than a simple statement portal:

- login happens on `https://mytargetcirclecard.target.com/ecs/auth/`
- post-login pages are a React app under `https://mytargetcirclecard.target.com/`
- the authenticated home page, statements page, and transaction-history page are
  separate subflows with different DOM structures
- transaction export is hidden behind a modal with multiple file formats
- the best activity coverage comes from combining multiple export formats

The scraper is state-machine based so `debug exec` can resume from login,
home, statements, or transaction history without restarting the browser.

## Login Flow

- Login URL:
    - `https://mytargetcirclecard.target.com/`
- Verified login selectors:
    - username: `input#username`
    - password: `input#password`
    - submit: `button#login`
- `page.type(...)` worked more reliably than `fill(...)` on this site.
- A pre-submit screenshot is useful because the site can appear to ignore
  login even when the fields were actually populated.
- Password secret access can pause on macOS keychain / Touch ID approval.

## MFA Flow

MFA is hosted at:

- `https://mytargetcirclecard.target.com/ecs/auth/multi-factor-auth`

Observed states:

- method-selection screen with visible radio choices such as:
    - `Email / Y*******@GMAIL.COM`
    - `Home / ***-***-8367`
    - `Work / ***-***-8367`
    - `I already have a passcode`
- code-entry screen with a text/tel/passcode input
- transient loading state on the same URL with neither radios nor code input

Important implementation detail:

- Prompt using the full visible redacted text from the page, not guessed
  labels like `email` or `sms`.

## Authenticated Home

Verified home URL:

- `https://mytargetcirclecard.target.com/home`

Observed controls / markers:

- blocking modal close button:
    - `button#close-btn-modal`
- statements navigation:
    - visible `View statements`
    - nav item `Statements`
- activity navigation:
    - `Print transactions`
    - `Go to full transaction history`

The financial-info modal can block progress and must be closed before
continuing.

## Statements Page

Verified statements URL:

- `https://mytargetcirclecard.target.com/statements`

Observed behavior:

- year tabs are exposed as DOM ids like:
    - `2026`
    - `2025`
    - `2024`
- statement rows include:
    - statement close date text like `03-03-2026`
    - `Download pdf`
    - `View`

Current scraper behavior:

- iterates all discovered year ids
- extracts visible rows from the statements table
- saves PDFs using the statement close date
- deduplicates with `listAccountDocuments()`

Verified saved filename pattern:

- `statement-YYYY-MM-DD.pdf`

## Transaction History Page

Verified transaction history URL:

- `https://mytargetcirclecard.target.com/account/transaction-history`

Observed controls:

- statement selector:
    - `select#security_q`
- transaction export launcher:
    - visible `Download transactions`
- print controls:
    - `Print transactions`
- visible recent transaction table
- separate pending-transactions table, often with `No data available`

Important site behavior:

- changing `select#security_q` changes the visible transaction table
- each option corresponds to a statement period
- examples observed:
    - `Current Statement`
    - `03-03-2026`
    - `02-03-2026`
    - ...
    - `04-04-2024`

## Download Transactions Modal

Clicking `Download transactions` does not immediately start a browser download.
It opens a modal with:

- modal file-type selector:
    - `select#user`
- modal submit action:
    - visible `Download`

Verified format choices:

- `Quickbooks (QBO)` with value `QBO`
- `Spreadsheet (Excel, CSV)` with value `CSV`
- `Quicken Web Connect (QFX)` with value `QFX`
- `Open Financial Exchange (OFX)` with value `OFX`

Important modal behavior:

- the modal closes after each successful download
- the scraper must reopen it for each format

## Activity Export Comparison

Live comparison on statement period `03-03-2026` showed:

- `QBO` and `QFX` are effectively redundant with `OFX`
- `QBO` / `QFX` mostly add Intuit wrapper fields like:
    - `INTU.USERID`
    - `INTU.BID`
- transaction records in `QBO`, `QFX`, and `OFX` otherwise matched closely

`CSV` contains fields not present in the OFX-family files:

- `Transaction Date`
- `Posting Date`
- `Ref#`
- `Description`
- `Last 4 of Card/Account`
- `Transaction Type`

`OFX` contains fields not present in CSV:

- `SIC` merchant category code
- statement period boundaries:
    - `DTSTART`
    - `DTEND`
- ledger balance:
    - `LEDGERBAL`
- normalized OFX transaction semantics:
    - `TRNTYPE`
    - `FITID`

Recommended scraper strategy:

- download `CSV` and `OFX`
- ignore `QBO` and `QFX` as redundant
- merge `CSV + OFX` during extraction if richer accounting output is needed

## Verified Probe Files

Example probe files captured for `03-03-2026`:

- CSV:
    - `probe-2026-03-03-csv.csv`
- OFX:
    - `probe-2026-03-03-ofx.ofx`
- redundant formats:
    - `probe-2026-03-03-qfx.qfx`
    - `probe-2026-03-03-qbo.qbo`

## Known Quirks

- The site can resume in arbitrary authenticated states, not just login.
- MFA uses multiple screens on the same URL.
- macOS secret retrieval can pause the run before password entry completes.
- The home modal blocks interaction until dismissed.
- `Download transactions` is modal-driven, not a direct file link.
- The transaction modal closes after each download.
- Browser-native download waits are required after clicking the modal's
  `Download` action.
- Two pages can remain open in the debug session with the same
  transaction-history URL; this has not blocked scraping so far.

## Current Direction

The current scraper direction is:

- statements:
    - download all missing statement PDFs
- activity:
    - download all missing `CSV + OFX` period exports
- extraction:
    - bronze extractor now emits rows from `CSV` and OFX-family files
    - later ETL can still merge `CSV + OFX` so we retain both bank-export
      semantics and the richer CSV-only fields

## Suggested Future Work

- Add extension-local extraction notes once the final `extract.mts` /
  built-`dist/extract.mjs` merge logic exists.
- Document any quirks found in `ofx-data-extractor` if Target starts shipping a
  different OFX/QFX shape.
- Confirm whether the login/session behavior changes for multiple card accounts
  under one login.
- Decide whether the scraper should save a structured HTML fallback only when
  native downloads fail, or always alongside downloaded files.
- Add stronger branch-specific logging for resumed sessions that start directly
  on transaction history or statements.
