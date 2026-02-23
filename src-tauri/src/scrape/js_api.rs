use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rquickjs::class::Trace;
use rquickjs::{function::Opt, Ctx, JsLifetime, Result as JsResult};
use tokio::sync::Mutex;

use super::locator::Locator;
use crate::secret::SecretStore;

pub(crate) fn js_err(msg: String) -> rquickjs::Error {
    rquickjs::Error::new_from_js_message("Error", "Error", msg)
}

const BROWSER_DISCONNECTED_ERROR: &str =
    "BrowserDisconnectedError: debug browser channel closed; restart debug session";

const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const POLL_INTERVAL_MS: u64 = 100;
const TAB_QUERY_TIMEOUT_MS: u64 = 5_000;

fn is_transport_disconnected_error(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("receiver is gone")
        || lower.contains("send failed")
        || lower.contains("channel closed")
        || lower.contains("connection closed")
        || lower.contains("broken pipe")
        || lower.contains("not connected")
        || (lower.contains("websocket") && lower.contains("closed"))
}

fn format_browser_error(context: &str, err: &str) -> String {
    if is_transport_disconnected_error(err) {
        return format!("{BROWSER_DISCONNECTED_ERROR} ({context}: {err})");
    }
    format!("{context}: {err}")
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct NetworkRequest {
    #[serde(default)]
    url: String,
    #[serde(default)]
    status: i64,
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    method: String,
    #[serde(default)]
    ts: i64,
    #[serde(default)]
    error: Option<String>,
}

struct ResponseCaptureState {
    entries: Arc<Mutex<Vec<NetworkRequest>>>,
    task: tokio::task::JoinHandle<()>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct SnapshotNode {
    #[serde(default)]
    r#ref: String,
    #[serde(default)]
    parent_ref: Option<String>,
    #[serde(default)]
    role: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    tag: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    value: String,
    #[serde(default)]
    visible: bool,
    #[serde(default)]
    disabled: bool,
    #[serde(default)]
    expanded: Option<bool>,
    #[serde(default)]
    selected: Option<bool>,
    #[serde(default)]
    checked: Option<String>,
    #[serde(default)]
    level: Option<u32>,
    #[serde(default)]
    aria_labelled_by: Option<String>,
    #[serde(default)]
    aria_described_by: Option<String>,
    #[serde(default)]
    selector_hint: String,
}

#[derive(Debug, Clone)]
struct SnapshotOptions {
    incremental: bool,
    track: String,
}

impl Default for SnapshotOptions {
    fn default() -> Self {
        Self {
            incremental: false,
            track: "default".to_string(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotDiffEntry {
    change: String,
    node: SnapshotNode,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotDiff {
    mode: String,
    track: String,
    base_node_count: usize,
    node_count: usize,
    changed_count: usize,
    removed_count: usize,
    unchanged_count: usize,
    changed: Vec<SnapshotDiffEntry>,
    removed_refs: Vec<String>,
}

#[derive(Debug, Clone)]
struct OpenTab {
    page: chromiumoxide::Page,
    target_id: String,
    opener_target_id: Option<String>,
}

pub type SecretDeclarations = BTreeMap<String, BTreeSet<String>>;
pub type PromptOverrides = BTreeMap<String, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugOutputStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugOutputEvent {
    pub stream: DebugOutputStream,
    pub line: String,
}

/// Shared state backing the `page` JS object.
pub struct PageInner {
    pub page: chromiumoxide::Page,
    pub browser: Arc<Mutex<chromiumoxide::browser::Browser>>,
    pub secret_store: Arc<SecretStore>,
    pub declared_secrets: Arc<SecretDeclarations>,
    pub download_dir: PathBuf,
    pub target_frame_id: Option<chromiumoxide::cdp::browser_protocol::page::FrameId>,
}

/// JS-visible `page` object with Playwright-like API.
///
/// All methods are async and return Promises in JS.
#[rquickjs::class(rename = "Page")]
#[derive(Trace)]
pub struct PageApi {
    #[qjs(skip_trace)]
    inner: Arc<Mutex<PageInner>>,
    #[qjs(skip_trace)]
    response_capture: Arc<Mutex<Option<ResponseCaptureState>>>,
    #[qjs(skip_trace)]
    snapshot_tracks: Arc<Mutex<BTreeMap<String, Vec<SnapshotNode>>>>,
}

// Safety: PageApi only contains Arc<Mutex<...>> which is 'static and has no JS lifetimes.
#[allow(unsafe_code)]
unsafe impl<'js> JsLifetime<'js> for PageApi {
    type Changed<'to> = PageApi;
}

/// JS-visible `browser` object for page discovery/waiting.
#[rquickjs::class(rename = "Browser")]
#[derive(Trace)]
pub struct BrowserApi {
    #[qjs(skip_trace)]
    page_inner: Arc<Mutex<PageInner>>,
}

#[allow(unsafe_code)]
unsafe impl<'js> JsLifetime<'js> for BrowserApi {
    type Changed<'to> = BrowserApi;
}

impl PageApi {
    pub fn new(inner: Arc<Mutex<PageInner>>) -> Self {
        Self {
            inner,
            response_capture: Arc::new(Mutex::new(None)),
            snapshot_tracks: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }
}

impl BrowserApi {
    pub fn new(page_inner: Arc<Mutex<PageInner>>) -> Self {
        Self { page_inner }
    }
}

#[rquickjs::methods]
impl PageApi {
    /// Create a locator for the given selector.
    pub fn locator(&self, selector: String) -> Locator {
        Locator::new(self.inner.clone(), selector)
    }

    /// Navigate to a URL.
    #[qjs(rename = "goto")]
    pub async fn js_goto(&self, url: String) -> JsResult<()> {
        let current_url = self.current_url().await?;
        if current_url == url {
            let inner = self.inner.lock().await;
            inner
                .page
                .reload()
                .await
                .map_err(|e| js_err(format!("goto failed (same-url reload): {e}")))?;
            return Ok(());
        }

        if urls_differ_only_by_fragment(&current_url, &url) {
            let url_json = serde_json::to_string(&url).unwrap_or_else(|_| "\"\"".to_string());
            let expression =
                format!("(() => {{ window.location.href = {url_json}; return true; }})()");

            {
                let inner = self.inner.lock().await;
                inner
                    .page
                    .evaluate(expression.as_str())
                    .await
                    .map_err(|e| js_err(format!("goto failed (hash-navigation): {e}")))?;
            }

            let deadline =
                tokio::time::Instant::now() + std::time::Duration::from_millis(DEFAULT_TIMEOUT_MS);
            loop {
                let observed = self.current_url().await?;
                if observed == url {
                    return Ok(());
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(js_err(format!(
                        "goto failed (hash-navigation): timeout {DEFAULT_TIMEOUT_MS}ms exceeded (current URL {observed})"
                    )));
                }
                tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
            }
        }

        let inner = self.inner.lock().await;
        if url.starts_with("data:") {
            // Robustly navigate to about:blank first
            let current_href = match inner.page.evaluate("window.location.href").await {
                Ok(res) => res
                    .value()
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                Err(_) => String::new(),
            };

            if current_href != "about:blank" {
                use chromiumoxide::cdp::browser_protocol::page::NavigateParams;
                // Use Page.navigate via execute() to force navigation without waiting for CDP events
                let params = NavigateParams::builder()
                    .url("about:blank")
                    .build()
                    .map_err(|e| js_err(format!("goto(data) prelude build failed: {e}")))?;
                inner.page.execute(params).await.ok();

                for _ in 0..50 {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    if let Ok(res) = inner.page.evaluate("window.location.href").await {
                        let h = res.value().and_then(|v| v.as_str()).unwrap_or("");
                        if h == "about:blank" {
                            break;
                        }
                    }
                }
            }

            let url_json = serde_json::to_string(&url).unwrap_or_else(|_| "\"\"".to_string());
            let expression = format!(
                r#"(() => {{
                    const data = {url_json};
                    const comma = data.indexOf(',');
                    if (comma === -1) return;
                    const html = decodeURIComponent(data.substring(comma + 1));
                    try {{
                        document.open();
                        document.write(html);
                        document.close();
                    }} catch (e) {{
                        console.warn('document.write failed, falling back to innerHTML', e);
                        document.body.innerHTML = html;
                    }}
                }})()"#
            );

            use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
            let eval = EvaluateParams::builder()
                .expression(expression)
                .await_promise(false)
                .return_by_value(false)
                .build()
                .map_err(|e| js_err(format!("goto(data) build failed: {e}")))?;
            inner
                .page
                .evaluate_expression(eval)
                .await
                .map_err(|e| js_err(format!("goto(data) failed: {e}")))?;

            // Brief wait for rendering
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        } else {
            inner
                .page
                .goto(&url)
                .await
                .map_err(|e| js_err(format!("goto failed: {e}")))?;
        }
        Ok(())
    }

    /// Get the current page URL.
    pub async fn url(&self) -> JsResult<String> {
        eprintln!("[js_api] page.url enter");
        let inner = self.inner.lock().await;
        let url = inner
            .page
            .url()
            .await
            .map_err(|e| {
                let err_text = e.to_string();
                eprintln!("[js_api] page.url error: {err_text}");
                js_err(format_browser_error("url() failed", &err_text))
            })?
            .unwrap_or_default();
        eprintln!("[js_api] page.url ok");
        Ok(url.to_string())
    }

    /// Reload the current page.
    pub async fn reload(&self) -> JsResult<()> {
        let inner = self.inner.lock().await;
        use chromiumoxide::cdp::browser_protocol::page::ReloadParams;
        inner
            .page
            .execute(ReloadParams::default())
            .await
            .map_err(|e| js_err(format!("reload failed: {e}")))?;
        Ok(())
    }

    /// List all frames in the page as a JSON string.
    ///
    /// Each element has `{ id, name, url, parentId }`.
    pub async fn frames(&self) -> JsResult<String> {
        let inner = self.inner.lock().await;
        use chromiumoxide::cdp::browser_protocol::page::GetFrameTreeParams;
        let tree = inner
            .page
            .execute(GetFrameTreeParams::default())
            .await
            .map_err(|e| js_err(format!("frames failed: {e}")))?;

        #[derive(serde::Serialize)]
        struct FrameInfo {
            id: String,
            name: String,
            url: String,
            #[serde(rename = "parentId")]
            parent_id: Option<String>,
        }

        let mut out = Vec::new();
        let mut stack = vec![tree.result.frame_tree];
        while let Some(node) = stack.pop() {
            out.push(FrameInfo {
                id: node.frame.id.as_ref().to_string(),
                name: node.frame.name.unwrap_or_default(),
                url: node.frame.url,
                parent_id: node.frame.parent_id.map(|p| p.as_ref().to_string()),
            });
            if let Some(children) = node.child_frames {
                for child in children {
                    stack.push(child);
                }
            }
        }
        serde_json::to_string(&out).map_err(|e| js_err(format!("frames serialization failed: {e}")))
    }

    /// Switch subsequent element interactions to the given frame.
    ///
    /// `frame_ref` may be a frame id, frame name, or frame URL substring.
    #[qjs(rename = "switchToFrame")]
    pub async fn js_switch_to_frame(&self, frame_ref: String) -> JsResult<()> {
        let mut inner = self.inner.lock().await;
        let frame_id = resolve_frame_id(&inner.page, &frame_ref)
            .await
            .map_err(|e| js_err(format!("switchToFrame failed: {e}")))?;
        inner.target_frame_id = Some(frame_id);
        Ok(())
    }

    /// Reset subsequent element interactions to the main frame.
    #[qjs(rename = "switchToMainFrame")]
    pub async fn js_switch_to_main_frame(&self) -> JsResult<()> {
        let mut inner = self.inner.lock().await;
        inner.target_frame_id = None;
        Ok(())
    }

    /// Wait for a CSS selector to appear in the DOM.
    #[qjs(rename = "waitForSelector")]
    pub async fn js_wait_for_selector(
        &self,
        selector: String,
        timeout_ms: Option<u64>,
    ) -> JsResult<()> {
        let timeout_ms = timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        let selector_json = serde_json::to_string(&selector).unwrap_or_else(|_| "\"\"".to_string());
        let probe = format!(
            r#"(() => {{
                try {{
                    return !!document.querySelector({selector_json});
                }} catch (err) {{
                    return {{ __refreshmintSelectorError: String(err) }};
                }}
            }})()"#
        );

        loop {
            let res = self
                .evaluate_in_active_context(probe.clone())
                .await
                .map_err(|e| js_err(format!("waitForSelector failed: {e}")))?;
            if res == "true" {
                return Ok(());
            }
            if res.contains("__refreshmintSelectorError") {
                let val: serde_json::Value = serde_json::from_str(&res).unwrap_or_default();
                if let Some(selector_error) = val
                    .get("__refreshmintSelectorError")
                    .and_then(serde_json::Value::as_str)
                {
                    return Err(js_err(format!(
                        "waitForSelector(\"{selector}\") failed: {selector_error}"
                    )));
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(js_err(format!(
                    "TimeoutError: waiting for selector \"{selector}\" failed: timeout {timeout_ms}ms exceeded"
                )));
            }
            tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
        }
    }

    /// Wait for the next navigation.
    #[qjs(rename = "waitForNavigation")]
    pub async fn js_wait_for_navigation(&self, timeout_ms: Option<u64>) -> JsResult<()> {
        let timeout_ms = timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
        let initial_url = self.current_url().await?;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);

        loop {
            let url = self.current_url().await?;
            if url != initial_url {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(js_err(format!(
                    "TimeoutError: waiting for navigation failed: timeout {timeout_ms}ms exceeded (still at {url})"
                )));
            }
            tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
        }
    }

    /// Wait until current URL matches a pattern (`*` wildcard supported).
    #[qjs(rename = "waitForURL")]
    pub async fn js_wait_for_url(&self, pattern: String, timeout_ms: Option<u64>) -> JsResult<()> {
        let timeout_ms = timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);

        loop {
            let url = self.current_url().await?;
            if url_matches_pattern(&url, &pattern) {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(js_err(format!(
                    "TimeoutError: waiting for URL pattern \"{pattern}\" failed: timeout {timeout_ms}ms exceeded (current URL {url})"
                )));
            }
            tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
        }
    }

    /// Wait for a page load state (`load`, `domcontentloaded`, or `networkidle`).
    #[qjs(rename = "waitForLoadState")]
    pub async fn js_wait_for_load_state(
        &self,
        state: Option<String>,
        timeout_ms: Option<u64>,
    ) -> JsResult<()> {
        let requested_state = state.unwrap_or_else(|| "load".to_string());
        let state = requested_state.to_ascii_lowercase();
        if state != "load" && state != "domcontentloaded" && state != "networkidle" {
            return Err(js_err(format!(
                "waitForLoadState unsupported state: {requested_state}"
            )));
        }

        let timeout_ms = timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
        if state == "networkidle" {
            let page = {
                let inner = self.inner.lock().await;
                inner.page.clone()
            };
            let timeout = std::time::Duration::from_millis(timeout_ms);
            return tokio::time::timeout(timeout, page.wait_for_network_idle())
                .await
                .map_err(|_| {
                    js_err(format!(
                        "TimeoutError: waiting for load state \"{requested_state}\" failed: timeout {timeout_ms}ms exceeded"
                    ))
                })
                .and_then(|result| {
                    result
                        .map(|_| ())
                        .map_err(|e| js_err(format!("waitForLoadState(networkidle) failed: {e}")))
                });
        }

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            let ready = match state.as_str() {
                "load" => self.ready_state_is_complete().await?,
                "domcontentloaded" => self.ready_state_is_interactive_or_complete().await?,
                _ => false,
            };
            if ready {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(js_err(format!(
                    "TimeoutError: waiting for load state \"{requested_state}\" failed: timeout {timeout_ms}ms exceeded"
                )));
            }
            tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
        }
    }

    /// Wait for a network response URL matching a pattern (`*` wildcard supported).
    #[qjs(rename = "waitForResponse")]
    pub async fn js_wait_for_response(
        &self,
        url_pattern: String,
        timeout_ms: Option<u64>,
    ) -> JsResult<String> {
        let timeout_ms = timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
        let entries = self.ensure_response_capture().await?;
        let baseline_len = entries.lock().await.len();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);

        loop {
            let requests = entries.lock().await.clone();
            if let Some(found) = requests
                .iter()
                .skip(baseline_len)
                .find(|req| url_matches_pattern(&req.url, &url_pattern))
            {
                return serde_json::to_string(found)
                    .map_err(|e| js_err(format!("waitForResponse serialization failed: {e}")));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(js_err(format!(
                    "TimeoutError: waiting for response pattern \"{url_pattern}\" failed: timeout {timeout_ms}ms exceeded"
                )));
            }
            tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
        }
    }

    /// List captured network requests as JSON.
    #[qjs(rename = "networkRequests")]
    pub async fn js_network_requests(&self) -> JsResult<String> {
        let entries = self.ensure_response_capture().await?;
        let requests = entries.lock().await.clone();
        serde_json::to_string(&requests)
            .map_err(|e| js_err(format!("networkRequests serialization failed: {e}")))
    }

    /// Clear captured network requests.
    #[qjs(rename = "clearNetworkRequests")]
    pub async fn js_clear_network_requests(&self) -> JsResult<()> {
        let entries = self.ensure_response_capture().await?;
        entries.lock().await.clear();
        Ok(())
    }

    /// Playwright-style alias for captured network responses.
    #[qjs(rename = "responsesReceived")]
    pub async fn js_responses_received(&self) -> JsResult<String> {
        self.js_network_requests().await
    }

    /// Configure JS dialog handling mode (`accept`, `dismiss`, or `none`).
    #[qjs(rename = "setDialogHandler")]
    pub async fn js_set_dialog_handler(
        &self,
        mode: String,
        prompt_text: Option<String>,
    ) -> JsResult<()> {
        let normalized_mode = mode.to_ascii_lowercase();
        if normalized_mode != "accept" && normalized_mode != "dismiss" && normalized_mode != "none"
        {
            return Err(js_err(format!(
                "setDialogHandler mode must be accept, dismiss, or none (got {mode})"
            )));
        }
        let mode_json =
            serde_json::to_string(&normalized_mode).unwrap_or_else(|_| "\"none\"".to_string());
        let prompt_json =
            serde_json::to_string(&prompt_text).unwrap_or_else(|_| "null".to_string());
        let script = format!(
            r#"(() => {{
                const mode = {mode_json};
                const promptText = {prompt_json};
                const state = window.__refreshmintDialogState || {{
                    events: [],
                    lastEvent: null,
                    originalAlert: window.alert,
                    originalConfirm: window.confirm,
                    originalPrompt: window.prompt,
                }};
                state.mode = mode;
                state.promptText = promptText;
                const pushEvent = (kind, message) => {{
                    const evt = {{ type: kind, message: String(message ?? ''), ts: Date.now() }};
                    state.lastEvent = evt;
                    state.events.push(evt);
                    if (state.events.length > 500) state.events.shift();
                }};
                window.alert = (message) => {{
                    pushEvent('alert', message);
                    return undefined;
                }};
                window.confirm = (message) => {{
                    pushEvent('confirm', message);
                    if (state.mode === 'dismiss') return false;
                    if (state.mode === 'none') return state.originalConfirm(message);
                    return true;
                }};
                window.prompt = (message, defaultValue) => {{
                    pushEvent('prompt', message);
                    if (state.mode === 'dismiss') return null;
                    if (state.mode === 'none') return state.originalPrompt(message, defaultValue);
                    if (typeof state.promptText === 'string') return state.promptText;
                    if (typeof defaultValue === 'string') return defaultValue;
                    return '';
                }};
                window.__refreshmintDialogState = state;
                return true;
            }})()"#
        );
        let inner = self.inner.lock().await;
        inner
            .page
            .evaluate(script)
            .await
            .map_err(|e| js_err(format!("setDialogHandler failed: {e}")))?;
        Ok(())
    }

    /// Return the most recent intercepted dialog event as JSON.
    #[qjs(rename = "lastDialog")]
    pub async fn js_last_dialog(&self) -> JsResult<String> {
        let inner = self.inner.lock().await;
        let result = inner
            .page
            .evaluate(
                r#"(() => {
                    const state = window.__refreshmintDialogState;
                    return state && state.lastEvent ? state.lastEvent : null;
                })()"#,
            )
            .await
            .map_err(|e| js_err(format!("lastDialog failed: {e}")))?;
        if let Some(value) = result.value() {
            serde_json::to_string(value)
                .map_err(|e| js_err(format!("lastDialog serialization failed: {e}")))
        } else {
            Ok("null".to_string())
        }
    }

    /// Configure popup handling mode (`ignore` or `same_tab`).
    #[qjs(rename = "setPopupHandler")]
    pub async fn js_set_popup_handler(&self, mode: String) -> JsResult<()> {
        let normalized_mode = mode.to_ascii_lowercase();
        if normalized_mode != "ignore" && normalized_mode != "same_tab" {
            return Err(js_err(format!(
                "setPopupHandler mode must be ignore or same_tab (got {mode})"
            )));
        }
        let mode_json =
            serde_json::to_string(&normalized_mode).unwrap_or_else(|_| "\"ignore\"".to_string());
        let script = format!(
            r#"(() => {{
                const mode = {mode_json};
                const state = window.__refreshmintPopupState || {{
                    events: [],
                    originalOpen: window.open,
                }};
                state.mode = mode;
                window.open = function(url, target, features) {{
                    const popupEvent = {{
                        url: String(url ?? ''),
                        target: String(target ?? ''),
                        ts: Date.now(),
                    }};
                    state.events.push(popupEvent);
                    if (state.events.length > 500) state.events.shift();
                    if (state.mode === 'same_tab' && popupEvent.url.length > 0) {{
                        window.location.href = popupEvent.url;
                        return null;
                    }}
                    if (typeof state.originalOpen === 'function') {{
                        return state.originalOpen.call(window, url, target, features);
                    }}
                    return null;
                }};
                window.__refreshmintPopupState = state;
                return true;
            }})()"#
        );
        let inner = self.inner.lock().await;
        inner
            .page
            .evaluate(script)
            .await
            .map_err(|e| js_err(format!("setPopupHandler failed: {e}")))?;
        Ok(())
    }

    /// Return captured popup events as JSON.
    #[qjs(rename = "popupEvents")]
    pub async fn js_popup_events(&self) -> JsResult<String> {
        let inner = self.inner.lock().await;
        let result = inner
            .page
            .evaluate(
                r#"(() => {
                    const state = window.__refreshmintPopupState;
                    return state && Array.isArray(state.events) ? state.events : [];
                })()"#,
            )
            .await
            .map_err(|e| js_err(format!("popupEvents failed: {e}")))?;
        if let Some(value) = result.value() {
            serde_json::to_string(value)
                .map_err(|e| js_err(format!("popupEvents serialization failed: {e}")))
        } else {
            Ok("[]".to_string())
        }
    }

    /// Deprecated legacy API. Use `browser.pages()` instead.
    #[qjs(rename = "tabs")]
    pub async fn js_tabs(&self) -> JsResult<String> {
        Err(js_err(
            "tabs() was removed. Use browser.pages() and work with Page handles directly."
                .to_string(),
        ))
    }

    /// Deprecated legacy API. Use `browser.pages()` and explicit Page handles.
    #[qjs(rename = "selectTab")]
    pub async fn js_select_tab(&self, index: i32) -> JsResult<String> {
        Err(js_err(format!(
            "selectTab({index}) was removed. Use browser.pages() and call methods on the selected Page handle."
        )))
    }

    /// Wait for a popup opened by this page and return it as a Page handle.
    #[qjs(rename = "waitForPopup")]
    pub async fn js_wait_for_popup(&self, timeout_ms: Option<u64>) -> JsResult<PageApi> {
        self.wait_for_popup_page(timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS))
            .await
    }

    /// Playwright-style event waiter.
    ///
    /// Currently supports only `popup`.
    #[qjs(rename = "waitForEvent")]
    pub async fn js_wait_for_event(
        &self,
        event: String,
        timeout_ms: Option<u64>,
    ) -> JsResult<PageApi> {
        let normalized = event.trim().to_ascii_lowercase();
        if normalized != "popup" {
            return Err(js_err(format!(
                "waitForEvent currently supports only \"popup\" (got {event})"
            )));
        }
        self.wait_for_popup_page(timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS))
            .await
    }

    /// Click an element matching the CSS selector.
    pub async fn click(&self, selector: String) -> JsResult<()> {
        let inner = self.inner.lock().await;
        if let Some(frame_id) = &inner.target_frame_id {
            // Frame context: evaluate JS click inside the frame's execution context.
            let context_id = wait_for_frame_execution_context(&inner.page, frame_id.clone())
                .await
                .map_err(|e| js_err(format!("click failed to get frame context: {e}")))?;
            let selector_json = serde_json::to_string(&selector).unwrap_or_default();
            let js = format!(
                r#"(() => {{
                    const el = document.querySelector({selector_json});
                    if (!el) throw new Error('click: element not found: ' + {selector_json});
                    if (!el.isConnected) throw new Error('click: element is detached');
                    el.scrollIntoView({{ block: 'center', inline: 'center', behavior: 'instant' }});
                    el.click();
                }})()"#
            );
            use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
            let eval = EvaluateParams::builder()
                .expression(js)
                .context_id(context_id)
                .await_promise(true)
                .return_by_value(true)
                .build()
                .map_err(|e| js_err(format!("click invalid params: {e}")))?;
            inner
                .page
                .evaluate_expression(eval)
                .await
                .map_err(|e| js_err(format!("click failed: {e}")))?;
        } else {
            // Main frame: use CDP element interaction for reliable pointer events.
            let element = inner
                .page
                .find_element(selector)
                .await
                .map_err(|e| js_err(format!("click find failed: {e}")))?;
            ensure_element_receives_pointer_events(&element)
                .await
                .map_err(|e| js_err(format!("click failed: {e}")))?;
            element
                .click()
                .await
                .map_err(|e| js_err(format!("click failed: {e}")))?;
        }
        Ok(())
    }

    /// Type text into an element, character by character.
    #[qjs(rename = "type")]
    pub async fn js_type(&self, selector: String, text: String) -> JsResult<()> {
        let inner = self.inner.lock().await;
        if let Some(frame_id) = &inner.target_frame_id {
            // Frame context: focus element via JS, then dispatch CDP key events
            // (Input.dispatchKeyEvent is global and targets the focused element).
            let context_id = wait_for_frame_execution_context(&inner.page, frame_id.clone())
                .await
                .map_err(|e| js_err(format!("type failed to get frame context: {e}")))?;
            let selector_json = serde_json::to_string(&selector).unwrap_or_default();
            let js = format!(
                r#"(() => {{
                    const el = document.querySelector({selector_json});
                    if (!el) throw new Error('type: element not found: ' + {selector_json});
                    el.focus();
                    el.click();
                }})()"#
            );
            use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
            let eval = EvaluateParams::builder()
                .expression(js)
                .context_id(context_id)
                .await_promise(true)
                .return_by_value(true)
                .build()
                .map_err(|e| js_err(format!("type invalid params: {e}")))?;
            inner
                .page
                .evaluate_expression(eval)
                .await
                .map_err(|e| js_err(format!("type failed: {e}")))?;
            inner
                .page
                .type_str(&text)
                .await
                .map_err(|e| js_err(format!("type failed: {e}")))?;
        } else {
            // Main frame: use CDP element interaction for reliable key events.
            let element = inner
                .page
                .find_element(selector)
                .await
                .map_err(|e| js_err(format!("type find failed: {e}")))?;
            ensure_element_receives_pointer_events(&element)
                .await
                .map_err(|e| js_err(format!("type click failed: {e}")))?;
            element
                .click()
                .await
                .map_err(|e| js_err(format!("type click failed: {e}")))?;
            element
                .type_str(&text)
                .await
                .map_err(|e| js_err(format!("type failed: {e}")))?;
        }
        Ok(())
    }

    /// Fill an input element's value.
    ///
    /// If `value` matches a manifest-declared secret name for the current
    /// top-level domain, the real secret is resolved from keychain and injected via CDP.
    /// The JS sandbox only ever sees the placeholder name.
    pub async fn fill(&self, selector: String, value: String) -> JsResult<()> {
        let actual_value = {
            let inner = self.inner.lock().await;
            resolve_secret_if_applicable(&inner, &value).await?
        };
        let selector_json = serde_json::to_string(&selector).unwrap_or_default();
        let value_json = serde_json::to_string(&actual_value).unwrap_or_default();
        let js = format!(
            r#"(() => {{
                const el = document.querySelector({selector_json});
                if (!el) throw new Error('fill: element not found: ' + {selector_json});
                el.focus();
                el.value = {value_json};
                el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                el.dispatchEvent(new Event('change', {{ bubbles: true }}));
            }})()"#,
        );
        self.evaluate_in_active_context(js)
            .await
            .map_err(|e| js_err(format!("fill failed: {e}")))?;
        Ok(())
    }

    /// Get an element's innerHTML.
    #[qjs(rename = "innerHTML")]
    pub async fn js_inner_html(&self, selector: String) -> JsResult<String> {
        let selector_json = serde_json::to_string(&selector).unwrap_or_else(|_| "\"\"".to_string());
        self.eval_string(
            format!(
                r#"(() => {{
                    const el = document.querySelector({selector_json});
                    if (!el) throw new Error('innerHTML: element not found: ' + {selector_json});
                    return el.innerHTML;
                }})()"#
            ),
            "innerHTML",
        )
        .await
    }

    /// Get an element's visible text.
    #[qjs(rename = "innerText")]
    pub async fn js_inner_text(&self, selector: String) -> JsResult<String> {
        let selector_json = serde_json::to_string(&selector).unwrap_or_else(|_| "\"\"".to_string());
        self.eval_string(
            format!(
                r#"(() => {{
                    const el = document.querySelector({selector_json});
                    if (!el) throw new Error('innerText: element not found: ' + {selector_json});
                    return el.innerText;
                }})()"#
            ),
            "innerText",
        )
        .await
    }

    /// Get an element's text content.
    #[qjs(rename = "textContent")]
    pub async fn js_text_content(&self, selector: String) -> JsResult<String> {
        let selector_json = serde_json::to_string(&selector).unwrap_or_else(|_| "\"\"".to_string());
        self.eval_string(
            format!(
                r#"(() => {{
                    const el = document.querySelector({selector_json});
                    if (!el) throw new Error('textContent: element not found: ' + {selector_json});
                    return el.textContent ?? '';
                }})()"#
            ),
            "textContent",
        )
        .await
    }

    /// Get an element attribute. Returns empty string if attribute is missing.
    #[qjs(rename = "getAttribute")]
    pub async fn js_get_attribute(&self, selector: String, name: String) -> JsResult<String> {
        let selector_json = serde_json::to_string(&selector).unwrap_or_else(|_| "\"\"".to_string());
        let name_json = serde_json::to_string(&name).unwrap_or_else(|_| "\"\"".to_string());
        self.eval_string(
            format!(
                r#"(() => {{
                    const el = document.querySelector({selector_json});
                    if (!el) throw new Error('getAttribute: element not found: ' + {selector_json});
                    return el.getAttribute({name_json}) ?? '';
                }})()"#
            ),
            "getAttribute",
        )
        .await
    }

    /// Get the current value of an input-like element.
    #[qjs(rename = "inputValue")]
    pub async fn js_input_value(&self, selector: String) -> JsResult<String> {
        let selector_json = serde_json::to_string(&selector).unwrap_or_else(|_| "\"\"".to_string());
        self.eval_string(
            format!(
                r#"(() => {{
                    const el = document.querySelector({selector_json});
                    if (!el) throw new Error('inputValue: element not found: ' + {selector_json});
                    if (!('value' in el)) throw new Error('inputValue: element has no value property: ' + {selector_json});
                    return String(el.value ?? '');
                }})()"#
            ),
            "inputValue",
        )
        .await
    }

    /// Return true if an element is visible.
    #[qjs(rename = "isVisible")]
    pub async fn js_is_visible(&self, selector: String) -> JsResult<bool> {
        let selector_json = serde_json::to_string(&selector).unwrap_or_else(|_| "\"\"".to_string());
        self.eval_bool(
            format!(
                r#"(() => {{
                    const el = document.querySelector({selector_json});
                    if (!el) return false;
                    const style = window.getComputedStyle(el);
                    if (!style) return false;
                    if (style.display === 'none' || style.visibility === 'hidden' || style.opacity === '0') return false;
                    const rect = el.getBoundingClientRect();
                    return rect.width > 0 && rect.height > 0;
                }})()"#
            ),
            "isVisible",
        )
        .await
    }

    /// Return true if an element is enabled.
    #[qjs(rename = "isEnabled")]
    pub async fn js_is_enabled(&self, selector: String) -> JsResult<bool> {
        let selector_json = serde_json::to_string(&selector).unwrap_or_else(|_| "\"\"".to_string());
        self.eval_bool(
            format!(
                r#"(() => {{
                    const el = document.querySelector({selector_json});
                    if (!el) return false;
                    return !el.disabled;
                }})()"#
            ),
            "isEnabled",
        )
        .await
    }

    /// Evaluate a JS expression inside a frame execution context.
    ///
    /// `frame_ref` may be a frame id, frame name, or frame URL.
    #[qjs(rename = "frameEvaluate")]
    pub async fn js_frame_evaluate(
        &self,
        frame_ref: String,
        expression: String,
    ) -> JsResult<String> {
        let inner = self.inner.lock().await;
        let frame_id = resolve_frame_id(&inner.page, &frame_ref)
            .await
            .map_err(|e| js_err(format!("frameEvaluate failed: {e}")))?;
        let context_id = wait_for_frame_execution_context(&inner.page, frame_id.clone())
            .await
            .map_err(|e| js_err(format!("frameEvaluate failed: {e}")))?;
        use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
        let eval = EvaluateParams::builder()
            .expression(expression)
            .context_id(context_id)
            .await_promise(true)
            .return_by_value(true)
            .build()
            .map_err(|e| js_err(format!("frameEvaluate invalid expression params: {e}")))?;
        let result = inner
            .page
            .evaluate_expression(eval)
            .await
            .map_err(|e| js_err(format!("frameEvaluate failed: {e}")))?;
        let mut text =
            stringify_evaluation_result(result.value(), result.object().description.as_deref());
        scrub_known_secrets(&inner.secret_store, &mut text);
        Ok(text)
    }

    /// Fill a value in a frame execution context.
    ///
    /// `frame_ref` may be a frame id, frame name, or frame URL.
    #[qjs(rename = "frameFill")]
    pub async fn js_frame_fill(
        &self,
        frame_ref: String,
        selector: String,
        value: String,
    ) -> JsResult<()> {
        let inner = self.inner.lock().await;
        let frame_id = resolve_frame_id(&inner.page, &frame_ref)
            .await
            .map_err(|e| js_err(format!("frameFill failed: {e}")))?;
        let actual_value = resolve_secret_if_applicable(&inner, &value).await?;
        let context_id = wait_for_frame_execution_context(&inner.page, frame_id)
            .await
            .map_err(|e| js_err(format!("frameFill failed: {e}")))?;
        let selector_json = serde_json::to_string(&selector).unwrap_or_else(|_| "\"\"".to_string());
        let value_json =
            serde_json::to_string(&actual_value).unwrap_or_else(|_| "\"\"".to_string());
        use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
        let script = format!(
            r#"(() => {{
                const el = document.querySelector({selector_json});
                if (!el) throw new Error('frameFill: element not found: ' + {selector_json});
                el.focus();
                el.value = {value_json};
                el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                el.dispatchEvent(new Event('change', {{ bubbles: true }}));
                return true;
            }})()"#
        );
        let eval = EvaluateParams::builder()
            .expression(script)
            .context_id(context_id)
            .await_promise(true)
            .return_by_value(true)
            .build()
            .map_err(|e| js_err(format!("frameFill invalid expression params: {e}")))?;
        inner
            .page
            .evaluate_expression(eval)
            .await
            .map_err(|e| js_err(format!("frameFill failed: {e}")))?;
        Ok(())
    }

    /// Accessibility-oriented snapshot of interactive page elements as JSON.
    ///
    /// Accepts optional options object:
    /// - `incremental: boolean` to return only changed nodes vs the previous snapshot in the same track
    /// - `track: string` to isolate snapshot history (default: `"default"`)
    pub async fn snapshot(&self, options: Opt<rquickjs::Value<'_>>) -> JsResult<String> {
        let options = parse_snapshot_options(options.0)?;
        let inner = self.inner.lock().await;
        let result = inner
            .page
            .evaluate(
                r#"(() => {
                    const nodes = [];
                    const interactiveTags = new Set(['a', 'button', 'input', 'select', 'textarea', 'summary', 'details', 'option']);
                    const implicitRole = (el) => {
                        const tag = (el.tagName || '').toLowerCase();
                        if (tag === 'a' && el.hasAttribute('href')) return 'link';
                        if (tag === 'button') return 'button';
                        if (tag === 'input') {
                            const type = (el.getAttribute('type') || 'text').toLowerCase();
                            if (type === 'checkbox') return 'checkbox';
                            if (type === 'radio') return 'radio';
                            if (type === 'submit' || type === 'button' || type === 'reset') return 'button';
                            return 'textbox';
                        }
                        if (tag === 'select') return 'combobox';
                        if (tag === 'textarea') return 'textbox';
                        if (tag === 'summary') return 'button';
                        return '';
                    };
                    const selectorHint = (el) => {
                        if (el.id) return '#' + el.id;
                        if (el.getAttribute('name')) return '[name="' + el.getAttribute('name') + '"]';
                        return (el.tagName || '').toLowerCase();
                    };
                    const domPath = (el) => {
                        const parts = [];
                        let node = el;
                        let depth = 0;
                        while (node && node.nodeType === Node.ELEMENT_NODE && depth < 10) {
                            const tag = (node.tagName || '').toLowerCase();
                            let part = tag;
                            if (node.id) {
                                part += '#' + node.id;
                                parts.unshift(part);
                                break;
                            }
                            let nth = 1;
                            let sib = node;
                            while ((sib = sib.previousElementSibling)) {
                                if ((sib.tagName || '').toLowerCase() === tag) nth++;
                            }
                            part += ':nth-of-type(' + nth + ')';
                            parts.unshift(part);
                            node = node.parentElement;
                            depth++;
                        }
                        return parts.join('>');
                    };
                    const isInteresting = (el) => {
                        const tag = (el.tagName || '').toLowerCase();
                        if (interactiveTags.has(tag)) return true;
                        if (el.hasAttribute('role')) return true;
                        if (el.hasAttribute('aria-label') || el.hasAttribute('aria-labelledby')) return true;
                        if (el.tabIndex >= 0) return true;
                        return false;
                    };
                    const resolveByReference = (el, attrName) => {
                        const ids = (el.getAttribute(attrName) || '')
                            .trim()
                            .split(/\s+/)
                            .filter(Boolean);
                        if (!ids.length) return '';
                        return ids
                            .map((id) => document.getElementById(id))
                            .filter(Boolean)
                            .map((node) => (node.innerText || node.textContent || '').trim())
                            .filter(Boolean)
                            .join(' ');
                    };
                    const computeLabel = (el) => {
                        const ariaLabel = (el.getAttribute('aria-label') || '').trim();
                        if (ariaLabel) return ariaLabel;
                        const labelledByText = resolveByReference(el, 'aria-labelledby');
                        if (labelledByText) return labelledByText;
                        if (typeof el.labels !== 'undefined' && el.labels && el.labels.length) {
                            const fromLabels = Array.from(el.labels)
                                .map((node) => (node.innerText || node.textContent || '').trim())
                                .filter(Boolean)
                                .join(' ');
                            if (fromLabels) return fromLabels;
                        }
                        const fallback = (el.getAttribute('placeholder') ||
                            el.getAttribute('name') ||
                            el.getAttribute('title') ||
                            el.getAttribute('alt') ||
                            el.innerText ||
                            el.textContent ||
                            el.value ||
                            '').trim();
                        return String(fallback).slice(0, 240);
                    };
                    const isVisible = (el) => {
                        const rect = el.getBoundingClientRect();
                        if (!(rect.width > 0 && rect.height > 0)) return false;
                        const style = window.getComputedStyle(el);
                        return style.visibility !== 'hidden' &&
                            style.display !== 'none' &&
                            style.opacity !== '0';
                    };

                    const elements = Array.from(document.querySelectorAll('*')).filter(isInteresting);
                    const refByElement = new Map();
                    for (const el of elements) refByElement.set(el, domPath(el));

                    for (const el of elements) {
                        const role = (el.getAttribute('role') || implicitRole(el) || (el.tagName || '').toLowerCase()).trim();
                        const label = computeLabel(el);
                        const value = typeof el.value === 'string' ? String(el.value) : '';
                        const text = String((el.innerText || el.textContent || '').trim()).slice(0, 240);
                        const ariaChecked = el.getAttribute('aria-checked');
                        let checked = null;
                        if (ariaChecked === 'mixed') checked = 'mixed';
                        else if (ariaChecked === 'true') checked = 'true';
                        else if (ariaChecked === 'false') checked = 'false';
                        else if (typeof el.checked === 'boolean') checked = el.checked ? 'true' : 'false';

                        let parentRef = null;
                        let parent = el.parentElement;
                        while (parent) {
                            if (refByElement.has(parent)) {
                                parentRef = refByElement.get(parent);
                                break;
                            }
                            parent = parent.parentElement;
                        }

                        const levelAttr = el.getAttribute('aria-level');
                        const parsedLevel = levelAttr ? Number.parseInt(levelAttr, 10) : Number.NaN;
                        nodes.push({
                            ref: refByElement.get(el) || '',
                            parentRef,
                            role,
                            label,
                            tag: (el.tagName || '').toLowerCase(),
                            text,
                            value,
                            visible: isVisible(el),
                            disabled: !!el.disabled || el.getAttribute('aria-disabled') === 'true',
                            expanded: el.hasAttribute('aria-expanded')
                                ? el.getAttribute('aria-expanded') === 'true'
                                : null,
                            selected: el.hasAttribute('aria-selected')
                                ? el.getAttribute('aria-selected') === 'true'
                                : null,
                            checked,
                            level: Number.isFinite(parsedLevel) ? parsedLevel : null,
                            ariaLabelledBy: (el.getAttribute('aria-labelledby') || '').trim() || null,
                            ariaDescribedBy: (el.getAttribute('aria-describedby') || '').trim() || null,
                            selectorHint: selectorHint(el),
                        });
                    }
                    return nodes;
                })()"#,
            )
            .await
            .map_err(|e| js_err(format!("snapshot failed: {e}")))?;
        drop(inner);

        let nodes = if let Some(value) = result.value() {
            serde_json::from_value::<Vec<SnapshotNode>>(value.clone())
                .map_err(|e| js_err(format!("snapshot parse failed: {e}")))?
        } else {
            Vec::new()
        };

        let mut tracks = self.snapshot_tracks.lock().await;
        let previous = tracks.get(&options.track).cloned().unwrap_or_default();
        tracks.insert(options.track.clone(), nodes.clone());
        drop(tracks);

        if options.incremental {
            let diff = build_snapshot_diff(&previous, &nodes, &options.track);
            serde_json::to_string_pretty(&diff)
                .map_err(|e| js_err(format!("snapshot serialization failed: {e}")))
        } else {
            serde_json::to_string_pretty(&nodes)
                .map_err(|e| js_err(format!("snapshot serialization failed: {e}")))
        }
    }

    /// Evaluate a JavaScript expression in the browser context.
    ///
    /// The return value is scrubbed: all known secret values are replaced
    /// with `[REDACTED]`.
    pub async fn evaluate(&self, expression: String) -> JsResult<String> {
        self.evaluate_in_active_context(expression).await
    }

    /// Take a screenshot and return the PNG bytes as a base64 string.
    pub async fn screenshot(&self) -> JsResult<String> {
        let inner = self.inner.lock().await;
        use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotParams;
        let screenshot = inner
            .page
            .execute(CaptureScreenshotParams::default())
            .await
            .map_err(|e| js_err(format!("screenshot failed: {e}")))?;
        Ok(screenshot.result.data.into())
    }

    /// Wait for the next download to complete and return its info.
    #[qjs(rename = "waitForDownload")]
    pub async fn js_wait_for_download(&self, timeout_ms: Option<u64>) -> JsResult<DownloadInfo> {
        let timeout_ms = timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
        let (page, download_dir) = {
            let inner = self.inner.lock().await;
            (inner.page.clone(), inner.download_dir.clone())
        };
        std::fs::create_dir_all(&download_dir)
            .map_err(|e| js_err(format!("waitForDownload mkdir failed: {e}")))?;
        let download_path = download_dir.to_string_lossy().to_string();

        // Set download behavior via CDP and explicitly request download events.
        use chromiumoxide::cdp::browser_protocol::browser::SetDownloadBehaviorParams;
        let behavior = SetDownloadBehaviorParams::builder()
            .behavior(
                chromiumoxide::cdp::browser_protocol::browser::SetDownloadBehaviorBehavior::AllowAndName,
            )
            .download_path(download_path.clone())
            .events_enabled(true)
            .build()
            .map_err(|e| js_err(format!("setDownloadBehavior params failed: {e}")))?;
        page.execute(behavior)
            .await
            .map_err(|e| js_err(format!("setDownloadBehavior failed: {e}")))?;

        let baseline = list_download_paths(&download_dir)
            .map_err(|e| js_err(format!("waitForDownload list failed: {e}")))?;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        let mut candidate_sizes = BTreeMap::new();

        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(js_err(format!(
                    "TimeoutError: waitForDownload timed out after {timeout_ms}ms"
                )));
            }

            let current = list_download_paths(&download_dir)
                .map_err(|e| js_err(format!("waitForDownload list failed: {e}")))?;

            for path in current {
                if baseline.contains(&path) || is_partial_download_file(&path) {
                    continue;
                }

                let meta = match std::fs::metadata(&path) {
                    Ok(meta) if meta.is_file() => meta,
                    Ok(_) => continue,
                    Err(_) => continue,
                };
                let size = meta.len();
                match candidate_sizes.get(&path) {
                    Some(previous) if *previous == size => {
                        let suggested_filename = path
                            .file_name()
                            .and_then(std::ffi::OsStr::to_str)
                            .unwrap_or("")
                            .to_string();
                        return Ok(DownloadInfo {
                            path: path.to_string_lossy().to_string(),
                            suggested_filename,
                        });
                    }
                    _ => {
                        candidate_sizes.insert(path.clone(), size);
                    }
                }
            }

            tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
        }
    }
}

#[rquickjs::methods]
impl BrowserApi {
    /// Return all currently open pages in this browser context.
    pub async fn pages(&self) -> JsResult<Vec<PageApi>> {
        eprintln!("[js_api] browser.pages enter");
        let page = PageApi::new(self.page_inner.clone());
        let tabs = page.fetch_open_tabs().await?;
        eprintln!("[js_api] browser.pages tabs={}", tabs.len());
        let mut out = Vec::with_capacity(tabs.len());
        for tab in tabs {
            out.push(build_page_api_from_template(&self.page_inner, tab.page).await);
        }
        eprintln!("[js_api] browser.pages return={}", out.len());
        Ok(out)
    }

    #[qjs(rename = "__debugPing")]
    pub async fn debug_ping(&self) -> JsResult<i32> {
        eprintln!("[js_api] browser.__debugPing");
        Ok(42)
    }

    #[qjs(rename = "__debugVec")]
    pub async fn debug_vec(&self) -> JsResult<Vec<i32>> {
        eprintln!("[js_api] browser.__debugVec");
        Ok(vec![1, 2, 3])
    }

    #[qjs(rename = "__debugPage")]
    pub async fn debug_page(&self) -> JsResult<PageApi> {
        eprintln!("[js_api] browser.__debugPage");
        Ok(PageApi::new(self.page_inner.clone()))
    }

    #[qjs(rename = "__debugPages")]
    pub async fn debug_pages(&self) -> JsResult<Vec<PageApi>> {
        eprintln!("[js_api] browser.__debugPages");
        Ok(vec![PageApi::new(self.page_inner.clone())])
    }

    #[qjs(rename = "__debugTabsCount")]
    pub async fn debug_tabs_count(&self) -> JsResult<i32> {
        eprintln!("[js_api] browser.__debugTabsCount enter");
        let page = PageApi::new(self.page_inner.clone());
        let tabs = page.fetch_open_tabs().await?;
        eprintln!("[js_api] browser.__debugTabsCount tabs={}", tabs.len());
        Ok(tabs.len() as i32)
    }

    /// Playwright-style event waiter for Browser.
    ///
    /// Currently supports only `page`.
    #[qjs(rename = "waitForEvent")]
    pub async fn js_wait_for_event(
        &self,
        event: String,
        timeout_ms: Option<u64>,
    ) -> JsResult<PageApi> {
        let normalized = event.trim().to_ascii_lowercase();
        if normalized != "page" {
            return Err(js_err(format!(
                "browser.waitForEvent currently supports only \"page\" (got {event})"
            )));
        }
        self.wait_for_page(timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS))
            .await
    }
}

impl PageApi {
    /// Evaluate `expression` in the active frame context (or the main frame if none is set).
    /// Secret values in the result are scrubbed.
    async fn evaluate_in_active_context(&self, expression: String) -> JsResult<String> {
        let inner = self.inner.lock().await;
        if let Some(frame_id) = &inner.target_frame_id {
            let context_id = wait_for_frame_execution_context(&inner.page, frame_id.clone())
                .await
                .map_err(|e| js_err(format!("failed to get frame context: {e}")))?;
            use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
            let eval = EvaluateParams::builder()
                .expression(expression)
                .context_id(context_id)
                .await_promise(true)
                .return_by_value(true)
                .build()
                .map_err(|e| js_err(format!("evaluate invalid params: {e}")))?;
            let result = inner
                .page
                .evaluate_expression(eval)
                .await
                .map_err(|e| js_err(format!("evaluate failed: {e}")))?;
            let mut text =
                stringify_evaluation_result(result.value(), result.object().description.as_deref());
            scrub_known_secrets(&inner.secret_store, &mut text);
            Ok(text)
        } else {
            let result = inner
                .page
                .evaluate(expression)
                .await
                .map_err(|e| js_err(format!("evaluate failed: {e}")))?;
            let text =
                stringify_evaluation_result(result.value(), result.object().description.as_deref());
            Ok(text)
        }
    }

    async fn fetch_open_tabs(&self) -> JsResult<Vec<OpenTab>> {
        eprintln!("[js_api] fetch_open_tabs enter");
        let (browser, current_page) = {
            let inner = self.inner.lock().await;
            (inner.browser.clone(), inner.page.clone())
        };

        let target_infos = match tokio::time::timeout(
            std::time::Duration::from_millis(TAB_QUERY_TIMEOUT_MS),
            async {
                let mut guard = browser.lock().await;
                guard.fetch_targets().await
            },
        )
        .await
        {
            Ok(Ok(infos)) => Some(infos),
            Ok(Err(err)) => {
                let err_text = err.to_string();
                if is_transport_disconnected_error(&err_text) {
                    eprintln!("[js_api] fetch_open_tabs disconnect on fetch_targets: {err_text}");
                    return Err(js_err(format_browser_error(
                        "browser.pages() fetch_targets failed",
                        &err_text,
                    )));
                }
                eprintln!(
                    "tab sync failed to fetch targets: {err}; falling back to current page handle"
                );
                return Ok(vec![OpenTab {
                    target_id: current_page.target_id().as_ref().to_string(),
                    opener_target_id: current_page
                        .opener_id()
                        .as_ref()
                        .map(|id| id.as_ref().to_string()),
                    page: current_page,
                }]);
            }
            Err(_) => {
                eprintln!(
                    "tab sync timed out fetching targets after {}ms; falling back to current page handle",
                    TAB_QUERY_TIMEOUT_MS
                );
                return Ok(vec![OpenTab {
                    target_id: current_page.target_id().as_ref().to_string(),
                    opener_target_id: current_page
                        .opener_id()
                        .as_ref()
                        .map(|id| id.as_ref().to_string()),
                    page: current_page,
                }]);
            }
        };
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let pages = match tokio::time::timeout(
            std::time::Duration::from_millis(TAB_QUERY_TIMEOUT_MS),
            async {
                let guard = browser.lock().await;
                guard.pages().await
            },
        )
        .await
        {
            Ok(Ok(pages)) => pages,
            Ok(Err(err)) => {
                let err_text = err.to_string();
                if is_transport_disconnected_error(&err_text) {
                    eprintln!("[js_api] fetch_open_tabs disconnect on pages listing: {err_text}");
                    return Err(js_err(format_browser_error(
                        "browser.pages() pages listing failed",
                        &err_text,
                    )));
                }
                eprintln!(
                    "tab sync failed to list pages: {err}; falling back to current page handle"
                );
                return Ok(vec![OpenTab {
                    target_id: current_page.target_id().as_ref().to_string(),
                    opener_target_id: current_page
                        .opener_id()
                        .as_ref()
                        .map(|id| id.as_ref().to_string()),
                    page: current_page,
                }]);
            }
            Err(_) => {
                eprintln!(
                    "tab sync timed out listing pages after {}ms; falling back to current page handle",
                    TAB_QUERY_TIMEOUT_MS
                );
                return Ok(vec![OpenTab {
                    target_id: current_page.target_id().as_ref().to_string(),
                    opener_target_id: current_page
                        .opener_id()
                        .as_ref()
                        .map(|id| id.as_ref().to_string()),
                    page: current_page,
                }]);
            }
        };
        let mut page_by_target = pages
            .into_iter()
            .map(|page| (page.target_id().as_ref().to_string(), page))
            .collect::<BTreeMap<_, _>>();

        let mut tabs = Vec::new();
        if let Some(target_infos) = target_infos {
            for info in target_infos {
                if info.r#type != "page" {
                    continue;
                }
                let target_id = info.target_id.as_ref().to_string();
                let Some(page) = page_by_target.remove(&target_id) else {
                    continue;
                };
                tabs.push(OpenTab {
                    page,
                    target_id,
                    opener_target_id: info.opener_id.as_ref().map(|id| id.as_ref().to_string()),
                });
            }
        }

        // Keep any pages that were not part of the current target snapshot.
        for (target_id, page) in page_by_target {
            tabs.push(OpenTab {
                opener_target_id: page.opener_id().as_ref().map(|id| id.as_ref().to_string()),
                page,
                target_id,
            });
        }

        if tabs.is_empty() {
            tabs.push(OpenTab {
                target_id: current_page.target_id().as_ref().to_string(),
                opener_target_id: current_page
                    .opener_id()
                    .as_ref()
                    .map(|id| id.as_ref().to_string()),
                page: current_page,
            });
        }

        Ok(tabs)
    }

    async fn wait_for_popup_page(&self, timeout_ms: u64) -> JsResult<PageApi> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        let opener_target = {
            let inner = self.inner.lock().await;
            inner.page.target_id().as_ref().to_string()
        };
        let baseline_tabs = self.fetch_open_tabs().await?;
        let baseline_ids = baseline_tabs
            .into_iter()
            .map(|tab| tab.target_id)
            .collect::<BTreeSet<_>>();

        loop {
            let tabs = self.fetch_open_tabs().await?;
            if let Some(popup_tab) = tabs.into_iter().find(|tab| {
                !baseline_ids.contains(&tab.target_id)
                    && tab.opener_target_id.as_deref() == Some(opener_target.as_str())
            }) {
                return Ok(build_page_api_from_template(&self.inner, popup_tab.page).await);
            }

            if tokio::time::Instant::now() >= deadline {
                return Err(js_err(format!(
                    "TimeoutError: waitForPopup timed out after {timeout_ms}ms (no popup opened by current page)"
                )));
            }
            tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
        }
    }

    async fn current_url(&self) -> JsResult<String> {
        let page = {
            let inner = self.inner.lock().await;
            inner.page.clone()
        };

        let (cdp_url, cdp_error) = match page.url().await {
            Ok(url) => (url.map(|value| value.to_string()), None),
            Err(error) => (None, Some(format!("{error}"))),
        };

        let runtime_url = match page
            .evaluate(
                "(() => { try { return String(window.location.href || ''); } catch (_) { return ''; } })()",
            )
            .await
        {
            Ok(result) => parse_runtime_location_href(result.value()),
            Err(_) => None,
        };

        if let Some(runtime_url) = runtime_url {
            return Ok(runtime_url);
        }
        if let Some(cdp_url) = cdp_url {
            return Ok(cdp_url);
        }
        if let Some(cdp_error) = cdp_error {
            return Err(js_err(format_browser_error("url() failed", &cdp_error)));
        }

        Ok(String::new())
    }

    async fn eval_string(&self, expression: String, _method_name: &str) -> JsResult<String> {
        self.evaluate_in_active_context(expression).await
    }

    async fn eval_bool(&self, expression: String, _method_name: &str) -> JsResult<bool> {
        let text = self.evaluate_in_active_context(expression).await?;
        Ok(text == "true")
    }

    async fn ready_state_is_complete(&self) -> JsResult<bool> {
        self.eval_bool(
            "(() => document.readyState === 'complete')()".to_string(),
            "waitForLoadState",
        )
        .await
    }

    async fn ready_state_is_interactive_or_complete(&self) -> JsResult<bool> {
        self.eval_bool(
            "(() => document.readyState === 'interactive' || document.readyState === 'complete')()"
                .to_string(),
            "waitForLoadState",
        )
        .await
    }

    async fn ensure_response_capture(&self) -> JsResult<Arc<Mutex<Vec<NetworkRequest>>>> {
        let mut guard = self.response_capture.lock().await;
        if let Some(state) = guard.as_ref() {
            if !state.task.is_finished() {
                return Ok(state.entries.clone());
            }
        }

        if let Some(previous) = guard.take() {
            previous.task.abort();
        }

        let page = {
            let inner = self.inner.lock().await;
            inner.page.clone()
        };

        use chromiumoxide::cdp::browser_protocol::network::{EnableParams, EventResponseReceived};
        page.execute(EnableParams::default())
            .await
            .map_err(|e| js_err(format!("failed to enable Network domain: {e}")))?;

        let mut events = page
            .event_listener::<EventResponseReceived>()
            .await
            .map_err(|e| js_err(format!("failed to attach response listener: {e}")))?;

        let entries = Arc::new(Mutex::new(Vec::new()));
        let entries_for_task = entries.clone();
        let task = tokio::spawn(async move {
            use futures::StreamExt;

            while let Some(ev) = events.next().await {
                let status = ev.response.status;
                let method = network_method_from_headers(ev.response.request_headers.as_ref());
                let ts = (*ev.timestamp.inner() * 1000.0) as i64;
                let item = NetworkRequest {
                    url: ev.response.url.clone(),
                    status,
                    ok: (200..400).contains(&status),
                    method,
                    ts,
                    error: None,
                };

                let mut guard = entries_for_task.lock().await;
                guard.push(item);
                if guard.len() > 5_000 {
                    let drop_count = guard.len() - 5_000;
                    guard.drain(0..drop_count);
                }
            }
        });

        *guard = Some(ResponseCaptureState {
            entries: entries.clone(),
            task,
        });
        Ok(entries)
    }
}

impl BrowserApi {
    async fn wait_for_page(&self, timeout_ms: u64) -> JsResult<PageApi> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        let watcher = PageApi::new(self.page_inner.clone());
        let baseline_tabs = watcher.fetch_open_tabs().await?;
        let baseline_ids = baseline_tabs
            .into_iter()
            .map(|tab| tab.target_id)
            .collect::<BTreeSet<_>>();

        loop {
            let tabs = watcher.fetch_open_tabs().await?;
            if let Some(new_tab) = tabs
                .into_iter()
                .find(|tab| !baseline_ids.contains(&tab.target_id))
            {
                return Ok(build_page_api_from_template(&self.page_inner, new_tab.page).await);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(js_err(format!(
                    "TimeoutError: browser.waitForEvent(\"page\") timed out after {timeout_ms}ms"
                )));
            }
            tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
        }
    }
}

async fn build_page_api_from_template(
    template: &Arc<Mutex<PageInner>>,
    page: chromiumoxide::Page,
) -> PageApi {
    let template = template.lock().await;
    let page_inner = PageInner {
        page,
        browser: template.browser.clone(),
        secret_store: template.secret_store.clone(),
        declared_secrets: template.declared_secrets.clone(),
        download_dir: template.download_dir.clone(),
        target_frame_id: None,
    };
    PageApi::new(Arc::new(Mutex::new(page_inner)))
}

pub(crate) fn stringify_evaluation_result(
    value: Option<&serde_json::Value>,
    description: Option<&str>,
) -> String {
    match value {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(other) => serde_json::to_string(other).unwrap_or_else(|_| other.to_string()),
        None => description.unwrap_or("undefined").to_string(),
    }
}

pub(crate) fn scrub_known_secrets(secret_store: &SecretStore, text: &mut String) {
    if let Ok(secrets) = secret_store.all_values() {
        for secret in &secrets {
            if !secret.is_empty() {
                *text = text.replace(secret, "[REDACTED]");
            }
        }
    }
}

fn list_download_paths(dir: &PathBuf) -> Result<BTreeSet<PathBuf>, std::io::Error> {
    let mut paths = BTreeSet::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        paths.insert(entry.path());
    }
    Ok(paths)
}

fn is_partial_download_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(std::ffi::OsStr::to_str)
        .map(|ext| ext.eq_ignore_ascii_case("crdownload"))
        .unwrap_or(false)
}

fn parse_runtime_location_href(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|href| !href.is_empty())
        .map(str::to_string)
}

fn url_matches_pattern(url: &str, pattern: &str) -> bool {
    if pattern == "*" || pattern == "**" {
        return true;
    }
    if !pattern.contains('*') {
        return url == pattern;
    }

    let parts: Vec<&str> = pattern.split('*').collect();
    let mut cursor = 0usize;
    let mut start_index = 0usize;

    if !pattern.starts_with('*') {
        let Some(first) = parts.first() else {
            return false;
        };
        if !url.starts_with(first) {
            return false;
        }
        cursor = first.len();
        start_index = 1;
    }

    let mut end_index = parts.len();
    if !pattern.ends_with('*') && end_index > 0 {
        end_index -= 1;
    }

    for part in &parts[start_index..end_index] {
        if part.is_empty() {
            continue;
        }
        let Some(found_at) = url[cursor..].find(part) else {
            return false;
        };
        cursor += found_at + part.len();
    }

    if !pattern.ends_with('*') {
        let Some(last) = parts.last() else {
            return false;
        };
        return url[cursor..].ends_with(last);
    }

    true
}

fn urls_differ_only_by_fragment(current_url: &str, target_url: &str) -> bool {
    if current_url == target_url {
        return false;
    }
    let (current_base, current_fragment) = split_url_fragment(current_url);
    let (target_base, target_fragment) = split_url_fragment(target_url);
    current_base == target_base && current_fragment != target_fragment
}

fn split_url_fragment(url: &str) -> (&str, Option<&str>) {
    match url.split_once('#') {
        Some((base, fragment)) => (base, Some(fragment)),
        None => (url, None),
    }
}

fn network_method_from_headers(
    headers: Option<&chromiumoxide::cdp::browser_protocol::network::Headers>,
) -> String {
    let Some(headers) = headers else {
        return "GET".to_string();
    };
    let Some(map) = headers.inner().as_object() else {
        return "GET".to_string();
    };

    for key in [":method", "method", "Method"] {
        if let Some(value) = map.get(key).and_then(serde_json::Value::as_str) {
            let method = value.trim();
            if !method.is_empty() {
                return method.to_string();
            }
        }
    }

    "GET".to_string()
}

async fn resolve_frame_id(
    page: &chromiumoxide::Page,
    frame_ref: &str,
) -> Result<chromiumoxide::cdp::browser_protocol::page::FrameId, String> {
    let trimmed = frame_ref.trim();

    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("main") {
        let main = page
            .mainframe()
            .await
            .map_err(|e| format!("failed to resolve main frame: {e}"))?;
        return main.ok_or_else(|| "main frame not available".to_string());
    }

    let frames = page
        .frames()
        .await
        .map_err(|e| format!("failed to list frames: {e}"))?;

    if let Some(found) = frames.iter().find(|frame_id| frame_id.as_ref() == trimmed) {
        return Ok(found.clone());
    }

    for frame_id in &frames {
        let name = page
            .frame_name(frame_id.clone())
            .await
            .map_err(|e| format!("failed to query frame name: {e}"))?;
        if name.as_deref() == Some(trimmed) {
            return Ok(frame_id.clone());
        }
    }

    for frame_id in &frames {
        let url = page
            .frame_url(frame_id.clone())
            .await
            .map_err(|e| format!("failed to query frame URL: {e}"))?
            .unwrap_or_default();
        if url == trimmed || url.contains(trimmed) {
            return Ok(frame_id.clone());
        }
    }

    let mut known_frames = Vec::new();
    for frame_id in &frames {
        let name = page
            .frame_name(frame_id.clone())
            .await
            .unwrap_or(None)
            .unwrap_or_default();
        let url = page
            .frame_url(frame_id.clone())
            .await
            .unwrap_or(None)
            .unwrap_or_default();
        known_frames.push(format!(
            "id={} name={} url={}",
            frame_id.as_ref(),
            name,
            url
        ));
    }

    Err(format!(
        "frame not found for reference '{trimmed}'. Available frames: {}",
        known_frames.join(" | ")
    ))
}

pub(crate) async fn wait_for_frame_execution_context(
    page: &chromiumoxide::Page,
    frame_id: chromiumoxide::cdp::browser_protocol::page::FrameId,
) -> Result<chromiumoxide::cdp::js_protocol::runtime::ExecutionContextId, String> {
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_millis(DEFAULT_TIMEOUT_MS);

    loop {
        let context = page
            .frame_execution_context(frame_id.clone())
            .await
            .map_err(|e| format!("failed to query frame execution context: {e}"))?;
        if let Some(context_id) = context {
            return Ok(context_id);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting for frame execution context (frame id {})",
                frame_id.as_ref()
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
}

async fn ensure_element_receives_pointer_events(
    element: &chromiumoxide::Element,
) -> Result<(), String> {
    let check = element
        .call_js_fn(
            r#"function() {
                if (!this.isConnected) return 'Node is detached from document';
                if (this.nodeType !== Node.ELEMENT_NODE) return 'Node is not of type HTMLElement';
                this.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
                const rect = this.getBoundingClientRect();
                if (rect.width <= 0 || rect.height <= 0) return 'Element is not visible';
                const x = rect.left + rect.width / 2;
                const y = rect.top + rect.height / 2;
                if (x < 0 || y < 0 || x > window.innerWidth || y > window.innerHeight) {
                    return 'Element is outside of the viewport';
                }
                const hit = document.elementFromPoint(x, y);
                if (!hit) return 'Element is outside of the viewport';
                const containsComposed = (root, node) => {
                    let current = node;
                    while (current) {
                        if (current === root) return true;
                        current = current.parentNode || (current instanceof ShadowRoot ? current.host : null);
                    }
                    return false;
                };
                if (containsComposed(this, hit)) return '';
                const describe = el => {
                    if (!(el instanceof Element)) return 'Another node';
                    let out = el.tagName.toLowerCase();
                    if (el.id) out += '#' + el.id;
                    if (el.classList && el.classList.length) {
                        out += '.' + Array.from(el.classList).slice(0, 3).join('.');
                    }
                    return out;
                };
                return describe(hit) + ' intercepts pointer events';
            }"#,
            false,
        )
        .await
        .map_err(|e| format!("pointer actionability check failed: {e}"))?;

    let message = check
        .result
        .value
        .as_ref()
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if message.is_empty() {
        Ok(())
    } else {
        Err(message.to_string())
    }
}

/// Resolve a secret value if `value` is a known secret name.
///
/// A secret name can only be used when it is declared in the extension
/// manifest for the current top-level navigation domain.
pub(crate) async fn resolve_secret_if_applicable(
    inner: &PageInner,
    value: &str,
) -> JsResult<String> {
    let all_known = inner
        .secret_store
        .list()
        .map_err(|e| js_err(format!("secret lookup failed: {e}")))?;
    let referenced_name = value.trim();
    if referenced_name.is_empty() {
        return Ok(value.to_string());
    }

    let declared_domains = declared_domains_for_secret(&inner.declared_secrets, referenced_name);
    let configured_in_store = all_known.iter().any(|(_, name)| name == referenced_name);
    if declared_domains.is_empty() && !configured_in_store {
        return Ok(value.to_string());
    }

    let current_url = inner.page.url().await.ok().flatten().unwrap_or_default();
    let top_level_domain = normalize_domain_like_input(&current_url.to_string());
    if top_level_domain.is_empty() {
        return Err(js_err(format!(
            "Secret '{referenced_name}' referenced before top-level navigation; call page.goto(...) first"
        )));
    }

    if !declared_domains.contains(&top_level_domain) {
        if declared_domains.is_empty() {
            return Err(js_err(format!(
                "Secret '{referenced_name}' is configured in keychain but not declared in manifest for domain '{top_level_domain}'"
            )));
        }
        return Err(js_err(format!(
            "Secret '{referenced_name}' was declared for domain(s) {} but current top-level domain is '{top_level_domain}'",
            declared_domains.join(", ")
        )));
    }

    for (domain, name) in &all_known {
        if name == referenced_name && domain.eq_ignore_ascii_case(&top_level_domain) {
            return inner.secret_store.get(domain, name).map_err(|e| {
                js_err(format!(
                    "failed to read secret '{name}' for domain '{domain}': {e}"
                ))
            });
        }
    }

    Err(js_err(format!(
        "Secret '{referenced_name}' was declared for '{top_level_domain}' but is not stored for that domain"
    )))
}

fn declared_domains_for_secret(declared: &SecretDeclarations, secret_name: &str) -> Vec<String> {
    let mut domains = declared
        .iter()
        .filter_map(|(domain, names)| {
            if names.contains(secret_name) {
                Some(domain.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    domains.sort();
    domains
}

fn normalize_domain_like_input(input: &str) -> String {
    extract_domain(input.trim()).to_ascii_lowercase()
}

fn extract_domain(url: &str) -> String {
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    without_scheme
        .split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_string()
}

/// JS-visible download info object.
#[rquickjs::class(rename = "Download")]
#[derive(Trace, Clone)]
pub struct DownloadInfo {
    #[qjs(skip_trace)]
    path: String,
    #[qjs(skip_trace)]
    suggested_filename: String,
}

// Safety: DownloadInfo only contains String which is 'static.
#[allow(unsafe_code)]
unsafe impl<'js> JsLifetime<'js> for DownloadInfo {
    type Changed<'to> = DownloadInfo;
}

#[rquickjs::methods]
impl DownloadInfo {
    #[qjs(get)]
    pub fn path(&self) -> String {
        self.path.clone()
    }

    #[qjs(get, rename = "suggestedFilename")]
    pub fn suggested_filename(&self) -> String {
        self.suggested_filename.clone()
    }
}

/// Metadata about the current scrape session, set by the driver via `setSessionMetadata`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SessionMetadata {
    #[serde(rename = "dateRangeStart", skip_serializing_if = "Option::is_none")]
    pub date_range_start: Option<String>,
    #[serde(rename = "dateRangeEnd", skip_serializing_if = "Option::is_none")]
    pub date_range_end: Option<String>,
}

/// A staged resource from `saveResource`, pending finalization.
#[derive(Debug, Clone)]
pub struct StagedResource {
    pub filename: String,
    pub staging_path: PathBuf,
    pub coverage_end_date: Option<String>,
    pub original_url: Option<String>,
    pub mime_type: Option<String>,
    pub label: Option<String>,
    pub metadata: std::collections::BTreeMap<String, serde_json::Value>,
}

/// Shared state backing the `refreshmint` JS namespace.
pub struct RefreshmintInner {
    pub output_dir: PathBuf,
    pub prompt_overrides: PromptOverrides,
    pub prompt_requires_override: bool,
    pub debug_output_sink: Option<tokio::sync::mpsc::UnboundedSender<DebugOutputEvent>>,
    pub session_metadata: SessionMetadata,
    pub staged_resources: Vec<StagedResource>,
    pub scrape_session_id: String,
    pub extension_name: String,
    pub account_name: String,
    pub login_name: String,
    pub ledger_dir: PathBuf,
}

/// JS-visible `refreshmint` namespace object.
#[rquickjs::class(rename = "Refreshmint")]
#[derive(Trace)]
pub struct RefreshmintApi {
    #[qjs(skip_trace)]
    inner: Arc<Mutex<RefreshmintInner>>,
}

// Safety: RefreshmintApi only contains Arc<Mutex<...>> which is 'static.
#[allow(unsafe_code)]
unsafe impl<'js> JsLifetime<'js> for RefreshmintApi {
    type Changed<'to> = RefreshmintApi;
}

impl RefreshmintApi {
    pub fn new(inner: Arc<Mutex<RefreshmintInner>>) -> Self {
        Self { inner }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountDocumentSummary {
    filename: String,
    metadata: std::collections::BTreeMap<String, serde_json::Value>,
}

fn missing_prompt_override_error(message: &str) -> String {
    format!(
        "missing prompt value for refreshmint.prompt(\"{message}\"); supply --prompt \"{message}=VALUE\""
    )
}

fn parse_document_filter(
    filter: Option<rquickjs::Value<'_>>,
) -> std::collections::BTreeMap<String, serde_json::Value> {
    let mut metadata = BTreeMap::new();

    if let Some(val) = filter {
        if let Some(s) = val.as_string() {
            if let Ok(label) = s.to_string() {
                metadata.insert("label".to_string(), serde_json::Value::String(label));
            }
        } else if let Some(obj) = val.as_object() {
            for (key, v) in obj.props::<String, rquickjs::Value>().flatten() {
                let json_val = if v.is_null() || v.is_undefined() {
                    serde_json::Value::Null
                } else if let Some(b) = v.as_bool() {
                    serde_json::Value::Bool(b)
                } else if let Some(s) = v.as_string() {
                    serde_json::Value::String(s.to_string().unwrap_or_default())
                } else if let Some(i) = v.as_int() {
                    serde_json::Value::Number(i.into())
                } else if let Some(f) = v.as_float() {
                    if let Some(n) = serde_json::Number::from_f64(f) {
                        serde_json::Value::Number(n)
                    } else {
                        continue;
                    }
                } else {
                    continue;
                };
                metadata.insert(key, json_val);
            }
        }
    }
    metadata
}

fn matches_filter(
    info: &crate::scrape::DocumentInfo,
    filter: &std::collections::BTreeMap<String, serde_json::Value>,
) -> bool {
    for (key, expected_val) in filter {
        let actual_val = match key.as_str() {
            "mimeType" => Some(serde_json::Value::String(info.mime_type.clone())),
            "coverageEndDate" => Some(serde_json::Value::String(info.coverage_end_date.clone())),
            "label" => Some(serde_json::Value::String(info.label.clone())),
            "originalUrl" => info
                .original_url
                .as_ref()
                .map(|s| serde_json::Value::String(s.clone())),
            "extensionName" => Some(serde_json::Value::String(info.extension_name.clone())),
            "scrapedAt" => Some(serde_json::Value::String(info.scraped_at.clone())),
            _ => info.metadata.get(key).cloned(),
        };

        if let Some(actual) = actual_val {
            if actual != *expected_val {
                return false;
            }
        } else if !expected_val.is_null() {
            return false;
        }
    }
    true
}

struct SaveResourceOptions {
    coverage_end_date: Option<String>,
    original_url: Option<String>,
    mime_type: Option<String>,
    label: Option<String>,
    metadata: BTreeMap<String, serde_json::Value>,
}

fn parse_save_resource_options(options: Option<rquickjs::Value<'_>>) -> SaveResourceOptions {
    let mut result = SaveResourceOptions {
        coverage_end_date: None,
        original_url: None,
        mime_type: None,
        label: None,
        metadata: BTreeMap::new(),
    };
    if let Some(opts) = options {
        if let Some(obj) = opts.as_object() {
            for (key, v) in obj.props::<String, rquickjs::Value>().flatten() {
                match key.as_str() {
                    "coverageEndDate" => {
                        result.coverage_end_date = v.as_string().and_then(|s| s.to_string().ok());
                    }
                    "originalUrl" => {
                        result.original_url = v.as_string().and_then(|s| s.to_string().ok());
                    }
                    "mimeType" => {
                        result.mime_type = v.as_string().and_then(|s| s.to_string().ok());
                    }
                    "label" => {
                        result.label = v.as_string().and_then(|s| s.to_string().ok());
                    }
                    _ => {
                        let json_val = if v.is_null() || v.is_undefined() {
                            serde_json::Value::Null
                        } else if let Some(b) = v.as_bool() {
                            serde_json::Value::Bool(b)
                        } else if let Some(s) = v.as_string() {
                            serde_json::Value::String(s.to_string().unwrap_or_default())
                        } else if let Some(i) = v.as_int() {
                            serde_json::Value::Number(i.into())
                        } else if let Some(f) = v.as_float() {
                            if let Some(n) = serde_json::Number::from_f64(f) {
                                serde_json::Value::Number(n)
                            } else {
                                continue;
                            }
                        } else {
                            continue;
                        };
                        result.metadata.insert(key, json_val);
                    }
                }
            }
        }
    }
    result
}

fn parse_snapshot_options(options: Option<rquickjs::Value<'_>>) -> JsResult<SnapshotOptions> {
    let mut result = SnapshotOptions::default();
    if let Some(opts) = options {
        let Some(obj) = opts.as_object() else {
            return Err(js_err(
                "snapshot options must be an object when provided".to_string(),
            ));
        };
        if let Ok(val) = obj.get::<_, Option<bool>>("incremental") {
            result.incremental = val.unwrap_or(false);
        }
        if let Ok(Some(track)) = obj.get::<_, Option<String>>("track") {
            let trimmed = track.trim();
            if !trimmed.is_empty() {
                result.track = trimmed.to_string();
            }
        }
    }
    Ok(result)
}

fn snapshot_nodes_by_ref(nodes: &[SnapshotNode]) -> BTreeMap<String, SnapshotNode> {
    let mut map = BTreeMap::new();
    for (index, node) in nodes.iter().enumerate() {
        let key = if node.r#ref.trim().is_empty() {
            format!("index:{index}")
        } else {
            node.r#ref.clone()
        };
        map.insert(key, node.clone());
    }
    map
}

fn build_snapshot_diff(
    previous: &[SnapshotNode],
    current: &[SnapshotNode],
    track: &str,
) -> SnapshotDiff {
    let previous_by_ref = snapshot_nodes_by_ref(previous);
    let current_by_ref = snapshot_nodes_by_ref(current);

    let mut changed = Vec::new();
    let mut unchanged_count = 0usize;
    for (ref_id, node) in &current_by_ref {
        match previous_by_ref.get(ref_id) {
            None => changed.push(SnapshotDiffEntry {
                change: "added".to_string(),
                node: node.clone(),
            }),
            Some(previous_node) if previous_node != node => changed.push(SnapshotDiffEntry {
                change: "updated".to_string(),
                node: node.clone(),
            }),
            Some(_) => unchanged_count += 1,
        }
    }

    let removed_refs = previous_by_ref
        .keys()
        .filter(|ref_id| !current_by_ref.contains_key(*ref_id))
        .cloned()
        .collect::<Vec<_>>();

    SnapshotDiff {
        mode: "incremental".to_string(),
        track: track.to_string(),
        base_node_count: previous.len(),
        node_count: current.len(),
        changed_count: changed.len(),
        removed_count: removed_refs.len(),
        unchanged_count,
        changed,
        removed_refs,
    }
}

fn unique_output_path(output_dir: &Path, filename: &str) -> PathBuf {
    let candidate = output_dir.join(filename);
    if !candidate.exists() {
        return candidate;
    }

    let original = Path::new(filename);
    let stem = original
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("resource");
    let ext = original.extension().and_then(|s| s.to_str()).unwrap_or("");
    let parent = original.parent().unwrap_or_else(|| Path::new(""));
    let suffix = if ext.is_empty() {
        String::new()
    } else {
        format!(".{ext}")
    };

    for i in 2..1000 {
        let candidate_name = format!("{stem}-{i}{suffix}");
        let rel = if parent.as_os_str().is_empty() {
            PathBuf::from(&candidate_name)
        } else {
            parent.join(&candidate_name)
        };
        let candidate = output_dir.join(&rel);
        if !candidate.exists() {
            return candidate;
        }
    }

    let fallback_name = format!("{stem}-{}{}", std::process::id(), suffix);
    if parent.as_os_str().is_empty() {
        output_dir.join(fallback_name)
    } else {
        output_dir.join(parent).join(fallback_name)
    }
}

#[rquickjs::methods]
impl RefreshmintApi {
    /// List existing account documents as JSON for "since last scrape" logic.
    ///
    /// If `filter` is a string, it is treated as the account label.
    /// If `filter` is an object, it is used to match metadata fields.
    #[qjs(rename = "listAccountDocuments")]
    pub async fn js_list_account_documents(
        &self,
        filter_val: Opt<rquickjs::Value<'_>>,
    ) -> JsResult<String> {
        let (ledger_dir, login_name) = {
            let inner = self.inner.lock().await;
            (inner.ledger_dir.clone(), inner.login_name.clone())
        };

        let filter = parse_document_filter(filter_val.0);
        let label_filter = filter
            .get("label")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut docs = Vec::new();
        let target_labels = if let Some(l) = label_filter {
            vec![l]
        } else {
            // List all accounts in logins/<login>/accounts/
            let accounts_dir = ledger_dir.join("logins").join(&login_name).join("accounts");
            let mut labels = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&accounts_dir) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        labels.push(entry.file_name().to_string_lossy().to_string());
                    }
                }
            }
            if labels.is_empty() {
                // Fallback to _default if no accounts dir (backward compat)
                vec!["_default".to_string()]
            } else {
                labels
            }
        };

        for acct_label in target_labels {
            let documents_dir = crate::login_config::login_account_documents_dir(
                &ledger_dir,
                &login_name,
                &acct_label,
            );

            if documents_dir.exists() {
                for entry in std::fs::read_dir(&documents_dir)
                    .map_err(|e| js_err(format!("listAccountDocuments failed: {e}")))?
                {
                    let entry =
                        entry.map_err(|e| js_err(format!("listAccountDocuments failed: {e}")))?;
                    let file_name = entry.file_name().to_string_lossy().to_string();
                    if file_name.ends_with("-info.json") || !entry.path().is_file() {
                        continue;
                    }

                    let sidecar_path = documents_dir.join(format!("{file_name}-info.json"));
                    let info = if sidecar_path.exists() {
                        std::fs::read_to_string(&sidecar_path)
                            .ok()
                            .and_then(|content| {
                                serde_json::from_str::<crate::scrape::DocumentInfo>(&content).ok()
                            })
                    } else {
                        None
                    };

                    if let Some(info) = info {
                        if matches_filter(&info, &filter) {
                            let mut metadata = info.metadata;
                            metadata.insert(
                                "label".to_string(),
                                serde_json::Value::String(acct_label.clone()),
                            );
                            metadata.insert(
                                "mimeType".to_string(),
                                serde_json::Value::String(info.mime_type),
                            );
                            metadata.insert(
                                "coverageEndDate".to_string(),
                                serde_json::Value::String(info.coverage_end_date),
                            );
                            metadata.insert(
                                "extensionName".to_string(),
                                serde_json::Value::String(info.extension_name),
                            );
                            metadata.insert(
                                "scrapedAt".to_string(),
                                serde_json::Value::String(info.scraped_at),
                            );
                            if let Some(url) = info.original_url {
                                metadata.insert(
                                    "originalUrl".to_string(),
                                    serde_json::Value::String(url),
                                );
                            }
                            docs.push(AccountDocumentSummary {
                                filename: file_name,
                                metadata,
                            });
                        }
                    } else if filter.is_empty() {
                        // Include docs without sidecars only if no filter is requested
                        let mut metadata = BTreeMap::new();
                        metadata.insert(
                            "label".to_string(),
                            serde_json::Value::String(acct_label.clone()),
                        );
                        docs.push(AccountDocumentSummary {
                            filename: file_name,
                            metadata,
                        });
                    }
                }
            }
        }
        docs.sort_by(|a, b| {
            let a_date = a.metadata.get("coverageEndDate").and_then(|v| v.as_str());
            let b_date = b.metadata.get("coverageEndDate").and_then(|v| v.as_str());
            b_date.cmp(&a_date)
        });
        serde_json::to_string(&docs)
            .map_err(|e| js_err(format!("listAccountDocuments serialization failed: {e}")))
    }

    /// Save binary data to a file in the extension output directory.
    ///
    /// Accepts an optional third argument: an options object with `coverageEndDate`.
    /// Files are staged during scraping and moved to their final location
    /// after extraction determines the coverage date.
    #[qjs(rename = "saveResource")]
    pub async fn js_save_resource(
        &self,
        filename: String,
        data: Vec<u8>,
        options: Opt<rquickjs::Value<'_>>,
    ) -> JsResult<()> {
        let mut inner = self.inner.lock().await;

        // Parse optional fields from options object
        let SaveResourceOptions {
            coverage_end_date,
            original_url,
            mime_type,
            label,
            metadata,
        } = parse_save_resource_options(options.0);

        // Always save to the legacy output dir for backward compatibility
        let path = unique_output_path(&inner.output_dir, &filename);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| js_err(format!("saveResource mkdir failed: {e}")))?;
        }
        std::fs::write(&path, &data)
            .map_err(|e| js_err(format!("saveResource write failed: {e}")))?;

        // Also stage the resource for the new evidence pipeline
        inner.staged_resources.push(StagedResource {
            filename: filename.clone(),
            staging_path: path,
            coverage_end_date,
            original_url,
            mime_type,
            label,
            metadata,
        });

        Ok(())
    }

    /// Save a completed local download file into staged resources.
    ///
    /// Useful with `page.waitForDownload(...)` where browser downloaded bytes
    /// are available on disk but not in JS memory.
    #[qjs(rename = "saveDownloadedResource")]
    pub async fn js_save_downloaded_resource(
        &self,
        download_path: String,
        filename: Option<String>,
        options: Opt<rquickjs::Value<'_>>,
    ) -> JsResult<()> {
        let input_path = PathBuf::from(download_path.clone());
        let data = std::fs::read(&input_path).map_err(|e| {
            js_err(format!(
                "saveDownloadedResource read failed ({}): {e}",
                input_path.display()
            ))
        })?;

        let final_name = filename.unwrap_or_else(|| {
            input_path
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or("download.bin")
                .to_string()
        });
        self.js_save_resource(final_name, data, options).await
    }

    /// Set session-level metadata (dateRangeStart, dateRangeEnd).
    #[qjs(rename = "setSessionMetadata")]
    pub async fn js_set_session_metadata(&self, metadata: rquickjs::Value<'_>) -> JsResult<()> {
        let mut inner = self.inner.lock().await;
        if let Some(obj) = metadata.as_object() {
            if let Ok(val) = obj.get::<_, Option<String>>("dateRangeStart") {
                inner.session_metadata.date_range_start = val;
            }
            if let Ok(val) = obj.get::<_, Option<String>>("dateRangeEnd") {
                inner.session_metadata.date_range_end = val;
            }
        }
        Ok(())
    }

    /// Report a key-value pair to stdout.
    #[qjs(rename = "reportValue")]
    pub fn js_report_value(&self, key: String, value: String) -> JsResult<()> {
        let message = format!("{key}: {value}");
        if !self.emit_debug_output(DebugOutputStream::Stdout, message.clone()) {
            println!("{message}");
        }
        Ok(())
    }

    /// Log a message to stderr.
    pub fn log(&self, message: String) -> JsResult<()> {
        if !self.emit_debug_output(DebugOutputStream::Stderr, message.clone()) {
            eprintln!("{message}");
        }
        Ok(())
    }

    /// Prompt the user: use CLI-provided override when available.
    pub fn prompt(&self, message: String) -> JsResult<String> {
        let (override_value, require_override) = {
            let inner = self
                .inner
                .try_lock()
                .map_err(|_| js_err("prompt unavailable: prompt state is busy".to_string()))?;
            (
                inner.prompt_overrides.get(&message).cloned().or_else(|| {
                    let trimmed = message.trim();
                    if trimmed == message {
                        None
                    } else {
                        inner.prompt_overrides.get(trimmed).cloned()
                    }
                }),
                inner.prompt_requires_override,
            )
        };

        if let Some(value) = override_value {
            return Ok(value);
        }

        if require_override {
            return Err(js_err(missing_prompt_override_error(&message)));
        }

        eprint!("{message} ");
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .map_err(|e| js_err(format!("prompt read failed: {e}")))?;
        Ok(line.trim_end().to_string())
    }
}

impl RefreshmintApi {
    fn emit_debug_output(&self, stream: DebugOutputStream, line: String) -> bool {
        let sender = match self.inner.try_lock() {
            Ok(inner) => inner.debug_output_sink.clone(),
            Err(_) => None,
        };

        if let Some(sender) = sender {
            return sender.send(DebugOutputEvent { stream, line }).is_ok();
        }

        false
    }
}

/// Register the `page`, `browser`, and `refreshmint` globals on a QuickJS context.
pub fn register_globals(
    ctx: &Ctx<'_>,
    page_inner: Arc<Mutex<PageInner>>,
    refreshmint_inner: Arc<Mutex<RefreshmintInner>>,
) -> JsResult<()> {
    let globals = ctx.globals();

    let page = PageApi::new(page_inner.clone());
    globals.set("page", page)?;

    let browser = BrowserApi::new(page_inner);
    globals.set("browser", browser)?;

    let rm = RefreshmintApi::new(refreshmint_inner);
    globals.set("refreshmint", rm)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_domain_https() {
        assert_eq!(extract_domain("https://example.com/path"), "example.com");
    }

    #[test]
    fn extract_domain_http() {
        assert_eq!(extract_domain("http://example.com/path"), "example.com");
    }

    #[test]
    fn extract_domain_with_port() {
        assert_eq!(
            extract_domain("https://example.com:8080/path"),
            "example.com"
        );
    }

    #[test]
    fn extract_domain_no_scheme() {
        assert_eq!(extract_domain("example.com/path"), "example.com");
    }

    #[test]
    fn extract_domain_empty() {
        assert_eq!(extract_domain(""), "");
    }

    #[test]
    fn extract_domain_just_scheme() {
        assert_eq!(extract_domain("https://"), "");
    }

    #[test]
    fn extract_domain_subdomain() {
        assert_eq!(
            extract_domain("https://www.bank.example.com/login"),
            "www.bank.example.com"
        );
    }

    #[test]
    fn stringify_evaluation_result_string_value() {
        let value = serde_json::json!("hello");
        assert_eq!(
            stringify_evaluation_result(Some(&value), Some("ignored")),
            "hello"
        );
    }

    #[test]
    fn stringify_evaluation_result_object_value() {
        let value = serde_json::json!({ "ok": true, "n": 1 });
        let as_text = stringify_evaluation_result(Some(&value), None);
        let parsed: serde_json::Value = match serde_json::from_str(&as_text) {
            Ok(parsed) => parsed,
            Err(err) => panic!("stringify should produce valid JSON: {err}"),
        };
        assert_eq!(parsed, value);
    }

    #[test]
    fn stringify_evaluation_result_uses_description_for_undefined() {
        assert_eq!(
            stringify_evaluation_result(None, Some("undefined")),
            "undefined"
        );
    }

    #[test]
    fn parse_runtime_location_href_trims_and_returns_string_values() {
        let value = serde_json::json!(" https://example.com/path ");
        assert_eq!(
            parse_runtime_location_href(Some(&value)),
            Some("https://example.com/path".to_string())
        );
    }

    #[test]
    fn parse_runtime_location_href_ignores_empty_and_non_string_values() {
        let empty = serde_json::json!("   ");
        let object = serde_json::json!({ "href": "https://example.com/path" });
        assert_eq!(parse_runtime_location_href(Some(&empty)), None);
        assert_eq!(parse_runtime_location_href(Some(&object)), None);
        assert_eq!(parse_runtime_location_href(None), None);
    }

    #[test]
    fn url_matches_pattern_exact() {
        assert!(url_matches_pattern(
            "https://example.com/login",
            "https://example.com/login"
        ));
        assert!(!url_matches_pattern(
            "https://example.com/home",
            "https://example.com/login"
        ));
    }

    #[test]
    fn url_matches_pattern_wildcards() {
        assert!(url_matches_pattern(
            "https://example.com/login/callback",
            "https://example.com/*/callback"
        ));
        assert!(url_matches_pattern(
            "https://example.com/a/b/c",
            "https://example.com/*"
        ));
        assert!(!url_matches_pattern(
            "https://example.com/a/b/c",
            "https://example.org/*"
        ));
    }

    #[test]
    fn urls_differ_only_by_fragment_true_for_hash_change() {
        assert!(urls_differ_only_by_fragment(
            "https://example.com/path#foo",
            "https://example.com/path#bar"
        ));
        assert!(urls_differ_only_by_fragment(
            "https://example.com/path",
            "https://example.com/path#foo"
        ));
    }

    #[test]
    fn urls_differ_only_by_fragment_false_for_other_changes() {
        assert!(!urls_differ_only_by_fragment(
            "https://example.com/path#foo",
            "https://example.com/path#foo"
        ));
        assert!(!urls_differ_only_by_fragment(
            "https://example.com/path#foo",
            "https://example.com/other#foo"
        ));
    }

    #[test]
    fn declared_domains_for_secret_returns_sorted_domains() {
        let mut declared = SecretDeclarations::new();
        declared.insert(
            "b.com".to_string(),
            ["password".to_string()].into_iter().collect(),
        );
        declared.insert(
            "a.com".to_string(),
            ["password".to_string(), "otp".to_string()]
                .into_iter()
                .collect(),
        );
        declared.insert(
            "c.com".to_string(),
            ["username".to_string()].into_iter().collect(),
        );

        let domains = declared_domains_for_secret(&declared, "password");
        assert_eq!(domains, vec!["a.com".to_string(), "b.com".to_string()]);
    }

    #[test]
    fn normalize_domain_like_input_accepts_url_or_host() {
        assert_eq!(
            normalize_domain_like_input("https://Example.com/login"),
            "example.com"
        );
        assert_eq!(normalize_domain_like_input("Example.com"), "example.com");
    }

    #[test]
    fn missing_prompt_override_error_mentions_message_and_flag() {
        let text = missing_prompt_override_error("OTP");
        assert!(text.contains("OTP"));
        assert!(text.contains("--prompt"));
    }

    #[test]
    fn unique_output_path_adds_suffix_on_collision() {
        let root = std::env::temp_dir().join(format!(
            "refreshmint-unique-output-path-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap_or_else(|err| {
            panic!("failed to create test dir: {err}");
        });
        std::fs::write(root.join("foo.csv"), "first").unwrap_or_else(|err| {
            panic!("failed to write fixture file: {err}");
        });

        let unique = unique_output_path(&root, "foo.csv");
        assert_eq!(
            unique.file_name().and_then(|s| s.to_str()),
            Some("foo-2.csv")
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    fn snapshot_node(reference: &str, label: &str) -> SnapshotNode {
        SnapshotNode {
            r#ref: reference.to_string(),
            parent_ref: None,
            role: "button".to_string(),
            label: label.to_string(),
            tag: "button".to_string(),
            text: label.to_string(),
            value: String::new(),
            visible: true,
            disabled: false,
            expanded: None,
            selected: None,
            checked: None,
            level: None,
            aria_labelled_by: None,
            aria_described_by: None,
            selector_hint: "button".to_string(),
        }
    }

    #[test]
    fn test_matches_filter_metadata() {
        let mut metadata = std::collections::BTreeMap::new();
        metadata.insert(
            "format".to_string(),
            serde_json::Value::String("qfx".to_string()),
        );
        metadata.insert("version".to_string(), serde_json::Value::Number(1.into()));

        let info = crate::scrape::DocumentInfo {
            mime_type: "application/x-ofx".to_string(),
            original_url: None,
            scraped_at: "2026-02-21T16:00:00Z".to_string(),
            extension_name: "test-ext".to_string(),
            login_name: "test-login".to_string(),
            label: "checking".to_string(),
            scrape_session_id: "session-1".to_string(),
            coverage_end_date: "2026-01-31".to_string(),
            date_range_start: None,
            date_range_end: None,
            metadata,
        };

        // Exact match
        let mut filter = std::collections::BTreeMap::new();
        filter.insert(
            "format".to_string(),
            serde_json::Value::String("qfx".to_string()),
        );
        assert!(matches_filter(&info, &filter));

        // Intrinsic field match
        let mut filter = std::collections::BTreeMap::new();
        filter.insert(
            "label".to_string(),
            serde_json::Value::String("checking".to_string()),
        );
        assert!(matches_filter(&info, &filter));

        // Multiple fields
        let mut filter = std::collections::BTreeMap::new();
        filter.insert(
            "format".to_string(),
            serde_json::Value::String("qfx".to_string()),
        );
        filter.insert("version".to_string(), serde_json::Value::Number(1.into()));
        assert!(matches_filter(&info, &filter));

        // Mismatch in metadata
        let mut filter = std::collections::BTreeMap::new();
        filter.insert(
            "format".to_string(),
            serde_json::Value::String("csv".to_string()),
        );
        assert!(!matches_filter(&info, &filter));

        // Mismatch in intrinsic
        let mut filter = std::collections::BTreeMap::new();
        filter.insert(
            "label".to_string(),
            serde_json::Value::String("savings".to_string()),
        );
        assert!(!matches_filter(&info, &filter));

        // Key missing in info
        let mut filter = std::collections::BTreeMap::new();
        filter.insert(
            "missing".to_string(),
            serde_json::Value::String("val".to_string()),
        );
        assert!(!matches_filter(&info, &filter));

        // Key missing in info but filter expects null
        let mut filter = std::collections::BTreeMap::new();
        filter.insert("missing".to_string(), serde_json::Value::Null);
        assert!(matches_filter(&info, &filter));
    }

    #[test]
    fn build_snapshot_diff_reports_added_updated_and_removed_nodes() {
        let previous = vec![snapshot_node("a", "Alpha"), snapshot_node("b", "Bravo")];
        let current = vec![snapshot_node("a", "Alpha"), snapshot_node("c", "Charlie")];

        let diff = build_snapshot_diff(&previous, &current, "main");
        assert_eq!(diff.mode, "incremental");
        assert_eq!(diff.track, "main");
        assert_eq!(diff.base_node_count, 2);
        assert_eq!(diff.node_count, 2);
        assert_eq!(diff.changed_count, 1);
        assert_eq!(diff.removed_count, 1);
        assert_eq!(diff.unchanged_count, 1);
        assert_eq!(diff.changed[0].change, "added");
        assert_eq!(diff.changed[0].node.r#ref, "c");
        assert_eq!(diff.removed_refs, vec!["b".to_string()]);
    }

    #[test]
    fn build_snapshot_diff_marks_existing_ref_changes_as_updated() {
        let previous = vec![snapshot_node("a", "Alpha")];
        let current = vec![snapshot_node("a", "Alpha updated")];

        let diff = build_snapshot_diff(&previous, &current, "main");
        assert_eq!(diff.changed_count, 1);
        assert_eq!(diff.changed[0].change, "updated");
        assert_eq!(diff.changed[0].node.r#ref, "a");
        assert_eq!(diff.removed_refs.len(), 0);
        assert_eq!(diff.unchanged_count, 0);
    }

    fn test_refreshmint_inner(overrides: PromptOverrides) -> RefreshmintInner {
        RefreshmintInner {
            output_dir: PathBuf::new(),
            prompt_overrides: overrides,
            prompt_requires_override: true,
            debug_output_sink: None,
            session_metadata: SessionMetadata::default(),
            staged_resources: Vec::new(),
            scrape_session_id: String::new(),
            extension_name: String::new(),
            account_name: String::new(),
            login_name: String::new(),
            ledger_dir: PathBuf::new(),
        }
    }

    #[test]
    fn report_value_uses_debug_output_sink_when_present() {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let mut inner = test_refreshmint_inner(PromptOverrides::new());
        inner.debug_output_sink = Some(sender);
        let api = RefreshmintApi::new(Arc::new(Mutex::new(inner)));

        api.js_report_value("alpha".to_string(), "42".to_string())
            .unwrap_or_else(|err| panic!("reportValue failed: {err}"));

        let event = receiver
            .try_recv()
            .unwrap_or_else(|err| panic!("missing output event: {err}"));
        assert_eq!(event.stream, DebugOutputStream::Stdout);
        assert_eq!(event.line, "alpha: 42");
    }

    #[test]
    fn log_uses_debug_output_sink_when_present() {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let mut inner = test_refreshmint_inner(PromptOverrides::new());
        inner.debug_output_sink = Some(sender);
        let api = RefreshmintApi::new(Arc::new(Mutex::new(inner)));

        api.log("hello".to_string())
            .unwrap_or_else(|err| panic!("log failed: {err}"));

        let event = receiver
            .try_recv()
            .unwrap_or_else(|err| panic!("missing output event: {err}"));
        assert_eq!(event.stream, DebugOutputStream::Stderr);
        assert_eq!(event.line, "hello");
    }

    #[test]
    fn prompt_returns_override_when_present_in_strict_mode() {
        let mut overrides = PromptOverrides::new();
        overrides.insert("OTP".to_string(), "123456".to_string());
        let api = RefreshmintApi::new(Arc::new(Mutex::new(test_refreshmint_inner(overrides))));

        let value = api
            .prompt("OTP".to_string())
            .unwrap_or_else(|err| panic!("prompt unexpectedly failed: {err}"));
        assert_eq!(value, "123456");
    }

    #[test]
    fn prompt_errors_when_missing_in_strict_mode() {
        let api = RefreshmintApi::new(Arc::new(Mutex::new(test_refreshmint_inner(
            PromptOverrides::new(),
        ))));

        let err = match api.prompt("Security answer".to_string()) {
            Ok(value) => panic!("expected missing prompt override error, got value: {value}"),
            Err(err) => err,
        };
        let message = err.to_string();
        assert!(message.contains("Security answer"));
        assert!(message.contains("--prompt"));
    }

    #[test]
    fn prompt_uses_trimmed_message_lookup() {
        let mut overrides = PromptOverrides::new();
        overrides.insert(
            "Enter the texted MFA code:".to_string(),
            "245221".to_string(),
        );
        let api = RefreshmintApi::new(Arc::new(Mutex::new(test_refreshmint_inner(overrides))));

        let value = api
            .prompt("Enter the texted MFA code: ".to_string())
            .unwrap_or_else(|err| panic!("prompt unexpectedly failed: {err}"));
        assert_eq!(value, "245221");
    }

    #[test]
    fn is_partial_download_file_detects_crdownload_extension() {
        assert!(is_partial_download_file(std::path::Path::new(
            "/tmp/file.csv.crdownload"
        )));
        assert!(!is_partial_download_file(std::path::Path::new(
            "/tmp/file.csv"
        )));
    }

    #[test]
    fn list_download_paths_returns_entries() {
        let root = std::env::temp_dir().join(format!(
            "refreshmint-list-download-paths-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap_or_else(|err| {
            panic!("failed to create test dir: {err}");
        });
        let first = root.join("a.csv");
        let second = root.join("b.csv.crdownload");
        std::fs::write(&first, "a").unwrap_or_else(|err| {
            panic!("failed to write first file: {err}");
        });
        std::fs::write(&second, "b").unwrap_or_else(|err| {
            panic!("failed to write second file: {err}");
        });

        let listed = list_download_paths(&root)
            .unwrap_or_else(|err| panic!("list_download_paths failed: {err}"));
        assert!(listed.contains(&first));
        assert!(listed.contains(&second));

        let _ = std::fs::remove_dir_all(&root);
    }
}
