---
name: scraper-scaffolder
description: Scaffolds a new Refreshmint bank scraper extension, including manifest and driver template.
---

# Scraper Scaffolder

This skill helps you create a new scraper extension for Refreshmint. It provides a standardized directory structure and a state-machine-based driver template.

## Workflow

1.  **Understand the site**: Identify the base URL, login URL, and typical login fields.
2.  **Scaffold the extension**: Create the extension directory and use the templates in `assets/` to initialize `manifest.json` and `driver.mjs`.
3.  **Refine the manifest**: Update the `manifest.json` with the correct domain and secret names.
4.  **Implement the driver**: Fill in the `handleLogin` and `handleMfa` functions, and implement state-based routing in the `main` loop.

## Template Files

- `assets/manifest.json`: Base manifest structure with secret definitions.
- `assets/driver.mjs`: A robust driver template using a state machine and a progress tracker to prevent infinite loops.

## Best Practices

- **State Machine**: Use URL or DOM content to route the scraper into different states (e.g., `handleLogin`, `handleMfa`, `handleDashboard`, `handleStatements`).
- **Progress Tracking**: Always update the `progressName` and monitor `lastProgressStep` to avoid stalling.
- **Human Cadence**: Use `humanPace(page, min, max)` to avoid bot detection.
- **Log Frequently**: Use `refreshmint.log()` at every step and transition to aid debugging.
- **Wait for Busy**: Implement a `waitForBusy` helper to detect and wait for site-specific loading spinners.
- **Fail Loud — never swallow errors**: A scrape that errors must FAIL, not silently report success. Two anti-patterns that caused weeks of silent data loss:
    1. **Top-level swallow**: `main().catch((err) => refreshmint.log(err))` logs the error but lets the promise resolve, so the framework records the scrape as successful. The template ends with `await main();` (no swallowing `.catch`), so a thrown error rejects the top-level promise and the scrape is recorded as failed. If you want a failure snapshot, log **and re-throw** from inside `main`'s own handlers (see providentcu's `run`).
    2. **Capture-loop swallow**: a download/save loop that wraps each item in `try { ... } catch (e) { failed++; }` and continues, where the handler returns success regardless. If the loop captured **zero** items _because of_ errors (as opposed to legitimate "already exists"/"no rows" skips), `throw` so the scrape fails and retries. Counting failures into a stat and only logging them is not enough — nothing reads those stats.

    Rule of thumb: only swallow an error when you are deliberately probing/falling back (e.g., trying an alternate selector) and the swallow is the intended behavior. Anywhere a swallow can result in "captured nothing but reported success," fail loud instead.
