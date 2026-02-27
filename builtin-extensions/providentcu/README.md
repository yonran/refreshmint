# Provident CU Scraper Notes

## Overview

The Provident extension has three scrape phases:

1. Account activity CSV exports (`TransactionHistoryModule`)
2. Check image attachment discovery/download (from activity rows with check numbers)
3. Statement PDF downloads (`Statements & Notices`)

The scraper is state-machine based so `debug exec` can be re-run from an in-progress page.

## Activity CSV Downloads

For each account label, the scraper tries these date ranges when available:

- `Last 30 Days`
- `Last 60 Days`
- `Last 90 Days`
- `Last 120 Days`
- `All`

CSV documents are deduplicated against existing account documents by filename and
date-prefixed collision patterns.

## Check Attachment Linking

Check images are stored as account documents and linked by `attachmentKey`.

- `attachmentType`: `check-image`
- `attachmentKey`: `check:<checkNumber>|<YYYY-MM-DD>|<normalizedAmount>`
- `attachmentPart`: `front` / `back` / `single`

The extractor emits the same `attachmentKey` for check transactions, and core
dedup logic links matching document attachments as evidence refs.

## Check Capture Strategy

Check image capture is row-scoped to avoid false positives from unrelated page
controls/links:

- Expand the candidate transaction row.
- Click `View Check` from that row context (with a guarded host-level fallback).
- Collect visible check media resources (typically `data:image/jpeg;base64,...`)
  and wait for resource-set stabilization so sequential `front`/`back` loads are
  captured in one pass.
- Save each discovered part as a separate attachment (`front`, `back`, or
  `single`) under the same `attachmentKey`.

If binary fetch for a discovered resource fails, the scraper falls back to a
screenshot save for that attachment part.

## Month-Level Checkpointing

Historical month scans are checkpointed to avoid re-checking old months:

- checkpoint doc metadata:
    - `attachmentCheckpoint: true`
    - `attachmentType: check-image`
    - `checkpointMonth: YYYY-MM`
    - `checkpointVersion: v1`
    - `checkpointScope: providentcu-history-module`
    - `checkpointFinal: true`
    - `checkpointResult: found|none`

Behavior:

- Historical months with matching finalized checkpoint are skipped.
- Current month is always re-scanned every scrape run.
- Individual attachments are still deduplicated per `attachmentKey + attachmentPart`.

## Debug Workflow

Use the standard debug flow:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin app -- debug start ...
cargo run --manifest-path src-tauri/Cargo.toml --bin app -- debug exec ...
```

`debug exec` logs include:

- per-range activity CSV stats
- per-month attachment scan stats
- attachment download failures with check number/part context

## Known Quirks / Troubleshooting

- `View Check` can appear in different DOM scopes depending on row state.
  The scraper first tries row-scoped controls and only then uses guarded fallback.
- Front/back images may load sequentially after `View Check`; capture waits for the
  resource set to stabilize so both parts can be collected in one pass.
- Generic page links are not treated as attachment resources because they produce
  false positives and hide real check artifacts.

## Success Criteria

For a known check row, a successful run should usually show:

- attachment summary with no failures (`failed=0`)
- either `downloaded=2` (new front/back) or `existing=2` (already captured)
- documents including:
    - `...-<amount>-front.png`
    - `...-<amount>-back.png`
- metadata with:
    - `attachmentType: check-image`
    - `attachmentKey: check:<checkNumber>|<date>|<amount>`
    - `attachmentPart: front|back`

## Validation Runbook

1. Run scraper in debug or full scrape mode for `provident-yonran`.
2. Verify front/back attachment docs exist under the account documents folder.
3. Run `account extract` for the latest activity CSV if needed.
4. Confirm account journal check txn has `; evidence: ...#attachment` refs for
   check image parts.
5. If GL linkage needs revalidation, `account unpost` then `account post` that
   entry and confirm the GL txn includes the same attachment evidence refs.

## Implementation Notes

- The activity table selectors are centered on
  `.IDS-Banking-Retail-Web-React-TransactionHistoryModule`.
- Attachment capture is anchored to the selected check row and visible check
  media resources; generic page links are intentionally ignored.
- Keep `ATTACHMENT_CHECKPOINT_VERSION` and `ATTACHMENT_CHECKPOINT_SCOPE`
  stable; bump version when checkpoint semantics/selectors change.
