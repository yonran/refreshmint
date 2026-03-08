# Scrape Browser Startup Investigation

## Problem

Linux browser integration tests started failing in GitHub Actions with:

```text
timed out after 30s creating initial page (about:blank)
```

The main failing run that triggered this investigation was:

- GitHub Actions run `22531366280`
- failing job `65262816741`

At that point every test in `scrape_integration` failed at browser startup, before driver logic ran.

## Observable Symptoms

The relevant startup sequence is:

1. Chrome binary is found.
2. `Browser::launch(...)` succeeds.
3. The Chromiumoxide handler loop starts.
4. Opening the first page hangs.

This is not a pure Linux-CI-only problem. A matching failure can be reproduced locally on macOS against the current browser stack.

## What Was Verified

### GitHub Actions environment

- The failing Linux job already runs inside `xvfb-run`.
- Chrome is installed by `browser-actions/setup-chrome`.
- Chrome launch succeeds.
- The failure happens after launch, during first-page creation.

So this is not simply “there is no window system in CI”.

### `browser-actions/setup-chrome` on ARM

Using `gh`, I checked `browser-actions/setup-chrome` directly.

Findings:

- Linux release-channel installs use Chrome for Testing downloads, not `apt`.
- Linux ARM is not supported for the stable/channel install path.
- There is no true Linux ARM equivalent of the GitHub Actions install path.

That means an ARM container cannot faithfully reproduce the GitHub Linux Chrome install path used in CI.

### Local container repro attempts

#### ARM repro

I built a persistent local repro container for ARM.

Findings:

- Ubuntu 24.04 ARM `chromium-browser` in-container is only a Snap wrapper.
- That path is unusable in this container setup.
- I added a fallback ARM browser path using Playwright Chromium only to get a real browser binary, but that does not match CI exactly.

#### AMD64 repro under Podman

I then switched to an `amd64` repro image and installed Chrome for Testing the same way `setup-chrome` does on Linux x64.

Findings:

- The container successfully installed Chrome for Testing.
- The browser binary reported:

```text
Google Chrome for Testing 146.0.7680.31
```

- But local `amd64` repro is blocked by emulation:
    - `rustc -vV` segfaults under the local Podman/QEMU path
    - therefore the Rust test binary cannot be used as a trustworthy local `amd64` repro on this machine

Conclusion:

- The `amd64` container path is not a reliable validation environment here.
- CI remains the real `amd64` source of truth.

## Local Non-Container Reproduction

I reproduced the startup failure locally by running the ignored scrape integration tests directly.

Representative command:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --test scrape_integration scrape_smoke_driver_writes_output -- --ignored --test-threads=1
```

And also with forced headless mode:

```sh
env REFRESHMINT_BROWSER_HEADLESS=1 cargo test --manifest-path src-tauri/Cargo.toml --test scrape_integration scrape_smoke_driver_writes_output -- --ignored --test-threads=1
```

Result:

- the first-page startup hang reproduces locally
- it is not specific to GitHub Actions

This is useful because it means the bug is likely in our browser automation stack or startup assumptions, not only the CI runner setup.

## Most Important Diagnostic Finding

Temporary logging in `src-tauri/src/scrape/browser.rs` showed this locally:

- Chrome launches successfully
- `fetch_targets()` sees real targets
- target list includes a page target such as `chrome://newtab/`
- those targets become `attached=true`
- but `browser.pages()` still returns `0`
- `browser.new_page("about:blank")` then times out as well

Representative pattern:

```text
Startup poll #1: targets=1, pages=0
Startup poll #2: targets=2, pages=0
...
page attached=true url=chrome://newtab/
...
Timed out creating about:blank ... final targets=3, final pages=0
```

That strongly suggests:

- Chrome is not failing to launch
- target discovery is not the core problem
- Chromiumoxide/chromey is failing somewhere between target attachment and usable `Page` creation

## Chromiumoxide / Chromey Investigation

The project currently depends on:

- `chromiumoxide = { package = "chromey", git = "https://github.com/spider-rs/chromey", branch = "main" }`

The lockfile originally pinned that to:

- `923e39d`

I inspected the dependency internals and found:

- `fetch_targets()` only discovers targets and queues attachment work
- `pages()` only returns pages once targets have a usable session/page handle
- `new_page()` relies on the same lower-level machinery

I also checked upstream `spider-rs/chromey` history and found a newer commit:

- `16b01771`
- message: `fix: tolerate missing params in CDP events for Chrome 145+ (#355)`

I updated `Cargo.lock` locally to that newer revision and reran the local repro.

Result:

- the browser startup hang still reproduced
- so that upstream Chrome-145 fix did not solve this specific issue

## Hypotheses Considered

### 1. Headed Chrome under Xvfb is flaky

Partially plausible, but not sufficient.

Why:

- the same failure reproduces locally
- forcing headless mode did not fix the hang
- both headed and headless paths still timed out creating the first page

### 2. Reusing existing startup tabs via `fetch_targets()` is wrong

This was plausible and worth testing.

Why:

- Chromiumoxide documents that `fetch_targets()` does not guarantee pages are ready immediately
- our startup code was trying to reuse existing tabs before falling back to `new_page`

I tested a direct `new_page("about:blank")` startup path without the `fetch_targets()` reuse path.

Result:

- direct `new_page()` still timed out

Conclusion:

- the bug is deeper than just the “reuse existing tab” strategy

### 3. Chrome / chromey compatibility problem around target attachment

This remains the strongest hypothesis.

Why:

- targets are visible
- targets are marked attached
- but no usable `Page` objects ever emerge
- both existing-page reuse and fresh-page creation fail

One specific sub-hypothesis is that chromey waits for an event-driven attachment transition that newer Chrome no longer guarantees in the way the library expects.

I inspected `Target.attachToTarget` handling and confirmed:

- the command response itself contains a `sessionId`

I started testing a temporary local dependency patch based on that idea, but did not get to a validated result before this checkpoint.

## Investigation Checkpoint Files

This checkpoint also includes the exact repo changes referenced during the investigation. They are still experimental and not validated as the final fix, but they are committed here so the note resolves to concrete artifacts:

- `.github/workflows/integration.yml`
- `src-tauri/Cargo.lock`
- `src-tauri/src/scrape/browser.rs`
- `.dockerignore`
- `docker/scrape-ci-repro.Dockerfile`
- `scripts/build-scrape-ci-container.sh`

These changes were used for investigation, not final resolution.

## Practical Conclusions So Far

1. The scrape startup failure is real and reproducible locally.
2. The problem is not simply missing X11/Xvfb on GitHub.
3. The problem is not ARM packaging or lack of Chrome alone.
4. The problem survives:
    - headed mode
    - new headless mode
    - old headless mode
    - direct `new_page()`
    - updated chromey commit `16b01771`
5. The likely fault boundary is chromey/Chromiumoxide page/session initialization against current Chrome.

## Best Next Steps

If investigation resumes, the highest-value next steps are:

1. Make the temporary local chromey patch reproducible and force a real rebuild of that dependency.
2. If that patch works, vendor or patch the dependency cleanly instead of adding more app-side launch heuristics.
3. If it does not work, add instrumentation inside chromey target/session initialization itself, not only in Refreshmint wrapper code.
4. Keep CI as the validation target for Linux `amd64`; local Podman `amd64` emulation is not trustworthy here because `rustc` segfaults under emulation.
