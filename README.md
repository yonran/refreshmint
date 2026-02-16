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
