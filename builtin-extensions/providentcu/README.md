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

## Implementation Notes

- The activity table selectors are centered on
  `.IDS-Banking-Retail-Web-React-TransactionHistoryModule`.
- Attachment capture is anchored to the selected check row and visible check
  media resources; generic page links are intentionally ignored.
- Keep `ATTACHMENT_CHECKPOINT_VERSION` and `ATTACHMENT_CHECKPOINT_SCOPE`
  stable; bump version when checkpoint semantics/selectors change.
