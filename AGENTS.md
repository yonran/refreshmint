# Agent Guidelines

## Local Overrides

Also read [AGENTS.local.md](./AGENTS.local.md) if it exists for machine-local defaults and overrides.

## Commits

- Use subject + body format (blank line between).
- If the change is non-obvious, the body must explain why.
- Add a separate co-author paragraph after the body.
- The co-author line must include the exact model name and use the format:
    - For codex: `Co-Authored-By: Codex (<MODEL_NAME>) <no-reply@users.noreply.github.com>`
    - For gemini: `Co-Authored-By: Gemini (<MODEL_NAME>[, <MODEL_NAME>]) <no-reply@google.com>`
    - For claude: follow the `attribution` in Claude Code settings.
- As of `codex` v0.101.0, the AI does not have access to the specific model name. If the context does not have this information, then ask the user to run `/model` to get the correct model.
- Never use `git commit --no-verify` without first confirming with the user.

## Scraping

Read [scraper.md](./docs/scraper.md) before you edit any extension driver which scrapes an account.
