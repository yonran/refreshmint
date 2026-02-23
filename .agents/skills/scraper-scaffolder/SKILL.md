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
