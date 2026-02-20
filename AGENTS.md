# Agent Guidelines

## Commits

- Use subject + body format (blank line between).
- If the change is non-obvious, the body must explain why.
- Add a separate co-author paragraph after the body.
- The co-author line must include the exact model name and use the format:
    - `Co-Authored-By: Codex (<MODEL_NAME>) <codex@users.noreply.github.com>`
- As of `codex` v0.101.0, the AI does not have access to the specific model name. If the context does not have this information, then ask the user to run `/model` to get the correct model.
- Never use `git commit --no-verify` without first confirming with the user.
