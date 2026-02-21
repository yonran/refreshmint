# Plan: Decouple Online Logins from Balance Accounts

## Implementation Status (2026-02-21)

Status: complete in `main`, across commits `9fde2bd` through `1600075`.

Recent completion milestones:

- Login-centric CLI and Tauri commands shipped (login CRUD, login-keyed scrape, login+label extraction/reconcile, migration).
- Ledger migration and migration UI prompt shipped.
- GL mapping conflict detection and warning UI shipped.
- Scraping tab moved to login-based command usage.
- Login mappings management UI shipped (create/select/delete login, extension updates, label mapping set/remove).
- Login secrets UX now keys to active login selection, matching the login-centric model.

## Context

Currently, each hledger account name (e.g. `Assets:Chase:Checking`) is the primary key for everything: extension assignment, credentials, browser profile, and scraped documents. This is a 1:1 binding between "web login" and "balance account."

**Problem:** One web login (e.g. Chase) can access multiple balance accounts (checking, savings, credit card), and two logins could share the same joint account. We need a separate "login" entity that owns the web session concerns (extension, credentials, browser profile), with accounts nested under the login.

**Document routing:** The login config stores a label→GL account mapping. The extension tags each `saveResource()` call with a label, and the system routes documents to `logins/<login>/accounts/<label>/documents/`. Sub-account directories are auto-created on disk when an extension uses a new label. A `gl_account` field (null = ignored) controls whether that sub-account's data feeds into the general ledger.

## New Filesystem Layout

The top-level `accounts/` directory is removed. All account data lives under logins:

```
<ledger>.refreshmint/
  logins/
    <login_name>/
      config.json                           # LoginConfig
      accounts/
        <label>/                            # e.g. "checking", "savings", "cc"
          documents/
            2026-01-31-statement.pdf
            2026-01-31-statement.pdf-info.json
          journal.ndjson
  extensions/
    chase-driver/
      manifest.json
      driver.mjs
  general.journal
```

**`LoginConfig`** (`logins/<login_name>/config.json`):

```json
{
    "extension": "chase-driver",
    "accounts": {
        "checking": { "gl_account": "Assets:Chase:Checking" },
        "savings": { "gl_account": "Assets:Chase:Savings" },
        "cc": { "gl_account": null }
    }
}
```

- `extension`: which scraper extension to use
- `accounts`: map of label → `LoginAccountConfig`. Each label is a login-local identifier used by the extension in `saveResource()`.
- `gl_account`: the hledger account name this maps to. If `null`, documents are still scraped/stored but not extracted or reconciled into the GL. This handles ignoring a CC or avoiding duplicate imports for a joint account.

**`LoginAccountConfig`**:

```rust
pub struct LoginAccountConfig {
    pub gl_account: Option<String>,  // None = ignored for GL purposes
}
```

## Per-Login Lock

A file lock (`logins/<login_name>/.lock`) is acquired before any operation that mutates login state: scraping, debug sessions, and config writes. This prevents concurrent scrapes or CLI+UI races on the same login.

- `acquire_login_lock(ledger_dir, login_name) → Result<LoginLock>` — creates `logins/<login_name>/.lock` and acquires an exclusive `flock`/`LockFile` on it. Returns a guard that releases on drop.
- `run_scrape_async` acquires the lock at the start and holds it for the duration.
- `debug start` acquires the lock similarly.
- Config-mutating commands (`set_login_account`, `set_login_extension`, `remove_login_account`) acquire the lock for the duration of the read-modify-write.
- Use the `fs2` crate (`File::lock_exclusive()`) which works cross-platform (flock on Unix, LockFileEx on Windows).
- If the lock is already held, the operation returns an error: "login '<name>' is currently in use by another operation".

## Label Validation

Labels are used as directory names. They must be validated:

- Allowed characters: alphanumeric, hyphens, underscores, dots (no colons, slashes, backslashes)
- Must not be empty, `.`, or `..`
- Max length: 255 characters
- Validation function: `validate_label(label: &str) -> Result<(), String>`
- Applied at: `set_login_account` command, and in `finalize_staged_resources` when the extension provides a label via `saveResource()`. If the extension provides an invalid label, `finalize_staged_resources` returns an error for that resource.

## GL Account Uniqueness Invariant

Each GL account name (e.g. `Assets:Chase:Checking`) must be populated by at most one login account across all logins in the ledger. This is enforced:

- **On `set_login_account`**: Before setting `gl_account`, scan all `logins/*/config.json` to check no other (login, label) pair already maps to the same GL account. Return an error if a conflict is found.
- **On ledger open**: Scan all login configs and return a list of duplicate GL account conflicts as structured data (Vec of `GlAccountConflict { gl_account, entries: Vec<(login, label)> }`). The UI displays these as actionable warnings with links to resolve (set one to `null`). Extraction and reconciliation for conflicting GL accounts are blocked until resolved — the `run_extraction` and `reconcile_entry` commands check for conflicts and return an error if the target GL account has duplicates.

## Migration from Old Format

Existing ledgers have `accounts/<account_name>/` at the top level with `config.json` containing `{ "extension": "..." }`. Secrets are in the OS keychain under `refreshmint/<account_name>`.

**`refreshmint migrate` CLI command** (also exposed as a Tauri command for UI):

1. Scan all `accounts/<account_name>/config.json` files
2. Group by extension name — accounts sharing the same extension become sub-accounts of one login
3. For each group, create a login:
    - Login name = extension name (e.g. `chase-driver`), or user-provided via `--login-name` flag
    - For each account in the group, create a label derived from the account name (last segment, sanitized). E.g. `Assets:Chase:Checking` → `checking`
    - Set `gl_account` to the original account name
4. Move `accounts/<account_name>/documents/` → `logins/<login_name>/accounts/<label>/documents/`
5. Move `accounts/<account_name>/journal.ndjson` → `logins/<login_name>/accounts/<label>/journal.ndjson`
6. Copy keychain entries from `refreshmint/<account_name>` to `refreshmint/login/<login_name>` (only once per login, not per account)
7. Remove the old `accounts/<account_name>/` directory after successful move
8. Update `DocumentInfo` sidecars in-place: add `loginName` and `label` fields

**Migration is idempotent:** If `logins/` already exists for some entries, skip them.

**Dry-run mode:** `refreshmint migrate --dry-run` prints what would happen without making changes.

## Implementation Steps

### Phase 1: Login storage layer — new `login_config.rs`

Create `src-tauri/src/login_config.rs`:

```rust
pub struct LoginAccountConfig {
    pub gl_account: Option<String>,
}

pub struct LoginConfig {
    pub extension: Option<String>,
    pub accounts: BTreeMap<String, LoginAccountConfig>,
}
```

Functions:

- `validate_label(label: &str) → Result<(), String>` — reject colons, slashes, `..`, empty, etc.
- `login_config_path(ledger_dir, login_name) → PathBuf` — `logins/<login_name>/config.json`
- `read_login_config(ledger_dir, login_name) → LoginConfig`
- `write_login_config(ledger_dir, login_name, config) → Result`
- `list_logins(ledger_dir) → Vec<String>` — scan `logins/` directory
- `login_account_documents_dir(ledger_dir, login_name, label) → PathBuf`
- `login_account_journal_path(ledger_dir, login_name, label) → PathBuf`
- `check_gl_account_uniqueness(ledger_dir, login_name, label, gl_account) → Result<(), String>` — scan all login configs, error if `gl_account` is already mapped elsewhere
- `acquire_login_lock(ledger_dir, login_name) → Result<LoginLock>` — exclusive file lock on `logins/<login>/.lock`; returns a guard that releases on drop

Extract the atomic `replace_file` helper from `account_config.rs` into a shared utility. Add `fs2` crate dependency for cross-platform file locking.

### Phase 2: Tauri commands for login CRUD

Add to `src-tauri/src/lib.rs`:

- `list_logins(ledger) → Vec<String>`
- `get_login_config(ledger, login_name) → LoginConfig`
- `create_login(ledger, login_name, extension) → ()`
- `set_login_extension(ledger, login_name, extension) → ()`
- `delete_login(ledger, login_name) → ()` — **refuses if any sub-account has documents or journal entries**. User must manually remove data first. (Future: `--force` flag for CLI.)
- `set_login_account(ledger, login_name, label, gl_account: Option<String>) → ()` — validates label, checks GL account uniqueness
- `remove_login_account(ledger, login_name, label) → ()` — refuses if sub-account dir has documents/journal data

### Phase 3: Login-keyed secrets

`SecretStore` is generic (takes any string). No internal changes needed. Use `SecretStore::new(format!("login/{login_name}"))` to namespace login secrets separately from legacy account-keyed secrets.

Add Tauri commands:

- `list_login_secrets(login_name) → Vec<AccountSecretEntry>`
- `sync_login_secrets_for_extension(ledger, login_name, extension) → SecretSyncResult`
- `add_login_secret(login_name, domain, name, value) → ()`
- `reenter_login_secret(login_name, domain, name, value) → ()`
- `remove_login_secret(login_name, domain, name) → ()`

### Phase 4: Login-centric scrape orchestration

**`ScrapeConfig`** — replace `account: String` with `login_name: String`:

```rust
pub struct ScrapeConfig {
    pub login_name: String,
    pub extension_name: String,
    pub ledger_dir: PathBuf,
    pub profile_override: Option<PathBuf>,
    pub prompt_overrides: js_api::PromptOverrides,
    pub prompt_requires_override: bool,
}
```

In `run_scrape_async`:

- `SecretStore::new(format!("login/{login_name}"))`
- `profile::resolve_profile_dir(ledger, login_name, ...)` — browser profile keyed by login
- `RefreshmintInner.account_name` → `RefreshmintInner.login_name`

**`StagedResource`** gains `label: Option<String>`. In `js_api.rs`, `saveResource()` JS API accepts an optional `label` string in the options object.

**`finalize_staged_resources`** changes:

- For each resource, validate the label (if provided) using `validate_label()`. Invalid labels cause that resource to error.
- Compute target dir: `logins/<login>/accounts/<label>/documents/`. If label is `None`, use `"_default"`.
- Auto-create the sub-account **directory** on disk (`create_dir_all`).
- Auto-add the label to `LoginConfig.accounts` with `gl_account: None` if not already present. This is a read-modify-write on the config file, safe because the per-login file lock is held for the duration of the scrape.
- `DocumentInfo` gains `login_name: String` and `label: String` fields (replacing `account_name`).

**Step 10 auto-save** writes extension to `logins/<login>/config.json`.

### Phase 5: Update extract, reconcile, and journal operations

`account_journal.rs` functions currently take `account_name` and compute `accounts/<name>/...`. Change these to take explicit paths or `(login_name, label)`:

- `account_documents_dir` → parameterize or add `login_account_documents_dir`
- `account_journal_path` → similarly
- `read_journal`, `write_journal`, `append_entry` — accept path directly or `(login, label)`

`extract.rs` (`list_documents`, `run_extraction`) — accept `(login_name, label)` and use login-relative paths. Check `LoginAccountConfig.gl_account` — if `None`, refuse extraction with a message explaining the account is ignored.

`reconcile.rs` — accept `(login_name, label)`, use `gl_account` as the hledger account name in postings.

### Phase 6: Migration

Add `refreshmint migrate` CLI command and `migrate_ledger` Tauri command:

- Scans `accounts/` directory
- Groups by extension, creates logins
- Moves documents/journals into `logins/<login>/accounts/<label>/`
- Copies keychain secrets from `refreshmint/<account>` to `refreshmint/login/<login>`
- Supports `--dry-run`
- Idempotent (skips already-migrated entries)

### Phase 7: CLI changes

Add `login` subcommand to `src-tauri/src/cli.rs`:

```
refreshmint login list
refreshmint login create --name NAME --extension EXT
refreshmint login set-extension --name NAME --extension EXT
refreshmint login delete --name NAME
refreshmint login set-account --name NAME --label LABEL [--gl-account ACCOUNT]
refreshmint login remove-account --name NAME --label LABEL
```

Change `scrape` to take `--login` instead of `--account`:

```
refreshmint scrape --login chase-personal [--extension override]
```

Change `secret` to take `--login` instead of `--account`:

```
refreshmint secret add --login chase-personal --domain chase.com --name password --value ...
refreshmint secret list --login chase-personal
```

Change `account` subcommands to take `--login NAME --label LABEL`:

```
refreshmint account extract --login chase-personal --label checking [--extension ext]
refreshmint account journal --login chase-personal --label checking
refreshmint account reconcile --login chase-personal --label checking --entry-id ID --counterpart-account ACCT
```

Change `debug start` to take `--login`:

```
refreshmint debug start --login chase-personal --extension chase-driver
```

Add migration:

```
refreshmint migrate [--dry-run]
```

### Phase 8: TypeScript types and command wrappers

Add to `src/tauri-commands.ts`:

- `LoginAccountConfig` type (`{ glAccount: string | null }`)
- `LoginConfig` type (`{ extension?: string, accounts: Record<string, LoginAccountConfig> }`)
- All new command wrappers from Phase 2, 3
- `runScrapeForLogin(ledger, loginName, extension)` — the Tauri command name is `run_scrape_for_login`
- `migrateLedger(ledger, dryRun)` — wraps `migrate_ledger`

Update existing wrappers that take `account_name` to take `login_name` + `label`.

### Phase 9: Frontend UI

Rework `src/App.tsx` Scraping tab to be login-centric:

1. **Login selector** — dropdown of existing logins + "Create new login" button
2. **Login details panel**:
    - Extension picker
    - Credentials panel (same UX, keyed by login name)
    - Account mapping table:
      | Label | GL Account | Status |
      |-------|-----------|--------|
      | checking | Assets:Chase:Checking | active |
      | savings | Assets:Chase:Savings | active |
      | cc | _(ignored)_ | disabled |
      | + Add mapping | | |
3. **Run scrape** button calls `runScrapeForLogin`
4. **Migration prompt** — if `accounts/` exists at ledger root, show a banner offering to run migration

The Accounts tab and Transactions tab enumerate accounts across all logins (iterate `logins/*/accounts/*/`).

## Key Files to Modify

- `src-tauri/src/login_config.rs` — **NEW**: LoginConfig, LoginAccountConfig, validate_label, path helpers, GL uniqueness check
- `src-tauri/src/account_config.rs` — keep for migration, may deprecate
- `src-tauri/src/account_journal.rs` — parameterize path functions to work with login-relative paths
- `src-tauri/src/scrape.rs` — ScrapeConfig becomes login-centric, finalize routes by label with validation
- `src-tauri/src/scrape/js_api.rs` — add `label` to StagedResource and saveResource() JS API, rename account_name → login_name in RefreshmintInner
- `src-tauri/src/scrape/profile.rs` — callers pass login_name instead of account_name
- `src-tauri/src/secret.rs` — no changes (callers use different key)
- `src-tauri/src/extract.rs` — accept (login, label), check gl_account before extraction
- `src-tauri/src/reconcile.rs` — accept (login, label), use gl_account for postings
- `src-tauri/src/lib.rs` — new Tauri commands, update existing command signatures
- `src-tauri/src/cli.rs` — new login subcommand, migrate command, update existing commands
- `src/tauri-commands.ts` — new types and wrappers
- `src/App.tsx` — login-centric UI, migration banner

## Verification

1. **Unit tests for login_config.rs**: read/write/list, validate_label (valid names, reject colons/slashes/`..`/empty), GL account uniqueness check
2. **Label validation test**: Extension provides invalid label in saveResource() → finalize returns error
3. **GL uniqueness test**: Setting the same GL account on two different login accounts → error
4. **delete_login test**: Refuses when documents exist, succeeds when empty
5. **Migration test**: Create old-format ledger with `accounts/`, run `migrate`, verify new layout and keychain entries
6. **CLI smoke test**: `refreshmint login create`, `login set-account`, `secret list --login`, `scrape --login`
7. **Scrape integration test**: Create login with mappings, run scrape with labeled saveResource calls, verify documents in `logins/<login>/accounts/<label>/documents/`
8. **Ignored account test**: `gl_account: null` → documents stored but extraction refused
9. **Build**: `cargo build` and `npm run build` pass
