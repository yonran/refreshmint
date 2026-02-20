# Extractor Authoring

This document covers extraction behavior and document metadata sidecars.

For extension structure and manifest details, see `docs/extension.md`.

## Run extraction

Run extraction for all documents in an account:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin app -- \
  account extract \
  --ledger /path/to/ledger.refreshmint \
  --account Assets:Checking \
  --extension my-extension
```

Run extraction for specific documents:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin app -- \
  account extract \
  --ledger /path/to/ledger.refreshmint \
  --account Assets:Checking \
  --document 2026-02-01-statement.csv \
  --document 2026-03-01-statement.csv
```

`--extension` is optional when account config already has `extension`.

Document selection behavior:

- no `--document`: use all listed account documents
- with `--document`: use only provided names (trimmed, deduplicated)
- empty/whitespace document names are rejected

## Extraction methods

Extraction config comes from extension `manifest.json`:

- `rules`: hledger CSV rules file path
- `extract`: JS extractor script path (`extract.mjs`)
- `idField`: optional source ID field

Manifest contract:

- exactly one of `rules` or `extract` must be defined
- defining both is an error
- defining neither is an error

## Example: JS extractor

Minimal extension shape:

```text
my-extension/
  manifest.json
  driver.mjs
  extract.mjs
```

`manifest.json`:

```json
{
    "name": "my-extension",
    "extract": "extract.mjs"
}
```

`extract.mjs`:

```js
export async function extract(context) {
    if (!Array.isArray(context.csv) || context.csv.length < 2) {
        return [];
    }

    // context.csv includes header row at index 0.
    return context.csv.slice(1).map((row, i) => ({
        tdate: row[0],
        tstatus: 'Cleared',
        tdescription: row[1],
        tcomment: '',
        ttags: [
            ['evidence', `${context.document.name}:${i + 2}:1`],
            ['bankId', row[2]],
        ],
    }));
}
```

`extract(context)` must return an array of transactions compatible with the `ExtractedTransaction` schema (`tdate`, `tstatus`, `tdescription`, `tcomment`, `ttags`, optional `tpostings`).

### `context` shape

`context` always includes:

- `ledgerDir`, `accountName`, `extensionName`
- `document`: `{ name, path, format }`
- `documentInfo` (if sidecar exists): parsed `*-info.json`

For CSV documents, `context.csv` is provided as `string[][]` (UTF-8 rows, including header row).

For PDF documents, `context.pdf` is provided as:

- `pages[]`
- each page: `pageNumber`, `width`, `height`, `text`, `items[]`
- each item: `text`, `left`, `top`, `width`, `height`

Current PDF implementation uses `lopdf` text extraction (no external shared library).  
`items[]` are line-based with synthetic geometry. Page `width` / `height` come from `CropBox` (fallback `MediaBox`) when present.

## Example: rules-based extractor

Minimal extension shape:

```text
my-extension/
  manifest.json
  driver.mjs
  account.rules
```

`manifest.json`:

```json
{
    "name": "my-extension",
    "rules": "account.rules",
    "idField": "txid"
}
```

`account.rules`:

```text
fields date, description, amount, transactionid
skip 1
date-format %Y/%m/%d
date %date
description %description
amount %amount
comment txid:%transactionid
account1 Assets:Checking
account2 Equity:Unreconciled:Checking
```

Sample CSV content (save as `2026-01-31-sample.csv` under account documents):

```csv
Date,Description,Amount,TransactionId
2026/01/03,Coffee Shop,-5.25,abc123
2026/01/04,Payroll,1000.00,abc124
```

Run extraction:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin app -- \
  account extract \
  --ledger /path/to/ledger.refreshmint \
  --account Assets:Checking \
  --extension my-extension \
  --document 2026-01-31-sample.csv
```

With `idField: "txid"`, extractor output includes `bankId` when the parsed transaction has a `txid` tag.

Rules mode only accepts CSV documents.

## Pipeline behavior

At a high level:

1. read account documents
2. extract proposed transactions per document
3. validate evidence references against each document name
4. run dedup against existing account journal
5. write updated journal entries

If no documents are available for the account, extraction exits cleanly with a message.

## Document metadata sidecars

Extensions should not write `*-info.json` files manually.
Refreshmint writes sidecars automatically when staged resources are finalized after scrape.

Finalized files:

```text
<ledger>.refreshmint/accounts/<account>/documents/<date>-<filename>
<ledger>.refreshmint/accounts/<account>/documents/<date>-<filename>-info.json
```

Typical sidecar payload:

```json
{
    "mimeType": "text/csv",
    "originalUrl": "https://example.com/export.csv",
    "scrapedAt": "2026-02-20T04:10:00Z",
    "extensionName": "my-extension",
    "accountName": "Assets:Checking",
    "scrapeSessionId": "20260219-201030",
    "coverageEndDate": "2026-01-31",
    "dateRangeStart": "2026-01-01",
    "dateRangeEnd": "2026-01-31"
}
```

To improve metadata from the scraper script:

- pass `coverageEndDate` to `refreshmint.saveResource(..., options)`
- call `refreshmint.setSessionMetadata({ dateRangeStart, dateRangeEnd })`
