use std::error::Error;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct DebugStartConfig {
    pub login_name: String,
    pub extension_name: String,
    pub ledger_dir: PathBuf,
    pub profile_override: Option<PathBuf>,
    pub socket_path: Option<PathBuf>,
    pub prompt_requires_override: bool,
}

pub fn default_debug_socket_path(login_name: &str) -> Result<PathBuf, Box<dyn Error>> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;

        let account_sanitized = sanitize_segment(login_name);
        let preferred_base = dirs::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("refreshmint")
            .join("debug");
        let preferred = preferred_base.join(format!(
            "rm-{}-{}.sock",
            std::process::id(),
            account_sanitized
        ));

        // Keep socket path short enough for sockaddr_un.
        if preferred.as_os_str().as_bytes().len() < 100 {
            return Ok(preferred);
        }

        let fallback = std::env::temp_dir().join(format!(
            "rm-debug-{}-{}.sock",
            std::process::id(),
            account_sanitized
        ));
        Ok(fallback)
    }

    #[cfg(not(unix))]
    {
        let _ = login_name;
        Err("debug sockets are currently supported only on unix platforms".into())
    }
}

pub fn run_debug_session(config: DebugStartConfig) -> Result<(), Box<dyn Error>> {
    #[cfg(unix)]
    {
        run_debug_session_unix(config)
    }

    #[cfg(not(unix))]
    {
        let _ = config;
        Err("debug sessions are currently supported only on unix platforms".into())
    }
}

pub fn exec_debug_script(socket_path: &Path, script_source: &str) -> Result<(), Box<dyn Error>> {
    exec_debug_script_with_options(socket_path, script_source, None, None, None)
}

pub fn exec_debug_script_with_options(
    socket_path: &Path,
    script_source: &str,
    declared_secrets: Option<super::js_api::SecretDeclarations>,
    prompt_overrides: Option<super::js_api::PromptOverrides>,
    prompt_requires_override: Option<bool>,
) -> Result<(), Box<dyn Error>> {
    #[cfg(unix)]
    {
        exec_debug_script_with_options_unix(
            socket_path,
            script_source,
            declared_secrets,
            prompt_overrides,
            prompt_requires_override,
        )
    }

    #[cfg(not(unix))]
    {
        let _ = (
            socket_path,
            script_source,
            declared_secrets,
            prompt_overrides,
            prompt_requires_override,
        );
        Err("debug sockets are currently supported only on unix platforms".into())
    }
}

pub fn stop_debug_session(socket_path: &Path) -> Result<(), Box<dyn Error>> {
    let response = send_request(socket_path, Request::Stop)?;
    if response.ok {
        return Ok(());
    }
    Err(response
        .error
        .unwrap_or_else(|| "stop failed".to_string())
        .into())
}

#[cfg(unix)]
fn exec_debug_script_with_options_unix(
    socket_path: &Path,
    script_source: &str,
    declared_secrets: Option<super::js_api::SecretDeclarations>,
    prompt_overrides: Option<super::js_api::PromptOverrides>,
    prompt_requires_override: Option<bool>,
) -> Result<(), Box<dyn Error>> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;

    let request = Request::Exec {
        script: script_source.to_string(),
        declared_secrets,
        prompt_overrides,
        prompt_requires_override,
    };

    let mut stream = UnixStream::connect(socket_path)?;
    serde_json::to_writer(&mut stream, &request)?;
    stream.write_all(b"\n")?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            return Err("exec failed: missing final result frame".into());
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Ok(frame) = serde_json::from_str::<ExecStreamFrame>(trimmed) {
            match frame {
                ExecStreamFrame::Output {
                    stream: ExecOutputStream::Stdout,
                    line,
                } => println!("{line}"),
                ExecStreamFrame::Output {
                    stream: ExecOutputStream::Stderr,
                    line,
                } => eprintln!("{line}"),
                ExecStreamFrame::Result { ok, error } => {
                    if ok {
                        return Ok(());
                    }
                    return Err(error.unwrap_or_else(|| "exec failed".to_string()).into());
                }
            }
            continue;
        }

        // Backward compatibility with pre-streaming response payloads.
        if let Ok(response) = serde_json::from_str::<Response>(trimmed) {
            if response.ok {
                return Ok(());
            }
            return Err(response
                .error
                .unwrap_or_else(|| "exec failed".to_string())
                .into());
        }

        return Err(format!("invalid exec response frame: {trimmed}").into());
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
enum Request {
    Exec {
        script: String,
        #[serde(default)]
        declared_secrets: Option<super::js_api::SecretDeclarations>,
        #[serde(default)]
        prompt_overrides: Option<super::js_api::PromptOverrides>,
        #[serde(default)]
        prompt_requires_override: Option<bool>,
    },
    Stop,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Response {
    ok: bool,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ExecOutputStream {
    Stdout,
    Stderr,
}

impl From<super::js_api::DebugOutputStream> for ExecOutputStream {
    fn from(value: super::js_api::DebugOutputStream) -> Self {
        match value {
            super::js_api::DebugOutputStream::Stdout => Self::Stdout,
            super::js_api::DebugOutputStream::Stderr => Self::Stderr,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ExecStreamFrame {
    Output {
        stream: ExecOutputStream,
        line: String,
    },
    Result {
        ok: bool,
        error: Option<String>,
    },
}

fn finalize_debug_exec_resources(
    refreshmint: &mut super::js_api::RefreshmintInner,
) -> Result<Vec<String>, String> {
    if refreshmint.staged_resources.is_empty() {
        return Ok(Vec::new());
    }

    eprintln!(
        "Finalizing {} staged resources from debug exec...",
        refreshmint.staged_resources.len()
    );
    let names = super::finalize_staged_resources(refreshmint).map_err(|err| err.to_string())?;
    refreshmint.staged_resources.clear();
    for name in &names {
        eprintln!("  -> {name}");
    }
    Ok(names)
}

#[cfg(unix)]
fn run_debug_session_unix(config: DebugStartConfig) -> Result<(), Box<dyn Error>> {
    use chromiumoxide::browser::Browser;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::net::UnixListener;
    use tokio::sync::Mutex;

    type DebugRuntimeState = (
        Arc<Mutex<Browser>>,
        tokio::task::JoinHandle<()>,
        Arc<Mutex<super::js_api::PageInner>>,
        Arc<Mutex<super::js_api::RefreshmintInner>>,
    );

    let _login_lock =
        crate::login_config::acquire_login_lock(&config.ledger_dir, &config.login_name)
            .map_err(|err| std::io::Error::other(err.to_string()))?;

    let socket_path = match config.socket_path {
        Some(path) => path,
        None => default_debug_socket_path(&config.login_name)?,
    };

    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if socket_path.exists() {
        std::fs::remove_file(&socket_path)?;
    }
    let _cleanup = SocketCleanup {
        path: socket_path.clone(),
    };

    let rt = tokio::runtime::Runtime::new()?;
    let (browser_instance, handler_handle, page_inner, refreshmint_inner): DebugRuntimeState =
        rt.block_on(async {
            let secret_store =
                crate::secret::SecretStore::new(format!("login/{}", config.login_name));
            let profile_dir = super::profile::resolve_profile_dir(
                &config.ledger_dir,
                &config.login_name,
                config.profile_override.as_deref(),
            )
            .map_err(|err| err.to_string())?;
            let download_dir = super::profile::resolve_download_dir(
                &config.extension_name,
                config.profile_override.as_deref(),
            )
            .map_err(|err| err.to_string())?;
            std::fs::create_dir_all(&download_dir).map_err(|err| err.to_string())?;

            let extension_dir = config
                .ledger_dir
                .join("extensions")
                .join(&config.extension_name);
            let declared_secrets = super::load_manifest_secret_declarations(&extension_dir)
                .map_err(|err| err.to_string())?;
            let output_dir = config
                .ledger_dir
                .join("cache")
                .join("extensions")
                .join(&config.extension_name)
                .join("output");
            std::fs::create_dir_all(&output_dir).map_err(|err| err.to_string())?;

            let chrome_path =
                super::browser::find_chrome_binary().map_err(|err| err.to_string())?;
            eprintln!("Using browser: {}", chrome_path.display());
            eprintln!("Profile dir: {}", profile_dir.display());

            let (browser_instance, handler) =
                super::browser::launch_browser(&chrome_path, &profile_dir)
                    .await
                    .map_err(|err| err.to_string())?;
            let browser = Arc::new(Mutex::new(browser_instance));
            let page = {
                let mut guard = browser.lock().await;
                super::browser::open_start_page(&mut guard)
                    .await
                    .map_err(|err| err.to_string())?
            };

            let page_inner = Arc::new(Mutex::new(super::js_api::PageInner {
                page,
                browser: browser.clone(),
                secret_store: Arc::new(secret_store),
                declared_secrets: Arc::new(declared_secrets),
                download_dir,
            }));
            let refreshmint_inner = Arc::new(Mutex::new(super::js_api::RefreshmintInner {
                output_dir,
                prompt_overrides: super::js_api::PromptOverrides::new(),
                prompt_requires_override: config.prompt_requires_override,
                debug_output_sink: None,
                session_metadata: super::js_api::SessionMetadata::default(),
                staged_resources: Vec::new(),
                scrape_session_id: String::new(),
                extension_name: config.extension_name.clone(),
                account_name: config.login_name.clone(),
                login_name: config.login_name.clone(),
                ledger_dir: config.ledger_dir.clone(),
            }));
            Ok::<_, Box<dyn Error>>((browser, handler, page_inner, refreshmint_inner))
        })?;

    rt.block_on(async move {
        let listener = UnixListener::bind(&socket_path)?;
        println!("Debug session socket: {}", socket_path.display());
        eprintln!("Debug session started. Press Ctrl+C to stop.");

        let mut running = true;
        while running {
            if handler_handle.is_finished() {
                eprintln!("Browser event handler stopped; ending debug session.");
                break;
            }

            match tokio::time::timeout(Duration::from_millis(100), listener.accept()).await {
                Ok(Ok((stream, _addr))) => {
                    let mut reader = BufReader::new(stream);
                    let mut body = String::new();
                    let read_result = reader.read_line(&mut body).await;
                    let mut stream = reader.into_inner();
                    match read_result {
                        Ok(0) => {
                            let response = Response {
                                ok: false,
                                error: Some("failed to read request: empty request".to_string()),
                            };
                            if let Err(err) = write_response_async(&mut stream, &response).await {
                                eprintln!("failed to write debug response: {err}");
                            }
                        }
                        Ok(_) => match serde_json::from_str::<Request>(body.trim()) {
                            Ok(Request::Exec {
                                script,
                                declared_secrets,
                                prompt_overrides,
                                prompt_requires_override,
                            }) => {
                                if let Err(err) = handle_exec_request_async(
                                    &mut stream,
                                    page_inner.clone(),
                                    refreshmint_inner.clone(),
                                    script,
                                    declared_secrets,
                                    prompt_overrides,
                                    prompt_requires_override,
                                )
                                .await
                                {
                                    eprintln!("failed to write debug exec stream: {err}");
                                }
                            }
                            Ok(Request::Stop) => {
                                running = false;
                                let response = Response {
                                    ok: true,
                                    error: None,
                                };
                                if let Err(err) = write_response_async(&mut stream, &response).await
                                {
                                    eprintln!("failed to write debug response: {err}");
                                }
                            }
                            Err(err) => {
                                let response = Response {
                                    ok: false,
                                    error: Some(format!("invalid request: {err}")),
                                };
                                if let Err(err) = write_response_async(&mut stream, &response).await
                                {
                                    eprintln!("failed to write debug response: {err}");
                                }
                            }
                        },
                        Err(err) => {
                            let response = Response {
                                ok: false,
                                error: Some(format!("failed to read request: {err}")),
                            };
                            if let Err(err) = write_response_async(&mut stream, &response).await {
                                eprintln!("failed to write debug response: {err}");
                            }
                        }
                    }
                }
                Ok(Err(err)) => return Err::<(), Box<dyn Error>>(err.into()),
                Err(_) => continue,
            }
        }

        drop(listener);
        {
            let guard = browser_instance.lock().await;
            let _ = guard.close().await;
        }
        drop(browser_instance);
        let _ = tokio::time::timeout(Duration::from_secs(5), handler_handle).await;
        Ok::<(), Box<dyn Error>>(())
    })?;

    Ok(())
}

#[cfg(unix)]
async fn handle_exec_request_async(
    stream: &mut tokio::net::UnixStream,
    page_inner: std::sync::Arc<tokio::sync::Mutex<super::js_api::PageInner>>,
    refreshmint_inner: std::sync::Arc<tokio::sync::Mutex<super::js_api::RefreshmintInner>>,
    script: String,
    declared_secrets: Option<super::js_api::SecretDeclarations>,
    prompt_overrides: Option<super::js_api::PromptOverrides>,
    prompt_requires_override: Option<bool>,
) -> std::io::Result<()> {
    if let Some(declared) = declared_secrets {
        let mut page_inner = page_inner.lock().await;
        page_inner.declared_secrets = std::sync::Arc::new(declared);
    }

    let (output_sender, mut output_receiver) =
        tokio::sync::mpsc::unbounded_channel::<super::js_api::DebugOutputEvent>();
    {
        let mut refreshmint = refreshmint_inner.lock().await;
        refreshmint.prompt_overrides = prompt_overrides.unwrap_or_default();
        if let Some(require_override) = prompt_requires_override {
            refreshmint.prompt_requires_override = require_override;
        }
        refreshmint.debug_output_sink = Some(output_sender);
    }

    let refreshmint_inner_for_task = refreshmint_inner.clone();
    let mut exec_task = tokio::spawn(async move {
        let run_result = super::sandbox::run_script_source_with_options(
            &script,
            page_inner,
            refreshmint_inner_for_task.clone(),
            super::sandbox::SandboxRunOptions {
                emit_diagnostics: false,
            },
        )
        .await;

        let finalize_result = {
            let mut refreshmint = refreshmint_inner_for_task.lock().await;
            finalize_debug_exec_resources(&mut refreshmint)
        };

        let result = match (run_result, finalize_result) {
            (Ok(()), Ok(_names)) => Ok(()),
            (Ok(()), Err(err)) => Err(format!("failed to finalize staged resources: {err}")),
            (Err(run_err), Ok(_names)) => Err(run_err.to_string()),
            (Err(run_err), Err(finalize_err)) => Err(format!(
                "{}; additionally failed to finalize staged resources: {}",
                run_err, finalize_err
            )),
        };

        {
            let mut refreshmint = refreshmint_inner_for_task.lock().await;
            refreshmint.debug_output_sink = None;
        }

        result
    });

    let mut exec_result: Option<Result<(), String>> = None;
    loop {
        tokio::select! {
            maybe_event = output_receiver.recv() => {
                match maybe_event {
                    Some(event) => {
                        let frame = ExecStreamFrame::Output {
                            stream: event.stream.into(),
                            line: event.line,
                        };
                        if let Err(err) = write_exec_stream_frame_async(stream, &frame).await {
                            eprintln!(
                                "debug exec client disconnected while streaming output; canceling script: {err}"
                            );
                            cancel_exec_task(&mut exec_task, &refreshmint_inner).await;
                            return Ok(());
                        }
                    }
                    None => {
                        if exec_result.is_some() {
                            break;
                        }
                    }
                }
            }
            readable = stream.readable(), if exec_result.is_none() => {
                match readable {
                    Ok(()) => {
                        let mut buf = [0u8; 64];
                        match stream.try_read(&mut buf) {
                            Ok(0) => {
                                eprintln!("debug exec client disconnected; canceling script.");
                                cancel_exec_task(&mut exec_task, &refreshmint_inner).await;
                                return Ok(());
                            }
                            Ok(_n) => {
                                // Ignore unexpected extra client bytes while script is running.
                            }
                            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {}
                            Err(err) => {
                                eprintln!(
                                    "failed to read debug exec client stream; canceling script: {err}"
                                );
                                cancel_exec_task(&mut exec_task, &refreshmint_inner).await;
                                return Ok(());
                            }
                        }
                    }
                    Err(err) => {
                        eprintln!("debug exec client stream readability error; canceling script: {err}");
                        cancel_exec_task(&mut exec_task, &refreshmint_inner).await;
                        return Ok(());
                    }
                }
            }
            joined = &mut exec_task, if exec_result.is_none() => {
                exec_result = Some(match joined {
                    Ok(result) => result,
                    Err(err) => Err(format!("failed to join debug exec task: {err}")),
                });

                // Ensure sender cleanup even if task exits unexpectedly.
                {
                    let mut refreshmint = refreshmint_inner.lock().await;
                    refreshmint.debug_output_sink = None;
                }

                if output_receiver.is_closed() {
                    while let Ok(event) = output_receiver.try_recv() {
                        let frame = ExecStreamFrame::Output {
                            stream: event.stream.into(),
                            line: event.line,
                        };
                        if let Err(err) = write_exec_stream_frame_async(stream, &frame).await {
                            eprintln!("debug exec client disconnected while draining output: {err}");
                            return Ok(());
                        }
                    }
                    break;
                }
            }
        }
    }

    let final_result = match exec_result {
        Some(result) => result,
        None => {
            let joined = exec_task.await.map_err(|err| {
                std::io::Error::other(format!("failed to join debug exec task: {err}"))
            })?;
            {
                let mut refreshmint = refreshmint_inner.lock().await;
                refreshmint.debug_output_sink = None;
            }
            joined
        }
    };

    let final_frame = match final_result {
        Ok(()) => ExecStreamFrame::Result {
            ok: true,
            error: None,
        },
        Err(err) => ExecStreamFrame::Result {
            ok: false,
            error: Some(err),
        },
    };
    if let Err(err) = write_exec_stream_frame_async(stream, &final_frame).await {
        eprintln!("debug exec client disconnected before final result frame: {err}");
    }
    Ok(())
}

#[cfg(unix)]
async fn cancel_exec_task(
    exec_task: &mut tokio::task::JoinHandle<Result<(), String>>,
    refreshmint_inner: &std::sync::Arc<tokio::sync::Mutex<super::js_api::RefreshmintInner>>,
) {
    exec_task.abort();
    let _ = exec_task.await;
    let mut refreshmint = refreshmint_inner.lock().await;
    refreshmint.debug_output_sink = None;
}

#[cfg(unix)]
fn send_request(socket_path: &Path, request: Request) -> Result<Response, Box<dyn Error>> {
    use std::io::{Read, Write};
    use std::net::Shutdown;
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(socket_path)?;
    serde_json::to_writer(&mut stream, &request)?;
    stream.write_all(b"\n")?;
    stream.shutdown(Shutdown::Write)?;

    let mut response_body = String::new();
    stream.read_to_string(&mut response_body)?;
    let response: Response = serde_json::from_str(response_body.trim())?;
    Ok(response)
}

#[cfg(not(unix))]
fn send_request(_socket_path: &Path, _request: Request) -> Result<Response, Box<dyn Error>> {
    Err("debug sockets are currently supported only on unix platforms".into())
}

#[cfg(unix)]
async fn write_response_async(
    stream: &mut tokio::net::UnixStream,
    response: &Response,
) -> std::io::Result<()> {
    let mut out = serde_json::to_vec(response)?;
    out.push(b'\n');
    tokio::io::AsyncWriteExt::write_all(stream, &out).await?;
    tokio::io::AsyncWriteExt::flush(stream).await
}

#[cfg(unix)]
async fn write_exec_stream_frame_async(
    stream: &mut tokio::net::UnixStream,
    frame: &ExecStreamFrame,
) -> std::io::Result<()> {
    let mut out = serde_json::to_vec(frame)?;
    out.push(b'\n');
    tokio::io::AsyncWriteExt::write_all(stream, &out).await?;
    tokio::io::AsyncWriteExt::flush(stream).await
}

fn sanitize_segment(input: &str) -> String {
    let cleaned: String = input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "default".to_string()
    } else {
        cleaned
    }
}

#[cfg(unix)]
struct SocketCleanup {
    path: PathBuf,
}

#[cfg(unix)]
impl Drop for SocketCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        finalize_debug_exec_resources, sanitize_segment, ExecOutputStream, ExecStreamFrame,
    };
    use crate::login_config::login_account_documents_dir;
    use crate::scrape::js_api::{
        PromptOverrides, RefreshmintInner, SessionMetadata, StagedResource,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn create_temp_dir(prefix: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("refreshmint-{prefix}-{}-{now}", std::process::id()));
        fs::create_dir_all(&dir).unwrap_or_else(|err| {
            panic!("failed to create temp dir: {err}");
        });
        dir
    }

    #[test]
    fn sanitize_segment_preserves_safe_chars() {
        assert_eq!(sanitize_segment("abc-DEF_123"), "abc-DEF_123");
    }

    #[test]
    fn sanitize_segment_replaces_unsafe_chars() {
        assert_eq!(sanitize_segment("a/b:c"), "a_b_c");
    }

    #[test]
    fn exec_stream_output_frame_roundtrip_json() {
        let frame = ExecStreamFrame::Output {
            stream: ExecOutputStream::Stdout,
            line: "hello".to_string(),
        };
        let json = serde_json::to_string(&frame).unwrap_or_else(|err| panic!("failed: {err}"));
        let parsed: ExecStreamFrame =
            serde_json::from_str(&json).unwrap_or_else(|err| panic!("failed: {err}"));
        assert_eq!(parsed, frame);
    }

    #[test]
    fn exec_stream_result_frame_roundtrip_json() {
        let frame = ExecStreamFrame::Result {
            ok: false,
            error: Some("boom".to_string()),
        };
        let json = serde_json::to_string(&frame).unwrap_or_else(|err| panic!("failed: {err}"));
        let parsed: ExecStreamFrame =
            serde_json::from_str(&json).unwrap_or_else(|err| panic!("failed: {err}"));
        assert_eq!(parsed, frame);
    }

    #[test]
    fn finalize_debug_exec_resources_moves_and_clears_staged_files() {
        let root = create_temp_dir("debug-finalize");
        let ledger_dir = root.join("ledger.refreshmint");
        fs::create_dir_all(&ledger_dir).unwrap_or_else(|err| {
            panic!("failed to create ledger dir: {err}");
        });

        let staged_path = root.join("staged-debug.bin");
        fs::write(&staged_path, b"ok").unwrap_or_else(|err| {
            panic!("failed to write staged file: {err}");
        });

        let login_name = "debug-login".to_string();
        let mut inner = RefreshmintInner {
            output_dir: root.join("output"),
            prompt_overrides: PromptOverrides::new(),
            prompt_requires_override: false,
            debug_output_sink: None,
            session_metadata: SessionMetadata::default(),
            staged_resources: vec![StagedResource {
                filename: "debug-smoke.bin".to_string(),
                staging_path: staged_path.clone(),
                coverage_end_date: Some("2026-02-01".to_string()),
                original_url: Some("https://example.com/export".to_string()),
                mime_type: Some("application/octet-stream".to_string()),
                label: Some("checking".to_string()),
                metadata: std::collections::BTreeMap::new(),
            }],
            scrape_session_id: "debug-session".to_string(),
            extension_name: "smoke-ext".to_string(),
            account_name: login_name.clone(),
            login_name: login_name.clone(),
            ledger_dir: ledger_dir.clone(),
        };

        let finalized =
            finalize_debug_exec_resources(&mut inner).unwrap_or_else(|err| panic!("failed: {err}"));
        assert_eq!(inner.staged_resources.len(), 0);
        assert_eq!(finalized.len(), 1);
        assert!(finalized[0].starts_with("2026-02-01-debug-smoke.bin"));

        let documents_dir = login_account_documents_dir(&ledger_dir, &login_name, "checking");
        let finalized_path = documents_dir.join(&finalized[0]);
        assert!(finalized_path.exists());
        let bytes = fs::read(finalized_path).unwrap_or_else(|err| {
            panic!("failed to read finalized file: {err}");
        });
        assert_eq!(bytes, b"ok");

        let sidecar_path = documents_dir.join(format!("{}-info.json", finalized[0]));
        assert!(sidecar_path.exists());

        let _ = fs::remove_dir_all(&root);
    }
}
