# Citi Website Notes

Verified against live Citi sessions while developing the `citi` Refreshmint extension.

## Login flow

- Initial login page reached at `https://www.citi.com/login?nextRoute=dashboard`
- Real login form is hosted on `www.citi.com`, not `online.citi.com`
- Verified login selectors:
    - username: `#username`
    - password: `#citi-input2-0`
    - submit: `#signInBtn`
- Secrets must therefore exist for `www.citi.com`
- `online.citi.com` secrets are still useful because authenticated pages move there after sign-in

## Authenticated states

- Successful login reached:
    - `https://online.citi.com/US/ag/dashboard/credit-card?...`
    - `https://online.citi.com/US/nga/accstatement`
- Verified logged-in markers:
    - `#signOffmainAnchor`
    - `#accountsmainAnchor0` or `#accountsMainLI`
- Verified account link pattern seen in nav:
    - `Costco Any...-3743`
- Verified live account title text on dashboard/statements page:
    - `Costco Anywhere Visa® Card by Citi - 3743`
- Current derived Refreshmint label for that account:
    - `costco_anywhere_visa_card_by_citi_3743`

## Stale logged-in shell

- Citi can leave the user authenticated but on a broken shell page instead of the dashboard
- Verified text marker for this state:
    - `looks like that information isn't here`
- Returning to `https://online.citi.com/US/ag/dashboard` recovers from that state

## Statements navigation

- In-app statements servicing page:
    - `https://online.citi.com/US/ag/servicing/index?pageName=StatementsAndDocumentServices`
- That page exposes a visible `Statements` link
- Clicking `Statements` leads to:
    - `https://online.citi.com/US/nga/accstatement`

## Dashboard activity

- Credit-card dashboard page reached at:
    - `https://online.citi.com/US/ag/dashboard/credit-card?accountId=...`
- Verified dashboard body markers:
    - `Your Activity`
    - `Transactions`
    - `Search & Filter`
    - `Time Period`
- Current debug viewport exposes the mobile-layout period selector:
    - button: `#ums-timePeriodDropdown-mobile`
    - listbox: `#ums-timePeriodDropdown-mobile-listbox`
- The desktop counterpart also exists in the DOM but is hidden in this viewport:
    - `#ums-timePeriodDropdown`
- Visible historical activity options observed in the dropdown:
    - `Statement closed Feb 16, 2026`
    - `Statement closed Jan 15, 2026`
    - `Statement closed Sep 15, 2025`
    - `Statement closed Aug 15, 2025`
    - `Statement closed Jul 15, 2025`
    - `Statement closed Jun 16, 2025`
    - `Last year (2025)`
    - `Year to date`
- Verified live behavior of broader activity options:
    - `Last year (2025)` shows posted 2025 transactions in the dashboard tile
    - `Year to date` shows current-year posted transactions and ends with `End of Activity`
- Current scraper behavior saves a per-account dashboard summary before activity extraction
- The summary is stored as:
    - `account-summary.json`
    - labeled with the Citi-derived account label rather than `_default`
- Verified dashboard summary fields available in body text:
    - `Statement closing Mar 16, 2026`
    - `Current Balance $172.56`
    - `Available Credit $5,327.44`
    - `Credit Limit: $5,500.00`
    - `Payment due on Mar 13, 2026`
    - `Last Statement Balance $172.56`
    - `Minimum Payment Due $41.00`
- Current scraper extracts and stores:
    - statement closing date
    - current balance
    - available credit
    - credit limit
    - payment due date
    - last statement balance
    - minimum payment due
- Current scraper reports these values live:
    - `citi_account_label`
    - `citi_current_balance`
    - `citi_rewards_earned_ytd`
- Plain `page.click(...)` on dropdown options was unreliable because Citi's custom menu had option hitbox overlap
- A synthetic pointer sequence on `.cds-option2-item-container` worked to change periods

## Transaction tile structure

- Main transaction tile:
    - `#ums-transaction-tile`
- Mobile-layout visible transaction rows:
    - `#ums-transaction-tile .transaction-body.onyx_enhanced_layout`
- Useful row descendants:
    - description: `.description`
    - top line with amount: `.top`
    - bottom line with date / notes: `.bottom`
    - note/chip text: `.transaction-chip`, `.chips-display`, `.chips-rewards-display`
- Example extracted rows from `Statement closed Feb 16, 2026`:
    - `COSTCO WHSE #0006 TUKWILA WA` / `$109.04` / `Feb 14, 2026`
    - `AUTOPAY 999990000025976RAUTOPAY AUTO-PMT` / `-$77.31` / `Feb 10, 2026`
    - `COSTCO WHSE #0001 SEATTLE WA` / `$63.52` / `Jan 27, 2026`

## Activity CSV output

- Citi does not yet have a verified live CSV download control in the visible mobile dashboard layout
- The DOM contains hidden `Export` / `Print` controls, but we have not activated Citi-native CSV download yet
- Verified hidden-only controls present in this viewport:
    - `Documents & Downloads`
    - `Export`
    - `Print`
    - aria-only `download`
- We can reliably extract transaction rows from the dashboard and save our own CSV files
- Activity CSVs are now also labeled with the derived account label instead of falling back to `_default`
- Verified saved activity files:
    - `activity/2026-02-16-transactions.csv`
    - `activity/2026-01-15-transactions.csv`
    - `activity/2025-09-15-transactions.csv`
    - `activity/2025-08-15-transactions.csv`
    - `activity/2025-07-15-transactions.csv`
    - `activity/2025-06-16-transactions.csv`
    - `activity/2025-last-year-transactions.csv`
    - `activity/2026-year-to-date-transactions.csv`

## Last year pagination

- The visible `Load More` text on `Last year (2025)` is currently rendered as:
    - `div.footer.footer-onyx-layout.ng-star-inserted`
- In this viewport it is not exposed as a normal clickable button or link
- Climbing ancestors from that text led only to the containing `cds-tile`
- We have not yet found a working automation path to extend `Last year (2025)` beyond the first visible 10 rows

## Account statements page

- Verified page title:
    - `Account Statements – Citibank`
- Verified body markers:
    - `Account Statements`
    - `Recent Statements`
    - `Select a Year`
    - `Older Statements`
    - `Your Requested Statements`
- Verified statement rows contain text like:
    - `February statement ending on February 16, 2026 View Download`
    - `January statement ending on January 15, 2026 View Download`
- Visible actions per recent statement row:
    - `View`
    - `Download`
- Current working scraper behavior anchors downloads to the row text containing `statement ending on`

## Year selector

- Verified year dropdown entries present on the page:
    - `2026`
    - `2025`
    - `2024`
- We have not automated year switching yet

## Statement downloads

- Verified recent statement downloads succeed from `/US/nga/accstatement`
- Saved filenames currently derived from statement end date:
    - `statements/YYYY-MM-DD-statement.pdf`
- Statement PDFs are now also labeled with the derived account label instead of falling back to `_default`
- Verified successful saves:
    - `statements/2026-02-16-statement.pdf`
    - `statements/2026-01-15-statement.pdf`
- If a filename already exists, the driver skips it

## Rewards

- Verified rewards text visible on the dashboard for this account:
    - `$5`
    - `Costco Cash Rewards`
    - `Earned YTD`
- Current scraper extracts dashboard rewards as:
    - `rewardsEarnedYtd`
- Verified live reported value:
    - `citi_rewards_earned_ytd: 5`
- Verified visible rewards tile on the dashboard:
    - `dashboard-rewards-tile#Rewards`
    - visible CTA: `View Details`
    - visible secondary action: `Explore Benefits`
- Verified `View Details` navigates to:
    - `https://online.citi.com/US/nga/reward/dashboard/costco/...`
- Verified rewards page title:
    - `Costco Rewards Details - Citibank`
- Verified rewards page fields:
    - `2026 Costco Cash Rewards`
    - `Earned Year to Date`
    - `2026 Costco Rewards Certificate`
    - `Certificate Status: Issued`
    - `On 02/16/2026 via Email`
    - `Current Certificate Number: 01`
    - `$60.71`
    - `2026 Credit Card Reward Certificate Amount`
    - `Rewards Summary`
    - statement-date selector currently showing `Year to Date`
    - per-category totals for:
        - `5%* Gas at Costco`
        - `4%* Other Eligible Gas & EV Charging`
        - `3% Restaurants`
        - `3% Eligible Travel`
        - `2% Costco and Costco.com`
        - `1% All Other Purchases`
- Verified rewards page controls:
    - `#CdsDropdown2_1` for rewards statement-date selection
    - `#certificate_cta` labeled `Access Certificate`
- Current scraper now saves a per-account rewards summary JSON:
    - `rewards/2026-costco-rewards-summary.json`
- Verified live reported values from rewards page:
    - `citi_rewards_earned_ytd: 5.00`
    - `citi_reward_certificate_amount: 60.71`
    - `citi_reward_certificate_status: Issued`
- We have not yet automated:
    - changing the rewards statement-date selector
    - downloading or opening the certificate behind `Access Certificate`
    - any rewards-specific PDF or CSV export

## Page structure notes

- The top-level page already contains the statement content needed for scraping
- Frame inspection showed several iframes, but the statement list did not require iframe traversal
- Extra UI observed on the page that may interfere later if surfaced more aggressively:
    - cookie consent widget
    - live chat / share-screen modal
    - income update prompt

## Open questions

- MFA has not been exercised in a live Citi session yet
- Older statement request flow has not been automated
- Multi-account behavior has not been exercised yet
- Annual account summary download has not been automated yet
- Citi-native activity export / CSV download has not been exercised yet

## TODO

- Exercise and implement the live MFA branch
- Remove the temporary statement PDF download limit and iterate all visible statement rows
- Automate the statements year selector for `2025`, `2024`, and older available years
- Probe `Request Older Statements` and capture how requested statements are surfaced for download
- Automate the rewards-page `Statement Date` selector for historical rewards snapshots
- Probe `Access Certificate` and determine whether the certificate can be downloaded or saved as an artifact
- Determine whether Citi exposes a native activity CSV export in a desktop or alternate layout
- Investigate custom date-range activity for pre-2025 transaction coverage
- Implement pagination or another path beyond the visible first page of `Last year (2025)` activity
- Exercise multi-account logins and ensure labels, rewards, activity, and statements stay account-scoped
- Automate annual account summary downloads from the visible `View 2025 Annual Account Summary` link
