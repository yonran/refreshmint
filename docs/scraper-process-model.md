# Scraper processes and MCP debugging

Refreshmint runs each GUI/CLI scrape or debug browser in a separate
`scraper-worker` process. The worker, rather than the Tauri app or MCP server,
owns Chrome, reads scraper credentials from Keychain, and holds the login lock.
Keeping this executable separate also keeps its macOS code identity stable when
unrelated app code is rebuilt.

## Processes and lifetimes

There is no global scraper daemon.

| Process                      | Lifetime                                             | Owns                                                                                   |
| ---------------------------- | ---------------------------------------------------- | -------------------------------------------------------------------------------------- |
| Tauri app                    | The desktop app                                      | UI and a private pipe connection to each GUI-started scrape worker                     |
| `scraper-worker run-scrape`  | One GUI or CLI scrape, unless retained after failure | One Chrome process tree, one initial tab, and the login lock                           |
| `scraper-worker debug-start` | One explicit debug session                           | One Chrome process tree, one initial tab, the debug socket, and the login lock         |
| `refreshmint-mcp`            | One MCP client connection (stdio)                    | Session selection and workers that this MCP process started; no browser or credentials |

A debug **session** is a discoverable worker/browser pair. A debugger **exec**
is one command within that session; it is not a new scrape run and does not
open another tab. Multiple MCP façade processes may connect to the same session
and therefore control the same tab. Separately started sessions have separate
workers, Chrome processes, profiles, and initial tabs.

A Refreshmint **login** is the configured identity under
`logins/<login-name>/`. It chooses the extension, Chrome profile, and Keychain
namespace. It is not an MCP login or an MCP protocol concept. An MCP **client**
is the host program that launches `refreshmint-mcp`, such as Codex or Claude.

## Locks and competing callers

The worker acquires the existing `logins/<login-name>/.lock` before launching
Chrome and keeps it until Chrome and the debug session close. The Tauri app and
MCP façade do not acquire this lock on the worker's behalf. There is no second
profile lock and no lock handoff. Chrome's own profile locking remains a backup
diagnostic, not the coordination mechanism.

If the app and MCP both try to start the same login, whichever worker acquires
the login lock first proceeds and the other fails immediately. The existing
`.gl.lock` is unrelated and remains limited to general-ledger mutations;
scraper workers do not acquire it.

## Failed scrapes

A manually started GUI scrape has failure retention enabled. If the driver
fails (as opposed to user cancellation), Refreshmint captures the normal
failure artifacts and turns that exact worker, Chrome process, and tab into a
`failed-scrape` debug session. It does not release and reacquire the login lock.
The GUI receives the scrape failure immediately; the detached worker remains
discoverable for 30 minutes, or until a debugger stops it.

Automatic/background scrapes are not retained. Successful and canceled scrapes
close their browsers normally.

## Interactive human challenges

An app-started manual scraper can call `page.solveHumanChallenge(...)` when a site
shows an interactive CAPTCHA or verification control. The worker temporarily
captures viewport JPEG frames and sends them over its inherited private pipe.
The app displays that live tab and relays the user's physical pointer
down/move/up sequence back to Chrome with `Input.dispatchMouseEvent`; Chromium
delivers those page events as trusted input. This works for headed and headless
workers because it operates on the CDP page target rather than the native
Chrome window.

The stream exists only while the challenge prompt is open. Frames and pointer
events are neither persisted as scrape artifacts nor published through the
debug socket or MCP façade. CLI and MCP-started sessions do not install this
app-only channel and fail clearly if a driver requests it. Automatic scrapes
also fail instead of waiting for an unattended prompt; manual prompts use the
scrape's configured prompt timeout.

## MCP routing

Configure an MCP host to launch the bundled `refreshmint-mcp-server` executable
(the development target is named `refreshmint-mcp`). Each launch creates a
small stdio façade with these tools:

- `refreshmint_list_sessions` lists manual, MCP-started, and failed-scrape
  browser sessions.
- `refreshmint_select_session` chooses the target for later commands.
- `refreshmint_start_debug` starts a new worker/browser for a ledger login and
  selects it.
- `refreshmint_debug_exec` runs JavaScript in the selected browser.
- `refreshmint_stop_debug` closes the selected browser and releases its login
  lock.

If exactly one session exists, the façade selects it automatically. That
automatic selection is cleared if another session appears, preventing a later
command from silently targeting an old tab. With more than one, the client must
list and explicitly select a session (or pass `sessionId` to an exec/stop call).
An explicit selection remains stable while that session exists. Selection is
local to one MCP façade process; it does not change another MCP client's
selection.

Workers publish credential-free routing records under the user's cache
directory at `refreshmint/debug-sessions/`. Each record contains a random
session ID, login name, kind, PID, ledger path, and Unix socket path. Registry
directories are mode `0700`, records and sockets are mode `0600`, and records
are removed when sessions end. Discovery also removes records whose socket has
disappeared after an unclean worker exit.

## Security model

This is a local convenience boundary, like browser-debugging automation, not a
sandbox against an already authorized debugger:

- Keychain values are resolved only inside `scraper-worker`; neither the Tauri
  app nor `refreshmint-mcp` receives the resolved credential as protocol data.
- A worker lazily reads both fields for a domain in one Keychain operation and
  caches them only for that worker session, avoiding separate index, username,
  and password authorization prompts.
- Known credentials and free-text prompt answers are redacted from logs,
  errors, debug output, failure URLs, and MCP results. Password fields are
  masked in snapshots.
- Screenshots and downloaded documents are raw evidence and are not exposed by
  the MCP tools.
- Live human-challenge frames and pointer events travel only over the private
  app/worker pipe and are not available to MCP clients.
- Owner-only Unix socket and registry permissions prevent other local OS users
  from attaching. Any process running as the same OS user can still find and
  connect to an open debug socket.

The MCP host's approval to configure and launch `refreshmint-mcp` is the caller
authorization step. Refreshmint does not maintain a second XPC-style caller
whitelist, and MCP itself does not standardize one for local stdio servers.
After approval, arbitrary debug JavaScript is intentionally powerful: it can
navigate, submit forms, and change page state. Output redaction prevents
ordinary accidental disclosure, but deliberately transforming a credential or
exfiltrating it through page/network behavior cannot be made safe by string
redaction. Do not configure this MCP server for clients you do not trust with
the selected browser session.

The app-to-worker protocol is line-delimited typed JSON over inherited pipes;
debug workers use a private Unix socket. HTTP would add routing and streaming
machinery without providing a stronger local authorization boundary for these
one-parent/one-worker lifetimes. MCP remains standard JSON-RPC over stdio at the
client-facing boundary. The current façade implements the legacy MCP
`2025-06-18` initialization lifecycle and negotiates unsupported requested
versions back to that version.
