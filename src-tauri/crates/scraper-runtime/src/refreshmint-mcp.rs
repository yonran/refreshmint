//! MCP stdio facade for independently owned Refreshmint debug browsers.
//!
//! The facade owns no browser and reads no Keychain values. It discovers
//! per-browser workers, selects one logical session for ordinary tool calls,
//! and can spawn a fresh worker on request.

#![cfg_attr(not(unix), allow(dead_code, unused_imports))]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

#[path = "../../../src/debug_registry.rs"]
#[allow(dead_code)]
mod debug_registry;
#[path = "../../../src/login_config.rs"]
#[allow(dead_code)]
mod login_config;

use debug_registry::DebugSessionDescriptor;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

struct Facade {
    selected_session: Option<String>,
    selection_is_explicit: bool,
    owned_workers: HashMap<String, Child>,
}

impl Facade {
    fn new() -> Self {
        Self {
            selected_session: None,
            selection_is_explicit: false,
            owned_workers: HashMap::new(),
        }
    }

    fn sessions(&mut self) -> Vec<DebugSessionDescriptor> {
        // Reap workers that this façade launched but that have since exited on
        // their own (for example after browser failure).
        self.owned_workers
            .retain(|_, child| !matches!(child.try_wait(), Ok(Some(_))));
        let sessions = debug_registry::list_sessions();
        self.reconcile_selection(&sessions);
        sessions
    }

    fn reconcile_selection(&mut self, sessions: &[DebugSessionDescriptor]) {
        if self.selected_session.as_ref().is_some_and(|selected| {
            !sessions
                .iter()
                .any(|session| &session.session_id == selected)
        }) {
            self.selected_session = None;
            self.selection_is_explicit = false;
        }
        if sessions.len() == 1 && self.selected_session.is_none() {
            self.selected_session = Some(sessions[0].session_id.clone());
            self.selection_is_explicit = false;
        } else if sessions.len() != 1 && !self.selection_is_explicit {
            // An automatic selection is only safe while it remains the sole
            // choice. If another browser appears, require the client to make
            // the target explicit instead of silently using the older tab.
            self.selected_session = None;
        }
    }

    fn selected(&mut self, requested: Option<&str>) -> Result<DebugSessionDescriptor, String> {
        let sessions = self.sessions();
        let selected = requested
            .map(ToOwned::to_owned)
            .or_else(|| self.selected_session.clone())
            .ok_or_else(|| {
                if sessions.is_empty() {
                    "no Refreshmint debug browser is open".to_string()
                } else {
                    "multiple debug browsers are open; call refreshmint_select_session first"
                        .to_string()
                }
            })?;
        sessions
            .into_iter()
            .find(|session| session.session_id == selected)
            .ok_or_else(|| format!("debug session '{selected}' is no longer available"))
    }

    fn select(&mut self, session_id: &str) -> Result<DebugSessionDescriptor, String> {
        let session = self
            .sessions()
            .into_iter()
            .find(|session| session.session_id == session_id)
            .ok_or_else(|| format!("unknown debug session '{session_id}'"))?;
        self.selected_session = Some(session_id.to_string());
        self.selection_is_explicit = true;
        Ok(session)
    }

    fn start_debug(&mut self, arguments: &Value) -> Result<DebugSessionDescriptor, String> {
        let ledger = required_string(arguments, "ledger")?;
        let login_name = required_string(arguments, "loginName")?;
        login_config::validate_label(&login_name)?;
        let headless = arguments
            .get("headless")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let profile = arguments.get("profile").and_then(Value::as_str);
        let extension_name =
            login_config::resolve_login_extension(Path::new(&ledger), &login_name)?;
        let socket_path =
            std::env::temp_dir().join(format!("rm-debug-{}.sock", uuid::Uuid::new_v4().simple()));

        let mut command = Command::new(worker_path()?);
        command
            .arg("debug-start")
            .arg("--ledger")
            .arg(&ledger)
            .arg("--login")
            .arg(&login_name)
            .arg("--extension")
            .arg(&extension_name)
            .arg("--socket")
            .arg(&socket_path)
            .arg("--prompt-requires-override")
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        if headless {
            command.arg("--headless");
        }
        if let Some(profile) = profile {
            command.arg("--profile").arg(profile);
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("failed to start scraper worker: {error}"))?;

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
        let descriptor = loop {
            if let Some(session) = debug_registry::list_sessions()
                .into_iter()
                .find(|session| session.socket_path == socket_path)
            {
                break session;
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    return Err(format!(
                        "scraper worker exited before its browser was ready ({status})"
                    ));
                }
                Ok(None) => {}
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("failed to inspect scraper worker: {error}"));
                }
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err("timed out waiting for debug browser to start".to_string());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        };
        self.selected_session = Some(descriptor.session_id.clone());
        self.selection_is_explicit = true;
        self.owned_workers
            .insert(descriptor.session_id.clone(), child);
        Ok(descriptor)
    }

    fn stop(&mut self, requested: Option<&str>) -> Result<DebugSessionDescriptor, String> {
        let session = self.selected(requested)?;
        send_stop(&session.socket_path)?;
        if let Some(mut child) = self.owned_workers.remove(&session.session_id) {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while child
                .try_wait()
                .map_err(|error| error.to_string())?
                .is_none()
            {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        if self.selected_session.as_deref() == Some(&session.session_id) {
            self.selected_session = None;
            self.selection_is_explicit = false;
        }
        Ok(session)
    }
}

impl Drop for Facade {
    fn drop(&mut self) {
        let sessions = debug_registry::list_sessions();
        for (session_id, mut child) in self.owned_workers.drain() {
            if let Some(session) = sessions
                .iter()
                .find(|session| session.session_id == session_id)
            {
                let _ = send_stop(&session.socket_path);
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while child.try_wait().ok().flatten().is_none() && std::time::Instant::now() < deadline
            {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(not(unix))]
    return Err("Refreshmint MCP debug sessions currently require Unix sockets".into());

    #[cfg(unix)]
    {
        let stdin = std::io::stdin();
        let mut stdout = std::io::stdout().lock();
        let mut facade = Facade::new();
        for line in stdin.lock().lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let request: Value = match serde_json::from_str(&line) {
                Ok(request) => request,
                Err(error) => {
                    write_json(
                        &mut stdout,
                        &json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":error.to_string()}}),
                    )?;
                    continue;
                }
            };
            let Some(id) = request.get("id").cloned() else {
                continue;
            };
            let response = handle_request(&mut facade, &request, id);
            write_json(&mut stdout, &response)?;
        }
        Ok(())
    }
}

fn handle_request(facade: &mut Facade, request: &Value, id: Value) -> Value {
    const LEGACY_PROTOCOL_VERSION: &str = "2025-06-18";

    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    match method {
        "initialize" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                // Per MCP version negotiation, echo the requested version only
                // when supported; otherwise advertise the version we implement.
                "protocolVersion": request.pointer("/params/protocolVersion")
                    .and_then(Value::as_str)
                    .filter(|version| *version == LEGACY_PROTOCOL_VERSION)
                    .unwrap_or(LEGACY_PROTOCOL_VERSION),
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "refreshmint", "version": env!("CARGO_PKG_VERSION") }
            }
        }),
        "ping" => json!({"jsonrpc":"2.0","id":id,"result":{}}),
        "tools/list" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "tools": tool_definitions() }
        }),
        "tools/call" => {
            let name = request
                .pointer("/params/name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let arguments = request
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let result = call_tool(facade, name, &arguments);
            let (text, is_error) = match result {
                Ok(value) => (
                    serde_json::to_string_pretty(&value).unwrap_or_default(),
                    false,
                ),
                Err(error) => (error, true),
            };
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "content": [{"type":"text","text":text}], "isError": is_error }
            })
        }
        _ => json!({
            "jsonrpc":"2.0",
            "id":id,
            "error":{"code":-32601,"message":format!("unknown method '{method}'")}
        }),
    }
}

fn call_tool(facade: &mut Facade, name: &str, arguments: &Value) -> Result<Value, String> {
    match name {
        "refreshmint_list_sessions" => {
            let sessions = facade.sessions();
            let selected = facade.selected_session.clone();
            Ok(json!({"selectedSessionId": selected, "sessions": sessions}))
        }
        "refreshmint_select_session" => {
            let session_id = required_string(arguments, "sessionId")?;
            serde_json::to_value(facade.select(&session_id)?).map_err(|error| error.to_string())
        }
        "refreshmint_start_debug" => {
            serde_json::to_value(facade.start_debug(arguments)?).map_err(|error| error.to_string())
        }
        "refreshmint_debug_exec" => {
            let script = required_string(arguments, "script")?;
            let session = facade.selected(arguments.get("sessionId").and_then(Value::as_str))?;
            let output = send_exec(&session.socket_path, &script)?;
            Ok(json!({"sessionId":session.session_id,"output":output}))
        }
        "refreshmint_stop_debug" => {
            let session = facade.stop(arguments.get("sessionId").and_then(Value::as_str))?;
            Ok(json!({"stopped":session.session_id}))
        }
        _ => Err(format!("unknown tool '{name}'")),
    }
}

fn tool_definitions() -> Value {
    json!([
        {
            "name":"refreshmint_list_sessions",
            "description":"List open Refreshmint debug browsers. The only session is selected automatically.",
            "inputSchema":{"type":"object","properties":{},"additionalProperties":false}
        },
        {
            "name":"refreshmint_select_session",
            "description":"Select which open debug browser subsequent calls control.",
            "inputSchema":{"type":"object","properties":{"sessionId":{"type":"string"}},"required":["sessionId"],"additionalProperties":false}
        },
        {
            "name":"refreshmint_start_debug",
            "description":"Start and select a new independently owned Refreshmint debug browser.",
            "inputSchema":{"type":"object","properties":{"ledger":{"type":"string"},"loginName":{"type":"string"},"headless":{"type":"boolean"},"profile":{"type":"string"}},"required":["ledger","loginName"],"additionalProperties":false}
        },
        {
            "name":"refreshmint_debug_exec",
            "description":"Execute JavaScript in the selected Refreshmint debug browser. Output is redacted by the worker.",
            "inputSchema":{"type":"object","properties":{"sessionId":{"type":"string"},"script":{"type":"string"}},"required":["script"],"additionalProperties":false}
        },
        {
            "name":"refreshmint_stop_debug",
            "description":"Close a debug browser and release its login lock.",
            "inputSchema":{"type":"object","properties":{"sessionId":{"type":"string"}},"additionalProperties":false}
        }
    ])
}

fn required_string(arguments: &Value, name: &str) -> Result<String, String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{name} is required"))
}

fn worker_path() -> Result<PathBuf, String> {
    let current = std::env::current_exe().map_err(|error| error.to_string())?;
    let names = if cfg!(windows) {
        ["scraper-worker.exe", "refreshmint-scraper-worker.exe"]
    } else {
        ["scraper-worker", "refreshmint-scraper-worker"]
    };
    let parent = current
        .parent()
        .ok_or_else(|| "MCP executable has no parent directory".to_string())?;
    for name in &names {
        let candidate = parent.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Ok(PathBuf::from(names[0]))
}

#[cfg(unix)]
fn send_exec(socket_path: &Path, script: &str) -> Result<Vec<Value>, String> {
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(socket_path).map_err(|error| error.to_string())?;
    let request = json!({
        "command":"exec",
        "script":script,
        "prompt_requires_override":true
    });
    serde_json::to_writer(&mut stream, &request).map_err(|error| error.to_string())?;
    stream.write_all(b"\n").map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    for line in std::io::BufReader::new(stream).lines() {
        let line = line.map_err(|error| error.to_string())?;
        let frame: Value = serde_json::from_str(&line).map_err(|error| error.to_string())?;
        if frame.get("type").and_then(Value::as_str) == Some("result") {
            if frame.get("ok").and_then(Value::as_bool) == Some(true) {
                return Ok(output);
            }
            return Err(frame
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("debug execution failed")
                .to_string());
        }
        output.push(frame);
    }
    Err("debug worker closed without a result".to_string())
}

#[cfg(not(unix))]
fn send_exec(_socket_path: &Path, _script: &str) -> Result<Vec<Value>, String> {
    Err("debug sessions currently require Unix sockets".to_string())
}

#[cfg(unix)]
fn send_stop(socket_path: &Path) -> Result<(), String> {
    use std::io::Read;
    use std::net::Shutdown;
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(socket_path).map_err(|error| error.to_string())?;
    serde_json::to_writer(&mut stream, &json!({"command":"stop"}))
        .map_err(|error| error.to_string())?;
    stream.write_all(b"\n").map_err(|error| error.to_string())?;
    stream
        .shutdown(Shutdown::Write)
        .map_err(|error| error.to_string())?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|error| error.to_string())?;
    let response: Value =
        serde_json::from_str(response.trim()).map_err(|error| error.to_string())?;
    if response.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(())
    } else {
        Err(response
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("failed to stop debug session")
            .to_string())
    }
}

#[cfg(not(unix))]
fn send_stop(_socket_path: &Path) -> Result<(), String> {
    Err("debug sessions currently require Unix sockets".to_string())
}

fn write_json(output: &mut impl Write, value: &Value) -> std::io::Result<()> {
    serde_json::to_writer(&mut *output, value)?;
    output.write_all(b"\n")?;
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::{handle_request, DebugSessionDescriptor, Facade};
    use serde_json::json;

    fn session(id: &str) -> DebugSessionDescriptor {
        DebugSessionDescriptor {
            session_id: id.to_string(),
            login_name: id.to_string(),
            kind: "manual-debug".to_string(),
            pid: 1,
            socket_path: format!("/tmp/{id}.sock").into(),
            ledger_dir: "/tmp/test.refreshmint".into(),
            started_at: "now".to_string(),
        }
    }

    #[test]
    fn initialize_advertises_tools() {
        let response = handle_request(
            &mut Facade::new(),
            &json!({"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),
            json!(1),
        );
        assert_eq!(response["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(response["result"]["serverInfo"]["name"], "refreshmint");
    }

    #[test]
    fn initialize_negotiates_unknown_versions_to_supported_version() {
        let response = handle_request(
            &mut Facade::new(),
            &json!({"method":"initialize","params":{"protocolVersion":"2099-01-01"}}),
            json!(1),
        );
        assert_eq!(response["result"]["protocolVersion"], "2025-06-18");
    }

    #[test]
    fn automatic_selection_is_revoked_when_a_second_session_appears() {
        let mut facade = Facade::new();
        facade.reconcile_selection(&[session("first")]);
        assert_eq!(facade.selected_session.as_deref(), Some("first"));

        facade.reconcile_selection(&[session("first"), session("second")]);
        assert_eq!(facade.selected_session, None);
    }

    #[test]
    fn explicit_selection_survives_additional_sessions() {
        let mut facade = Facade::new();
        facade.selected_session = Some("first".to_string());
        facade.selection_is_explicit = true;
        facade.reconcile_selection(&[session("first"), session("second")]);
        assert_eq!(facade.selected_session.as_deref(), Some("first"));
    }

    #[test]
    fn tools_list_includes_session_routing() {
        let response = handle_request(
            &mut Facade::new(),
            &json!({"method":"tools/list"}),
            json!(1),
        );
        let names = response["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert!(names.contains(&"refreshmint_list_sessions"));
        assert!(names.contains(&"refreshmint_start_debug"));
        assert!(names.contains(&"refreshmint_select_session"));
    }
}
