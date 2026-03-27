# Extension Authoring

This document covers extension structure, manifest fields, loading, and extension resolution.

## Directory layout

A runnable extension directory must include:

```text
my-extension/
  manifest.json
  driver.mjs         # legacy default
  # or another manifest-declared driver path such as src/driver.ts
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
    "driver": "src/driver.ts",
    "extract": "extract.mjs",
    "idField": "bankId",
    "autoExtract": true,
    "secrets": {
        "example.com": {
            "username": "bank_username",
            "password": "bank_password"
        }
    }
}
```

Fields:

- `name` (required for extension load): extension folder name under `<ledger>.refreshmint/extensions/<name>/`
- `driver` (optional): scraper entry module path. Defaults to `driver.mjs`.
- `secrets` (optional): map of domain to declared secret roles
    - preferred format:
        - `"domain": { "username": "secret_name", "password": "secret_name" }`
    - legacy format is still accepted during migration:
        - `"domain": ["secret_name_a", "secret_name_b"]`
- `rules` or `extract` (required, exactly one): choose one extraction method
- `rules`: hledger CSV rules path used by extraction
- `extract`: JS extraction script path exporting `extract(context)`
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

Builtin extensions may be authored as `.js`, `.mjs`, `.ts`, or `.mts` modules.
Scraper and extractor entrypoints can import sibling modules with relative ESM
imports, and TypeScript files are loaded with runtime type stripping for
erasable syntax only.

Shared extension compiler defaults live in `builtin-extensions/tsconfig.json`.
Individual extensions can add their own `tsconfig.json` that extends that base
when they need local `include` settings or additional editor configuration.

Non-erasable TypeScript syntax is intentionally unsupported at runtime. Avoid
constructs such as:

- `enum`
- `namespace`
- decorators
- parameter properties

Keep relative import specifiers explicit so runtime resolution matches checked
source, for example:

```ts
import { login } from './shared.ts';
```

Run:

```bash
npm run typecheck
npm run lint
```

Both commands include builtin extensions and `.agents/skills` source.
