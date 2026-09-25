use std::error::Error;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct DebugStartConfig {
    pub login_name: String,
    pub extension_name: String,
    pub ledger_dir: PathBuf,
    pub profile_override: Option<PathBuf>,
    pub headless: bool,
    pub socket_path: Option<PathBuf>,
    pub prompt_requires_override: bool,
}

#[cfg(unix)]
pub type DebugSessionReadyListener =
    std::sync::Arc<dyn Fn(&crate::debug_registry::DebugSessionDescriptor) + Send + Sync>;

/// Configuration for exposing an already-running scrape browser through the
/// debug socket. The caller continues to own the login lock while this future
/// runs, so promoting a failed scrape never releases and reacquires that lock.
#[cfg(unix)]
pub struct ExistingDebugSessionConfig {
    pub login_name: String,
    pub ledger_dir: PathBuf,
    pub socket_path: PathBuf,
    pub kind: String,
    pub max_duration: Option<std::time::Duration>,
    pub ready_listener: Option<DebugSessionReadyListener>,
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
    exec_debug_script_with_options(socket_path, script_source, None, None, None, None)
}

pub fn exec_debug_script_with_options(
    socket_path: &Path,
    script_source: &str,
    declared_secrets: Option<super::js_api::SecretDeclarations>,
    prompt_overrides: Option<super::js_api::PromptOverrides>,
    prompt_requires_override: Option<bool>,
    script_options: Option<super::js_api::ScriptOptions>,
) -> Result<(), Box<dyn Error>> {
    #[cfg(unix)]
    {
        exec_debug_script_with_options_unix(
            socket_path,
            script_source,
            declared_secrets,
            prompt_overrides,
            prompt_requires_override,
            script_options,
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
            script_options,
        );
        Err("debug sockets are currently supported only on unix platforms".into())
    }
}

pub fn exec_debug_entry_module_with_options(
    socket_path: &Path,
    extension_root: &Path,
    entry_path: &Path,
    declared_secrets: Option<super::js_api::SecretDeclarations>,
    prompt_overrides: Option<super::js_api::PromptOverrides>,
    prompt_requires_override: Option<bool>,
    script_options: Option<super::js_api::ScriptOptions>,
) -> Result<(), Box<dyn Error>> {
    #[cfg(unix)]
    {
        exec_debug_entry_module_with_options_unix(
            socket_path,
            extension_root,
            entry_path,
            declared_secrets,
            prompt_overrides,
            prompt_requires_override,
            script_options,
        )
    }

    #[cfg(not(unix))]
    {
        let _ = (
            socket_path,
            extension_root,
            entry_path,
            declared_secrets,
            prompt_overrides,
            prompt_requires_override,
            script_options,
        );
        Err("debug sockets are currently supported only on unix platforms".into())
    }
}

pub fn stop_debug_session(socket_path: &Path) -> Result<(), Box<dyn Error>> {
    let response: Response = send_request(socket_path, Request::Stop)?;
    if response.ok {
        return Ok(());
    }
    Err(response
        .error
        .unwrap_or_else(|| "stop failed".to_string())
        .into())
}

/// `timeout` bounds the wait for a reply: a session still running a worker
/// binary from before `status` existed serves one connection at a time, so
/// mid-exec it would not answer until the script finished.
pub fn debug_session_status(
    socket_path: &Path,
    timeout: Option<std::time::Duration>,
) -> Result<DebugSessionStatus, Box<dyn Error>> {
    let response: StatusResponse =
        send_request_with_timeout(socket_path, Request::Status, timeout)?;
    match (response.ok, response.status) {
        (true, Some(status)) => Ok(status),
        (_, _) => Err(response
            .error
            .unwrap_or_else(|| "status failed".to_string())
            .into()),
    }
}

#[cfg(unix)]
fn exec_debug_script_with_options_unix(
    socket_path: &Path,
    script_source: &str,
    declared_secrets: Option<super::js_api::SecretDeclarations>,
    prompt_overrides: Option<super::js_api::PromptOverrides>,
    prompt_requires_override: Option<bool>,
    script_options: Option<super::js_api::ScriptOptions>,
) -> Result<(), Box<dyn Error>> {
    let request = Request::Exec {
        script: Some(script_source.to_string()),
        entry_root: None,
        entry_path: None,
        declared_secrets,
        prompt_overrides,
        prompt_requires_override,
        script_options,
    };

    exec_debug_request_unix(socket_path, request)
}

#[cfg(unix)]
fn exec_debug_entry_module_with_options_unix(
    socket_path: &Path,
    extension_root: &Path,
    entry_path: &Path,
    declared_secrets: Option<super::js_api::SecretDeclarations>,
    prompt_overrides: Option<super::js_api::PromptOverrides>,
    prompt_requires_override: Option<bool>,
    script_options: Option<super::js_api::ScriptOptions>,
) -> Result<(), Box<dyn Error>> {
    let request = Request::Exec {
        script: None,
        entry_root: Some(extension_root.to_path_buf()),
        entry_path: Some(entry_path.to_path_buf()),
        declared_secrets,
        prompt_overrides,
        prompt_requires_override,
        script_options,
    };

    exec_debug_request_unix(socket_path, request)
}

#[cfg(unix)]
fn exec_debug_request_unix(socket_path: &Path, request: Request) -> Result<(), Box<dyn Error>> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;

    let connect_path = resolve_socket_bind_path(socket_path);
    let mut stream = UnixStream::connect(&connect_path)?;
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
        #[serde(default)]
        script: Option<String>,
        #[serde(default)]
        entry_root: Option<PathBuf>,
        #[serde(default)]
        entry_path: Option<PathBuf>,
        #[serde(default)]
        declared_secrets: Option<super::js_api::SecretDeclarations>,
        #[serde(default)]
        prompt_overrides: Option<super::js_api::PromptOverrides>,
        #[serde(default)]
        prompt_requires_override: Option<bool>,
        #[serde(default)]
        script_options: Option<super::js_api::ScriptOptions>,
    },
    Stop,
    Status,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Response {
    ok: bool,
    error: Option<String>,
}

/// Snapshot of a running debug session, answered by a `status` request even
/// while another client's `exec` is in progress.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugSessionStatus {
    pub session: crate::debug_registry::DebugSessionDescriptor,
    pub exec_running: bool,
    pub uptime_secs: u64,
    /// `None` for sessions without a lifetime limit (e.g. `debug start`).
    pub expires_in_secs: Option<u64>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StatusResponse {
    ok: bool,
    error: Option<String>,
    status: Option<DebugSessionStatus>,
}

#[cfg(any(unix, test))]
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ExecOutputStream {
    Stdout,
    Stderr,
}

#[cfg(any(unix, test))]
impl From<super::js_api::DebugOutputStream> for ExecOutputStream {
    fn from(value: super::js_api::DebugOutputStream) -> Self {
        match value {
            super::js_api::DebugOutputStream::Stdout => Self::Stdout,
            super::js_api::DebugOutputStream::Stderr => Self::Stderr,
        }
    }
}

#[cfg(any(unix, test))]
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

#[cfg(any(unix, test))]
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
    use tokio::sync::Mutex;

    type DebugRuntimeState = (
        Arc<Mutex<Browser>>,
        tokio::task::JoinHandle<()>,
        Arc<Mutex<super::js_api::PageInner>>,
        Arc<Mutex<super::js_api::RefreshmintInner>>,
        super::browser::BrowserPidGuard,
    );

    let _login_lock = crate::login_config::acquire_login_lock_with_metadata(
        &config.ledger_dir,
        &config.login_name,
        "scrape-debug",
        "debug-session",
    )
    .map_err(|err| std::io::Error::other(err.to_string()))?;

    let socket_path = match config.socket_path {
        Some(path) => path,
        None => default_debug_socket_path(&config.login_name)?,
    };

    let rt = tokio::runtime::Runtime::new()?;
    let (browser_instance, handler_handle, page_inner, refreshmint_inner, _browser_pid_guard): DebugRuntimeState =
        rt.block_on(async {
            let secret_store = std::sync::Arc::new(crate::secret::SecretStore::new(format!(
                "login/{}",
                config.login_name
            )));
            let sensitive_data = std::sync::Arc::new(
                super::js_api::SensitiveData::for_secret_store(&secret_store),
            );
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

            let extension_dir = crate::account_config::resolve_extension_dir(
                &config.ledger_dir,
                &config.extension_name,
            );
            let declared_secrets = super::load_manifest_secret_declarations(&extension_dir)
                .map_err(|err| err.to_string())?;
            let ext_cache_key = std::path::Path::new(&config.extension_name)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&config.extension_name);
            let output_dir = config
                .ledger_dir
                .join("cache")
                .join("extensions")
                .join(ext_cache_key)
                .join("output");
            std::fs::create_dir_all(&output_dir).map_err(|err| err.to_string())?;

            let chrome_path =
                super::browser::find_chrome_binary().map_err(|err| err.to_string())?;
            eprintln!("Using browser: {}", chrome_path.display());
            eprintln!("Profile dir: {}", profile_dir.display());

            let (browser_instance, handler, browser_pid_guard) =
                super::browser::launch_browser(&chrome_path, &profile_dir, config.headless)
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
                target_id: page.target_id().as_ref().to_string(),
                page,
                browser: browser.clone(),
                secret_store: secret_store.clone(),
                sensitive_data: sensitive_data.clone(),
                declared_secrets: Arc::new(declared_secrets),
                download_dir,
                target_frame_id: None,
                human_challenge_ui_handler: None,
            }));
            let refreshmint_inner = Arc::new(Mutex::new(super::js_api::RefreshmintInner {
                output_dir,
                prompt_overrides: super::js_api::PromptOverrides::new(),
                prompt_requires_override: config.prompt_requires_override,
                script_options: super::js_api::ScriptOptions::new(),
                debug_output_sink: None,
                sensitive_data,
                session_metadata: super::js_api::SessionMetadata::default(),
                staged_resources: Vec::new(),
                scrape_session_id: String::new(),
                extension_name: config.extension_name.clone(),
                account_name: config.login_name.clone(),
                login_name: config.login_name.clone(),
                ledger_dir: config.ledger_dir.clone(),
                prompt_ui_handler: None,
            }));
            Ok::<_, Box<dyn Error>>((
                browser,
                handler,
                page_inner,
                refreshmint_inner,
                browser_pid_guard,
            ))
        })?;

    rt.block_on(serve_existing_debug_session(
        ExistingDebugSessionConfig {
            login_name: config.login_name,
            ledger_dir: config.ledger_dir,
            socket_path,
            kind: "manual-debug".to_string(),
            max_duration: None,
            ready_listener: None,
        },
        browser_instance,
        handler_handle,
        page_inner,
        refreshmint_inner,
    ))?;

    Ok(())
}

/// Serve debugger requests against an existing browser/page pair and close the
/// browser when stopped. This is shared by fresh debug workers and failed
/// scrapes promoted in place.
#[cfg(unix)]
pub async fn serve_existing_debug_session(
    config: ExistingDebugSessionConfig,
    browser_instance: std::sync::Arc<tokio::sync::Mutex<chromiumoxide::browser::Browser>>,
    handler_handle: tokio::task::JoinHandle<()>,
    page_inner: std::sync::Arc<tokio::sync::Mutex<super::js_api::PageInner>>,
    refreshmint_inner: std::sync::Arc<tokio::sync::Mutex<super::js_api::RefreshmintInner>>,
) -> Result<(), Box<dyn Error>> {
    use std::time::Duration;
    use tokio::net::UnixListener;

    let socket_path = config.socket_path;
    let bind_socket_path = resolve_socket_bind_path(&socket_path);
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if socket_path.exists() {
        std::fs::remove_file(&socket_path)?;
    }
    if bind_socket_path.exists() {
        std::fs::remove_file(&bind_socket_path)?;
    }
    let _cleanup = SocketCleanup {
        paths: if bind_socket_path == socket_path {
            vec![socket_path.clone()]
        } else {
            vec![socket_path.clone(), bind_socket_path.clone()]
        },
    };

    let listener = UnixListener::bind(&bind_socket_path)?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bind_socket_path, std::fs::Permissions::from_mode(0o600))?;
    }
    if bind_socket_path != socket_path {
        std::os::unix::fs::symlink(&bind_socket_path, &socket_path)?;
    }
    let (_registration, descriptor) = crate::debug_registry::DebugSessionRegistration::create(
        &config.login_name,
        &config.kind,
        &socket_path,
        &config.ledger_dir,
    )?;
    eprintln!("Debug session id: {}", descriptor.session_id);
    eprintln!("Debug session socket: {}", socket_path.display());
    if let Some(listener) = config.ready_listener {
        listener(&descriptor);
    }

    let context = std::sync::Arc::new(ServerContext::new(
        std::sync::Arc::new(RealExecDispatcher {
            page_inner,
            refreshmint_inner,
        }),
        descriptor,
        config.max_duration,
    ));
    serve_connections(listener, context, &handler_handle).await?;

    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        let guard = browser_instance.lock().await;
        let _ = tokio::time::timeout(Duration::from_secs(5), guard.close()).await;
    })
    .await;
    drop(browser_instance);
    let _ = tokio::time::timeout(Duration::from_secs(5), handler_handle).await;
    Ok(())
}

/// State shared by every connection task of one debug session.
#[cfg(unix)]
struct ServerContext {
    dispatcher: std::sync::Arc<dyn ExecDispatcher>,
    /// Held for the whole of an `Exec`. Scripts share one page and one
    /// `RefreshmintInner` (whose `debug_output_sink` routes output to the
    /// requesting client), so a second `Exec` queues here instead of running
    /// alongside; `Status` and `Stop` never take it.
    exec_lock: tokio::sync::Mutex<()>,
    exec_running: std::sync::atomic::AtomicBool,
    descriptor: crate::debug_registry::DebugSessionDescriptor,
    started: std::time::Instant,
    max_duration: Option<std::time::Duration>,
    shutdown: tokio::sync::watch::Sender<bool>,
}

#[cfg(unix)]
impl ServerContext {
    fn new(
        dispatcher: std::sync::Arc<dyn ExecDispatcher>,
        descriptor: crate::debug_registry::DebugSessionDescriptor,
        max_duration: Option<std::time::Duration>,
    ) -> Self {
        Self {
            dispatcher,
            exec_lock: tokio::sync::Mutex::new(()),
            exec_running: std::sync::atomic::AtomicBool::new(false),
            descriptor,
            started: std::time::Instant::now(),
            max_duration,
            shutdown: tokio::sync::watch::channel(false).0,
        }
    }

    fn status(&self) -> DebugSessionStatus {
        let uptime = self.started.elapsed();
        DebugSessionStatus {
            session: self.descriptor.clone(),
            exec_running: self.exec_running.load(std::sync::atomic::Ordering::SeqCst),
            uptime_secs: uptime.as_secs(),
            expires_in_secs: self
                .max_duration
                .map(|max| max.saturating_sub(uptime).as_secs()),
        }
    }

    fn expired(&self) -> bool {
        self.max_duration
            .is_some_and(|max| self.started.elapsed() >= max)
    }
}

/// Clears `exec_running` even when the connection task is aborted mid-exec.
#[cfg(unix)]
struct ExecRunningGuard<'a>(&'a std::sync::atomic::AtomicBool);

#[cfg(unix)]
impl Drop for ExecRunningGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Accepts connections until stopped, expired, or the browser event handler
/// exits. Each connection runs in its own task so `Status` and `Stop` are
/// answered while an `Exec` is still running; connections still open at
/// shutdown (including a running `Exec`) are aborted.
#[cfg(unix)]
async fn serve_connections(
    listener: tokio::net::UnixListener,
    context: std::sync::Arc<ServerContext>,
    handler_handle: &tokio::task::JoinHandle<()>,
) -> std::io::Result<()> {
    let mut shutdown = context.shutdown.subscribe();
    let mut connections = tokio::task::JoinSet::new();
    let result = loop {
        if *shutdown.borrow_and_update() {
            break Ok(());
        }
        if handler_handle.is_finished() {
            eprintln!("Browser event handler stopped; ending debug session.");
            break Ok(());
        }
        if context.expired() {
            eprintln!("Debug session expired; closing browser.");
            break Ok(());
        }

        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _addr)) => {
                    let context = context.clone();
                    connections.spawn(async move { handle_connection(stream, &context).await });
                }
                Err(err) => break Err(err),
            },
            _ = shutdown.changed() => {}
            // Reap finished connections so the set does not grow unbounded.
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
            // Wake periodically to re-check expiry and the browser handler.
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
        }
    };
    connections.shutdown().await;
    result
}

/// Executes an `Exec` request against a stream. Boxed-future rather than
/// `async fn` in a trait so `RealExecDispatcher` and test fakes can share a
/// `dyn` seam without pulling `chromiumoxide` types into the seam itself;
/// that seam is what lets the connection-handling flow below (request
/// parsing, dispatch, response writing) be unit-tested with a fake browser.
#[cfg(unix)]
trait ExecDispatcher: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    fn dispatch<'a>(
        &'a self,
        stream: &'a mut tokio::net::UnixStream,
        script: Option<String>,
        entry_root: Option<PathBuf>,
        entry_path: Option<PathBuf>,
        declared_secrets: Option<super::js_api::SecretDeclarations>,
        prompt_overrides: Option<super::js_api::PromptOverrides>,
        prompt_requires_override: Option<bool>,
        script_options: Option<super::js_api::ScriptOptions>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = std::io::Result<()>> + Send + 'a>>;
}

#[cfg(unix)]
struct RealExecDispatcher {
    page_inner: std::sync::Arc<tokio::sync::Mutex<super::js_api::PageInner>>,
    refreshmint_inner: std::sync::Arc<tokio::sync::Mutex<super::js_api::RefreshmintInner>>,
}

#[cfg(unix)]
impl ExecDispatcher for RealExecDispatcher {
    #[allow(clippy::too_many_arguments)]
    fn dispatch<'a>(
        &'a self,
        stream: &'a mut tokio::net::UnixStream,
        script: Option<String>,
        entry_root: Option<PathBuf>,
        entry_path: Option<PathBuf>,
        declared_secrets: Option<super::js_api::SecretDeclarations>,
        prompt_overrides: Option<super::js_api::PromptOverrides>,
        prompt_requires_override: Option<bool>,
        script_options: Option<super::js_api::ScriptOptions>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = std::io::Result<()>> + Send + 'a>> {
        Box::pin(handle_exec_request_async(
            stream,
            self.page_inner.clone(),
            self.refreshmint_inner.clone(),
            script,
            entry_root,
            entry_path,
            declared_secrets,
            prompt_overrides,
            prompt_requires_override,
            script_options,
        ))
    }
}

/// Reads and handles one request off an accepted connection: `Exec` goes to
/// the dispatcher (one at a time, via `exec_lock`), `Stop` signals the
/// session to shut down after replying, and `Status` replies with a snapshot.
#[cfg(unix)]
async fn handle_connection(stream: tokio::net::UnixStream, context: &ServerContext) {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let mut reader = BufReader::new(stream);
    let mut body = String::new();
    let read_result = reader.read_line(&mut body).await;
    let mut stream = reader.into_inner();
    match read_result {
        // A client that connects and closes without sending anything is a
        // liveness probe (see `debug_registry::is_reachable`); there is no
        // one left to reply to.
        Ok(0) => {}
        Ok(_) => match serde_json::from_str::<Request>(body.trim()) {
            Ok(Request::Exec {
                script,
                entry_root,
                entry_path,
                declared_secrets,
                prompt_overrides,
                prompt_requires_override,
                script_options,
            }) => {
                let _exec_lock = context.exec_lock.lock().await;
                context
                    .exec_running
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                let _running = ExecRunningGuard(&context.exec_running);
                if let Err(err) = context
                    .dispatcher
                    .dispatch(
                        &mut stream,
                        script,
                        entry_root,
                        entry_path,
                        declared_secrets,
                        prompt_overrides,
                        prompt_requires_override,
                        script_options,
                    )
                    .await
                {
                    eprintln!("failed to write debug exec stream: {err}");
                }
            }
            Ok(Request::Stop) => {
                let response = Response {
                    ok: true,
                    error: None,
                };
                // Reply before signaling: shutdown aborts every connection
                // task, this one included.
                if let Err(err) = write_response_async(&mut stream, &response).await {
                    eprintln!("failed to write debug response: {err}");
                }
                context.shutdown.send_replace(true);
            }
            Ok(Request::Status) => {
                let response = StatusResponse {
                    ok: true,
                    error: None,
                    status: Some(context.status()),
                };
                if let Err(err) = write_response_async(&mut stream, &response).await {
                    eprintln!("failed to write debug response: {err}");
                }
            }
            Err(err) => {
                let response = Response {
                    ok: false,
                    error: Some(format!("invalid request: {err}")),
                };
                if let Err(err) = write_response_async(&mut stream, &response).await {
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

#[cfg(unix)]
#[allow(clippy::too_many_arguments)]
async fn handle_exec_request_async(
    stream: &mut tokio::net::UnixStream,
    page_inner: std::sync::Arc<tokio::sync::Mutex<super::js_api::PageInner>>,
    refreshmint_inner: std::sync::Arc<tokio::sync::Mutex<super::js_api::RefreshmintInner>>,
    script: Option<String>,
    entry_root: Option<PathBuf>,
    entry_path: Option<PathBuf>,
    declared_secrets: Option<super::js_api::SecretDeclarations>,
    prompt_overrides: Option<super::js_api::PromptOverrides>,
    prompt_requires_override: Option<bool>,
    script_options: Option<super::js_api::ScriptOptions>,
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
        if let Some(options) = script_options {
            refreshmint.script_options = options;
        }
        refreshmint.debug_output_sink = Some(output_sender);
    }

    let refreshmint_inner_for_task = refreshmint_inner.clone();
    let mut exec_task = tokio::spawn(async move {
        let run_result = match (script, entry_root, entry_path) {
            (Some(script), None, None) => {
                super::sandbox::run_script_source_with_options(
                    &script,
                    page_inner,
                    refreshmint_inner_for_task.clone(),
                    super::sandbox::SandboxRunOptions {
                        emit_diagnostics: false,
                    },
                )
                .await
            }
            (None, Some(extension_root), Some(entry_path)) => {
                super::sandbox::run_module_path_with_options(
                    &extension_root,
                    &entry_path,
                    page_inner,
                    refreshmint_inner_for_task.clone(),
                    super::sandbox::SandboxRunOptions {
                        emit_diagnostics: false,
                    },
                )
                .await
            }
            _ => Err(
                "invalid debug exec request: expected either script source or module entrypoint"
                    .into(),
            ),
        };

        let finalize_result = {
            let mut refreshmint = refreshmint_inner_for_task.lock().await;
            finalize_debug_exec_resources(&mut refreshmint)
        };

        // Same run+finalize combining as the full scrape (see
        // super::combine_run_and_finalize). `debug exec` has no cancel watch, so
        // it is never a user cancel.
        let result = super::combine_run_and_finalize(run_result, finalize_result, false);

        {
            let mut refreshmint = refreshmint_inner_for_task.lock().await;
            refreshmint.debug_output_sink = None;
        }

        result
    });
    // A `Stop` from another client aborts this connection's task; without
    // this the detached script would keep driving the page while the
    // browser is being closed.
    let _abort_script_on_drop = AbortOnDrop(exec_task.abort_handle());

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

    let sensitive_data = {
        let refreshmint = refreshmint_inner.lock().await;
        refreshmint.sensitive_data.clone()
    };
    let final_frame = match final_result {
        Ok(()) => ExecStreamFrame::Result {
            ok: true,
            error: None,
        },
        Err(err) => ExecStreamFrame::Result {
            ok: false,
            error: Some(sensitive_data.redact(&err)),
        },
    };
    if let Err(err) = write_exec_stream_frame_async(stream, &final_frame).await {
        eprintln!("debug exec client disconnected before final result frame: {err}");
    }
    Ok(())
}

#[cfg(unix)]
struct AbortOnDrop(tokio::task::AbortHandle);

#[cfg(unix)]
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
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

fn send_request<T: serde::de::DeserializeOwned>(
    socket_path: &Path,
    request: Request,
) -> Result<T, Box<dyn Error>> {
    send_request_with_timeout(socket_path, request, None)
}

#[cfg(unix)]
fn send_request_with_timeout<T: serde::de::DeserializeOwned>(
    socket_path: &Path,
    request: Request,
    timeout: Option<std::time::Duration>,
) -> Result<T, Box<dyn Error>> {
    use std::io::{Read, Write};
    use std::net::Shutdown;
    use std::os::unix::net::UnixStream;

    let connect_path = resolve_socket_bind_path(socket_path);
    let mut stream = UnixStream::connect(&connect_path)?;
    stream.set_read_timeout(timeout)?;
    stream.set_write_timeout(timeout)?;
    serde_json::to_writer(&mut stream, &request)?;
    stream.write_all(b"\n")?;
    stream.shutdown(Shutdown::Write)?;

    let mut response_body = String::new();
    stream.read_to_string(&mut response_body)?;
    Ok(serde_json::from_str(response_body.trim())?)
}

#[cfg(not(unix))]
fn send_request_with_timeout<T: serde::de::DeserializeOwned>(
    _socket_path: &Path,
    _request: Request,
    _timeout: Option<std::time::Duration>,
) -> Result<T, Box<dyn Error>> {
    Err("debug sockets are currently supported only on unix platforms".into())
}

#[cfg(unix)]
async fn write_response_async<T: serde::Serialize>(
    stream: &mut tokio::net::UnixStream,
    response: &T,
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

#[cfg(any(unix, test))]
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
    paths: Vec<PathBuf>,
}

#[cfg(unix)]
impl Drop for SocketCleanup {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(unix)]
fn unix_socket_path_len(path: &Path) -> usize {
    use std::os::unix::ffi::OsStrExt;

    path.as_os_str().as_bytes().len()
}

#[cfg(unix)]
fn resolve_socket_bind_path(requested_path: &Path) -> PathBuf {
    if unix_socket_path_len(requested_path) < 100 {
        return requested_path.to_path_buf();
    }

    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let stem = requested_path
        .file_stem()
        .and_then(|value| value.to_str())
        .map(sanitize_segment)
        .unwrap_or_else(|| "debug".to_string());
    let mut hasher = DefaultHasher::new();
    requested_path.hash(&mut hasher);
    let suffix = format!("{:08x}", hasher.finish() as u32);
    let shortened = stem.chars().take(24).collect::<String>();
    std::env::temp_dir().join(format!("rm-{}-{}.sock", shortened, suffix))
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::{
        debug_session_status, handle_connection, serve_connections, stop_debug_session,
        ExecDispatcher, Request, Response, ServerContext,
    };
    use super::{
        finalize_debug_exec_resources, sanitize_segment, ExecOutputStream, ExecStreamFrame,
    };
    use crate::login_config::login_account_documents_dir;
    use crate::scrape::js_api::{
        PromptOverrides, RefreshmintInner, ScriptOptions, SensitiveData, SessionMetadata,
        StagedResource,
    };
    use std::fs;
    use std::path::PathBuf;
    #[cfg(unix)]
    use std::sync::Arc;
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
            script_options: ScriptOptions::new(),
            debug_output_sink: None,
            sensitive_data: std::sync::Arc::new(SensitiveData::default()),
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
            prompt_ui_handler: None,
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

    #[cfg(unix)]
    fn test_descriptor(socket_path: PathBuf) -> crate::debug_registry::DebugSessionDescriptor {
        crate::debug_registry::DebugSessionDescriptor {
            session_id: "debug-test".to_string(),
            login_name: "bank".to_string(),
            kind: "failed-scrape".to_string(),
            pid: std::process::id(),
            socket_path,
            ledger_dir: "/tmp/ledger.refreshmint".into(),
            started_at: "now".to_string(),
        }
    }

    /// Stands in for `RealExecDispatcher` so the socket protocol can be
    /// tested without a browser: streams one output line, then (optionally)
    /// holds the exec open until `release` is notified, like a long script.
    #[cfg(unix)]
    #[derive(Default)]
    struct FakeExecDispatcher {
        dispatched_scripts: std::sync::Mutex<Vec<String>>,
        hold_until_released: bool,
        release: tokio::sync::Notify,
    }

    #[cfg(unix)]
    impl FakeExecDispatcher {
        fn dispatched_scripts(&self) -> Vec<String> {
            self.dispatched_scripts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }
    }

    #[cfg(unix)]
    impl ExecDispatcher for FakeExecDispatcher {
        fn dispatch<'a>(
            &'a self,
            stream: &'a mut tokio::net::UnixStream,
            script: Option<String>,
            _entry_root: Option<PathBuf>,
            _entry_path: Option<PathBuf>,
            _declared_secrets: Option<crate::scrape::js_api::SecretDeclarations>,
            _prompt_overrides: Option<PromptOverrides>,
            _prompt_requires_override: Option<bool>,
            _script_options: Option<ScriptOptions>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = std::io::Result<()>> + Send + 'a>>
        {
            self.dispatched_scripts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(script.unwrap_or_default());
            Box::pin(async move {
                super::write_exec_stream_frame_async(
                    stream,
                    &ExecStreamFrame::Output {
                        stream: ExecOutputStream::Stdout,
                        line: "hello from fake browser".to_string(),
                    },
                )
                .await?;
                if self.hold_until_released {
                    self.release.notified().await;
                }
                super::write_exec_stream_frame_async(
                    stream,
                    &ExecStreamFrame::Result {
                        ok: true,
                        error: None,
                    },
                )
                .await
            })
        }
    }

    #[cfg(unix)]
    fn exec_request(script: &str) -> Request {
        Request::Exec {
            script: Some(script.to_string()),
            entry_root: None,
            entry_path: None,
            declared_secrets: None,
            prompt_overrides: None,
            prompt_requires_override: None,
            script_options: None,
        }
    }

    #[cfg(unix)]
    async fn write_request(client: &mut tokio::net::UnixStream, request: &Request) {
        use tokio::io::AsyncWriteExt;

        let mut bytes =
            serde_json::to_vec(request).unwrap_or_else(|err| panic!("serialize failed: {err}"));
        bytes.push(b'\n');
        client
            .write_all(&bytes)
            .await
            .unwrap_or_else(|err| panic!("failed to write request: {err}"));
    }

    #[cfg(unix)]
    async fn read_line<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut R) -> String {
        use tokio::io::AsyncBufReadExt;

        let mut line = String::new();
        match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            reader.read_line(&mut line),
        )
        .await
        {
            Ok(Ok(_)) => line,
            other => panic!("failed to read a line: {other:?}"),
        }
    }

    #[cfg(unix)]
    fn parse_frame(line: &str) -> ExecStreamFrame {
        serde_json::from_str(line.trim())
            .unwrap_or_else(|err| panic!("failed to parse frame {line:?}: {err}"))
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn handle_connection_serves_a_normal_exec_request() {
        let (mut client, server) = tokio::net::UnixStream::pair()
            .unwrap_or_else(|err| panic!("failed to create socket pair: {err}"));
        write_request(&mut client, &exec_request("console.log('hi')")).await;

        let dispatcher = Arc::new(FakeExecDispatcher::default());
        let context =
            ServerContext::new(dispatcher.clone(), test_descriptor("/unused".into()), None);
        handle_connection(server, &context).await;

        assert_eq!(dispatcher.dispatched_scripts(), vec!["console.log('hi')"]);
        assert!(
            !*context.shutdown.borrow(),
            "Exec must not stop the session"
        );
        assert!(!context.status().exec_running);

        let mut reader = tokio::io::BufReader::new(&mut client);
        assert_eq!(
            parse_frame(&read_line(&mut reader).await),
            ExecStreamFrame::Output {
                stream: ExecOutputStream::Stdout,
                line: "hello from fake browser".to_string(),
            }
        );
        assert_eq!(
            parse_frame(&read_line(&mut reader).await),
            ExecStreamFrame::Result {
                ok: true,
                error: None,
            }
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn handle_connection_serves_a_normal_stop_request() {
        let (mut client, server) = tokio::net::UnixStream::pair()
            .unwrap_or_else(|err| panic!("failed to create socket pair: {err}"));
        write_request(&mut client, &Request::Stop).await;

        let dispatcher = Arc::new(FakeExecDispatcher::default());
        let context =
            ServerContext::new(dispatcher.clone(), test_descriptor("/unused".into()), None);
        handle_connection(server, &context).await;

        assert!(*context.shutdown.borrow(), "Stop must signal shutdown");
        assert!(
            dispatcher.dispatched_scripts().is_empty(),
            "Stop must not be routed through the exec dispatcher"
        );
        let mut reader = tokio::io::BufReader::new(&mut client);
        let response: Response = serde_json::from_str(read_line(&mut reader).await.trim())
            .unwrap_or_else(|err| panic!("failed to parse stop response: {err}"));
        assert!(response.ok);
        assert_eq!(response.error, None);
    }

    /// Runs the real accept loop on a real socket: while one client's exec
    /// is still running, a second client's `Status` is answered (the old
    /// one-connection-at-a-time loop would hang here), a third client's
    /// `Exec` queues rather than running alongside, and a `Stop` ends the
    /// session and cuts off both in-flight exec connections.
    #[cfg(unix)]
    #[tokio::test]
    async fn serve_connections_answers_status_and_stop_while_an_exec_is_running() {
        let socket_path =
            std::env::temp_dir().join(format!("rm-mux-test-{}.sock", std::process::id()));
        let _ = fs::remove_file(&socket_path);
        let listener = tokio::net::UnixListener::bind(&socket_path)
            .unwrap_or_else(|err| panic!("failed to bind test socket: {err}"));

        let dispatcher = Arc::new(FakeExecDispatcher {
            hold_until_released: true,
            ..FakeExecDispatcher::default()
        });
        let context = Arc::new(ServerContext::new(
            dispatcher.clone(),
            test_descriptor(socket_path.clone()),
            Some(std::time::Duration::from_secs(600)),
        ));
        let server = tokio::spawn({
            let context = context.clone();
            async move {
                let browser_handler = tokio::spawn(std::future::pending::<()>());
                let result = serve_connections(listener, context, &browser_handler).await;
                browser_handler.abort();
                result
            }
        });

        let mut first_exec = tokio::net::UnixStream::connect(&socket_path)
            .await
            .unwrap_or_else(|err| panic!("failed to connect: {err}"));
        write_request(&mut first_exec, &exec_request("first")).await;
        let mut first_exec = tokio::io::BufReader::new(first_exec);
        // Its first output frame proves the exec is running (and holding the
        // exec lock) before the other clients connect.
        assert!(matches!(
            parse_frame(&read_line(&mut first_exec).await),
            ExecStreamFrame::Output { .. }
        ));

        let status_path = socket_path.clone();
        let status = match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio::task::spawn_blocking(move || {
                debug_session_status(&status_path, None).map_err(|err| err.to_string())
            }),
        )
        .await
        {
            Ok(Ok(Ok(status))) => status,
            other => panic!("status request was not answered during an exec: {other:?}"),
        };
        assert!(status.exec_running);
        assert_eq!(status.session.session_id, "debug-test");
        assert!(status
            .expires_in_secs
            .is_some_and(|secs| (590..=600).contains(&secs)));

        let mut second_exec = tokio::net::UnixStream::connect(&socket_path)
            .await
            .unwrap_or_else(|err| panic!("failed to connect: {err}"));
        write_request(&mut second_exec, &exec_request("second")).await;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert_eq!(
            dispatcher.dispatched_scripts(),
            vec!["first"],
            "a second exec must wait for the first instead of running alongside it"
        );

        let stop_path = socket_path.clone();
        match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio::task::spawn_blocking(move || {
                stop_debug_session(&stop_path).map_err(|err| err.to_string())
            }),
        )
        .await
        {
            Ok(Ok(Ok(()))) => {}
            other => panic!("stop request failed: {other:?}"),
        }
        match tokio::time::timeout(std::time::Duration::from_secs(5), server).await {
            Ok(Ok(Ok(()))) => {}
            other => panic!("server did not shut down after Stop: {other:?}"),
        }

        assert_eq!(
            read_line(&mut first_exec).await,
            "",
            "the running exec's connection must be closed without a result"
        );
        let mut second_exec = tokio::io::BufReader::new(second_exec);
        assert_eq!(read_line(&mut second_exec).await, "");
        assert_eq!(dispatcher.dispatched_scripts(), vec!["first"]);
        assert!(!context.status().exec_running);

        let _ = fs::remove_file(&socket_path);
    }
}
