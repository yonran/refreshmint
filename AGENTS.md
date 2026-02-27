# Agent Guidelines

## Local Overrides

Also read [AGENTS.local.md](./AGENTS.local.md) if it exists for machine-local defaults and overrides.

## Commits

- Use subject + body format (blank line between).
- If the change is non-obvious, the body must explain why.
- Add a separate co-author paragraph after the body.
- The co-author line must include the exact model name and use the format:
    - For codex: `Co-Authored-By: Codex (<MODEL_NAME>) <199175422+chatgpt-codex-connector[bot]@users.noreply.github.com>`
    - For gemini: `Co-Authored-By: Gemini (<MODEL_NAME>[, <MODEL_NAME>]) <218195315+gemini-cli@users.noreply.github.com>`
    - For claude: follow the `attribution` in Claude Code settings.
- As of `codex` v0.101.0, the AI does not have access to the specific model name. If the context does not have this information, then ask the user to run `/model` to get the correct model.
- Never use `git commit --no-verify` without first confirming with the user.
- If the user asks to "commit between each change", create a commit after each logically complete fix (not one large batch at the end).
- Stage only files that belong to the requested change; do not include generated output directories by accident.

## Debug Sessions

- Keep `debug start` running while iterating on scraper code.
- After script edits, re-run `debug exec`; do not restart `debug start` unless the socket/session is broken or login state must be reset.
- If a debug run is interrupted/aborted, verify whether partial staged resources were finalized before re-running.

## Scraping

Read [scraper.md](./docs/scraper.md) before you edit any extension driver which scrapes an account.
