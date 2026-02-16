# refreshmint

Desktop ledger app built with Tauri, React, TypeScript, and Rust.

## Prerequisites

- Node.js 20+
- Rust stable (`rustup`, `cargo`)
- Tauri system dependencies for your OS:
    - Linux: `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev`
- `hledger` (for running in dev)

## Install Dependencies

```bash
npm ci
```

If you have `npm config get ignore-scripts` enabled for security purposes, then the `prepare` script will not be run when you `npm install`. If you wish to install husky precommit hooks, then you need to manually set up git hooks:

```bash
npx husky
```

This installs pre-commit hooks that run type checking and linting before each commit.

To bypass hooks when needed (e.g., WIP commits):

```bash
git commit --no-verify -m "WIP: debugging"
```

## Run Locally

```bash
npm exec tauri dev
```

## Build a Bundle Locally

CI currently builds one bundle type per target:

- `aarch64-apple-darwin` -> `app`
- `x86_64-apple-darwin` -> `app`
- `x86_64-unknown-linux-gnu` -> `deb`
- `x86_64-pc-windows-msvc` -> `nsis`

Example (macOS arm64):

```bash
npm ci
bash scripts/download-sidecar.sh aarch64-apple-darwin
npm exec tauri build -- --target aarch64-apple-darwin --bundles app
```

Bundle output is written under:

```text
src-tauri/target/<target>/release/bundle/
```

## Run CI Workflow Locally with `act` (No Docker)

Example matching the macOS arm64 CI matrix entry:

```bash
act pull_request \
  -W .github/workflows/build.yml \
  -j build \
  --matrix os:macos-latest \
  --matrix target:aarch64-apple-darwin \
  --matrix bundle:app \
  -P macos-latest=-self-hosted \
  --container-daemon-socket - \
  --no-cache-server \
  --env PATH="$HOME/.cargo/bin:$PATH"
```

Notes:

- Use `-P macos-13=-self-hosted` for `x86_64-apple-darwin`.
- The workflow skips `actions/upload-artifact` when `ACT=true`.

## Scrape Smoke Test

There is an ignored integration test that verifies the scrape pipeline can launch a browser, execute a driver script, and write output.

Prerequisites:

- Chrome or Edge installed locally

Run it manually (recommended for periodic checks / CI jobs):

```bash
cargo test --manifest-path src-tauri/Cargo.toml --test scrape_integration -- --ignored --nocapture
```
