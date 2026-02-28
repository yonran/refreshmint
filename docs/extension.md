# Extension Authoring

This document covers extension structure, manifest fields, loading, and extension resolution.

## Directory layout

A runnable extension directory must include:

```text
my-extension/
  manifest.json
  driver.mjs
```

For the full scrape + extract pipeline, add at least one extraction method:

```text
my-extension/
  manifest.json
  driver.mjs
  account.rules     # CSV rules-based extraction
  # or extract.mjs  # JS extraction script
```

## `manifest.json`

Example:

```json
{
    "name": "my-extension",
    "extract": "extract.mjs",
    "idField": "bankId",
    "autoExtract": true,
    "secrets": {
        "example.com": ["bank_username", "bank_password"]
    }
}
```

Fields:

- `name` (required for extension load): extension folder name under `<ledger>.refreshmint/extensions/<name>/`
- `secrets` (optional): map of domain to secret names
- `rules` or `extract` (required, exactly one): choose one extraction method
- `rules`: hledger CSV rules path used by extraction
- `extract`: JS extraction script path (`extract.mjs`) exporting `extract(context)`
- `idField` (optional): source ID field used by extraction mapping
- `autoExtract` (optional): extraction preference flag (defaults to `true`)

## Extension locations

Loaded extension path:

```text
<ledger>.refreshmint/extensions/<name>/
```

Extension output path used by `refreshmint.saveResource(...)`:

```text
<ledger>.refreshmint/extensions/<name>/output/
```

Account document finalization target:

```text
<ledger>.refreshmint/accounts/<account>/documents/
```

## Load an extension

From directory:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin app -- \
  extension load /path/to/my-extension --ledger /path/to/ledger.refreshmint
```

From zip:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin app -- \
  extension load /path/to/my-extension.zip --ledger /path/to/ledger.refreshmint
```

Use `--replace` to overwrite an existing extension with the same manifest `name`.

## Built-in extensions

The following extensions are bundled with the app and available automatically
without a prior `extension load` step:

- `bankofamerica`
- `chase`
- `citi`
- `paypal`
- `providentcu`
- `target`

In **release builds** the extension files are embedded in the binary. In **debug
builds** the app reads directly from the `builtin-extensions/` source tree (via
the compile-time `CARGO_MANIFEST_DIR` constant), so edits to those files are
reflected immediately without recompiling.

## Extension resolution order

For scrape and extract commands, a plain extension name (e.g. `"paypal"`) is
resolved in this order:

1. Explicit `--extension` (if provided)
2. Account config value in `accounts/<account>/config.json`
3. Error

Once a name is selected it is resolved to a directory:

1. Built-in extension with that name (see list above)
2. Ledger-local copy under `extensions/<name>/`

Account config example:

```json
{
    "extension": "my-extension"
}
```

`extension` may also be a path to an unpacked extension directory, which bypasses
built-in resolution entirely.

## Type checking and linting

Builtin extension scripts are plain `driver.mjs` files, but they can still be
checked with TypeScript and ESLint using project-provided globals declarations.

Run:

```bash
npm run typecheck:extensions
npm run lint:extensions
```

`npm run typecheck` also includes extension type checks.
