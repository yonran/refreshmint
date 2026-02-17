use std::error::Error;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct DebugStartConfig {
    pub account: String,
    pub extension_name: String,
    pub ledger_dir: PathBuf,
    pub profile_override: Option<PathBuf>,
    pub socket_path: Option<PathBuf>,
    pub prompt_requires_override: bool,
}

pub fn default_debug_socket_path(account: &str) -> Result<PathBuf, Box<dyn Error>> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;

        let account_sanitized = sanitize_segment(account);
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
        let _ = account;
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
    let response = send_request(
        socket_path,
        Request::Exec {
            script: script_source.to_string(),
            declared_secrets,
            prompt_overrides,
            prompt_requires_override,
        },
    )?;
    if response.ok {
        return Ok(());
    }
    Err(response
        .error
        .unwrap_or_else(|| "exec failed".to_string())
        .into())
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

#[cfg(unix)]
fn run_debug_session_unix(config: DebugStartConfig) -> Result<(), Box<dyn Error>> {
    use chromiumoxide::browser::Browser;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::net::UnixListener;
    use tokio::sync::Mutex;

    type DebugRuntimeState = (
        Browser,
        tokio::task::JoinHandle<()>,
        Arc<Mutex<super::js_api::PageInner>>,
        Arc<Mutex<super::js_api::RefreshmintInner>>,
    );

    let socket_path = match config.socket_path {
        Some(path) => path,
        None => default_debug_socket_path(&config.account)?,
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
            let secret_store = crate::secret::SecretStore::new(config.account.clone());
            let profile_dir = super::profile::resolve_profile_dir(
                &config.ledger_dir,
                &config.account,
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
            let output_dir = extension_dir.join("output");
            std::fs::create_dir_all(&output_dir).map_err(|err| err.to_string())?;

            let chrome_path =
                super::browser::find_chrome_binary().map_err(|err| err.to_string())?;
            eprintln!("Using browser: {}", chrome_path.display());
            eprintln!("Profile dir: {}", profile_dir.display());

            let (mut browser, handler) = super::browser::launch_browser(&chrome_path, &profile_dir)
                .await
                .map_err(|err| err.to_string())?;
            let page = super::browser::open_start_page(&mut browser)
                .await
                .map_err(|err| err.to_string())?;

            let page_inner = Arc::new(Mutex::new(super::js_api::PageInner {
                page,
                secret_store,
                declared_secrets,
                download_dir,
            }));
            let refreshmint_inner = Arc::new(Mutex::new(super::js_api::RefreshmintInner {
                output_dir,
                prompt_overrides: super::js_api::PromptOverrides::new(),
                prompt_requires_override: config.prompt_requires_override,
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
                    let response = match reader.read_line(&mut body).await {
                        Ok(0) => Response {
                            ok: false,
                            error: Some("failed to read request: empty request".to_string()),
                        },
                        Ok(_) => match serde_json::from_str::<Request>(body.trim()) {
                            Ok(Request::Exec {
                                script,
                                declared_secrets,
                                prompt_overrides,
                                prompt_requires_override,
                            }) => {
                                if let Some(declared) = declared_secrets {
                                    let mut page_inner = page_inner.lock().await;
                                    page_inner.declared_secrets = declared;
                                }
                                {
                                    let mut refreshmint = refreshmint_inner.lock().await;
                                    refreshmint.prompt_overrides =
                                        prompt_overrides.unwrap_or_default();
                                    if let Some(require_override) = prompt_requires_override {
                                        refreshmint.prompt_requires_override = require_override;
                                    }
                                }
                                match super::sandbox::run_script_source(
                                    &script,
                                    page_inner.clone(),
                                    refreshmint_inner.clone(),
                                )
                                .await
                                {
                                    Ok(()) => Response {
                                        ok: true,
                                        error: None,
                                    },
                                    Err(err) => Response {
                                        ok: false,
                                        error: Some(err.to_string()),
                                    },
                                }
                            }
                            Ok(Request::Stop) => {
                                running = false;
                                Response {
                                    ok: true,
                                    error: None,
                                }
                            }
                            Err(err) => Response {
                                ok: false,
                                error: Some(format!("invalid request: {err}")),
                            },
                        },
                        Err(err) => Response {
                            ok: false,
                            error: Some(format!("failed to read request: {err}")),
                        },
                    };

                    let mut stream = reader.into_inner();
                    if let Err(err) = write_response_async(&mut stream, &response).await {
                        eprintln!("failed to write debug response: {err}");
                    }
                }
                Ok(Err(err)) => return Err::<(), Box<dyn Error>>(err.into()),
                Err(_) => continue,
            }
        }

        drop(listener);
        drop(browser_instance);
        let _ = tokio::time::timeout(Duration::from_secs(5), handler_handle).await;
        Ok::<(), Box<dyn Error>>(())
    })?;

    Ok(())
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
    use super::sanitize_segment;

    #[test]
    fn sanitize_segment_preserves_safe_chars() {
        assert_eq!(sanitize_segment("abc-DEF_123"), "abc-DEF_123");
    }

    #[test]
    fn sanitize_segment_replaces_unsafe_chars() {
        assert_eq!(sanitize_segment("a/b:c"), "a_b_c");
    }
}
