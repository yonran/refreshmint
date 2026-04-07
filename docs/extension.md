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

Source-tree extensions that use npm dependencies may also include:

```text
my-extension/
  manifest.json
  package.json
  driver.mts
  extract.mts
```

For the full scrape + extract pipeline, add at least one extraction method:

```text
my-extension/
  manifest.json
  driver.mjs
  account.rules     # CSV rules-based extraction
  # or extract.mjs  # JS extraction script
```

Built/package artifacts should contain runtime-ready files only:

```text
my-extension/
  manifest.json
  dist/
    driver.mjs
    extract.mjs
    ...
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

`extension load` expects a runtime-ready directory or zip. If an extension uses
package imports in source, build it first so the loaded artifact only contains
relative runtime files.

## Built-in extensions

The following extensions are bundled with the app and available automatically
without a prior `extension load` step:

- `bankofamerica`
- `chase`
- `citi`
- `paypal`
- `providentcu`
- `target`

In **release builds** builtin extensions are built to runtime-ready artifacts
and then embedded in the binary. In **debug builds** the app reads directly from
the `builtin-extensions/` source tree (via the compile-time
`CARGO_MANIFEST_DIR` constant), so edits to those files are reflected
immediately without recompiling.

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

Source-tree extensions that include `package.json` may also import ESM packages
from ancestor `node_modules` directories during development. Package resolution
is intentionally disabled for built/package artifacts; built runtime files must
use only relative imports.

Development package resolution is browser-oriented and ESM-only:

- prefer `exports.browser`, then `exports.import`, then `exports.default`
- otherwise fall back to `module`
- otherwise fall back to `main` only when it resolves to ESM
- CommonJS / `require` entrypoints are rejected

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
npm run build:extensions
npm run typecheck
npm run lint
npm run lint-diff
```

Both `typecheck` and `lint` include builtin extensions and `.agents/skills` source.

`npm run lint-diff` uses `eslint-plugin-diff` to apply strict type-checked rules
(`no-unsafe-*`, etc.) only to lines present in the current `git diff --cached`
(staged changes). This allows gradual adoption of strict rules without requiring
all existing violations to be fixed at once.

To check all lines changed since a base commit (e.g. before opening a PR):

```bash
ESLINT_PLUGIN_DIFF_COMMIT=main npm run lint-diff
```

`npm run lint` uses `eslint-plugin-diff` and only reports ESLint violations on
lines that appear in the current `git diff --cached` (staged changes). This
allows strict type-checked rules (`no-unsafe-*`, etc.) to be enabled globally
while existing violations in untouched lines are suppressed until those lines
are modified.

To check all lines changed since a base commit (e.g. before opening a PR):

```bash
ESLINT_PLUGIN_DIFF_COMMIT=main npm run lint
```

## Build runtime-ready artifacts

Build one extension source tree into a loadable/packageable artifact:

```bash
node scripts/build-extensions.mjs \
  --extension-dir /path/to/my-extension \
  --out-dir /tmp/my-extension-built
```

Build all builtin extensions into a release-style output tree:

```bash
node scripts/build-extensions.mjs \
  --builtin-out-dir /tmp/refreshmint-builtins
```

The builder resolves package imports at build time and rewrites the emitted
runtime graph to relative imports under `dist/`. The exact emitted layout under
`dist/` is a build-tool detail and should not be hard-coded by runtime code.
