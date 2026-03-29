use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use base64::Engine;
use rquickjs::class::Trace;
use rquickjs::promise::MaybePromise;
use rquickjs::{
    function::Opt, Class, Ctx, FromJs, Function, IntoJs, JsLifetime, Object, Persistent,
    Result as JsResult, TypedArray, Value,
};
use tokio::sync::{oneshot, Mutex};

use super::locator::{build_role_selector, Locator};
use crate::secret::SecretStore;

pub(crate) fn js_err(msg: String) -> rquickjs::Error {
    rquickjs::Error::new_from_js_message("Error", "Error", msg)
}

const BROWSER_DISCONNECTED_ERROR: &str =
    "BrowserDisconnectedError: debug browser channel closed; restart debug session";

const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const POLL_INTERVAL_MS: u64 = 100;
const REQUEST_CAPTURE_SETTLE_MS: u64 = 25;
const REQUEST_LINK_SETTLE_ATTEMPTS: usize = 8;
const TAB_QUERY_TIMEOUT_MS: u64 = 5_000;
const SCREENSHOT_PREPARE_STATE_KEY: &str = "__refreshmintScreenshotState";
const SCREENSHOT_CONTEXT_RETRY_ATTEMPTS: usize = 10;
const SCREENSHOT_CONTEXT_RETRY_MS: u64 = 100;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ScreenshotClip {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScreenshotImageFormat {
    Png,
    Jpeg,
}

#[derive(Clone)]
pub(crate) struct ParsedScreenshotOptions {
    pub format: ScreenshotImageFormat,
    pub quality: Option<i64>,
    pub full_page: bool,
    pub clip: Option<ScreenshotClip>,
    pub omit_background: bool,
    pub caret: String,
    pub animations: String,
    pub scale: String,
    pub path: Option<String>,
    pub style: Option<String>,
    pub mask_color: String,
    pub mask_locators: Vec<Locator>,
}

impl Default for ParsedScreenshotOptions {
    fn default() -> Self {
        Self {
            format: ScreenshotImageFormat::Png,
            quality: None,
            full_page: false,
            clip: None,
            omit_background: false,
            caret: "hide".to_string(),
            animations: "allow".to_string(),
            scale: "device".to_string(),
            path: None,
            style: None,
            mask_color: "#FF00FF".to_string(),
            mask_locators: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
struct GotoOptions {
    wait_until: String,
    timeout_ms: u64,
}

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

fn goto_deadline(timeout_ms: u64) -> Option<tokio::time::Instant> {
    if timeout_ms == 0 {
        None
    } else {
        Some(tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms))
    }
}

fn goto_timeout_err(timeout_ms: u64, url: &str) -> rquickjs::Error {
    js_err(format!(
        "TimeoutError: page.goto(\"{url}\"): Timeout {timeout_ms}ms exceeded."
    ))
}

fn goto_remaining(
    deadline: Option<tokio::time::Instant>,
    timeout_ms: u64,
    url: &str,
) -> JsResult<Option<std::time::Duration>> {
    let Some(deadline) = deadline else {
        return Ok(None);
    };
    let now = tokio::time::Instant::now();
    if now >= deadline {
        return Err(goto_timeout_err(timeout_ms, url));
    }
    Ok(Some(deadline.saturating_duration_since(now)))
}

fn is_browser_error_url(url: &str) -> bool {
    url.starts_with("chrome-error://")
}

fn is_cdp_request_timeout(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("request timed out") || (lower.contains("timeout") && lower.contains("request"))
}

fn is_missing_execution_context_error(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("cannot find context with specified id")
        || lower.contains("context with specified id")
        || lower.contains("execution context was destroyed")
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct NetworkRequest {
    #[serde(default)]
    request_id: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    status: i64,
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    method: String,
    #[serde(default)]
    status_text: String,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    frame_id: Option<String>,
    #[serde(default)]
    from_service_worker: bool,
    #[serde(default)]
    ts: i64,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    finished: bool,
    #[serde(default)]
    timing: RequestTiming,
    #[serde(default)]
    server_addr: Option<RemoteAddr>,
    #[serde(default)]
    security_details: Option<ResponseSecurityDetails>,
    /// CDP request ID, used by waitForResponseBody to fetch the body.
    /// Not serialized to JS (internal use only).
    #[serde(skip)]
    request_id_raw: Option<chromiumoxide::cdp::browser_protocol::network::RequestId>,
}

struct ResponseCaptureState {
    task: tokio::task::JoinHandle<()>,
}

#[derive(Debug, Clone)]
struct RequestCaptureItem {
    request_id: String,
    raw_request_id: String,
    url: String,
    method: String,
    headers: BTreeMap<String, String>,
    resource_type: String,
    post_data: Option<String>,
    frame_id: Option<String>,
    is_navigation_request: bool,
    redirected_from: Option<String>,
    error: Option<String>,
    finished: bool,
    timing: RequestTiming,
}

struct RequestCaptureState {
    task: tokio::task::JoinHandle<()>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CapturedFrameInfo {
    id: String,
    name: String,
    url: String,
    parent_id: Option<String>,
}

struct FrameCaptureState {
    task: tokio::task::JoinHandle<()>,
}

struct RequestWaiter {
    id: u64,
    matcher: UrlWaiterMatcher,
    sender: oneshot::Sender<RequestCaptureItem>,
}

struct ResponseWaiter {
    id: u64,
    matcher: UrlWaiterMatcher,
    sender: oneshot::Sender<NetworkRequest>,
}

struct RequestLifecycleWaiter {
    id: u64,
    event: RequestLifecycleEvent,
    sender: oneshot::Sender<RequestCaptureItem>,
}

#[derive(Debug, Clone)]
enum PendingRequestLifecycleState {
    Finished,
    Failed(String),
}

#[derive(Debug, Clone)]
enum UrlWaiterMatcher {
    Any,
    Pattern(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestLifecycleEvent {
    Finished,
    Failed,
}

enum JsNetworkMatcher {
    String(String),
    RegExp(Persistent<Value<'static>>),
    Predicate(Persistent<Function<'static>>),
}

struct EventWaitOptions {
    timeout_ms: u64,
    predicate: Option<Persistent<Function<'static>>>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct RequestTiming {
    start_time: f64,
    domain_lookup_start: f64,
    domain_lookup_end: f64,
    connect_start: f64,
    secure_connection_start: f64,
    connect_end: f64,
    request_start: f64,
    response_start: f64,
    response_end: f64,
}

impl RequestTiming {
    fn default_playwright() -> Self {
        // Matches Playwright's default ResourceTiming shape in client/network.ts.
        Self {
            start_time: 0.0,
            domain_lookup_start: -1.0,
            domain_lookup_end: -1.0,
            connect_start: -1.0,
            secure_connection_start: -1.0,
            connect_end: -1.0,
            request_start: -1.0,
            response_start: -1.0,
            response_end: -1.0,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteAddr {
    ip_address: String,
    port: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponseSecurityDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    protocol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    subject_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    issuer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    valid_from: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    valid_to: Option<f64>,
}

enum WaiterOutcome<T> {
    Value(T),
    Timeout,
    ChannelClosed,
    PageGone(rquickjs::Error),
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

/// Per-domain credential role declaration from the manifest.
///
/// A manifest may declare one or both of these names.  The `username` name
/// resolves to the Account field (no biometric on macOS); the `password` name
/// resolves to the Data field (biometric on macOS).
///
/// `extra_names` holds secret names from legacy array-format manifest
/// declarations that could not be assigned to a specific role.  They are
/// resolved via the legacy keychain fallback path.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct DomainCredentials {
    /// Secret name whose value is the account/username (stored without biometric).
    pub username: Option<String>,
    /// Secret name whose value is the password (biometric-protected on macOS).
    pub password: Option<String>,
    /// Legacy: names from an array-format manifest declaration (no role assigned).
    pub extra_names: Vec<String>,
}

/// Maps each declared domain to its credential role assignments.
pub type SecretDeclarations = BTreeMap<String, DomainCredentials>;
pub type PromptOverrides = BTreeMap<String, String>;
pub type ScriptOptions = serde_json::Map<String, serde_json::Value>;

// Transitional policy: keep legacy secret fallback enabled until the
// `migrate_login_secrets` flow is considered fully rolled out.
// See `src-tauri/src/lib.rs` `migrate_login_secrets` command.
const ENABLE_LEGACY_SECRET_FALLBACK: bool = true;

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
    pub target_id: String,
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
#[derive(Trace, Clone)]
pub struct PageApi {
    #[qjs(skip_trace)]
    inner: Arc<Mutex<PageInner>>,
    #[qjs(skip_trace)]
    request_entries: Arc<Mutex<Vec<RequestCaptureItem>>>,
    #[qjs(skip_trace)]
    response_entries: Arc<Mutex<Vec<NetworkRequest>>>,
    #[qjs(skip_trace)]
    response_capture: Arc<Mutex<Option<ResponseCaptureState>>>,
    #[qjs(skip_trace)]
    request_capture: Arc<Mutex<Option<RequestCaptureState>>>,
    #[qjs(skip_trace)]
    frame_entries: Arc<Mutex<BTreeMap<String, CapturedFrameInfo>>>,
    #[qjs(skip_trace)]
    frame_capture: Arc<Mutex<Option<FrameCaptureState>>>,
    #[qjs(skip_trace)]
    request_waiters: Arc<Mutex<Vec<RequestWaiter>>>,
    #[qjs(skip_trace)]
    response_waiters: Arc<Mutex<Vec<ResponseWaiter>>>,
    #[qjs(skip_trace)]
    request_lifecycle_waiters: Arc<Mutex<Vec<RequestLifecycleWaiter>>>,
    #[qjs(skip_trace)]
    pending_request_lifecycle:
        Arc<std::sync::Mutex<BTreeMap<String, PendingRequestLifecycleState>>>,
    #[qjs(skip_trace)]
    next_waiter_id: Arc<AtomicU64>,
    #[qjs(skip_trace)]
    snapshot_tracks: Arc<Mutex<BTreeMap<String, Vec<SnapshotNode>>>>,
    #[qjs(skip_trace)]
    request_timings: Arc<std::sync::Mutex<BTreeMap<String, RequestTiming>>>,
    #[qjs(skip_trace)]
    raw_request_current_ids: Arc<std::sync::Mutex<BTreeMap<String, String>>>,
    #[qjs(skip_trace)]
    next_request_id: Arc<AtomicU64>,
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
    fn allocate_waiter_id(&self) -> u64 {
        self.next_waiter_id.fetch_add(1, Ordering::Relaxed)
    }

    async fn register_request_waiter(
        &self,
        id: u64,
        matcher: UrlWaiterMatcher,
        sender: oneshot::Sender<RequestCaptureItem>,
    ) {
        self.request_waiters.lock().await.push(RequestWaiter {
            id,
            matcher,
            sender,
        });
    }

    async fn register_response_waiter(
        &self,
        id: u64,
        matcher: UrlWaiterMatcher,
        sender: oneshot::Sender<NetworkRequest>,
    ) {
        self.response_waiters.lock().await.push(ResponseWaiter {
            id,
            matcher,
            sender,
        });
    }

    async fn register_request_lifecycle_waiter(
        &self,
        id: u64,
        event: RequestLifecycleEvent,
        sender: oneshot::Sender<RequestCaptureItem>,
    ) {
        self.request_lifecycle_waiters
            .lock()
            .await
            .push(RequestLifecycleWaiter { id, event, sender });
    }

    async fn fulfill_request_waiter(&self, waiter_id: u64, item: RequestCaptureItem) -> bool {
        let waiter = {
            let mut waiters = self.request_waiters.lock().await;
            let Some(index) = waiters.iter().position(|waiter| waiter.id == waiter_id) else {
                return false;
            };
            waiters.swap_remove(index)
        };
        waiter.sender.send(item).is_ok()
    }

    async fn fulfill_response_waiter(&self, waiter_id: u64, item: NetworkRequest) -> bool {
        let waiter = {
            let mut waiters = self.response_waiters.lock().await;
            let Some(index) = waiters.iter().position(|waiter| waiter.id == waiter_id) else {
                return false;
            };
            waiters.swap_remove(index)
        };
        waiter.sender.send(item).is_ok()
    }

    async fn cancel_request_waiter(&self, waiter_id: u64) {
        let mut waiters = self.request_waiters.lock().await;
        if let Some(index) = waiters.iter().position(|waiter| waiter.id == waiter_id) {
            waiters.swap_remove(index);
        }
    }

    async fn cancel_response_waiter(&self, waiter_id: u64) {
        let mut waiters = self.response_waiters.lock().await;
        if let Some(index) = waiters.iter().position(|waiter| waiter.id == waiter_id) {
            waiters.swap_remove(index);
        }
    }

    async fn cancel_request_lifecycle_waiter(&self, waiter_id: u64) {
        let mut waiters = self.request_lifecycle_waiters.lock().await;
        if let Some(index) = waiters.iter().position(|waiter| waiter.id == waiter_id) {
            waiters.swap_remove(index);
        }
    }

    async fn latest_request_entry(
        entries: &Arc<Mutex<Vec<RequestCaptureItem>>>,
        request_id: &str,
    ) -> Option<RequestCaptureItem> {
        entries
            .lock()
            .await
            .iter()
            .rev()
            .find(|entry| entry.request_id == request_id)
            .cloned()
    }

    async fn latest_request_entry_for_raw(
        entries: &Arc<Mutex<Vec<RequestCaptureItem>>>,
        raw_request_id: &str,
    ) -> Option<RequestCaptureItem> {
        entries
            .lock()
            .await
            .iter()
            .rev()
            .find(|entry| entry.raw_request_id == raw_request_id)
            .cloned()
    }

    async fn latest_response_entry(
        entries: &Arc<Mutex<Vec<NetworkRequest>>>,
        request_id: &str,
    ) -> Option<NetworkRequest> {
        entries
            .lock()
            .await
            .iter()
            .rev()
            .find(|entry| entry.request_id == request_id)
            .cloned()
    }

    async fn resolve_response_request_id(
        request_entries: &Arc<Mutex<Vec<RequestCaptureItem>>>,
        response_entries: &Arc<Mutex<Vec<NetworkRequest>>>,
        raw_request_current_ids: &Arc<std::sync::Mutex<BTreeMap<String, String>>>,
        raw_request_id: &str,
        status: i64,
    ) -> Option<String> {
        let current_request_id = Self::settle_request_id_for_raw(
            request_entries,
            raw_request_current_ids,
            raw_request_id,
        )
        .await;
        if !(300..400).contains(&status) {
            return Some(current_request_id);
        }

        let redirected_from = request_entries
            .lock()
            .await
            .iter()
            .rev()
            .find(|entry| entry.request_id == current_request_id)
            .and_then(|entry| entry.redirected_from.clone());

        let Some(previous_request_id) = redirected_from else {
            return Some(current_request_id);
        };

        let previous_response_exists = response_entries
            .lock()
            .await
            .iter()
            .any(|entry| entry.request_id == previous_request_id);
        if previous_response_exists {
            None
        } else {
            Some(previous_request_id)
        }
    }

    async fn settle_request_id_for_raw(
        entries: &Arc<Mutex<Vec<RequestCaptureItem>>>,
        raw_request_current_ids: &Arc<std::sync::Mutex<BTreeMap<String, String>>>,
        raw_request_id: &str,
    ) -> String {
        for _ in 0..REQUEST_LINK_SETTLE_ATTEMPTS {
            let request_id = current_request_id_for_raw(raw_request_current_ids, raw_request_id);
            if request_id != raw_request_id {
                return request_id;
            }
            if let Some(entry) = Self::latest_request_entry_for_raw(entries, raw_request_id).await {
                return entry.request_id;
            }
            tokio::time::sleep(std::time::Duration::from_millis(REQUEST_CAPTURE_SETTLE_MS)).await;
        }

        Self::latest_request_entry_for_raw(entries, raw_request_id)
            .await
            .map(|entry| entry.request_id)
            .unwrap_or_else(|| current_request_id_for_raw(raw_request_current_ids, raw_request_id))
    }

    async fn wait_for_response_pattern(
        &self,
        url_pattern: String,
        timeout_ms: u64,
    ) -> JsResult<ResponseApi> {
        let entries = self.ensure_response_capture().await?;
        let baseline_len = entries.lock().await.len();
        let waiter_id = self.allocate_waiter_id();
        let (sender, receiver) = oneshot::channel();
        self.register_response_waiter(
            waiter_id,
            UrlWaiterMatcher::Pattern(url_pattern.clone()),
            sender,
        )
        .await;

        if let Some(found) = entries
            .lock()
            .await
            .iter()
            .skip(baseline_len)
            .find(|req| url_matches_pattern(&req.url, &url_pattern))
            .cloned()
        {
            let _ = self.fulfill_response_waiter(waiter_id, found.clone()).await;
            return Ok(self.response_api_from_entry(found));
        }

        match self
            .wait_for_receiver_with_page_liveness(
                receiver,
                timeout_ms,
                &format!("response pattern \"{url_pattern}\""),
            )
            .await
        {
            WaiterOutcome::Value(found) => Ok(self.response_api_from_entry(found)),
            WaiterOutcome::ChannelClosed => Err(js_err(format!(
                "waitForResponse failed for pattern \"{url_pattern}\": response waiter channel closed"
            ))),
            WaiterOutcome::Timeout => {
                self.cancel_response_waiter(waiter_id).await;
                Err(js_err(format!(
                    "TimeoutError: waiting for response pattern \"{url_pattern}\" failed: timeout {timeout_ms}ms exceeded"
                )))
            }
            WaiterOutcome::PageGone(err) => {
                self.cancel_response_waiter(waiter_id).await;
                Err(err)
            }
        }
    }

    async fn wait_for_request_pattern(
        &self,
        url_pattern: String,
        timeout_ms: u64,
    ) -> JsResult<RequestApi> {
        let entries = self.ensure_request_capture().await?;
        let baseline_len = entries.lock().await.len();
        let waiter_id = self.allocate_waiter_id();
        let (sender, receiver) = oneshot::channel();
        self.register_request_waiter(
            waiter_id,
            UrlWaiterMatcher::Pattern(url_pattern.clone()),
            sender,
        )
        .await;

        if let Some(found) = entries
            .lock()
            .await
            .iter()
            .skip(baseline_len)
            .find(|req| url_matches_pattern(&req.url, &url_pattern))
            .cloned()
        {
            let settled = Self::settle_request_entry(&entries, found).await;
            let _ = self
                .fulfill_request_waiter(waiter_id, settled.clone())
                .await;
            return Ok(self.request_api_from_entry(settled));
        }

        match self
            .wait_for_receiver_with_page_liveness(
                receiver,
                timeout_ms,
                &format!("request pattern \"{url_pattern}\""),
            )
            .await
        {
            WaiterOutcome::Value(found) => Ok(self.request_api_from_entry(found)),
            WaiterOutcome::ChannelClosed => Err(js_err(format!(
                "waitForRequest failed for pattern \"{url_pattern}\": request waiter channel closed"
            ))),
            WaiterOutcome::Timeout => {
                self.cancel_request_waiter(waiter_id).await;
                Err(js_err(format!(
                    "TimeoutError: waiting for request pattern \"{url_pattern}\" failed: timeout {timeout_ms}ms exceeded"
                )))
            }
            WaiterOutcome::PageGone(err) => {
                self.cancel_request_waiter(waiter_id).await;
                Err(err)
            }
        }
    }

    async fn settle_request_entry(
        entries: &Arc<Mutex<Vec<RequestCaptureItem>>>,
        found: RequestCaptureItem,
    ) -> RequestCaptureItem {
        tokio::time::sleep(std::time::Duration::from_millis(REQUEST_CAPTURE_SETTLE_MS)).await;
        Self::latest_request_entry(entries, &found.request_id)
            .await
            .unwrap_or(found)
    }

    async fn settle_response_entry(
        entries: &Arc<Mutex<Vec<NetworkRequest>>>,
        request_id: &str,
    ) -> Option<NetworkRequest> {
        for _ in 0..REQUEST_LINK_SETTLE_ATTEMPTS {
            if let Some(found) = Self::latest_response_entry(entries, request_id).await {
                return Some(found);
            }
            tokio::time::sleep(std::time::Duration::from_millis(REQUEST_CAPTURE_SETTLE_MS)).await;
        }
        Self::latest_response_entry(entries, request_id).await
    }

    async fn settle_redirected_request_entry(
        entries: &Arc<Mutex<Vec<RequestCaptureItem>>>,
        request_id: &str,
    ) -> Option<RequestCaptureItem> {
        for _ in 0..REQUEST_LINK_SETTLE_ATTEMPTS {
            if let Some(found) = entries
                .lock()
                .await
                .iter()
                .find(|entry| entry.redirected_from.as_ref() == Some(&request_id.to_string()))
                .cloned()
            {
                return Some(found);
            }
            tokio::time::sleep(std::time::Duration::from_millis(REQUEST_CAPTURE_SETTLE_MS)).await;
        }
        entries
            .lock()
            .await
            .iter()
            .find(|entry| entry.redirected_from.as_ref() == Some(&request_id.to_string()))
            .cloned()
    }

    async fn advance_request_cursor(
        entries: &Arc<Mutex<Vec<RequestCaptureItem>>>,
        cursor: &mut usize,
        request_id: &str,
    ) {
        let guard = entries.lock().await;
        if *cursor > guard.len() {
            *cursor = guard.len();
        }
        if let Some((index, _)) = guard
            .iter()
            .enumerate()
            .skip(*cursor)
            .find(|(_, entry)| entry.request_id == request_id)
        {
            *cursor = index + 1;
        } else {
            *cursor = guard.len();
        }
    }

    async fn advance_response_cursor(
        entries: &Arc<Mutex<Vec<NetworkRequest>>>,
        cursor: &mut usize,
        request_id: &str,
    ) {
        let guard = entries.lock().await;
        if *cursor > guard.len() {
            *cursor = guard.len();
        }
        if let Some((index, _)) = guard
            .iter()
            .enumerate()
            .skip(*cursor)
            .find(|(_, entry)| entry.request_id == request_id)
        {
            *cursor = index + 1;
        } else {
            *cursor = guard.len();
        }
    }

    async fn wait_for_next_request_entry(
        &self,
        entries: &Arc<Mutex<Vec<RequestCaptureItem>>>,
        cursor: &mut usize,
        timeout_ms: u64,
    ) -> JsResult<RequestCaptureItem> {
        let waiter_id = self.allocate_waiter_id();
        let (sender, receiver) = oneshot::channel();
        self.register_request_waiter(waiter_id, UrlWaiterMatcher::Any, sender)
            .await;

        if let Some(found) = entries.lock().await.get(*cursor).cloned() {
            self.cancel_request_waiter(waiter_id).await;
            *cursor += 1;
            return Ok(Self::settle_request_entry(entries, found).await);
        }

        match self
            .wait_for_receiver_with_page_liveness(receiver, timeout_ms, "request")
            .await
        {
            WaiterOutcome::Value(found) => {
                Self::advance_request_cursor(entries, cursor, &found.request_id).await;
                Ok(found)
            }
            WaiterOutcome::ChannelClosed => Err(js_err(
                "waitForRequest failed: request waiter channel closed".to_string(),
            )),
            WaiterOutcome::Timeout => {
                self.cancel_request_waiter(waiter_id).await;
                Err(js_err(format!(
                    "TimeoutError: waiting for request failed: timeout {timeout_ms}ms exceeded"
                )))
            }
            WaiterOutcome::PageGone(err) => {
                self.cancel_request_waiter(waiter_id).await;
                Err(err)
            }
        }
    }

    async fn wait_for_next_response_entry(
        &self,
        entries: &Arc<Mutex<Vec<NetworkRequest>>>,
        cursor: &mut usize,
        timeout_ms: u64,
    ) -> JsResult<NetworkRequest> {
        let waiter_id = self.allocate_waiter_id();
        let (sender, receiver) = oneshot::channel();
        self.register_response_waiter(waiter_id, UrlWaiterMatcher::Any, sender)
            .await;

        if let Some(found) = entries.lock().await.get(*cursor).cloned() {
            self.cancel_response_waiter(waiter_id).await;
            *cursor += 1;
            return Ok(found);
        }

        match self
            .wait_for_receiver_with_page_liveness(receiver, timeout_ms, "response")
            .await
        {
            WaiterOutcome::Value(found) => {
                Self::advance_response_cursor(entries, cursor, &found.request_id).await;
                Ok(found)
            }
            WaiterOutcome::ChannelClosed => Err(js_err(
                "waitForResponse failed: response waiter channel closed".to_string(),
            )),
            WaiterOutcome::Timeout => {
                self.cancel_response_waiter(waiter_id).await;
                Err(js_err(format!(
                    "TimeoutError: waiting for response failed: timeout {timeout_ms}ms exceeded"
                )))
            }
            WaiterOutcome::PageGone(err) => {
                self.cancel_response_waiter(waiter_id).await;
                Err(err)
            }
        }
    }

    async fn wait_for_next_request_lifecycle_entry(
        &self,
        entries: &Arc<Mutex<Vec<RequestCaptureItem>>>,
        lifecycle_event: RequestLifecycleEvent,
        cursor: &mut usize,
        timeout_ms: u64,
    ) -> JsResult<RequestCaptureItem> {
        let waiter_id = self.allocate_waiter_id();
        let (sender, receiver) = oneshot::channel();
        self.register_request_lifecycle_waiter(waiter_id, lifecycle_event, sender)
            .await;

        if let Some((index, found)) = entries
            .lock()
            .await
            .iter()
            .enumerate()
            .skip(*cursor)
            .find(|(_, entry)| request_entry_matches_lifecycle_event(entry, lifecycle_event))
            .map(|(index, entry)| (index, entry.clone()))
        {
            self.cancel_request_lifecycle_waiter(waiter_id).await;
            *cursor = index + 1;
            return Ok(found);
        }

        match self
            .wait_for_receiver_with_page_liveness(receiver, timeout_ms, "lifecycle event")
            .await
        {
            WaiterOutcome::Value(found) => {
                Self::advance_request_cursor(entries, cursor, &found.request_id).await;
                Ok(found)
            }
            WaiterOutcome::ChannelClosed => Err(js_err(
                "waitForEvent lifecycle waiter channel closed".to_string(),
            )),
            WaiterOutcome::Timeout => {
                self.cancel_request_lifecycle_waiter(waiter_id).await;
                Err(js_err(format!(
                    "TimeoutError: waiting for lifecycle event failed: timeout {timeout_ms}ms exceeded"
                )))
            }
            WaiterOutcome::PageGone(err) => {
                self.cancel_request_lifecycle_waiter(waiter_id).await;
                Err(err)
            }
        }
    }

    async fn wait_for_request_lifecycle_event(
        &self,
        lifecycle_event: &str,
        timeout_ms: u64,
    ) -> JsResult<RequestApi> {
        let entries = self.ensure_request_capture().await?;
        let baseline_len = entries.lock().await.len();
        let event = parse_request_lifecycle_event_name(lifecycle_event).ok_or_else(|| {
            js_err(format!(
                "unsupported request lifecycle event \"{lifecycle_event}\""
            ))
        })?;
        let waiter_id = self.allocate_waiter_id();
        let (sender, receiver) = oneshot::channel();
        self.register_request_lifecycle_waiter(waiter_id, event, sender)
            .await;

        if let Some(entry) = entries
            .lock()
            .await
            .iter()
            .skip(baseline_len)
            .find(|entry| request_entry_matches_lifecycle_event(entry, event))
            .cloned()
        {
            self.cancel_request_lifecycle_waiter(waiter_id).await;
            return Ok(self.request_api_from_entry(entry));
        }

        match self
            .wait_for_receiver_with_page_liveness(
                receiver,
                timeout_ms,
                &format!("event \"{lifecycle_event}\""),
            )
            .await
        {
            WaiterOutcome::Value(entry) => Ok(self.request_api_from_entry(entry)),
            WaiterOutcome::ChannelClosed => Err(js_err(format!(
                "waitForEvent(\"{lifecycle_event}\") failed: lifecycle waiter channel closed"
            ))),
            WaiterOutcome::Timeout => {
                self.cancel_request_lifecycle_waiter(waiter_id).await;
                Err(js_err(format!(
                    "TimeoutError: waiting for event \"{lifecycle_event}\" failed: timeout {timeout_ms}ms exceeded"
                )))
            }
            WaiterOutcome::PageGone(err) => {
                self.cancel_request_lifecycle_waiter(waiter_id).await;
                Err(err)
            }
        }
    }

    async fn refresh_page_handle(&self) -> Result<chromiumoxide::Page, String> {
        let (browser, target_id) = {
            let inner = self.inner.lock().await;
            (inner.browser.clone(), inner.target_id.clone())
        };

        let pages = {
            let guard = browser.lock().await;
            guard.pages().await.map_err(|e| e.to_string())?
        };
        let refreshed = pages
            .into_iter()
            .find(|page| page.target_id().as_ref() == target_id.as_str())
            .ok_or_else(|| format!("page target {target_id} not found during refresh"))?;

        let mut inner = self.inner.lock().await;
        inner.page = refreshed.clone();
        Ok(refreshed)
    }

    async fn ensure_page_waiter_alive(&self, wait_context: &str) -> JsResult<()> {
        let (browser, target_id) = {
            let inner = self.inner.lock().await;
            (inner.browser.clone(), inner.target_id.clone())
        };

        let pages = {
            let guard = browser.lock().await;
            guard.pages().await.map_err(|err| {
                let err_text = err.to_string();
                js_err(format_browser_error(wait_context, &err_text))
            })?
        };

        if pages
            .iter()
            .any(|candidate| candidate.target_id().as_ref() == target_id.as_str())
        {
            return Ok(());
        }

        Err(js_err(format!(
            "TargetClosedError: page was closed while waiting for {wait_context}"
        )))
    }

    async fn wait_for_receiver_with_page_liveness<T>(
        &self,
        mut receiver: oneshot::Receiver<T>,
        timeout_ms: u64,
        wait_context: &str,
    ) -> WaiterOutcome<T> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);

        loop {
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return WaiterOutcome::Timeout;
            }
            let remaining = deadline.saturating_duration_since(now);
            let poll_for = remaining.min(std::time::Duration::from_millis(POLL_INTERVAL_MS));

            tokio::select! {
                result = &mut receiver => {
                    return match result {
                        Ok(value) => WaiterOutcome::Value(value),
                        Err(_) => WaiterOutcome::ChannelClosed,
                    };
                }
                _ = tokio::time::sleep(poll_for) => {
                    if let Err(err) = self.ensure_page_waiter_alive(wait_context).await {
                        return WaiterOutcome::PageGone(err);
                    }
                }
            }
        }
    }

    async fn wait_for_popup_event<'js>(
        &self,
        ctx: &Ctx<'js>,
        options: &EventWaitOptions,
    ) -> JsResult<PageApi> {
        if options.predicate.is_none() {
            return self.wait_for_popup_page(options.timeout_ms).await;
        }

        let opener_target = {
            let inner = self.inner.lock().await;
            inner.page.target_id().as_ref().to_string()
        };
        let baseline_tabs = self.fetch_open_tabs().await?;
        let mut seen_ids = baseline_tabs
            .into_iter()
            .map(|tab| tab.target_id)
            .collect::<BTreeSet<_>>();
        let started_at = tokio::time::Instant::now();

        loop {
            let tabs = self.fetch_open_tabs().await?;
            if !tabs.iter().any(|tab| tab.target_id == opener_target) {
                return Err(js_err(
                    "TargetClosedError: page was closed while waiting for popup".to_string(),
                ));
            }
            for tab in tabs {
                if seen_ids.contains(&tab.target_id) {
                    continue;
                }
                seen_ids.insert(tab.target_id.clone());
                let candidate = build_page_api_from_template(&self.inner, tab.page).await;
                if page_matches_event_predicate(ctx, options.predicate.as_ref(), &candidate).await?
                {
                    return Ok(candidate);
                }
            }

            let _ = remaining_timeout_ms(options.timeout_ms, started_at, "popup")?;
            tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
        }
    }

    async fn wait_for_request_event<'js>(
        &self,
        ctx: &Ctx<'js>,
        options: &EventWaitOptions,
    ) -> JsResult<RequestApi> {
        let entries = self.ensure_request_capture().await?;
        let mut cursor = entries.lock().await.len();
        let started_at = tokio::time::Instant::now();
        loop {
            while let Some(entry) = entries.lock().await.get(cursor).cloned() {
                cursor += 1;
                let candidate = self.request_api_from_entry(entry);
                if request_matches_event_predicate(ctx, options.predicate.as_ref(), &candidate)
                    .await?
                {
                    return Ok(candidate);
                }
            }

            let remaining = remaining_timeout_ms(options.timeout_ms, started_at, "request event")?;
            let next = self
                .wait_for_next_request_entry(&entries, &mut cursor, remaining)
                .await?;
            let candidate = self.request_api_from_entry(next);
            if request_matches_event_predicate(ctx, options.predicate.as_ref(), &candidate).await? {
                return Ok(candidate);
            }
        }
    }

    async fn wait_for_response_event<'js>(
        &self,
        ctx: &Ctx<'js>,
        options: &EventWaitOptions,
    ) -> JsResult<ResponseApi> {
        let entries = self.ensure_response_capture().await?;
        let mut cursor = entries.lock().await.len();
        let started_at = tokio::time::Instant::now();
        loop {
            while let Some(entry) = entries.lock().await.get(cursor).cloned() {
                cursor += 1;
                let candidate = self.response_api_from_entry(entry);
                if response_matches_event_predicate(ctx, options.predicate.as_ref(), &candidate)
                    .await?
                {
                    return Ok(candidate);
                }
            }

            let remaining = remaining_timeout_ms(options.timeout_ms, started_at, "response event")?;
            let next = self
                .wait_for_next_response_entry(&entries, &mut cursor, remaining)
                .await?;
            let candidate = self.response_api_from_entry(next);
            if response_matches_event_predicate(ctx, options.predicate.as_ref(), &candidate).await?
            {
                return Ok(candidate);
            }
        }
    }

    async fn wait_for_request_lifecycle_event_filtered<'js>(
        &self,
        ctx: &Ctx<'js>,
        lifecycle_event: &str,
        options: &EventWaitOptions,
    ) -> JsResult<RequestApi> {
        if options.predicate.is_none() {
            return self
                .wait_for_request_lifecycle_event(lifecycle_event, options.timeout_ms)
                .await;
        }

        let entries = self.ensure_request_capture().await?;
        let mut cursor = entries.lock().await.len();
        let started_at = tokio::time::Instant::now();
        let event = parse_request_lifecycle_event_name(lifecycle_event).ok_or_else(|| {
            js_err(format!(
                "unsupported request lifecycle event \"{lifecycle_event}\""
            ))
        })?;

        loop {
            while let Some((index, entry)) = entries
                .lock()
                .await
                .iter()
                .enumerate()
                .skip(cursor)
                .find(|(_, entry)| request_entry_matches_lifecycle_event(entry, event))
                .map(|(index, entry)| (index, entry.clone()))
            {
                cursor = index + 1;
                let candidate = self.request_api_from_entry(entry);
                if request_matches_event_predicate(ctx, options.predicate.as_ref(), &candidate)
                    .await?
                {
                    return Ok(candidate);
                }
            }

            let remaining = remaining_timeout_ms(options.timeout_ms, started_at, lifecycle_event)?;
            let next = self
                .wait_for_next_request_lifecycle_entry(&entries, event, &mut cursor, remaining)
                .await?;
            let candidate = self.request_api_from_entry(next);
            if request_matches_event_predicate(ctx, options.predicate.as_ref(), &candidate).await? {
                return Ok(candidate);
            }
        }
    }

    async fn live_frame_infos(&self) -> Result<Vec<CapturedFrameInfo>, String> {
        let page = {
            let inner = self.inner.lock().await;
            inner.page.clone()
        };
        let frame_ids = page
            .frames()
            .await
            .map_err(|e| format!("failed to list live frames: {e}"))?;
        let mut frames = Vec::with_capacity(frame_ids.len());
        for frame_id in frame_ids {
            let id = frame_id.as_ref().to_string();
            let name = page
                .frame_name(frame_id.clone())
                .await
                .map_err(|e| format!("failed to query frame name for {id}: {e}"))?
                .unwrap_or_default();
            let url = page
                .frame_url(frame_id.clone())
                .await
                .map_err(|e| format!("failed to query frame url for {id}: {e}"))?
                .unwrap_or_default();
            let parent_id = page
                .frame_parent(frame_id)
                .await
                .map_err(|e| format!("failed to query frame parent for {id}: {e}"))?
                .map(|parent| parent.as_ref().to_string());
            frames.push(CapturedFrameInfo {
                id,
                name,
                url,
                parent_id,
            });
        }
        Ok(frames)
    }

    async fn ensure_frame_capture(
        &self,
    ) -> JsResult<Arc<Mutex<BTreeMap<String, CapturedFrameInfo>>>> {
        let mut guard = self.frame_capture.lock().await;
        if let Some(state) = guard.as_ref() {
            if !state.task.is_finished() {
                return Ok(self.frame_entries.clone());
            }
        }

        if let Some(previous) = guard.take() {
            previous.task.abort();
        }

        let page = {
            let inner = self.inner.lock().await;
            inner.page.clone()
        };

        use chromiumoxide::cdp::browser_protocol::page::{
            EnableParams, EventFrameAttached, EventFrameDetached, EventFrameNavigated,
            GetFrameTreeParams,
        };
        use chromiumoxide::cdp::browser_protocol::target::{
            EventAttachedToTarget, EventDetachedFromTarget,
        };
        page.execute(EnableParams::default())
            .await
            .map_err(|e| js_err(format!("failed to enable Page domain: {e}")))?;

        let tree = page
            .execute(GetFrameTreeParams::default())
            .await
            .map_err(|e| js_err(format!("failed to query frame tree: {e}")))?;

        let attached_events = page
            .event_listener::<EventFrameAttached>()
            .await
            .map_err(|e| js_err(format!("failed to attach frameAttached listener: {e}")))?;
        let navigated_events = page
            .event_listener::<EventFrameNavigated>()
            .await
            .map_err(|e| js_err(format!("failed to attach frameNavigated listener: {e}")))?;
        let detached_events = page
            .event_listener::<EventFrameDetached>()
            .await
            .map_err(|e| js_err(format!("failed to attach frameDetached listener: {e}")))?;
        let attached_target_events = page
            .event_listener::<EventAttachedToTarget>()
            .await
            .map_err(|e| js_err(format!("failed to attach attachedToTarget listener: {e}")))?;
        let detached_target_events = page
            .event_listener::<EventDetachedFromTarget>()
            .await
            .map_err(|e| js_err(format!("failed to attach detachedFromTarget listener: {e}")))?;

        let entries_for_task = self.frame_entries.clone();
        {
            let mut entries = entries_for_task.lock().await;
            entries.clear();
            seed_frame_entries_from_tree(&mut entries, tree.result.frame_tree);
        }

        let task = tokio::spawn(async move {
            use futures::StreamExt;
            let mut attached_target_sessions = BTreeMap::<String, String>::new();
            tokio::pin!(attached_events);
            tokio::pin!(navigated_events);
            tokio::pin!(detached_events);
            tokio::pin!(attached_target_events);
            tokio::pin!(detached_target_events);

            loop {
                tokio::select! {
                    ev = attached_events.next() => {
                        let Some(ev) = ev else {
                            break;
                        };
                        let mut entries = entries_for_task.lock().await;
                        entries
                            .entry(ev.frame_id.as_ref().to_string())
                            .or_insert_with(|| CapturedFrameInfo {
                                id: ev.frame_id.as_ref().to_string(),
                                name: String::new(),
                                url: String::new(),
                                parent_id: Some(ev.parent_frame_id.as_ref().to_string()),
                            });
                    }
                    ev = navigated_events.next() => {
                        let Some(ev) = ev else {
                            break;
                        };
                        let mut entries = entries_for_task.lock().await;
                        let frame = &ev.frame;
                        entries.insert(
                            frame.id.as_ref().to_string(),
                            CapturedFrameInfo {
                                id: frame.id.as_ref().to_string(),
                                name: frame.name.clone().unwrap_or_default(),
                                url: frame.url.clone(),
                                parent_id: frame
                                    .parent_id
                                    .clone()
                                    .map(|parent| parent.as_ref().to_string()),
                            },
                        );
                    }
                    ev = detached_events.next() => {
                        let Some(ev) = ev else {
                            break;
                        };
                        let mut entries = entries_for_task.lock().await;
                        remove_frame_entry_and_descendants(
                            &mut entries,
                            ev.frame_id.as_ref(),
                        );
                    }
                    ev = attached_target_events.next() => {
                        let Some(ev) = ev else {
                            break;
                        };
                        if ev.target_info.r#type == "iframe" {
                            let frame_id = ev.target_info.target_id.as_ref().to_string();
                            attached_target_sessions
                                .insert(ev.session_id.as_ref().to_string(), frame_id.clone());
                            let parent_id = ev
                                .target_info
                                .parent_frame_id
                                .as_ref()
                                .map(|parent| parent.as_ref().to_string());
                            let mut entries = entries_for_task.lock().await;
                            entries
                                .entry(frame_id.clone())
                                .and_modify(|entry| {
                                    entry.parent_id = parent_id.clone();
                                    if entry.url.is_empty() {
                                        entry.url = ev.target_info.url.clone();
                                    }
                                })
                                .or_insert_with(|| CapturedFrameInfo {
                                    id: frame_id,
                                    name: String::new(),
                                    url: ev.target_info.url.clone(),
                                    parent_id,
                                });
                        }
                    }
                    ev = detached_target_events.next() => {
                        let Some(ev) = ev else {
                            break;
                        };
                        if let Some(frame_id) =
                            attached_target_sessions.remove(ev.session_id.as_ref())
                        {
                            let mut entries = entries_for_task.lock().await;
                            remove_frame_entry_and_descendants(&mut entries, &frame_id);
                        }
                    }
                }
            }
        });

        *guard = Some(FrameCaptureState { task });
        Ok(self.frame_entries.clone())
    }

    async fn resolve_frame_id_live(
        &self,
        frame_ref: &str,
    ) -> Result<chromiumoxide::cdp::browser_protocol::page::FrameId, String> {
        let trimmed = frame_ref.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("main") {
            let inner = self.inner.lock().await;
            let main = inner
                .page
                .mainframe()
                .await
                .map_err(|e| format!("failed to resolve main frame: {e}"))?;
            return main.ok_or_else(|| "main frame not available".to_string());
        }

        let frames = self
            .ensure_frame_capture()
            .await
            .map_err(|e| e.to_string())?;
        let entries = frames.lock().await;
        if let Some(entry) = entries.get(trimmed) {
            return Ok(entry.id.clone().into());
        }
        if let Some(entry) = entries.values().find(|entry| entry.name == trimmed) {
            return Ok(entry.id.clone().into());
        }
        if let Some(entry) = entries
            .values()
            .find(|entry| entry.url == trimmed || entry.url.contains(trimmed))
        {
            return Ok(entry.id.clone().into());
        }
        drop(entries);

        for entry in self.live_frame_infos().await? {
            if entry.id == trimmed
                || entry.name == trimmed
                || entry.url == trimmed
                || entry.url.contains(trimmed)
            {
                return Ok(entry.id.into());
            }
        }

        if discovered_frame_ids_from_network(self)
            .await
            .contains(trimmed)
        {
            return Ok(trimmed.to_string().into());
        }

        let entries = frames.lock().await;
        let mut known_frames = entries
            .values()
            .map(|entry| format!("id={} name={} url={}", entry.id, entry.name, entry.url))
            .collect::<Vec<_>>();
        for frame_id in discovered_frame_ids_from_network(self).await {
            if !entries.contains_key(&frame_id) {
                known_frames.push(format!("id={} name= url=", frame_id));
            }
        }
        Err(format!(
            "frame not found for reference '{trimmed}'. Available frames: {}",
            known_frames.join(" | ")
        ))
    }

    pub fn new(inner: Arc<Mutex<PageInner>>) -> Self {
        Self {
            inner,
            request_entries: Arc::new(Mutex::new(Vec::new())),
            response_entries: Arc::new(Mutex::new(Vec::new())),
            response_capture: Arc::new(Mutex::new(None)),
            request_capture: Arc::new(Mutex::new(None)),
            frame_entries: Arc::new(Mutex::new(BTreeMap::new())),
            frame_capture: Arc::new(Mutex::new(None)),
            request_waiters: Arc::new(Mutex::new(Vec::new())),
            response_waiters: Arc::new(Mutex::new(Vec::new())),
            request_lifecycle_waiters: Arc::new(Mutex::new(Vec::new())),
            pending_request_lifecycle: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
            next_waiter_id: Arc::new(AtomicU64::new(1)),
            snapshot_tracks: Arc::new(Mutex::new(BTreeMap::new())),
            request_timings: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
            raw_request_current_ids: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
            next_request_id: Arc::new(AtomicU64::new(1)),
        }
    }
}

impl BrowserApi {
    pub fn new(page_inner: Arc<Mutex<PageInner>>) -> Self {
        Self { page_inner }
    }
}

/// A reference to a non-serializable JavaScript object in the browser.
///
/// Returned by `page.evaluate()` when the result cannot be serialised by value
/// (e.g. functions, symbols, complex circular graphs).  Matches the Playwright
/// `JSHandle` API.
#[rquickjs::class(rename = "JSHandle")]
#[derive(Trace)]
pub struct JsHandle {
    #[qjs(skip_trace)]
    object_id: String,
    #[qjs(skip_trace)]
    description: String,
    #[qjs(skip_trace)]
    page_inner: Arc<Mutex<PageInner>>,
}

// Safety: JsHandle only contains Arc<Mutex<...>> and String which are 'static.
#[allow(unsafe_code)]
unsafe impl<'js> JsLifetime<'js> for JsHandle {
    type Changed<'to> = JsHandle;
}

#[rquickjs::methods]
impl JsHandle {
    #[qjs(rename = "toString")]
    pub fn to_string_repr(&self) -> String {
        format!("JSHandle@{}", self.description)
    }

    pub async fn dispose(&self) -> JsResult<()> {
        use chromiumoxide::cdp::js_protocol::runtime::ReleaseObjectParams;
        let inner = self.page_inner.lock().await;
        inner
            .page
            .execute(ReleaseObjectParams::new(self.object_id.clone()))
            .await
            .map_err(|e| js_err(format!("JSHandle.dispose failed: {e}")))?;
        Ok(())
    }

    /// Return the serialised value of this handle as a JSON string.
    ///
    /// Equivalent to Playwright's `jsHandle.jsonValue()`, but returns a JSON
    /// string rather than a typed value (callers can `JSON.parse()` if needed).
    #[qjs(rename = "jsonValue")]
    pub async fn json_value(&self) -> JsResult<String> {
        let inner = self.page_inner.lock().await;
        let result = call_function_on_handle(
            &inner.page,
            &self.object_id,
            "function() { return this; }",
            &[],
            true,
        )
        .await
        .map_err(|e| js_err(format!("JSHandle.jsonValue failed: {e}")))?;
        let mut text =
            stringify_evaluation_result(result.value.as_ref(), result.description.as_deref());
        scrub_known_secrets(&inner.secret_store, &mut text);
        Ok(text)
    }
}

/// A reference to a DOM element in the browser.
///
/// Returned by `page.evaluate()` when the CDP result is a DOM node,
/// and by `page.$()`, `page.$$()`, and `elementHandle.$()`.
/// Matches the Playwright `ElementHandle` API (subset).
#[rquickjs::class(rename = "ElementHandle")]
#[derive(Trace)]
pub struct ElementHandle {
    #[qjs(skip_trace)]
    object_id: String,
    #[qjs(skip_trace)]
    description: String,
    #[qjs(skip_trace)]
    page_inner: Arc<Mutex<PageInner>>,
}

// Safety: ElementHandle only contains Arc<Mutex<...>> and String which are 'static.
#[allow(unsafe_code)]
unsafe impl<'js> JsLifetime<'js> for ElementHandle {
    type Changed<'to> = ElementHandle;
}

#[rquickjs::methods]
impl ElementHandle {
    #[qjs(rename = "toString")]
    pub fn to_string_repr(&self) -> String {
        format!("ElementHandle@{}", self.description)
    }

    pub async fn dispose(&self) -> JsResult<()> {
        use chromiumoxide::cdp::js_protocol::runtime::ReleaseObjectParams;
        let inner = self.page_inner.lock().await;
        inner
            .page
            .execute(ReleaseObjectParams::new(self.object_id.clone()))
            .await
            .map_err(|e| js_err(format!("ElementHandle.dispose failed: {e}")))?;
        Ok(())
    }

    /// Return the element's `outerHTML` as a string.
    #[qjs(rename = "jsonValue")]
    pub async fn json_value(&self) -> JsResult<String> {
        let inner = self.page_inner.lock().await;
        let result = call_function_on_handle(
            &inner.page,
            &self.object_id,
            "function() { return this.outerHTML !== undefined ? this.outerHTML : this.textContent; }",
            &[],
            true,
        )
        .await
        .map_err(|e| js_err(format!("ElementHandle.jsonValue failed: {e}")))?;
        let mut text =
            stringify_evaluation_result(result.value.as_ref(), result.description.as_deref());
        scrub_known_secrets(&inner.secret_store, &mut text);
        Ok(text)
    }

    pub async fn click(&self) -> JsResult<()> {
        let inner = self.page_inner.lock().await;
        call_function_on_handle(
            &inner.page,
            &self.object_id,
            r#"function() {
                if (!this.isConnected) throw new Error('click: element is detached');
                this.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
                this.click();
            }"#,
            &[],
            true,
        )
        .await
        .map_err(|e| js_err(format!("ElementHandle.click failed: {e}")))?;
        Ok(())
    }

    pub async fn fill(&self, value: String) -> JsResult<()> {
        use chromiumoxide::cdp::js_protocol::runtime::CallArgument;
        let actual_value = {
            let inner = self.page_inner.lock().await;
            resolve_secret_if_applicable(&inner, &value).await?
        };
        let inner = self.page_inner.lock().await;
        let value_arg = CallArgument {
            value: Some(serde_json::Value::String(actual_value)),
            unserializable_value: None,
            object_id: None,
        };
        call_function_on_handle(
            &inner.page,
            &self.object_id,
            r#"function(v) {
                if (!this.isConnected) throw new Error('fill: element is detached');
                this.focus();
                this.value = v;
                this.dispatchEvent(new Event('input', { bubbles: true }));
                this.dispatchEvent(new Event('change', { bubbles: true }));
            }"#,
            &[value_arg],
            true,
        )
        .await
        .map_err(|e| js_err(format!("ElementHandle.fill failed: {e}")))?;
        Ok(())
    }

    #[qjs(rename = "textContent")]
    pub async fn text_content(&self) -> JsResult<Option<String>> {
        let inner = self.page_inner.lock().await;
        let result = call_function_on_handle(
            &inner.page,
            &self.object_id,
            "function() { return this.textContent; }",
            &[],
            true,
        )
        .await
        .map_err(|e| js_err(format!("ElementHandle.textContent failed: {e}")))?;
        Ok(result
            .value
            .as_ref()
            .and_then(serde_json::Value::as_str)
            .map(str::to_string))
    }

    #[qjs(rename = "innerText")]
    pub async fn inner_text(&self) -> JsResult<Option<String>> {
        let inner = self.page_inner.lock().await;
        let result = call_function_on_handle(
            &inner.page,
            &self.object_id,
            "function() { return 'innerText' in this ? this.innerText : this.textContent; }",
            &[],
            true,
        )
        .await
        .map_err(|e| js_err(format!("ElementHandle.innerText failed: {e}")))?;
        Ok(result
            .value
            .as_ref()
            .and_then(serde_json::Value::as_str)
            .map(str::to_string))
    }

    #[qjs(rename = "getAttribute")]
    pub async fn get_attribute(&self, name: String) -> JsResult<Option<String>> {
        use chromiumoxide::cdp::js_protocol::runtime::CallArgument;
        let inner = self.page_inner.lock().await;
        let name_arg = CallArgument {
            value: Some(serde_json::Value::String(name)),
            unserializable_value: None,
            object_id: None,
        };
        let result = call_function_on_handle(
            &inner.page,
            &self.object_id,
            "function(n) { return this.getAttribute(n); }",
            &[name_arg],
            true,
        )
        .await
        .map_err(|e| js_err(format!("ElementHandle.getAttribute failed: {e}")))?;
        Ok(result
            .value
            .as_ref()
            .and_then(serde_json::Value::as_str)
            .map(str::to_string))
    }

    #[qjs(rename = "isVisible")]
    pub async fn is_visible(&self) -> JsResult<bool> {
        let inner = self.page_inner.lock().await;
        let result = call_function_on_handle(
            &inner.page,
            &self.object_id,
            r#"function() {
                if (!this.isConnected) return false;
                const rect = this.getBoundingClientRect();
                if (rect.width === 0 && rect.height === 0) return false;
                const style = window.getComputedStyle(this);
                return style.display !== 'none'
                    && style.visibility !== 'hidden'
                    && style.opacity !== '0';
            }"#,
            &[],
            true,
        )
        .await
        .map_err(|e| js_err(format!("ElementHandle.isVisible failed: {e}")))?;
        Ok(result
            .value
            .as_ref()
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false))
    }

    pub async fn screenshot<'js>(
        &self,
        ctx: Ctx<'js>,
        options: Opt<Value<'js>>,
    ) -> JsResult<TypedArray<'js, u8>> {
        let parsed = parse_screenshot_options(options.0.as_ref(), false)?;
        let (page, download_dir) = {
            let inner = self.page_inner.lock().await;
            (inner.page.clone(), inner.download_dir.clone())
        };
        let clip = screenshot_clip_for_object_id(&page, self.object_id.clone()).await?;
        let path = resolve_screenshot_output_path(&download_dir, parsed.path.as_deref())?;
        let bytes =
            run_screenshot_capture(self.page_inner.clone(), &parsed, Some(clip), &[], path).await?;
        TypedArray::new_copy(ctx, bytes)
            .map_err(|e| js_err(format!("ElementHandle.screenshot failed: {e}")))
    }

    /// Return the first descendant element matching `selector`, or `null`.
    #[qjs(rename = "$")]
    pub async fn query_selector(&self, selector: String) -> JsResult<Option<ElementHandle>> {
        use chromiumoxide::cdp::js_protocol::runtime::{CallArgument, RemoteObjectSubtype};
        let inner = self.page_inner.lock().await;
        let sel_arg = CallArgument {
            value: Some(serde_json::Value::String(selector.clone())),
            unserializable_value: None,
            object_id: None,
        };
        let result = call_function_on_handle(
            &inner.page,
            &self.object_id,
            "function(sel) { return this.querySelector(sel); }",
            &[sel_arg],
            false,
        )
        .await
        .map_err(|e| js_err(format!("ElementHandle.$({selector}) failed: {e}")))?;
        let Some(object_id) = result.object_id else {
            return Ok(None);
        };
        if result.subtype == Some(RemoteObjectSubtype::Null) {
            return Ok(None);
        }
        Ok(Some(ElementHandle {
            object_id: object_id.as_ref().to_string(),
            description: result.description.unwrap_or_default(),
            page_inner: self.page_inner.clone(),
        }))
    }

    /// Return all descendant elements matching `selector`.
    #[qjs(rename = "$$")]
    pub async fn query_selector_all(&self, selector: String) -> JsResult<Vec<ElementHandle>> {
        use chromiumoxide::cdp::js_protocol::runtime::CallArgument;
        let inner = self.page_inner.lock().await;
        let sel_arg = CallArgument {
            value: Some(serde_json::Value::String(selector.clone())),
            unserializable_value: None,
            object_id: None,
        };
        let array_result = call_function_on_handle(
            &inner.page,
            &self.object_id,
            "function(sel) { return Array.from(this.querySelectorAll(sel)); }",
            &[sel_arg],
            false,
        )
        .await
        .map_err(|e| js_err(format!("ElementHandle.$$({selector}) failed: {e}")))?;
        let array_id = match array_result.object_id {
            Some(id) => id,
            None => return Ok(vec![]),
        };
        collect_element_handles_from_array(&inner.page, array_id, self.page_inner.clone())
            .await
            .map_err(|e| js_err(format!("ElementHandle.$$({selector}) collect failed: {e}")))
    }
}

#[rquickjs::class(rename = "Frame")]
#[derive(Trace, Clone)]
pub struct FrameApi {
    frame_id: String,
    #[qjs(skip_trace)]
    page_inner: Arc<Mutex<PageInner>>,
}

#[allow(unsafe_code)]
unsafe impl<'js> JsLifetime<'js> for FrameApi {
    type Changed<'to> = FrameApi;
}

#[rquickjs::class(rename = "Request")]
#[derive(Trace, Clone)]
pub struct RequestApi {
    request_id: String,
    raw_request_id: String,
    url: String,
    method: String,
    resource_type: String,
    headers: BTreeMap<String, String>,
    frame_id: Option<String>,
    is_navigation_request: bool,
    post_data: Option<String>,
    redirected_from: Option<String>,
    error: Option<String>,
    finished: bool,
    #[qjs(skip_trace)]
    timing: RequestTiming,
    #[qjs(skip_trace)]
    page_api: PageApi,
}

#[allow(unsafe_code)]
unsafe impl<'js> JsLifetime<'js> for RequestApi {
    type Changed<'to> = RequestApi;
}

#[rquickjs::class(rename = "Response")]
#[derive(Trace, Clone)]
pub struct ResponseApi {
    request_id: String,
    url: String,
    status: i64,
    ok: bool,
    method: String,
    status_text: String,
    headers: BTreeMap<String, String>,
    frame_id: Option<String>,
    from_service_worker: bool,
    error: Option<String>,
    finished: bool,
    #[qjs(skip_trace)]
    server_addr: Option<RemoteAddr>,
    #[qjs(skip_trace)]
    security_details: Option<ResponseSecurityDetails>,
    #[qjs(skip_trace)]
    request_id_raw: Option<chromiumoxide::cdp::browser_protocol::network::RequestId>,
    #[qjs(skip_trace)]
    page_api: PageApi,
}

#[allow(unsafe_code)]
unsafe impl<'js> JsLifetime<'js> for ResponseApi {
    type Changed<'to> = ResponseApi;
}

/// Return value from `evaluate` / `evaluateHandle` / `callFunction`.
///
/// Serialisable primitives and JSON-safe objects are returned as their native
/// JS types; non-serialisable values (functions, DOM nodes, circular graphs,
/// …) are returned as `JSHandle` or `ElementHandle` instances.
pub enum JsEvalResult {
    /// A JS string.  Secret values have been scrubbed to `[REDACTED]`.
    Str(String),
    /// A JSON literal (number / boolean / null / array / plain object).
    /// Stored as a JSON string so it can be parsed via `ctx.eval()`.
    Json(String),
    /// A special numeric literal that JSON cannot represent: `NaN`, `Infinity`,
    /// `-Infinity`, or `-0`.
    Unserializable(String),
    /// A non-DOM remote object (function, symbol, Map, …).
    JsHandleResult(JsHandle),
    /// A DOM element remote object.
    ElementHandleResult(ElementHandle),
    /// A page handle.
    PageResult(PageApi),
    /// A captured network request.
    RequestResult(RequestApi),
    /// A captured network response.
    ResponseResult(ResponseApi),
    /// `undefined`.
    Undefined,
}

impl JsEvalResult {
    /// Convert to the legacy string representation used by internal helpers
    /// (`eval_string`, `eval_bool`, `waitForSelector`, …).
    pub(crate) fn into_string_repr(self) -> String {
        match self {
            JsEvalResult::Str(s) => s,
            JsEvalResult::Json(s) => s,
            JsEvalResult::Unserializable(s) => s,
            JsEvalResult::JsHandleResult(h) => format!("JSHandle@{}", h.description),
            JsEvalResult::ElementHandleResult(h) => {
                format!("ElementHandle@{}", h.description)
            }
            JsEvalResult::PageResult(_) => "Page".to_string(),
            JsEvalResult::RequestResult(_) => "Request".to_string(),
            JsEvalResult::ResponseResult(_) => "Response".to_string(),
            JsEvalResult::Undefined => "undefined".to_string(),
        }
    }
}

impl<'js> IntoJs<'js> for JsEvalResult {
    fn into_js(self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        match self {
            JsEvalResult::Str(s) => rquickjs::String::from_str(ctx.clone(), &s).map(Value::from),
            JsEvalResult::Json(s) => ctx.eval(s.into_bytes()),
            JsEvalResult::Unserializable(s) => ctx.eval(s.into_bytes()),
            JsEvalResult::JsHandleResult(h) => {
                Class::instance(ctx.clone(), h).map(|c| c.into_value())
            }
            JsEvalResult::ElementHandleResult(h) => {
                Class::instance(ctx.clone(), h).map(|c| c.into_value())
            }
            JsEvalResult::PageResult(p) => Class::instance(ctx.clone(), p).map(|c| c.into_value()),
            JsEvalResult::RequestResult(r) => {
                Class::instance(ctx.clone(), r).map(|c| c.into_value())
            }
            JsEvalResult::ResponseResult(r) => {
                Class::instance(ctx.clone(), r).map(|c| c.into_value())
            }
            JsEvalResult::Undefined => Ok(Value::new_undefined(ctx.clone())),
        }
    }
}

#[rquickjs::methods]
impl FrameApi {
    pub async fn url(&self) -> JsResult<String> {
        let page = {
            let inner = self.page_inner.lock().await;
            inner.page.clone()
        };
        let info = lookup_frame_info(&page, &self.frame_id)
            .await
            .map_err(|e| js_err(format!("Frame.url failed: {e}")))?;
        Ok(info.map(|frame| frame.url).unwrap_or_default())
    }

    pub async fn name(&self) -> JsResult<String> {
        let page = {
            let inner = self.page_inner.lock().await;
            inner.page.clone()
        };
        let info = lookup_frame_info(&page, &self.frame_id)
            .await
            .map_err(|e| js_err(format!("Frame.name failed: {e}")))?;
        Ok(info.map(|frame| frame.name).unwrap_or_default())
    }

    #[qjs(rename = "parentFrame")]
    pub async fn parent_frame(&self) -> JsResult<Option<FrameApi>> {
        let page = {
            let inner = self.page_inner.lock().await;
            inner.page.clone()
        };
        let info = lookup_frame_info(&page, &self.frame_id)
            .await
            .map_err(|e| js_err(format!("Frame.parentFrame failed: {e}")))?;
        Ok(info.and_then(|frame| {
            frame.parent_id.map(|parent_id| FrameApi {
                frame_id: parent_id,
                page_inner: self.page_inner.clone(),
            })
        }))
    }

    pub fn page(&self) -> PageApi {
        PageApi::new(self.page_inner.clone())
    }
}

#[rquickjs::methods]
impl RequestApi {
    pub fn url(&self) -> String {
        self.url.clone()
    }

    pub fn method(&self) -> String {
        self.method.clone()
    }

    #[qjs(rename = "resourceType")]
    pub fn resource_type(&self) -> String {
        self.resource_type.clone()
    }

    pub fn headers(&self) -> JsResult<JsEvalResult> {
        json_string_to_eval_result(headers_to_json_expr(&self.headers))
    }

    #[qjs(rename = "allHeaders")]
    pub async fn all_headers(&self) -> JsResult<JsEvalResult> {
        let entries = self.page_api.ensure_request_capture().await?;
        let headers = PageApi::latest_request_entry(&entries, &self.request_id)
            .await
            .map(|entry| entry.headers)
            .unwrap_or_else(|| self.headers.clone());
        json_string_to_eval_result(headers_to_json_expr(&headers))
    }

    #[qjs(rename = "headersArray")]
    pub async fn headers_array(&self) -> JsResult<JsEvalResult> {
        let entries = self.page_api.ensure_request_capture().await?;
        let headers = PageApi::latest_request_entry(&entries, &self.request_id)
            .await
            .map(|entry| entry.headers)
            .unwrap_or_else(|| self.headers.clone());
        json_string_to_eval_result(headers_array_json_expr(&headers))
    }

    #[qjs(rename = "headerValue")]
    pub async fn header_value(&self, name: String) -> JsResult<Option<String>> {
        let entries = self.page_api.ensure_request_capture().await?;
        let headers = PageApi::latest_request_entry(&entries, &self.request_id)
            .await
            .map(|entry| entry.headers)
            .unwrap_or_else(|| self.headers.clone());
        Ok(header_value(&headers, &name))
    }

    #[qjs(rename = "isNavigationRequest")]
    pub fn is_navigation_request(&self) -> bool {
        self.is_navigation_request
    }

    #[qjs(rename = "postData")]
    pub async fn post_data(&self) -> JsResult<Option<String>> {
        if self.post_data.is_some() {
            return Ok(self.post_data.clone());
        }

        let request_id = chromiumoxide::cdp::browser_protocol::network::RequestId::new(
            self.raw_request_id.clone(),
        );
        let page = {
            let inner = self.page_api.inner.lock().await;
            inner.page.clone()
        };
        match get_request_post_data(&page, request_id).await {
            Ok(post_data) => Ok(Some(post_data)),
            Err(_) => Ok(None),
        }
    }

    #[qjs(rename = "postDataBuffer")]
    pub async fn post_data_buffer<'js>(
        &self,
        ctx: Ctx<'js>,
    ) -> JsResult<Option<TypedArray<'js, u8>>> {
        let Some(post_data) = self.post_data().await? else {
            return Ok(None);
        };
        TypedArray::new_copy(ctx, post_data.into_bytes())
            .map(Some)
            .map_err(|e| js_err(format!("Request.postDataBuffer failed: {e}")))
    }

    #[qjs(rename = "postDataJSON")]
    pub async fn post_data_json(&self) -> JsResult<JsEvalResult> {
        let Some(post_data) = self.post_data().await? else {
            return Ok(JsEvalResult::Json("null".to_string()));
        };
        if self
            .headers
            .get("content-type")
            .is_some_and(|content_type| content_type.contains("application/x-www-form-urlencoded"))
        {
            let mut map = serde_json::Map::new();
            for (key, value) in parse_form_urlencoded_simple(&post_data) {
                map.insert(key, serde_json::Value::String(value));
            }
            let value = serde_json::Value::Object(map);
            let json = serde_json::to_string(&value)
                .map_err(|e| js_err(format!("Request.postDataJSON serialization failed: {e}")))?;
            return Ok(JsEvalResult::Json(wrap_json_for_eval(&value, json)));
        }
        let value: serde_json::Value = serde_json::from_str(&post_data)
            .map_err(|e| js_err(format!("Request.postDataJSON parse failed: {e}")))?;
        let json = serde_json::to_string(&value)
            .map_err(|e| js_err(format!("Request.postDataJSON serialization failed: {e}")))?;
        Ok(JsEvalResult::Json(wrap_json_for_eval(&value, json)))
    }

    pub async fn failure(&self) -> JsResult<JsEvalResult> {
        let entries = self.page_api.ensure_request_capture().await?;
        let error = PageApi::latest_request_entry(&entries, &self.request_id)
            .await
            .and_then(|entry| entry.error)
            .or_else(|| self.error.clone());
        match error {
            Some(error_text) => json_string_to_eval_result(format!(
                r#"({{"errorText":{}}})"#,
                serde_json::to_string(&error_text).unwrap_or_else(|_| "\"\"".to_string())
            )),
            None => Ok(JsEvalResult::Json("null".to_string())),
        }
    }

    pub async fn response(&self) -> JsResult<Option<ResponseApi>> {
        let entries = self.page_api.ensure_response_capture().await?;
        let found = {
            let guard = entries.lock().await;
            linked_response_for_request(
                &guard,
                &self.request_id,
                &self.raw_request_id,
                &self.url,
                self.redirected_from.as_ref(),
            )
        };
        let found = match found {
            Some(found) => Some(found),
            None => PageApi::settle_response_entry(&entries, &self.request_id).await,
        };
        Ok(found.map(|entry| self.page_api.response_api_from_entry(entry)))
    }

    pub fn timing(&self) -> JsResult<JsEvalResult> {
        let latest = self
            .page_api
            .request_timings
            .lock()
            .ok()
            .and_then(|timings| timings.get(&self.request_id).cloned())
            .unwrap_or_else(|| self.timing.clone());
        serialize_to_js_eval_result(&latest)
    }

    pub fn frame(&self) -> Option<FrameApi> {
        self.frame_id.as_ref().map(|frame_id| FrameApi {
            frame_id: frame_id.clone(),
            page_inner: self.page_api.inner.clone(),
        })
    }

    #[qjs(rename = "redirectedFrom")]
    pub async fn redirected_from(&self) -> JsResult<Option<RequestApi>> {
        let entries = self.page_api.ensure_request_capture().await?;
        let found = {
            let guard = entries.lock().await;
            linked_redirected_from_request(
                &guard,
                &self.request_id,
                &self.raw_request_id,
                self.redirected_from.as_ref(),
            )
        };
        Ok(found.map(|entry| self.page_api.request_api_from_entry(entry)))
    }

    #[qjs(rename = "redirectedTo")]
    pub async fn redirected_to(&self) -> JsResult<Option<RequestApi>> {
        let entries = self.page_api.ensure_request_capture().await?;
        let found = {
            let guard = entries.lock().await;
            linked_redirected_to_request(&guard, &self.request_id, &self.raw_request_id)
        };
        let found = match found {
            Some(found) => Some(found),
            None => PageApi::settle_redirected_request_entry(&entries, &self.request_id).await,
        };
        Ok(found.map(|entry| self.page_api.request_api_from_entry(entry)))
    }
}

#[rquickjs::methods]
impl ResponseApi {
    pub fn url(&self) -> String {
        self.url.clone()
    }

    pub fn status(&self) -> i64 {
        self.status
    }

    pub fn ok(&self) -> bool {
        self.ok
    }

    #[qjs(rename = "statusText")]
    pub fn status_text(&self) -> String {
        self.status_text.clone()
    }

    pub fn headers(&self) -> JsResult<JsEvalResult> {
        json_string_to_eval_result(headers_to_json_expr(&self.headers))
    }

    #[qjs(rename = "allHeaders")]
    pub async fn all_headers(&self) -> JsResult<JsEvalResult> {
        let entries = self.page_api.ensure_response_capture().await?;
        let headers = PageApi::latest_response_entry(&entries, &self.request_id)
            .await
            .map(|entry| entry.headers)
            .unwrap_or_else(|| self.headers.clone());
        json_string_to_eval_result(headers_to_json_expr(&headers))
    }

    #[qjs(rename = "headersArray")]
    pub async fn headers_array(&self) -> JsResult<JsEvalResult> {
        let entries = self.page_api.ensure_response_capture().await?;
        let headers = PageApi::latest_response_entry(&entries, &self.request_id)
            .await
            .map(|entry| entry.headers)
            .unwrap_or_else(|| self.headers.clone());
        json_string_to_eval_result(headers_array_json_expr(&headers))
    }

    #[qjs(rename = "headerValue")]
    pub async fn header_value(&self, name: String) -> JsResult<Option<String>> {
        let entries = self.page_api.ensure_response_capture().await?;
        let headers = PageApi::latest_response_entry(&entries, &self.request_id)
            .await
            .map(|entry| entry.headers)
            .unwrap_or_else(|| self.headers.clone());
        Ok(header_value(&headers, &name))
    }

    #[qjs(rename = "headerValues")]
    pub async fn header_values(&self, name: String) -> JsResult<Vec<String>> {
        let entries = self.page_api.ensure_response_capture().await?;
        let headers = PageApi::latest_response_entry(&entries, &self.request_id)
            .await
            .map(|entry| entry.headers)
            .unwrap_or_else(|| self.headers.clone());
        Ok(header_values(&headers, &name))
    }

    pub async fn body<'js>(&self, ctx: Ctx<'js>) -> JsResult<TypedArray<'js, u8>> {
        let request_id = self
            .request_id_raw
            .clone()
            .ok_or_else(|| js_err("Response.body failed: missing request id".to_string()))?;
        let page = {
            let inner = self.page_api.inner.lock().await;
            inner.page.clone()
        };
        let body = get_response_body_bytes(&page, request_id)
            .await
            .map_err(|e| js_err(format!("Response.body failed: {e}")))?;
        TypedArray::new_copy(ctx, body).map_err(|e| js_err(format!("Response.body failed: {e}")))
    }

    pub async fn text(&self) -> JsResult<String> {
        let request_id = self
            .request_id_raw
            .clone()
            .ok_or_else(|| js_err("Response.text failed: missing request id".to_string()))?;
        let page = {
            let inner = self.page_api.inner.lock().await;
            inner.page.clone()
        };
        let body = get_response_body_bytes(&page, request_id)
            .await
            .map_err(|e| js_err(format!("Response.text failed: {e}")))?;
        String::from_utf8(body).map_err(|e| js_err(format!("Response.text utf8 failed: {e}")))
    }

    pub async fn json(&self) -> JsResult<JsEvalResult> {
        let text = self.text().await?;
        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| js_err(format!("Response.json parse failed: {e}")))?;
        let json = serde_json::to_string(&value)
            .map_err(|e| js_err(format!("Response.json serialization failed: {e}")))?;
        Ok(JsEvalResult::Json(wrap_json_for_eval(&value, json)))
    }

    pub async fn request(&self) -> JsResult<Option<RequestApi>> {
        let entries = self.page_api.ensure_request_capture().await?;
        let found = {
            let guard = entries.lock().await;
            linked_request_for_response(
                &guard,
                &self.request_id,
                self.request_id_raw.as_ref(),
                &self.url,
                self.status,
            )
        };
        Ok(found.map(|entry| self.page_api.request_api_from_entry(entry)))
    }

    pub fn frame(&self) -> Option<FrameApi> {
        self.frame_id.as_ref().map(|frame_id| FrameApi {
            frame_id: frame_id.clone(),
            page_inner: self.page_api.inner.clone(),
        })
    }

    pub async fn finished(&self) -> JsResult<JsEvalResult> {
        let entries = self.page_api.ensure_response_capture().await?;
        let latest = PageApi::latest_response_entry(&entries, &self.request_id)
            .await
            .unwrap_or_else(|| NetworkRequest {
                request_id: self.request_id.clone(),
                url: self.url.clone(),
                status: self.status,
                ok: self.ok,
                method: self.method.clone(),
                status_text: self.status_text.clone(),
                headers: self.headers.clone(),
                frame_id: self.frame_id.clone(),
                from_service_worker: self.from_service_worker,
                ts: 0,
                error: self.error.clone(),
                finished: self.finished,
                timing: RequestTiming::default_playwright(),
                server_addr: self.server_addr.clone(),
                security_details: self.security_details.clone(),
                request_id_raw: self.request_id_raw.clone(),
            });
        if !latest.finished {
            return Ok(JsEvalResult::Json("null".to_string()));
        }
        match latest.error {
            Some(error_text) => json_string_to_eval_result(format!(
                r#"({{"errorText":{}}})"#,
                serde_json::to_string(&error_text).unwrap_or_else(|_| "\"\"".to_string())
            )),
            None => Ok(JsEvalResult::Json("null".to_string())),
        }
    }

    #[qjs(rename = "fromServiceWorker")]
    pub fn from_service_worker(&self) -> bool {
        self.from_service_worker
    }

    #[qjs(rename = "serverAddr")]
    pub async fn server_addr(&self) -> JsResult<JsEvalResult> {
        let entries = self.page_api.ensure_response_capture().await?;
        let latest = PageApi::latest_response_entry(&entries, &self.request_id).await;
        match latest
            .and_then(|entry| entry.server_addr)
            .or_else(|| self.server_addr.clone())
        {
            Some(server_addr) => serialize_to_js_eval_result(&server_addr),
            None => Ok(JsEvalResult::Json("null".to_string())),
        }
    }

    #[qjs(rename = "securityDetails")]
    pub async fn security_details(&self) -> JsResult<JsEvalResult> {
        let entries = self.page_api.ensure_response_capture().await?;
        let latest = PageApi::latest_response_entry(&entries, &self.request_id).await;
        match latest
            .and_then(|entry| entry.security_details)
            .or_else(|| self.security_details.clone())
        {
            Some(security_details) => serialize_to_js_eval_result(&security_details),
            None => Ok(JsEvalResult::Json("null".to_string())),
        }
    }
}

#[rquickjs::methods]
impl PageApi {
    /// Wait for a response matching `url_pattern` and return its body as a string.
    ///
    /// Uses `Network.getResponseBody` (CDP) which works across all frames including
    /// cross-origin OOP iframes. Returns the decoded body (base64 is handled automatically).
    /// Throws `TimeoutError` if no matching response is received within `timeout_ms`.
    #[qjs(rename = "waitForResponseBody")]
    pub async fn js_wait_for_response_body(
        &self,
        url_pattern: String,
        timeout_ms: Option<u64>,
    ) -> JsResult<String> {
        use chromiumoxide::cdp::browser_protocol::network::GetResponseBodyParams;

        let timeout_ms = timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
        let entries = self.ensure_response_capture().await?;
        let baseline_len = entries.lock().await.len();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);

        let page = {
            let inner = self.inner.lock().await;
            inner.page.clone()
        };

        loop {
            let maybe_request_id = {
                let guard = entries.lock().await;
                guard
                    .iter()
                    .skip(baseline_len)
                    .find(|req| url_matches_pattern(&req.url, &url_pattern))
                    .and_then(|req| req.request_id_raw.clone())
            };

            if let Some(request_id) = maybe_request_id {
                let result = page
                    .execute(GetResponseBodyParams::new(request_id))
                    .await
                    .map_err(|e| {
                        js_err(format!("waitForResponseBody getResponseBody failed: {e}"))
                    })?;

                let body = if result.result.base64_encoded {
                    let decoded = base64::Engine::decode(
                        &base64::engine::general_purpose::STANDARD,
                        &result.result.body,
                    )
                    .map_err(|e| {
                        js_err(format!("waitForResponseBody base64 decode failed: {e}"))
                    })?;
                    String::from_utf8(decoded).map_err(|e| {
                        js_err(format!("waitForResponseBody UTF-8 decode failed: {e}"))
                    })?
                } else {
                    result.result.body.clone()
                };
                return Ok(body);
            }

            if tokio::time::Instant::now() >= deadline {
                return Err(js_err(format!(
                    "TimeoutError: waiting for response body for pattern \"{url_pattern}\" failed: timeout {timeout_ms}ms exceeded"
                )));
            }
            tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
        }
    }

    /// Create a locator for the given selector.
    pub fn locator(&self, selector: String) -> Locator {
        Locator::new(self.inner.clone(), selector)
    }

    /// Create a locator for elements with the given ARIA role.
    #[qjs(rename = "getByRole")]
    pub fn get_by_role(
        &self,
        role: String,
        options: rquickjs::function::Opt<rquickjs::Value<'_>>,
    ) -> Locator {
        let selector = build_role_selector(&role, options.0);
        Locator::new(self.inner.clone(), selector)
    }

    /// Navigate to a URL.
    #[qjs(rename = "goto")]
    pub async fn js_goto(&self, url: String, options: Opt<rquickjs::Value<'_>>) -> JsResult<()> {
        let GotoOptions {
            wait_until,
            timeout_ms,
        } = parse_goto_options(options.0)?;
        let deadline = goto_deadline(timeout_ms);
        let current_url = self.current_url().await?;
        let page = {
            let inner = self.inner.lock().await;
            inner.page.clone()
        };
        if current_url == url {
            if let Some(remaining) = goto_remaining(deadline, timeout_ms, &url)? {
                tokio::time::timeout(remaining, page.reload())
                    .await
                    .map_err(|_| goto_timeout_err(timeout_ms, &url))?
                    .map_err(|e| js_err(format!("goto failed (same-url reload): {e}")))?;
            } else {
                page.reload()
                    .await
                    .map_err(|e| js_err(format!("goto failed (same-url reload): {e}")))?;
            }
            self.wait_for_goto_wait_until(&wait_until, deadline, timeout_ms, &url)
                .await?;
            self.ensure_not_browser_error_page(&url).await?;
            return Ok(());
        }

        use chromiumoxide::cdp::browser_protocol::page::NavigateParams;
        let params = NavigateParams::builder()
            .url(url.clone())
            .build()
            .map_err(|e| js_err(format!("goto build failed: {e}")))?;
        let nav_outcome = if let Some(remaining) = goto_remaining(deadline, timeout_ms, &url)? {
            tokio::time::timeout(remaining, page.execute(params))
                .await
                .ok()
        } else {
            Some(page.execute(params).await)
        };

        if let Some(nav_result) = nav_outcome {
            match nav_result {
                Ok(nav_result) => {
                    if let Some(error_text) = nav_result.result.error_text {
                        return Err(js_err(format!("goto failed: {error_text} at {url}")));
                    }
                }
                Err(err) => {
                    let err_text = err.to_string();
                    if is_cdp_request_timeout(&err_text) {
                        // Chromiumoxide wraps Page.navigate as a navigation request and can
                        // surface a timeout before our explicit waitUntil completes.
                        // Keep observing URL/lifecycle up to the caller's timeout.
                    } else {
                        return Err(js_err(format!("goto failed: {err}")));
                    }
                }
            }
        }

        loop {
            let observed = self.current_url().await?;
            if observed != current_url {
                break;
            }
            if let Some(limit) = deadline {
                if tokio::time::Instant::now() >= limit {
                    return Err(goto_timeout_err(timeout_ms, &url));
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
        }
        self.wait_for_goto_wait_until(&wait_until, deadline, timeout_ms, &url)
            .await?;
        self.ensure_not_browser_error_page(&url).await?;
        Ok(())
    }

    /// Get the current page URL.
    pub async fn url(&self) -> JsResult<String> {
        let page = {
            let inner = self.inner.lock().await;
            inner.page.clone()
        };
        let url = match page.url().await {
            Ok(url) => url.unwrap_or_default(),
            Err(err) => {
                let err_text = err.to_string();
                if !is_transport_disconnected_error(&err_text) {
                    return Err(js_err(format_browser_error("url() failed", &err_text)));
                }
                let refreshed = self.refresh_page_handle().await.map_err(|refresh_err| {
                    js_err(format_browser_error(
                        "url() page refresh failed",
                        &refresh_err,
                    ))
                })?;
                refreshed
                    .url()
                    .await
                    .map_err(|retry_err| {
                        js_err(format_browser_error("url() failed", &retry_err.to_string()))
                    })?
                    .unwrap_or_default()
            }
        };
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
        let entries = self.ensure_frame_capture().await?;
        let entries = entries.lock().await;
        let mut out = entries.values().cloned().collect::<Vec<_>>();
        let mut known_ids = out
            .iter()
            .map(|entry| entry.id.clone())
            .collect::<BTreeSet<_>>();
        drop(entries);
        for frame in self
            .live_frame_infos()
            .await
            .map_err(|e| js_err(format!("frames failed: {e}")))?
        {
            if known_ids.insert(frame.id.clone()) {
                out.push(frame);
            }
        }
        for frame_id in discovered_frame_ids_from_network(self).await {
            if known_ids.contains(&frame_id) {
                continue;
            }
            out.push(CapturedFrameInfo {
                id: frame_id,
                name: String::new(),
                url: String::new(),
                parent_id: None,
            });
        }
        serde_json::to_string(&out).map_err(|e| js_err(format!("frames serialization failed: {e}")))
    }

    /// Switch subsequent element interactions to the given frame.
    ///
    /// `frame_ref` may be a frame id, frame name, or frame URL substring.
    #[qjs(rename = "switchToFrame")]
    pub async fn js_switch_to_frame(&self, frame_ref: String) -> JsResult<()> {
        let frame_id = self
            .resolve_frame_id_live(&frame_ref)
            .await
            .map_err(|e| js_err(format!("switchToFrame failed: {e}")))?;
        let mut inner = self.inner.lock().await;
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
                .eval_string(probe.clone(), "waitForSelector")
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
        if state != "load"
            && state != "domcontentloaded"
            && state != "networkidle"
            && state != "commit"
        {
            return Err(js_err(format!(
                "waitForLoadState unsupported state: {requested_state}"
            )));
        }

        let timeout_ms = timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
        if state == "commit" {
            return Ok(());
        }
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

    /// Wait for a network response URL matching a string glob, RegExp, or predicate.
    #[qjs(rename = "waitForResponse")]
    pub async fn js_wait_for_response<'js>(
        &self,
        ctx: Ctx<'js>,
        url_or_predicate: Value<'js>,
        options: Opt<rquickjs::Value<'_>>,
    ) -> JsResult<ResponseApi> {
        let timeout_ms = parse_timeout_option(options.0.as_ref())?;
        let matcher = parse_wait_for_network_matcher(&ctx, url_or_predicate, "waitForResponse")?;
        match matcher {
            JsNetworkMatcher::String(url_pattern) => {
                self.wait_for_response_pattern(url_pattern, timeout_ms)
                    .await
            }
            JsNetworkMatcher::RegExp(_) | JsNetworkMatcher::Predicate(_) => {
                let entries = self.ensure_response_capture().await?;
                let mut cursor = entries.lock().await.len();
                let started_at = tokio::time::Instant::now();
                loop {
                    while let Some(entry) = entries.lock().await.get(cursor).cloned() {
                        cursor += 1;
                        let candidate = self.response_api_from_entry(entry);
                        if response_matches_js_matcher(&ctx, &matcher, &candidate).await? {
                            return Ok(candidate);
                        }
                    }

                    let remaining = remaining_timeout_ms(timeout_ms, started_at, "response")?;
                    let next = self
                        .wait_for_next_response_entry(&entries, &mut cursor, remaining)
                        .await?;
                    let candidate = self.response_api_from_entry(next);
                    if response_matches_js_matcher(&ctx, &matcher, &candidate).await? {
                        return Ok(candidate);
                    }
                }
            }
        }
    }

    #[qjs(rename = "waitForRequest")]
    pub async fn js_wait_for_request<'js>(
        &self,
        ctx: Ctx<'js>,
        url_or_predicate: Value<'js>,
        options: Opt<rquickjs::Value<'_>>,
    ) -> JsResult<RequestApi> {
        let timeout_ms = parse_timeout_option(options.0.as_ref())?;
        let matcher = parse_wait_for_network_matcher(&ctx, url_or_predicate, "waitForRequest")?;
        match matcher {
            JsNetworkMatcher::String(url_pattern) => {
                self.wait_for_request_pattern(url_pattern, timeout_ms).await
            }
            JsNetworkMatcher::RegExp(_) | JsNetworkMatcher::Predicate(_) => {
                let entries = self.ensure_request_capture().await?;
                let mut cursor = entries.lock().await.len();
                let started_at = tokio::time::Instant::now();
                loop {
                    while let Some(entry) = entries.lock().await.get(cursor).cloned() {
                        cursor += 1;
                        let candidate = self.request_api_from_entry(entry);
                        if request_matches_js_matcher(&ctx, &matcher, &candidate).await? {
                            return Ok(candidate);
                        }
                    }

                    let remaining = remaining_timeout_ms(timeout_ms, started_at, "request")?;
                    let next = self
                        .wait_for_next_request_entry(&entries, &mut cursor, remaining)
                        .await?;
                    let candidate = self.request_api_from_entry(next);
                    if request_matches_js_matcher(&ctx, &matcher, &candidate).await? {
                        return Ok(candidate);
                    }
                }
            }
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
                    if (state.mode === 'ignore') {{
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
    /// Currently supports `popup`, `request`, `response`, `requestfinished`,
    /// and `requestfailed`.
    #[qjs(rename = "waitForEvent")]
    pub async fn js_wait_for_event<'js>(
        &self,
        ctx: Ctx<'js>,
        event: String,
        options_or_predicate: Opt<Value<'js>>,
    ) -> JsResult<JsEvalResult> {
        let normalized = event.trim().to_ascii_lowercase();
        let options =
            parse_wait_for_event_options(&ctx, options_or_predicate.0.as_ref(), "waitForEvent")?;
        match normalized.as_str() {
            "popup" => Ok(JsEvalResult::PageResult(
                self.wait_for_popup_event(&ctx, &options).await?,
            )),
            "request" => Ok(JsEvalResult::RequestResult(
                self.wait_for_request_event(&ctx, &options).await?,
            )),
            "response" => Ok(JsEvalResult::ResponseResult(
                self.wait_for_response_event(&ctx, &options).await?,
            )),
            "requestfinished" | "requestfailed" => Ok(JsEvalResult::RequestResult(
                self.wait_for_request_lifecycle_event_filtered(&ctx, normalized.as_str(), &options)
                    .await?,
            )),
            _ => Err(js_err(format!(
                "waitForEvent currently supports only \"popup\", \"request\", \"response\", \"requestfinished\", and \"requestfailed\" (got {event})"
            ))),
        }
    }

    /// Click an element matching the CSS selector.
    pub async fn click(&self, selector: String) -> JsResult<()> {
        let inner = self.inner.lock().await;
        if let Some(frame_id) = &inner.target_frame_id {
            // Frame context: evaluate JS click inside the frame's execution context.
            let (context_id, session_id) =
                wait_for_frame_execution_target(&inner.page, frame_id.clone())
                    .await
                    .map_err(|e| js_err(format!("click failed to get frame target: {e}")))?;
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
                .evaluate_expression_with_session(eval, session_id)
                .await
                .map_err(|e| js_err(format!("click failed: {e}")))?;
        } else {
            drop(inner);
            Locator::new(self.inner.clone(), selector)
                .click_with_timeout(DEFAULT_TIMEOUT_MS)
                .await?;
            return Ok(());
        }
        Ok(())
    }

    /// Type text into an element, character by character.
    #[qjs(rename = "type")]
    pub async fn js_type(&self, selector: String, text: String) -> JsResult<()> {
        let actual_text = {
            let inner = self.inner.lock().await;
            resolve_secret_if_applicable(&inner, &text).await?
        };

        let inner = self.inner.lock().await;
        if let Some(frame_id) = &inner.target_frame_id {
            // Frame context: focus element via JS, then dispatch CDP key events
            // (Input.dispatchKeyEvent is global and targets the focused element).
            let (context_id, session_id) =
                wait_for_frame_execution_target(&inner.page, frame_id.clone())
                    .await
                    .map_err(|e| js_err(format!("type failed to get frame target: {e}")))?;
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
                .evaluate_expression_with_session(eval, session_id)
                .await
                .map_err(|e| js_err(format!("type failed: {e}")))?;
            inner
                .page
                .type_str(&actual_text)
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
                .type_str(&actual_text)
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
    ) -> JsResult<JsEvalResult> {
        use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
        let page_inner_arc = self.inner.clone();
        let frame_id = self
            .resolve_frame_id_live(&frame_ref)
            .await
            .map_err(|e| js_err(format!("frameEvaluate failed: {e}")))?;
        let inner = self.inner.lock().await;
        let (context_id, session_id) =
            wait_for_frame_execution_target(&inner.page, frame_id.clone())
                .await
                .map_err(|e| js_err(format!("frameEvaluate failed: {e}")))?;
        let eval = EvaluateParams::builder()
            .expression(expression)
            .context_id(context_id)
            .await_promise(true)
            .return_by_value(false)
            .build()
            .map_err(|e| js_err(format!("frameEvaluate invalid expression params: {e}")))?;
        let result = inner
            .page
            .evaluate_expression_with_session(eval, session_id)
            .await
            .map_err(|e| js_err(format!("frameEvaluate failed: {e}")))?;
        let mut eval_result = remote_object_to_eval_result(result.object().clone(), page_inner_arc);
        if let JsEvalResult::Str(ref mut s) = eval_result {
            scrub_known_secrets(&inner.secret_store, s);
        }
        Ok(eval_result)
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
        let frame_id = self
            .resolve_frame_id_live(&frame_ref)
            .await
            .map_err(|e| js_err(format!("frameFill failed: {e}")))?;
        let inner = self.inner.lock().await;
        let actual_value = resolve_secret_if_applicable(&inner, &value).await?;
        let (context_id, session_id) = wait_for_frame_execution_target(&inner.page, frame_id)
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
            .evaluate_expression_with_session(eval, session_id)
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
    /// Returns the result as a native JS value: number, boolean, string, object,
    /// array, `null`, or `undefined` for serialisable values; a `JSHandle` or
    /// `ElementHandle` for non-serialisable ones (functions, DOM nodes, …).
    ///
    /// Secret string values in the result are scrubbed to `[REDACTED]`.
    pub async fn evaluate(&self, expression: String) -> JsResult<JsEvalResult> {
        self.evaluate_in_active_context(expression).await
    }

    /// Like `evaluate`, but always returns a handle regardless of whether the
    /// value is serialisable.  Kept for Playwright API compatibility.
    #[qjs(rename = "evaluateHandle")]
    pub async fn js_evaluate_handle(&self, expression: String) -> JsResult<JsEvalResult> {
        self.evaluate_in_active_context(expression).await
    }

    /// Call a JS function expression with the given arguments.
    ///
    /// Arguments may be primitive values **or** `JSHandle` / `ElementHandle`
    /// instances returned by a previous `evaluate` call.  Handles are passed
    /// directly to the browser by `objectId` without serialisation, so this
    /// works even for non-serialisable values.
    ///
    /// ```js
    /// const el = await page.evaluate("document.querySelector('#submit')");
    /// const tag = await page.callFunction("el => el.tagName", el);
    /// ```
    #[qjs(rename = "callFunction")]
    pub async fn js_call_function(
        &self,
        fn_expression: String,
        args: Opt<rquickjs::Value<'_>>,
    ) -> JsResult<JsEvalResult> {
        use chromiumoxide::cdp::js_protocol::runtime::{
            CallArgument, CallFunctionOnParams, ExecutionContextId,
        };

        // Build the argument list, extracting objectIds from any handles.
        let call_args: Vec<CallArgument> = if let Some(arg_val) = args.0 {
            js_value_to_call_args(&arg_val).map_err(|e| js_err(format!("callFunction: {e}")))?
        } else {
            Vec::new()
        };

        let inner = self.inner.lock().await;
        let page_inner_arc = self.inner.clone();

        // Obtain the execution context id for the active context.
        let (context_id, session_id_opt): (
            ExecutionContextId,
            Option<chromiumoxide::cdp::browser_protocol::target::SessionId>,
        ) = if let Some(frame_id) = &inner.target_frame_id {
            let (context_id, session_id) =
                wait_for_frame_execution_target(&inner.page, frame_id.clone())
                    .await
                    .map_err(|e| js_err(format!("callFunction failed to get frame target: {e}")))?;
            (context_id, Some(session_id))
        } else {
            // Main frame: get the main frame id and then its context.
            let main_frame = inner
                .page
                .mainframe()
                .await
                .map_err(|e| js_err(format!("callFunction failed to get main frame: {e}")))?
                .ok_or_else(|| js_err("callFunction: main frame not available".to_string()))?;
            (
                wait_for_frame_execution_context(&inner.page, main_frame)
                    .await
                    .map_err(|e| js_err(format!("callFunction failed to get main context: {e}")))?,
                None,
            )
        };

        let mut builder = CallFunctionOnParams::builder()
            .function_declaration(fn_expression)
            .execution_context_id(context_id)
            .return_by_value(false)
            .await_promise(true);
        for arg in &call_args {
            builder = builder.argument(arg.clone());
        }
        let params = builder
            .build()
            .map_err(|e| js_err(format!("callFunction build failed: {e}")))?;
        let response = if let Some(session_id) = session_id_opt {
            inner
                .page
                .execute_with_session(params, session_id)
                .await
                .map_err(|e| js_err(format!("callFunction CDP failed: {e}")))?
        } else {
            inner
                .page
                .execute(params)
                .await
                .map_err(|e| js_err(format!("callFunction CDP failed: {e}")))?
        };
        if let Some(exc) = &response.result.exception_details {
            let msg = exc
                .exception
                .as_ref()
                .and_then(|o| o.description.as_deref())
                .unwrap_or(&exc.text);
            return Err(js_err(msg.to_string()));
        }
        let mut eval_result = remote_object_to_eval_result(response.result.result, page_inner_arc);
        if let JsEvalResult::Str(ref mut s) = eval_result {
            scrub_known_secrets(&inner.secret_store, s);
        }
        Ok(eval_result)
    }

    /// Return the first element in the document matching `selector`, or `null`.
    ///
    /// Equivalent to `document.querySelector(selector)`.
    #[qjs(rename = "$")]
    pub async fn js_query_selector(&self, selector: String) -> JsResult<Option<ElementHandle>> {
        use chromiumoxide::cdp::js_protocol::runtime::RemoteObjectSubtype;
        let selector_json = serde_json::to_string(&selector).unwrap_or_else(|_| "\"\"".to_string());
        let js = format!("document.querySelector({selector_json})");
        let result = self.evaluate_in_active_context(js).await?;
        match result {
            JsEvalResult::ElementHandleResult(eh) => Ok(Some(eh)),
            JsEvalResult::Json(ref s) if s == "null" => Ok(None),
            // querySelector returns null (subtype "null") when nothing found.
            // remote_object_to_eval_result maps that to Json("null").
            _ => {
                // If the result was a handle with Node subtype it's already
                // caught above; anything else means no match.
                let _ = RemoteObjectSubtype::Null; // keep import happy
                Ok(None)
            }
        }
    }

    /// Return all elements in the document matching `selector`.
    ///
    /// Equivalent to `Array.from(document.querySelectorAll(selector))`.
    #[qjs(rename = "$$")]
    pub async fn js_query_selector_all(&self, selector: String) -> JsResult<Vec<ElementHandle>> {
        use chromiumoxide::cdp::js_protocol::runtime::{CallArgument, EvaluateParams};
        let inner = self.inner.lock().await;
        let page_inner_arc = self.inner.clone();

        // Evaluate `Array.from(document.querySelectorAll(sel))` with
        // returnByValue:false so we get the array as a remote object.
        let sel_json = serde_json::to_string(&selector).unwrap_or_else(|_| "\"\"".to_string());
        let expr = format!("Array.from(document.querySelectorAll({sel_json}))");

        let (array_obj, session_id_opt) = if let Some(frame_id) = &inner.target_frame_id {
            let (ctx_id, session_id) =
                wait_for_frame_execution_target(&inner.page, frame_id.clone())
                    .await
                    .map_err(|e| js_err(format!("$$({selector}) frame target: {e}")))?;
            let eval = EvaluateParams::builder()
                .expression(expr)
                .context_id(ctx_id)
                .await_promise(false)
                .return_by_value(false)
                .build()
                .map_err(|e| js_err(format!("$$({selector}) params: {e}")))?;
            let res = inner
                .page
                .evaluate_expression_with_session(eval, session_id.clone())
                .await
                .map_err(|e| js_err(format!("$$({selector}) eval: {e}")))?;
            (res.object().clone(), Some(session_id))
        } else {
            let eval = EvaluateParams::builder()
                .expression(expr)
                .await_promise(false)
                .return_by_value(false)
                .build()
                .map_err(|e| js_err(format!("$$({selector}) params: {e}")))?;
            let res = inner
                .page
                .evaluate_expression(eval)
                .await
                .map_err(|e| js_err(format!("$$({selector}) eval: {e}")))?;
            (res.object().clone(), None)
        };
        let _ = (
            CallArgument {
                value: None,
                unserializable_value: None,
                object_id: None,
            },
            session_id_opt,
        );

        let array_id = match array_obj.object_id {
            Some(id) => id,
            None => return Ok(vec![]),
        };
        collect_element_handles_from_array(&inner.page, array_id, page_inner_arc)
            .await
            .map_err(|e| js_err(format!("$$({selector}) collect: {e}")))
    }

    /// Take a screenshot and return image bytes as Uint8Array.
    pub async fn screenshot<'js>(
        &self,
        ctx: Ctx<'js>,
        options: Opt<Value<'js>>,
    ) -> JsResult<TypedArray<'js, u8>> {
        let parsed = parse_screenshot_options(options.0.as_ref(), true)?;
        let mask_clips = {
            let mut clips = Vec::with_capacity(parsed.mask_locators.len());
            for mask in &parsed.mask_locators {
                clips.push(mask.screenshot_clip().await?);
            }
            clips
        };
        let path = {
            let inner = self.inner.lock().await;
            resolve_screenshot_output_path(&inner.download_dir, parsed.path.as_deref())?
        };
        let bytes =
            run_screenshot_capture(self.inner.clone(), &parsed, None, &mask_clips, path).await?;
        TypedArray::new_copy(ctx, bytes).map_err(|e| js_err(format!("Page.screenshot failed: {e}")))
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
        let page = PageApi::new(self.page_inner.clone());
        let tabs = page.fetch_open_tabs().await?;
        let mut out = Vec::with_capacity(tabs.len());
        for tab in tabs {
            out.push(build_page_api_from_template(&self.page_inner, tab.page).await);
        }
        Ok(out)
    }

    /// Playwright-style event waiter for Browser.
    ///
    /// Currently supports only `page`.
    #[qjs(rename = "waitForEvent")]
    pub async fn js_wait_for_event<'js>(
        &self,
        ctx: Ctx<'js>,
        event: String,
        options_or_predicate: Opt<Value<'js>>,
    ) -> JsResult<PageApi> {
        let normalized = event.trim().to_ascii_lowercase();
        if normalized != "page" {
            return Err(js_err(format!(
                "browser.waitForEvent currently supports only \"page\" (got {event})"
            )));
        }
        let options = parse_wait_for_event_options(
            &ctx,
            options_or_predicate.0.as_ref(),
            "browser.waitForEvent",
        )?;
        self.wait_for_page_event(&ctx, &options).await
    }
}

impl PageApi {
    /// Evaluate `expression` in the active frame context (or the main frame if none is set).
    ///
    /// Uses `returnByValue: false` so non-serialisable results (DOM nodes, functions, …)
    /// come back as remote-object handles rather than `undefined`.
    /// Secret string values in the result are scrubbed to `[REDACTED]`.
    async fn evaluate_in_active_context(&self, expression: String) -> JsResult<JsEvalResult> {
        use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
        let (page, frame_id, secret_store) = {
            let inner = self.inner.lock().await;
            (
                inner.page.clone(),
                inner.target_frame_id.clone(),
                inner.secret_store.clone(),
            )
        };
        let page_inner_arc = self.inner.clone();
        if let Some(frame_id) = frame_id {
            let (context_id, session_id) = wait_for_frame_execution_target(&page, frame_id.clone())
                .await
                .map_err(|e| js_err(format!("failed to get frame target: {e}")))?;
            let eval = EvaluateParams::builder()
                .expression(expression.clone())
                .context_id(context_id)
                .await_promise(true)
                .return_by_value(false)
                .build()
                .map_err(|e| js_err(format!("evaluate invalid params: {e}")))?;
            let result = match page
                .evaluate_expression_with_session(eval, session_id.clone())
                .await
            {
                Ok(result) => result,
                Err(err) => {
                    let err_text = err.to_string();
                    if !is_transport_disconnected_error(&err_text) {
                        return Err(js_err(format!("evaluate failed: {err_text}")));
                    }
                    let refreshed = self.refresh_page_handle().await.map_err(|refresh_err| {
                        js_err(format_browser_error(
                            "evaluate page refresh failed",
                            &refresh_err,
                        ))
                    })?;
                    let (refreshed_context_id, refreshed_session_id) =
                        wait_for_frame_execution_target(&refreshed, frame_id.clone())
                            .await
                            .map_err(|e| {
                                js_err(format!(
                                    "failed to get frame target after page refresh: {e}"
                                ))
                            })?;
                    let retry_eval = EvaluateParams::builder()
                        .expression(expression)
                        .context_id(refreshed_context_id)
                        .await_promise(true)
                        .return_by_value(false)
                        .build()
                        .map_err(|e| js_err(format!("evaluate invalid params: {e}")))?;
                    refreshed
                        .evaluate_expression_with_session(retry_eval, refreshed_session_id)
                        .await
                        .map_err(|retry_err| {
                            js_err(format_browser_error(
                                "evaluate failed",
                                &retry_err.to_string(),
                            ))
                        })?
                }
            };
            let mut eval_result =
                remote_object_to_eval_result(result.object().clone(), page_inner_arc);
            if let JsEvalResult::Str(ref mut s) = eval_result {
                scrub_known_secrets(&secret_store, s);
            }
            Ok(eval_result)
        } else {
            let eval = EvaluateParams::builder()
                .expression(expression.clone())
                .await_promise(true)
                .return_by_value(false)
                .build()
                .map_err(|e| js_err(format!("evaluate invalid params: {e}")))?;
            let result = match page.evaluate_expression(eval).await {
                Ok(result) => result,
                Err(err) => {
                    let err_text = err.to_string();
                    if !is_transport_disconnected_error(&err_text) {
                        return Err(js_err(format!("evaluate failed: {err_text}")));
                    }
                    let refreshed = self.refresh_page_handle().await.map_err(|refresh_err| {
                        js_err(format_browser_error(
                            "evaluate page refresh failed",
                            &refresh_err,
                        ))
                    })?;
                    let retry_eval = EvaluateParams::builder()
                        .expression(expression)
                        .await_promise(true)
                        .return_by_value(false)
                        .build()
                        .map_err(|e| js_err(format!("evaluate invalid params: {e}")))?;
                    refreshed
                        .evaluate_expression(retry_eval)
                        .await
                        .map_err(|retry_err| {
                            js_err(format_browser_error(
                                "evaluate failed",
                                &retry_err.to_string(),
                            ))
                        })?
                }
            };
            let mut eval_result =
                remote_object_to_eval_result(result.object().clone(), page_inner_arc);
            if let JsEvalResult::Str(ref mut s) = eval_result {
                scrub_known_secrets(&secret_store, s);
            }
            Ok(eval_result)
        }
    }

    async fn fetch_open_tabs(&self) -> JsResult<Vec<OpenTab>> {
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

        loop {
            let tabs = self.fetch_open_tabs().await?;
            if !tabs.iter().any(|tab| tab.target_id == opener_target) {
                return Err(js_err(
                    "TargetClosedError: page was closed while waiting for popup".to_string(),
                ));
            }
            if let Some(popup_tab) = tabs.iter().find(|tab| {
                tab.target_id != opener_target
                    && tab.opener_target_id.as_deref() == Some(opener_target.as_str())
            }) {
                return Ok(build_page_api_from_template(&self.inner, popup_tab.page.clone()).await);
            }
            if let Some(popup_tab) = tabs.iter().find(|tab| tab.target_id != opener_target) {
                return Ok(build_page_api_from_template(&self.inner, popup_tab.page.clone()).await);
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

    async fn wait_for_goto_wait_until(
        &self,
        wait_until: &str,
        deadline: Option<tokio::time::Instant>,
        timeout_ms: u64,
        url: &str,
    ) -> JsResult<()> {
        match wait_until {
            "commit" => Ok(()),
            "networkidle" => {
                let page = {
                    let inner = self.inner.lock().await;
                    inner.page.clone()
                };
                if let Some(remaining) = goto_remaining(deadline, timeout_ms, url)? {
                    tokio::time::timeout(remaining, page.wait_for_network_idle())
                        .await
                        .map_err(|_| goto_timeout_err(timeout_ms, url))?
                        .map(|_| ())
                        .map_err(|e| js_err(format!("waitForLoadState(networkidle) failed: {e}")))
                } else {
                    page.wait_for_network_idle()
                        .await
                        .map(|_| ())
                        .map_err(|e| js_err(format!("waitForLoadState(networkidle) failed: {e}")))
                }
            }
            "load" | "domcontentloaded" => loop {
                let ready = match wait_until {
                    "load" => self.ready_state_is_complete().await?,
                    "domcontentloaded" => self.ready_state_is_interactive_or_complete().await?,
                    _ => false,
                };
                if ready {
                    return Ok(());
                }
                if let Some(limit) = deadline {
                    if tokio::time::Instant::now() >= limit {
                        return Err(goto_timeout_err(timeout_ms, url));
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
            },
            _ => Err(js_err(format!(
                "waitUntil: expected one of (load|domcontentloaded|networkidle|commit), got {wait_until}"
            ))),
        }
    }

    async fn ensure_not_browser_error_page(&self, requested_url: &str) -> JsResult<()> {
        let observed = self.current_url().await?;
        if is_browser_error_url(&observed) {
            return Err(js_err(format!(
                "goto failed: navigation to \"{requested_url}\" failed (landed on {observed})"
            )));
        }
        Ok(())
    }

    async fn eval_string(&self, expression: String, _method_name: &str) -> JsResult<String> {
        let result = self.evaluate_in_active_context(expression).await?;
        Ok(result.into_string_repr())
    }

    async fn eval_bool(&self, expression: String, _method_name: &str) -> JsResult<bool> {
        let text = self.eval_string(expression, _method_name).await?;
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

    fn request_api_from_entry(&self, entry: RequestCaptureItem) -> RequestApi {
        RequestApi {
            request_id: entry.request_id,
            raw_request_id: entry.raw_request_id,
            url: entry.url,
            method: entry.method,
            resource_type: entry.resource_type,
            headers: entry.headers,
            frame_id: entry.frame_id,
            is_navigation_request: entry.is_navigation_request,
            post_data: entry.post_data,
            redirected_from: entry.redirected_from,
            error: entry.error,
            finished: entry.finished,
            timing: entry.timing,
            page_api: self.clone(),
        }
    }

    fn response_api_from_entry(&self, entry: NetworkRequest) -> ResponseApi {
        ResponseApi {
            request_id: entry.request_id.clone(),
            url: entry.url,
            status: entry.status,
            ok: entry.status == 0 || (200..300).contains(&entry.status),
            method: entry.method,
            status_text: entry.status_text,
            headers: entry.headers,
            frame_id: entry.frame_id,
            from_service_worker: entry.from_service_worker,
            error: entry.error,
            finished: entry.finished,
            server_addr: entry.server_addr,
            security_details: entry.security_details,
            request_id_raw: entry.request_id_raw,
            page_api: self.clone(),
        }
    }

    async fn ensure_request_capture(&self) -> JsResult<Arc<Mutex<Vec<RequestCaptureItem>>>> {
        let mut guard = self.request_capture.lock().await;
        let had_previous = guard.is_some();
        if let Some(state) = guard.as_ref() {
            if !state.task.is_finished() {
                return Ok(self.request_entries.clone());
            }
        }

        if let Some(previous) = guard.take() {
            previous.task.abort();
        }

        let page = {
            let inner = self.inner.lock().await;
            inner.page.clone()
        };

        use chromiumoxide::cdp::browser_protocol::network::{
            EnableParams, EventLoadingFailed, EventLoadingFinished, EventRequestWillBeSent,
            EventRequestWillBeSentExtraInfo,
        };
        if let Err(e) = page.execute(EnableParams::default()).await {
            // CDP -32001 ("Session with given id not found") means the target
            // was closed between when we obtained the page handle and now.
            // Surface TargetClosedError so callers see the right error type;
            // if the page is actually still alive, propagate the original error.
            self.ensure_page_waiter_alive("network domain setup")
                .await?;
            return Err(js_err(format!("failed to enable Network domain: {e}")));
        }

        let events = page
            .event_listener::<EventRequestWillBeSent>()
            .await
            .map_err(|e| js_err(format!("failed to attach request listener: {e}")))?;
        let extra_events = page
            .event_listener::<EventRequestWillBeSentExtraInfo>()
            .await
            .map_err(|e| js_err(format!("failed to attach request extra-info listener: {e}")))?;
        let loading_finished_events = page
            .event_listener::<EventLoadingFinished>()
            .await
            .map_err(|e| js_err(format!("failed to attach request finished listener: {e}")))?;
        let loading_failed_events = page
            .event_listener::<EventLoadingFailed>()
            .await
            .map_err(|e| js_err(format!("failed to attach request failed listener: {e}")))?;

        if had_previous {
            let mut entries = self.request_entries.lock().await;
            entries.clear();
        }
        if had_previous {
            if let Ok(mut timings) = self.request_timings.lock() {
                timings.clear();
            }
            if let Ok(mut ids) = self.raw_request_current_ids.lock() {
                ids.clear();
            }
            if let Ok(mut pending) = self.pending_request_lifecycle.lock() {
                pending.clear();
            }
            self.next_request_id.store(1, Ordering::Relaxed);
        }

        let entries_for_task = self.request_entries.clone();
        let response_entries_for_task = self.response_entries.clone();
        let request_timings_for_task = self.request_timings.clone();
        let raw_request_current_ids = self.raw_request_current_ids.clone();
        let next_request_id = self.next_request_id.clone();
        let request_waiters = self.request_waiters.clone();
        let response_waiters = self.response_waiters.clone();
        let request_lifecycle_waiters = self.request_lifecycle_waiters.clone();
        let pending_request_lifecycle = self.pending_request_lifecycle.clone();
        let task = tokio::spawn(async move {
            use futures::StreamExt;
            tokio::pin!(events);
            tokio::pin!(extra_events);
            tokio::pin!(loading_finished_events);
            tokio::pin!(loading_failed_events);

            loop {
                tokio::select! {
                    ev = events.next() => {
                        let Some(ev) = ev else {
                            break;
                        };
                        let raw_request_id = ev.request_id.as_ref().to_string();
                        let (request_id, previous_request_id) = {
                            let mut ids = raw_request_current_ids
                                .lock()
                                .unwrap_or_else(|err| err.into_inner());
                            allocate_request_hop(
                                &raw_request_id,
                                ev.redirect_response.is_some(),
                                &mut ids,
                                &next_request_id,
                            )
                        };
                        let mut item = RequestCaptureItem {
                            request_id: request_id.clone(),
                            raw_request_id: raw_request_id.clone(),
                            url: ev.request.url.clone(),
                            method: ev.request.method.clone(),
                            headers: headers_to_map(Some(&ev.request.headers)),
                            resource_type: ev
                                .r#type
                                .as_ref()
                                .map(|resource_type| resource_type.as_ref().to_ascii_lowercase())
                                .unwrap_or_else(|| "other".to_string()),
                            post_data: None,
                            frame_id: ev
                                .frame_id
                                .as_ref()
                                .map(|frame_id| frame_id.as_ref().to_string()),
                            is_navigation_request: is_navigation_request(&ev),
                            redirected_from: previous_request_id.clone(),
                            error: None,
                            finished: false,
                            timing: RequestTiming::default_playwright(),
                        };
                        let pending_lifecycle = {
                            let mut pending = pending_request_lifecycle
                                .lock()
                                .unwrap_or_else(|err| err.into_inner());
                            pending.remove(&raw_request_id)
                        };
                        apply_pending_request_lifecycle(&mut item, pending_lifecycle.as_ref());

                        if let (Some(previous_request_id), Some(redirect_response)) =
                            (previous_request_id, ev.redirect_response.as_ref())
                        {
                            let redirect_item = build_redirect_response_entry(
                                previous_request_id,
                                &item,
                                redirect_response,
                                (*ev.timestamp.inner() * 1000.0) as i64,
                            );
                            let mut response_guard = response_entries_for_task.lock().await;
                            response_guard.push(redirect_item.clone());
                            if response_guard.len() > 5_000 {
                                let drop_count = response_guard.len() - 5_000;
                                response_guard.drain(0..drop_count);
                            }
                            drop(response_guard);

                            let matched_waiters = {
                                let mut waiters = response_waiters.lock().await;
                                let mut matched = Vec::new();
                                let mut remaining = Vec::with_capacity(waiters.len());
                                for waiter in waiters.drain(..) {
                                    if url_waiter_matches(&redirect_item.url, &waiter.matcher) {
                                        matched.push(waiter);
                                    } else {
                                        remaining.push(waiter);
                                    }
                                }
                                *waiters = remaining;
                                matched
                            };

                            for waiter in matched_waiters {
                                let _ = waiter.sender.send(redirect_item.clone());
                            }
                        }

                        let mut guard = entries_for_task.lock().await;
                        guard.push(item.clone());
                        if guard.len() > 5_000 {
                            let drop_count = guard.len() - 5_000;
                            guard.drain(0..drop_count);
                        }
                        drop(guard);

                        if let Ok(mut timings) = request_timings_for_task.lock() {
                            timings.insert(
                                item.request_id.clone(),
                                RequestTiming::default_playwright(),
                            );
                        }
                        let matched_waiters = {
                            let mut waiters = request_waiters.lock().await;
                            let mut matched = Vec::new();
                            let mut remaining = Vec::with_capacity(waiters.len());
                            for waiter in waiters.drain(..) {
                                if url_waiter_matches(&item.url, &waiter.matcher) {
                                    matched.push(waiter);
                                } else {
                                    remaining.push(waiter);
                                }
                            }
                            *waiters = remaining;
                            matched
                        };

                        let matched_lifecycle_waiters = if let Some(state) = pending_lifecycle {
                            let event = pending_request_lifecycle_event(&state);
                            let mut waiters = request_lifecycle_waiters.lock().await;
                            let mut matched = Vec::new();
                            let mut remaining = Vec::with_capacity(waiters.len());
                            for waiter in waiters.drain(..) {
                                if waiter.event == event {
                                    matched.push(waiter);
                                } else {
                                    remaining.push(waiter);
                                }
                            }
                            *waiters = remaining;
                            matched
                        } else {
                            Vec::new()
                        };

                        for waiter in matched_waiters {
                            let entries_for_waiter = entries_for_task.clone();
                            let fallback = item.clone();
                            tokio::spawn(async move {
                                let latest =
                                    PageApi::settle_request_entry(&entries_for_waiter, fallback)
                                        .await;
                                let _ = waiter.sender.send(latest);
                            });
                        }

                        for waiter in matched_lifecycle_waiters {
                            let _ = waiter.sender.send(item.clone());
                        }
                    }
                    ev = extra_events.next() => {
                        let Some(ev) = ev else {
                            break;
                        };
                        let raw_request_id = ev.request_id.as_ref().to_string();
                        let request_id = PageApi::settle_request_id_for_raw(
                            &entries_for_task,
                            &raw_request_current_ids,
                            &raw_request_id,
                        ).await;
                        let merged_headers = headers_to_map(Some(&ev.headers));
                        let mut guard = entries_for_task.lock().await;
                        if let Some(entry) = guard
                            .iter_mut()
                            .rev()
                            .find(|entry| entry.request_id == request_id)
                        {
                            entry.headers = merged_headers;
                        }
                    }
                    ev = loading_finished_events.next() => {
                        let Some(ev) = ev else {
                            break;
                        };
                        let raw_request_id = ev.request_id.as_ref().to_string();
                        let request_id = PageApi::settle_request_id_for_raw(
                            &entries_for_task,
                            &raw_request_current_ids,
                            &raw_request_id,
                        ).await;
                        let mut guard = entries_for_task.lock().await;
                        let latest = if let Some(entry) = guard
                            .iter_mut()
                            .rev()
                            .find(|entry| entry.request_id == request_id)
                        {
                            entry.finished = true;
                            entry.error = None;
                            Some(entry.clone())
                        } else {
                            None
                        };
                        drop(guard);

                        if let Some(latest) = latest {
                            let matched_waiters = {
                                let mut waiters = request_lifecycle_waiters.lock().await;
                                let mut matched = Vec::new();
                                let mut remaining = Vec::with_capacity(waiters.len());
                                for waiter in waiters.drain(..) {
                                    if waiter.event == RequestLifecycleEvent::Finished {
                                        matched.push(waiter);
                                    } else {
                                        remaining.push(waiter);
                                    }
                                }
                                *waiters = remaining;
                                matched
                            };

                            for waiter in matched_waiters {
                                let _ = waiter.sender.send(latest.clone());
                            }
                        } else {
                            let mut pending = pending_request_lifecycle
                                .lock()
                                .unwrap_or_else(|err| err.into_inner());
                            pending.insert(raw_request_id, PendingRequestLifecycleState::Finished);
                        }
                    }
                    ev = loading_failed_events.next() => {
                        let Some(ev) = ev else {
                            break;
                        };
                        let raw_request_id = ev.request_id.as_ref().to_string();
                        let request_id = PageApi::settle_request_id_for_raw(
                            &entries_for_task,
                            &raw_request_current_ids,
                            &raw_request_id,
                        ).await;
                        let error_text = ev.error_text.clone();
                        let mut guard = entries_for_task.lock().await;
                        let latest = if let Some(entry) = guard
                            .iter_mut()
                            .rev()
                            .find(|entry| entry.request_id == request_id)
                        {
                            entry.finished = true;
                            entry.error = Some(error_text.clone());
                            Some(entry.clone())
                        } else {
                            None
                        };
                        drop(guard);

                        if let Some(latest) = latest {
                            let matched_waiters = {
                                let mut waiters = request_lifecycle_waiters.lock().await;
                                let mut matched = Vec::new();
                                let mut remaining = Vec::with_capacity(waiters.len());
                                for waiter in waiters.drain(..) {
                                    if waiter.event == RequestLifecycleEvent::Failed {
                                        matched.push(waiter);
                                    } else {
                                        remaining.push(waiter);
                                    }
                                }
                                *waiters = remaining;
                                matched
                            };

                            for waiter in matched_waiters {
                                let _ = waiter.sender.send(latest.clone());
                            }
                        } else {
                            let mut pending = pending_request_lifecycle
                                .lock()
                                .unwrap_or_else(|err| err.into_inner());
                            pending.insert(
                                raw_request_id,
                                PendingRequestLifecycleState::Failed(error_text),
                            );
                        }
                    }
                }
            }
        });

        *guard = Some(RequestCaptureState { task });
        Ok(self.request_entries.clone())
    }

    async fn ensure_response_capture(&self) -> JsResult<Arc<Mutex<Vec<NetworkRequest>>>> {
        let mut guard = self.response_capture.lock().await;
        let had_previous = guard.is_some();
        if let Some(state) = guard.as_ref() {
            if !state.task.is_finished() {
                return Ok(self.response_entries.clone());
            }
        }

        if let Some(previous) = guard.take() {
            previous.task.abort();
        }

        let page = {
            let inner = self.inner.lock().await;
            inner.page.clone()
        };

        use chromiumoxide::cdp::browser_protocol::network::{
            EnableParams, EventLoadingFailed, EventLoadingFinished, EventResponseReceived,
            EventResponseReceivedExtraInfo,
        };
        if let Err(e) = page.execute(EnableParams::default()).await {
            // CDP -32001 ("Session with given id not found") means the target
            // was closed between when we obtained the page handle and now.
            // Surface TargetClosedError so callers see the right error type;
            // if the page is actually still alive, propagate the original error.
            self.ensure_page_waiter_alive("network domain setup")
                .await?;
            return Err(js_err(format!("failed to enable Network domain: {e}")));
        }

        let events = page
            .event_listener::<EventResponseReceived>()
            .await
            .map_err(|e| js_err(format!("failed to attach response listener: {e}")))?;
        let loading_finished_events = page
            .event_listener::<EventLoadingFinished>()
            .await
            .map_err(|e| js_err(format!("failed to attach response finished listener: {e}")))?;
        let loading_failed_events = page
            .event_listener::<EventLoadingFailed>()
            .await
            .map_err(|e| js_err(format!("failed to attach response failed listener: {e}")))?;
        let extra_events = page
            .event_listener::<EventResponseReceivedExtraInfo>()
            .await
            .map_err(|e| {
                js_err(format!(
                    "failed to attach response extra-info listener: {e}"
                ))
            })?;

        if had_previous {
            let mut entries = self.response_entries.lock().await;
            entries.clear();
        }
        let entries_for_task = self.response_entries.clone();
        let request_entries_for_task = self.ensure_request_capture().await?;
        let request_timings_for_task = self.request_timings.clone();
        let raw_request_current_ids = self.raw_request_current_ids.clone();
        let response_waiters = self.response_waiters.clone();
        let task = tokio::spawn(async move {
            use futures::StreamExt;
            tokio::pin!(events);
            tokio::pin!(loading_finished_events);
            tokio::pin!(loading_failed_events);
            tokio::pin!(extra_events);

            loop {
                tokio::select! {
                    ev = events.next() => {
                        let Some(ev) = ev else {
                            break;
                        };
                        let status = ev.response.status;
                        let method = network_method_from_headers(ev.response.request_headers.as_ref());
                        let ts = (*ev.timestamp.inner() * 1000.0) as i64;
                        // Keep the field mapping aligned with Playwright's ResourceTiming shape.
                        let timing = response_timing_to_request_timing(ev.response.timing.as_ref());
                        let raw_request_id = ev.request_id.as_ref().to_string();
                        let Some(request_id) = PageApi::resolve_response_request_id(
                            &request_entries_for_task,
                            &entries_for_task,
                            &raw_request_current_ids,
                            &raw_request_id,
                            status,
                        ).await else {
                            continue;
                        };
                        let item = NetworkRequest {
                            request_id: request_id.clone(),
                            url: ev.response.url.clone(),
                            status,
                            ok: status == 0 || (200..300).contains(&status),
                            method,
                            status_text: ev.response.status_text.clone(),
                            headers: headers_to_map(Some(&ev.response.headers)),
                            frame_id: ev
                                .frame_id
                                .as_ref()
                                .map(|frame_id| frame_id.as_ref().to_string()),
                            from_service_worker: ev.response.from_service_worker.unwrap_or(false),
                            ts,
                            error: None,
                            finished: false,
                            timing: timing.clone(),
                            server_addr: remote_addr_from_response(&ev.response),
                            security_details: response_security_details(&ev.response),
                            request_id_raw: Some(ev.request_id.clone()),
                        };

                        let mut guard = entries_for_task.lock().await;
                        guard.push(item);
                        if guard.len() > 5_000 {
                            let drop_count = guard.len() - 5_000;
                            guard.drain(0..drop_count);
                        }
                        let latest = guard.last().cloned();
                        drop(guard);

                        let request_id = request_id.clone();
                        let mut request_guard = request_entries_for_task.lock().await;
                        if let Some(entry) = request_guard
                            .iter_mut()
                            .rev()
                            .find(|entry| entry.request_id == request_id)
                        {
                            entry.timing = timing.clone();
                        }
                        drop(request_guard);

                        if let Ok(mut timings) = request_timings_for_task.lock() {
                            timings.insert(request_id, timing);
                        }

                        if let Some(latest) = latest {
                            let matched_waiters = {
                                let mut waiters = response_waiters.lock().await;
                                let mut matched = Vec::new();
                                let mut remaining = Vec::with_capacity(waiters.len());
                                for waiter in waiters.drain(..) {
                                    if url_waiter_matches(&latest.url, &waiter.matcher) {
                                        matched.push(waiter);
                                    } else {
                                        remaining.push(waiter);
                                    }
                                }
                                *waiters = remaining;
                                matched
                            };

                            for waiter in matched_waiters {
                                let _ = waiter.sender.send(latest.clone());
                            }
                        }
                    }
                    ev = loading_finished_events.next() => {
                        let Some(ev) = ev else {
                            break;
                        };
                        let request_id = ev.request_id.as_ref().to_string();
                        let request_id = PageApi::settle_request_id_for_raw(
                            &request_entries_for_task,
                            &raw_request_current_ids,
                            &request_id,
                        ).await;
                        let mut guard = entries_for_task.lock().await;
                        if let Some(entry) = guard
                            .iter_mut()
                            .rev()
                            .find(|entry| entry.request_id == request_id)
                        {
                            entry.finished = true;
                            entry.error = None;
                            entry.timing.response_end = timing_response_end_from_timestamp(
                                &entry.timing,
                                *ev.timestamp.inner(),
                            );
                        }
                        drop(guard);

                        let mut request_guard = request_entries_for_task.lock().await;
                        if let Some(entry) = request_guard
                            .iter_mut()
                            .rev()
                            .find(|entry| entry.request_id == request_id)
                        {
                            entry.finished = true;
                            entry.error = None;
                            entry.timing.response_end = timing_response_end_from_timestamp(
                                &entry.timing,
                                *ev.timestamp.inner(),
                            );
                        }
                        drop(request_guard);

                        if let Ok(mut timings) = request_timings_for_task.lock() {
                            if let Some(timing) = timings.get_mut(&request_id) {
                                timing.response_end = timing_response_end_from_timestamp(
                                    timing,
                                    *ev.timestamp.inner(),
                                );
                            }
                        }
                    }
                    ev = loading_failed_events.next() => {
                        let Some(ev) = ev else {
                            break;
                        };
                        let request_id = ev.request_id.as_ref().to_string();
                        let request_id = PageApi::settle_request_id_for_raw(
                            &request_entries_for_task,
                            &raw_request_current_ids,
                            &request_id,
                        ).await;
                        let error_text = ev.error_text.clone();
                        let mut guard = entries_for_task.lock().await;
                        if let Some(entry) = guard
                            .iter_mut()
                            .rev()
                            .find(|entry| entry.request_id == request_id)
                        {
                            entry.finished = true;
                            entry.error = Some(error_text);
                            entry.timing.response_end = timing_response_end_from_timestamp(
                                &entry.timing,
                                *ev.timestamp.inner(),
                            );
                        }
                        drop(guard);

                        let mut request_guard = request_entries_for_task.lock().await;
                        if let Some(entry) = request_guard
                            .iter_mut()
                            .rev()
                            .find(|entry| entry.request_id == request_id)
                        {
                            entry.finished = true;
                            entry.error = Some(ev.error_text.clone());
                            entry.timing.response_end = timing_response_end_from_timestamp(
                                &entry.timing,
                                *ev.timestamp.inner(),
                            );
                        }
                        drop(request_guard);

                        if let Ok(mut timings) = request_timings_for_task.lock() {
                            if let Some(timing) = timings.get_mut(&request_id) {
                                timing.response_end = timing_response_end_from_timestamp(
                                    timing,
                                    *ev.timestamp.inner(),
                                );
                            }
                        }
                    }
                    ev = extra_events.next() => {
                        let Some(ev) = ev else {
                            break;
                        };
                        let request_id = ev.request_id.as_ref().to_string();
                        let request_id = PageApi::settle_request_id_for_raw(
                            &request_entries_for_task,
                            &raw_request_current_ids,
                            &request_id,
                        ).await;
                        let merged_headers = headers_to_map(Some(&ev.headers));
                        let mut guard = entries_for_task.lock().await;
                        if let Some(entry) = guard
                            .iter_mut()
                            .rev()
                            .find(|entry| entry.request_id == request_id)
                        {
                            entry.headers = merged_headers;
                        }
                    }
                }
            }
        });

        *guard = Some(ResponseCaptureState { task });
        Ok(self.response_entries.clone())
    }
}

impl BrowserApi {
    async fn wait_for_page_event<'js>(
        &self,
        ctx: &Ctx<'js>,
        options: &EventWaitOptions,
    ) -> JsResult<PageApi> {
        let watcher = PageApi::new(self.page_inner.clone());
        let baseline_tabs = watcher.fetch_open_tabs().await?;
        let mut seen_ids = baseline_tabs
            .into_iter()
            .map(|tab| tab.target_id)
            .collect::<BTreeSet<_>>();
        let started_at = tokio::time::Instant::now();

        loop {
            let tabs = watcher.fetch_open_tabs().await?;
            for tab in tabs {
                if seen_ids.contains(&tab.target_id) {
                    continue;
                }
                seen_ids.insert(tab.target_id.clone());
                let candidate = build_page_api_from_template(&self.page_inner, tab.page).await;
                if page_matches_event_predicate(ctx, options.predicate.as_ref(), &candidate).await?
                {
                    return Ok(candidate);
                }
            }

            let _ = remaining_timeout_ms(options.timeout_ms, started_at, "browser page event")?;
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
        target_id: page.target_id().as_ref().to_string(),
        page,
        browser: template.browser.clone(),
        secret_store: template.secret_store.clone(),
        declared_secrets: template.declared_secrets.clone(),
        download_dir: template.download_dir.clone(),
        target_frame_id: None,
    };
    PageApi::new(Arc::new(Mutex::new(page_inner)))
}

/// Call a JS function on a CDP remote object by `objectId`.
///
/// If `return_by_value` is `true`, the result is serialised to JSON and
/// returned in `RemoteObject.value`.  If `false`, the result is a new remote
/// object (useful for chaining handles).
async fn call_function_on_handle(
    page: &chromiumoxide::Page,
    object_id: &str,
    function_declaration: &str,
    args: &[chromiumoxide::cdp::js_protocol::runtime::CallArgument],
    return_by_value: bool,
) -> Result<chromiumoxide::cdp::js_protocol::runtime::RemoteObject, String> {
    use chromiumoxide::cdp::js_protocol::runtime::CallFunctionOnParams;
    let mut builder = CallFunctionOnParams::builder()
        .function_declaration(function_declaration)
        .object_id(object_id.to_string())
        .return_by_value(return_by_value)
        .await_promise(true);
    for arg in args {
        builder = builder.argument(arg.clone());
    }
    let params = builder
        .build()
        .map_err(|e| format!("callFunctionOn build failed: {e}"))?;
    let response = page
        .execute(params)
        .await
        .map_err(|e| format!("callFunctionOn CDP failed: {e}"))?;
    if let Some(exc) = &response.result.exception_details {
        let msg = exc
            .exception
            .as_ref()
            .and_then(|o| o.description.as_deref())
            .unwrap_or(&exc.text);
        return Err(msg.to_string());
    }
    Ok(response.result.result)
}

/// Enumerate an array handle returned by CDP and collect its DOM-element items
/// as `ElementHandle` instances.
///
/// Uses `Runtime.getProperties` with `ownProperties: true` to get
/// integer-indexed properties, which each correspond to an array element.
async fn collect_element_handles_from_array(
    page: &chromiumoxide::Page,
    array_id: chromiumoxide::cdp::js_protocol::runtime::RemoteObjectId,
    page_inner: Arc<Mutex<PageInner>>,
) -> Result<Vec<ElementHandle>, String> {
    use chromiumoxide::cdp::js_protocol::runtime::GetPropertiesParams;
    let params = GetPropertiesParams::builder()
        .object_id(array_id)
        .own_properties(true)
        .build()
        .map_err(|e| format!("getProperties build failed: {e}"))?;
    let response = page
        .execute(params)
        .await
        .map_err(|e| format!("getProperties CDP failed: {e}"))?;
    let mut elements = Vec::new();
    for prop in &response.result.result {
        // Only numeric-indexed array elements (skip "length", etc.)
        if prop.name.parse::<u64>().is_err() {
            continue;
        }
        let Some(remote) = &prop.value else {
            continue;
        };
        let Some(object_id) = &remote.object_id else {
            continue;
        };
        elements.push(ElementHandle {
            object_id: object_id.as_ref().to_string(),
            description: remote.description.clone().unwrap_or_default(),
            page_inner: page_inner.clone(),
        });
    }
    Ok(elements)
}

/// Convert a CDP `RemoteObject` to a `JsEvalResult`.
///
/// DOM nodes (subtype `"node"`) become `ElementHandle`, other remote objects
/// with an `objectId` become `JsHandle`, and serialisable primitives/objects
/// are returned as their JSON representation (or a raw string for JS strings).
fn remote_object_to_eval_result(
    obj: chromiumoxide::cdp::js_protocol::runtime::RemoteObject,
    page_inner: Arc<Mutex<PageInner>>,
) -> JsEvalResult {
    use chromiumoxide::cdp::js_protocol::runtime::{RemoteObjectSubtype, RemoteObjectType};

    // --- Serialisable value path (value field is populated) ---
    if let Some(value) = obj.value {
        return match value {
            serde_json::Value::String(s) => JsEvalResult::Str(s),
            other => {
                let json = serde_json::to_string(&other).unwrap_or_else(|_| other.to_string());
                JsEvalResult::Json(json)
            }
        };
    }

    // --- Unserializable JS number (NaN, Infinity, -Infinity, -0) ---
    if let Some(unserializable) = obj.unserializable_value {
        return JsEvalResult::Unserializable(unserializable.as_ref().to_string());
    }

    // --- Remote object handle ---
    if let Some(object_id) = obj.object_id {
        let description = obj.description.unwrap_or_default();
        let oid = object_id.as_ref().to_string();
        if obj.subtype == Some(RemoteObjectSubtype::Node) {
            return JsEvalResult::ElementHandleResult(ElementHandle {
                object_id: oid,
                description,
                page_inner,
            });
        }
        return JsEvalResult::JsHandleResult(JsHandle {
            object_id: oid,
            description,
            page_inner,
        });
    }

    // --- Null (subtype "null", type "object", no objectId) ---
    if obj.subtype == Some(RemoteObjectSubtype::Null) {
        return JsEvalResult::Json("null".to_string());
    }

    // --- Undefined ---
    if obj.r#type == RemoteObjectType::Undefined {
        return JsEvalResult::Undefined;
    }

    JsEvalResult::Undefined
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
    // Usernames are readable without biometric and are the most likely to
    // appear in page-evaluation results.  Passwords are typed into form
    // fields and rarely returned by JS evaluation.
    if let Ok(usernames) = secret_store.all_usernames() {
        for username in &usernames {
            if !username.is_empty() {
                *text = text.replace(username.as_str(), "[REDACTED]");
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

// Keep this matcher parsing aligned with Playwright's waitForRequest/waitForResponse
// contract in local notes at docs/playwright.md and playwright-core client/page.ts.
fn parse_wait_for_network_matcher<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
    api_name: &str,
) -> JsResult<JsNetworkMatcher> {
    if value.is_function() {
        let function = value
            .into_function()
            .ok_or_else(|| js_err(format!("{api_name} matcher was not callable")))?;
        return Ok(JsNetworkMatcher::Predicate(Persistent::save(ctx, function)));
    }
    if js_value_is_regexp(ctx, &value)? {
        return Ok(JsNetworkMatcher::RegExp(Persistent::save(ctx, value)));
    }
    if value.is_string() {
        let pattern = String::from_js(ctx, value)
            .map_err(|e| js_err(format!("{api_name} matcher string decode failed: {e}")))?;
        return Ok(JsNetworkMatcher::String(pattern));
    }
    Err(js_err(format!(
        "{api_name} matcher must be a string, RegExp, or predicate function"
    )))
}

fn js_value_is_regexp<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> JsResult<bool> {
    let detector: Function<'js> = ctx
        .eval("(value) => value instanceof RegExp")
        .map_err(|e| js_err(format!("failed to build RegExp detector: {e}")))?;
    detector
        .call((value.clone(),))
        .map_err(|e| js_err(format!("failed to evaluate RegExp matcher: {e}")))
}

fn js_regexp_matches<'js>(ctx: &Ctx<'js>, matcher: &Value<'js>, url: &str) -> JsResult<bool> {
    let tester: Function<'js> = ctx
        .eval("(matcher, url) => new RegExp(matcher.source, matcher.flags).test(url)")
        .map_err(|e| js_err(format!("failed to build RegExp tester: {e}")))?;
    tester
        .call((matcher.clone(), url.to_string()))
        .map_err(|e| js_err(format!("failed to test RegExp matcher: {e}")))
}

fn js_value_to_bool<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> JsResult<bool> {
    let coercer: Function<'js> = ctx
        .eval("(value) => Boolean(value)")
        .map_err(|e| js_err(format!("failed to build boolean coercer: {e}")))?;
    coercer
        .call((value,))
        .map_err(|e| js_err(format!("failed to coerce predicate result to boolean: {e}")))
}

async fn call_event_predicate<'js>(
    ctx: &Ctx<'js>,
    predicate: Option<&Persistent<Function<'static>>>,
    value: Value<'js>,
    predicate_name: &str,
) -> JsResult<bool> {
    let Some(predicate) = predicate else {
        return Ok(true);
    };
    let predicate = predicate
        .clone()
        .restore(ctx)
        .map_err(|e| js_err(format!("failed to restore {predicate_name} predicate: {e}")))?;
    let result: MaybePromise<'js> = predicate
        .call((value,))
        .map_err(|e| js_err(format!("{predicate_name} predicate threw: {e}")))?;
    let resolved = result
        .into_future::<Value<'js>>()
        .await
        .map_err(|e| js_err(format!("{predicate_name} predicate rejected: {e}")))?;
    js_value_to_bool(ctx, resolved)
}

async fn page_matches_event_predicate<'js>(
    ctx: &Ctx<'js>,
    predicate: Option<&Persistent<Function<'static>>>,
    page: &PageApi,
) -> JsResult<bool> {
    let page_value = Class::instance(ctx.clone(), page.clone())
        .map(|instance| instance.into_value())
        .map_err(|e| js_err(format!("failed to materialize page for predicate: {e}")))?;
    call_event_predicate(ctx, predicate, page_value, "page event").await
}

async fn request_matches_event_predicate<'js>(
    ctx: &Ctx<'js>,
    predicate: Option<&Persistent<Function<'static>>>,
    request: &RequestApi,
) -> JsResult<bool> {
    let request_value = Class::instance(ctx.clone(), request.clone())
        .map(|instance| instance.into_value())
        .map_err(|e| js_err(format!("failed to materialize request for predicate: {e}")))?;
    call_event_predicate(ctx, predicate, request_value, "request event").await
}

async fn response_matches_event_predicate<'js>(
    ctx: &Ctx<'js>,
    predicate: Option<&Persistent<Function<'static>>>,
    response: &ResponseApi,
) -> JsResult<bool> {
    let response_value = Class::instance(ctx.clone(), response.clone())
        .map(|instance| instance.into_value())
        .map_err(|e| js_err(format!("failed to materialize response for predicate: {e}")))?;
    call_event_predicate(ctx, predicate, response_value, "response event").await
}

async fn request_matches_js_matcher<'js>(
    ctx: &Ctx<'js>,
    matcher: &JsNetworkMatcher,
    request: &RequestApi,
) -> JsResult<bool> {
    match matcher {
        JsNetworkMatcher::String(pattern) => Ok(url_matches_pattern(&request.url, pattern)),
        JsNetworkMatcher::RegExp(regexp) => {
            let regexp = regexp
                .clone()
                .restore(ctx)
                .map_err(|e| js_err(format!("failed to restore request RegExp matcher: {e}")))?;
            js_regexp_matches(ctx, &regexp, &request.url)
        }
        JsNetworkMatcher::Predicate(predicate) => {
            let predicate = predicate
                .clone()
                .restore(ctx)
                .map_err(|e| js_err(format!("failed to restore request predicate matcher: {e}")))?;
            let request_value = Class::instance(ctx.clone(), request.clone())
                .map(|instance| instance.into_value())
                .map_err(|e| js_err(format!("failed to materialize request for predicate: {e}")))?;
            let result: MaybePromise<'js> = predicate
                .call((request_value,))
                .map_err(|e| js_err(format!("request predicate threw: {e}")))?;
            let resolved = result
                .into_future::<Value<'js>>()
                .await
                .map_err(|e| js_err(format!("request predicate rejected: {e}")))?;
            js_value_to_bool(ctx, resolved)
        }
    }
}

async fn response_matches_js_matcher<'js>(
    ctx: &Ctx<'js>,
    matcher: &JsNetworkMatcher,
    response: &ResponseApi,
) -> JsResult<bool> {
    match matcher {
        JsNetworkMatcher::String(pattern) => Ok(url_matches_pattern(&response.url, pattern)),
        JsNetworkMatcher::RegExp(regexp) => {
            let regexp = regexp
                .clone()
                .restore(ctx)
                .map_err(|e| js_err(format!("failed to restore response RegExp matcher: {e}")))?;
            js_regexp_matches(ctx, &regexp, &response.url)
        }
        JsNetworkMatcher::Predicate(predicate) => {
            let predicate = predicate.clone().restore(ctx).map_err(|e| {
                js_err(format!("failed to restore response predicate matcher: {e}"))
            })?;
            let response_value = Class::instance(ctx.clone(), response.clone())
                .map(|instance| instance.into_value())
                .map_err(|e| {
                    js_err(format!("failed to materialize response for predicate: {e}"))
                })?;
            let result: MaybePromise<'js> = predicate
                .call((response_value,))
                .map_err(|e| js_err(format!("response predicate threw: {e}")))?;
            let resolved = result
                .into_future::<Value<'js>>()
                .await
                .map_err(|e| js_err(format!("response predicate rejected: {e}")))?;
            js_value_to_bool(ctx, resolved)
        }
    }
}

fn remaining_timeout_ms(
    timeout_ms: u64,
    started_at: tokio::time::Instant,
    kind: &str,
) -> JsResult<u64> {
    let elapsed = started_at.elapsed();
    let timeout = std::time::Duration::from_millis(timeout_ms);
    let Some(remaining) = timeout.checked_sub(elapsed) else {
        return Err(js_err(format!(
            "TimeoutError: waiting for {kind} failed: timeout {timeout_ms}ms exceeded"
        )));
    };
    Ok(remaining.as_millis().try_into().unwrap_or(u64::MAX).max(1))
}

fn parse_request_lifecycle_event_name(event: &str) -> Option<RequestLifecycleEvent> {
    match event {
        "requestfinished" => Some(RequestLifecycleEvent::Finished),
        "requestfailed" => Some(RequestLifecycleEvent::Failed),
        _ => None,
    }
}

fn pending_request_lifecycle_event(state: &PendingRequestLifecycleState) -> RequestLifecycleEvent {
    match state {
        PendingRequestLifecycleState::Finished => RequestLifecycleEvent::Finished,
        PendingRequestLifecycleState::Failed(_) => RequestLifecycleEvent::Failed,
    }
}

fn apply_pending_request_lifecycle(
    entry: &mut RequestCaptureItem,
    state: Option<&PendingRequestLifecycleState>,
) {
    match state {
        Some(PendingRequestLifecycleState::Finished) => {
            entry.finished = true;
            entry.error = None;
        }
        Some(PendingRequestLifecycleState::Failed(error_text)) => {
            entry.finished = true;
            entry.error = Some(error_text.clone());
        }
        None => {}
    }
}

fn request_entry_matches_lifecycle_event(
    entry: &RequestCaptureItem,
    event: RequestLifecycleEvent,
) -> bool {
    match event {
        RequestLifecycleEvent::Finished => entry.finished && entry.error.is_none(),
        RequestLifecycleEvent::Failed => entry.finished && entry.error.is_some(),
    }
}

fn url_waiter_matches(url: &str, matcher: &UrlWaiterMatcher) -> bool {
    match matcher {
        UrlWaiterMatcher::Any => true,
        UrlWaiterMatcher::Pattern(pattern) => url_matches_pattern(url, pattern),
    }
}

// Keep string-glob semantics aligned with Playwright's
// packages/playwright-core/src/utils/isomorphic/urlMatch.ts `globToRegexPattern`.
fn glob_to_regex_pattern(glob: &str) -> String {
    let mut tokens = String::from("^");
    let mut in_group = false;
    let chars = glob.chars().collect::<Vec<_>>();
    let escaped_chars = [
        '$', '^', '+', '.', '*', '(', ')', '|', '\\', '?', '{', '}', '[', ']',
    ];
    let mut i = 0usize;

    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            let escaped = chars[i + 1];
            if escaped_chars.contains(&escaped) {
                tokens.push('\\');
            }
            tokens.push(escaped);
            i += 2;
            continue;
        }
        if c == '*' {
            let char_before = i.checked_sub(1).and_then(|index| chars.get(index)).copied();
            let mut star_count = 1usize;
            while i + 1 < chars.len() && chars[i + 1] == '*' {
                star_count += 1;
                i += 1;
            }
            if star_count > 1 {
                let char_after = chars.get(i + 1).copied();
                if char_after == Some('/') {
                    if char_before == Some('/') {
                        tokens.push_str("((.+/)|)");
                    } else {
                        tokens.push_str("(.*/)");
                    }
                    i += 2;
                    continue;
                }
                tokens.push_str("(.*)");
                i += 1;
                continue;
            }
            tokens.push_str("([^/]*)");
            i += 1;
            continue;
        }

        match c {
            '{' => {
                in_group = true;
                tokens.push('(');
            }
            '}' => {
                in_group = false;
                tokens.push(')');
            }
            ',' if in_group => tokens.push('|'),
            _ => {
                if escaped_chars.contains(&c) {
                    tokens.push('\\');
                }
                tokens.push(c);
            }
        }
        i += 1;
    }

    tokens.push('$');
    tokens
}

fn url_matches_pattern(url: &str, pattern: &str) -> bool {
    regex::Regex::new(&glob_to_regex_pattern(pattern))
        .map(|regex| regex.is_match(url))
        .unwrap_or(false)
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

fn parse_timeout_option(option: Option<&Value<'_>>) -> JsResult<u64> {
    let Some(option) = option else {
        return Ok(DEFAULT_TIMEOUT_MS);
    };
    if let Ok(timeout_ms) = i32::from_js(&option.ctx().clone(), option.clone()) {
        return Ok(timeout_ms.max(0) as u64);
    }
    let object = Object::from_value(option.clone())
        .map_err(|_| js_err("expected timeout number or options object".to_string()))?;
    let timeout = object
        .get::<_, Option<i32>>("timeout")
        .map_err(|e| js_err(format!("invalid timeout option: {e}")))?;
    Ok(timeout.unwrap_or(DEFAULT_TIMEOUT_MS as i32).max(0) as u64)
}

fn parse_wait_for_event_options<'js>(
    ctx: &Ctx<'js>,
    option: Option<&Value<'js>>,
    api_name: &str,
) -> JsResult<EventWaitOptions> {
    let Some(option) = option else {
        return Ok(EventWaitOptions {
            timeout_ms: DEFAULT_TIMEOUT_MS,
            predicate: None,
        });
    };

    if let Ok(timeout_ms) = i32::from_js(&option.ctx().clone(), option.clone()) {
        return Ok(EventWaitOptions {
            timeout_ms: timeout_ms.max(0) as u64,
            predicate: None,
        });
    }

    if option.is_function() {
        let predicate = option
            .clone()
            .into_function()
            .ok_or_else(|| js_err(format!("{api_name} predicate was not callable")))?;
        return Ok(EventWaitOptions {
            timeout_ms: DEFAULT_TIMEOUT_MS,
            predicate: Some(Persistent::save(ctx, predicate)),
        });
    }

    let object = Object::from_value(option.clone()).map_err(|_| {
        js_err(format!(
            "{api_name} expected a timeout number, predicate function, or options object"
        ))
    })?;
    let timeout = object
        .get::<_, Option<i32>>("timeout")
        .map_err(|e| js_err(format!("invalid timeout option: {e}")))?;
    let predicate = object
        .get::<_, Option<Function<'js>>>("predicate")
        .map_err(|e| js_err(format!("invalid predicate option: {e}")))?
        .map(|predicate| Persistent::save(ctx, predicate));
    Ok(EventWaitOptions {
        timeout_ms: timeout.unwrap_or(DEFAULT_TIMEOUT_MS as i32).max(0) as u64,
        predicate,
    })
}

pub(crate) fn parse_screenshot_options(
    option: Option<&Value<'_>>,
    allow_full_page: bool,
) -> JsResult<ParsedScreenshotOptions> {
    let Some(option) = option else {
        return Ok(ParsedScreenshotOptions::default());
    };
    let object = Object::from_value(option.clone())
        .map_err(|_| js_err("screenshot options must be an object".to_string()))?;
    let mut parsed = ParsedScreenshotOptions::default();

    if let Some(type_name) = object
        .get::<_, Option<String>>("type")
        .map_err(|e| js_err(format!("invalid screenshot.type: {e}")))?
    {
        parsed.format = match type_name.as_str() {
            "png" => ScreenshotImageFormat::Png,
            "jpeg" => ScreenshotImageFormat::Jpeg,
            other => {
                return Err(js_err(format!(
                    "Unknown screenshot type: {other}. Expected 'png' or 'jpeg'"
                )))
            }
        };
    }

    parsed.quality = object
        .get::<_, Option<i64>>("quality")
        .map_err(|e| js_err(format!("invalid screenshot.quality: {e}")))?;
    if parsed.quality.is_some() && parsed.format != ScreenshotImageFormat::Jpeg {
        return Err(js_err(
            "options.quality is unsupported for png screenshots".to_string(),
        ));
    }
    if let Some(quality) = parsed.quality {
        if !(0..=100).contains(&quality) {
            return Err(js_err(format!(
                "Expected screenshot quality to be between 0 and 100, got {quality}"
            )));
        }
    }

    parsed.full_page = object
        .get::<_, Option<bool>>("fullPage")
        .map_err(|e| js_err(format!("invalid screenshot.fullPage: {e}")))?
        .unwrap_or(false);
    if parsed.full_page && !allow_full_page {
        return Err(js_err(
            "options.fullPage is only supported for page screenshots".to_string(),
        ));
    }

    if let Some(clip) = object
        .get::<_, Option<Object<'_>>>("clip")
        .map_err(|e| js_err(format!("invalid screenshot.clip: {e}")))?
    {
        let clip = ScreenshotClip {
            x: clip
                .get("x")
                .map_err(|e| js_err(format!("invalid screenshot.clip.x: {e}")))?,
            y: clip
                .get("y")
                .map_err(|e| js_err(format!("invalid screenshot.clip.y: {e}")))?,
            width: clip
                .get("width")
                .map_err(|e| js_err(format!("invalid screenshot.clip.width: {e}")))?,
            height: clip
                .get("height")
                .map_err(|e| js_err(format!("invalid screenshot.clip.height: {e}")))?,
        };
        if clip.width <= 0.0 || clip.height <= 0.0 {
            return Err(js_err(
                "Expected screenshot clip width and height to be greater than 0".to_string(),
            ));
        }
        parsed.clip = Some(clip);
    }

    parsed.omit_background = object
        .get::<_, Option<bool>>("omitBackground")
        .map_err(|e| js_err(format!("invalid screenshot.omitBackground: {e}")))?
        .unwrap_or(false);
    parsed.caret = object
        .get::<_, Option<String>>("caret")
        .map_err(|e| js_err(format!("invalid screenshot.caret: {e}")))?
        .unwrap_or_else(|| "hide".to_string());
    if !matches!(parsed.caret.as_str(), "hide" | "initial") {
        return Err(js_err(
            "options.caret must be 'hide' or 'initial'".to_string(),
        ));
    }
    parsed.animations = object
        .get::<_, Option<String>>("animations")
        .map_err(|e| js_err(format!("invalid screenshot.animations: {e}")))?
        .unwrap_or_else(|| "allow".to_string());
    if !matches!(parsed.animations.as_str(), "allow" | "disabled") {
        return Err(js_err(
            "options.animations must be 'allow' or 'disabled'".to_string(),
        ));
    }
    parsed.scale = object
        .get::<_, Option<String>>("scale")
        .map_err(|e| js_err(format!("invalid screenshot.scale: {e}")))?
        .unwrap_or_else(|| "device".to_string());
    if !matches!(parsed.scale.as_str(), "device" | "css") {
        return Err(js_err(
            "options.scale must be 'device' or 'css'".to_string(),
        ));
    }
    parsed.path = object
        .get::<_, Option<String>>("path")
        .map_err(|e| js_err(format!("invalid screenshot.path: {e}")))?;
    parsed.style = object
        .get::<_, Option<String>>("style")
        .map_err(|e| js_err(format!("invalid screenshot.style: {e}")))?;
    parsed.mask_color = object
        .get::<_, Option<String>>("maskColor")
        .map_err(|e| js_err(format!("invalid screenshot.maskColor: {e}")))?
        .unwrap_or_else(|| "#FF00FF".to_string());

    if let Some(mask_value) = object
        .get::<_, Option<Value<'_>>>("mask")
        .map_err(|e| js_err(format!("invalid screenshot.mask: {e}")))?
    {
        let mask_obj = Object::from_value(mask_value)
            .map_err(|_| js_err("screenshot.mask must be an array of Locator".to_string()))?;
        let len = mask_obj
            .get::<_, i32>("length")
            .map_err(|e| js_err(format!("invalid screenshot.mask length: {e}")))?;
        for index in 0..len {
            let value = mask_obj
                .get::<_, Value<'_>>(index)
                .map_err(|e| js_err(format!("invalid screenshot.mask[{index}]: {e}")))?;
            let class = Class::<Locator>::from_value(&value).map_err(|_| {
                js_err(format!(
                    "screenshot.mask[{index}] must be a Locator from this runtime"
                ))
            })?;
            parsed.mask_locators.push(class.borrow().clone());
        }
    }

    Ok(parsed)
}

pub(crate) fn resolve_screenshot_output_path(
    download_dir: &Path,
    path: Option<&str>,
) -> JsResult<Option<PathBuf>> {
    let Some(path) = path else {
        return Ok(None);
    };
    let candidate = PathBuf::from(path);
    if candidate.has_root() {
        return Err(js_err(
            "screenshot.path must be relative to the browser download directory".to_string(),
        ));
    }
    if candidate
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(js_err(
            "screenshot.path must not contain parent directory traversals".to_string(),
        ));
    }
    let resolved = download_dir.join(&candidate);
    if !resolved.starts_with(download_dir) {
        return Err(js_err(
            "screenshot.path must stay within the browser download directory".to_string(),
        ));
    }
    Ok(Some(resolved))
}

fn rect_to_viewport(
    clip: &ScreenshotClip,
    scale: f64,
) -> JsResult<chromiumoxide::cdp::browser_protocol::page::Viewport> {
    chromiumoxide::cdp::browser_protocol::page::Viewport::builder()
        .x(clip.x)
        .y(clip.y)
        .width(clip.width)
        .height(clip.height)
        .scale(scale)
        .build()
        .map_err(|e| js_err(format!("invalid screenshot viewport: {e}")))
}

fn decode_binary_base64(binary: &chromiumoxide::Binary) -> JsResult<Vec<u8>> {
    let encoded: String = binary.clone().into();
    base64::engine::general_purpose::STANDARD
        .decode(encoded.as_bytes())
        .map_err(|e| js_err(format!("screenshot decode failed: {e}")))
}

async fn current_scroll_offsets(page: &chromiumoxide::Page) -> JsResult<(f64, f64)> {
    use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
    let expression = "({ x: window.scrollX || 0, y: window.scrollY || 0 })".to_string();
    let mut last_error = None;
    let result = loop {
        let eval = EvaluateParams::builder()
            .expression(expression.clone())
            .await_promise(false)
            .return_by_value(true)
            .build()
            .map_err(|e| js_err(format!("screenshot scroll params failed: {e}")))?;
        match page.evaluate_expression(eval).await {
            Ok(result) => break result,
            Err(err) => {
                let err_text = err.to_string();
                if !is_missing_execution_context_error(&err_text) {
                    return Err(js_err(format!("screenshot scroll eval failed: {err}")));
                }
                let attempts = last_error
                    .as_ref()
                    .map(|(attempts, _): &(usize, String)| *attempts)
                    .unwrap_or(0);
                if attempts + 1 >= SCREENSHOT_CONTEXT_RETRY_ATTEMPTS {
                    let message = last_error.map(|(_, message)| message).unwrap_or(err_text);
                    return Err(js_err(format!("screenshot scroll eval failed: {message}")));
                }
                last_error = Some((attempts + 1, err_text));
                tokio::time::sleep(std::time::Duration::from_millis(
                    SCREENSHOT_CONTEXT_RETRY_MS,
                ))
                .await;
            }
        }
    };
    let value = result
        .value()
        .and_then(serde_json::Value::as_object)
        .cloned()
        .unwrap_or_default();
    Ok((
        value
            .get("x")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0),
        value
            .get("y")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0),
    ))
}

pub(crate) async fn screenshot_clip_for_object_id(
    page: &chromiumoxide::Page,
    object_id: String,
) -> JsResult<ScreenshotClip> {
    use chromiumoxide::cdp::browser_protocol::dom::GetContentQuadsParams;
    use chromiumoxide::layout::ElementQuad;
    let quads = page
        .execute(
            GetContentQuadsParams::builder()
                .object_id(object_id)
                .build(),
        )
        .await
        .map_err(|e| js_err(format!("screenshot clip failed: {e}")))?;
    quads
        .quads
        .iter()
        .filter(|q| q.inner().len() == 8)
        .map(ElementQuad::from_quad)
        .filter(|q| q.quad_area() > 1.)
        .map(|q| {
            let min_x = q
                .top_left
                .x
                .min(q.top_right.x)
                .min(q.bottom_left.x)
                .min(q.bottom_right.x);
            let max_x = q
                .top_left
                .x
                .max(q.top_right.x)
                .max(q.bottom_left.x)
                .max(q.bottom_right.x);
            let min_y = q
                .top_left
                .y
                .min(q.top_right.y)
                .min(q.bottom_left.y)
                .min(q.bottom_right.y);
            let max_y = q
                .top_left
                .y
                .max(q.top_right.y)
                .max(q.bottom_left.y)
                .max(q.bottom_right.y);
            ScreenshotClip {
                x: min_x,
                y: min_y,
                width: max_x - min_x,
                height: max_y - min_y,
            }
        })
        .next()
        .ok_or_else(|| js_err("screenshot failed: element not visible in viewport".to_string()))
}

async fn install_screenshot_overrides(
    page: &chromiumoxide::Page,
    options: &ParsedScreenshotOptions,
    mask_clips: &[ScreenshotClip],
) -> JsResult<()> {
    use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
    let (scroll_x, scroll_y) = current_scroll_offsets(page).await?;
    let mask_rects = mask_clips
        .iter()
        .map(|clip| {
            serde_json::json!({
                "left": clip.x + scroll_x,
                "top": clip.y + scroll_y,
                "width": clip.width,
                "height": clip.height,
            })
        })
        .collect::<Vec<_>>();
    let style_json = serde_json::to_string(&options.style.clone().unwrap_or_default())
        .unwrap_or_else(|_| "\"\"".to_string());
    let mask_color_json =
        serde_json::to_string(&options.mask_color).unwrap_or_else(|_| "\"#FF00FF\"".to_string());
    let mask_rects_json = serde_json::to_string(&mask_rects).unwrap_or_else(|_| "[]".to_string());
    let disable_animations = options.animations == "disabled";
    let hide_caret = options.caret == "hide";
    let expression = format!(
        r#"(function() {{
            const prev = window[{key:?}];
            if (prev && prev.cleanup) prev.cleanup();
            const styleEl = document.createElement('style');
            styleEl.setAttribute('data-refreshmint-screenshot', 'true');
            let css = '';
            if ({disable_animations}) {{
              css += '*,*::before,*::after{{animation:none!important;transition:none!important;}}';
            }}
            if ({hide_caret}) {{
              css += '*,input,textarea{{caret-color:transparent!important;}}';
            }}
            const extraStyle = {style_json};
            if (extraStyle) css += extraStyle;
            styleEl.textContent = css;
            document.documentElement.appendChild(styleEl);

            const overlayRoot = document.createElement('div');
            overlayRoot.setAttribute('data-refreshmint-screenshot-mask-root', 'true');
            overlayRoot.style.position = 'absolute';
            overlayRoot.style.left = '0px';
            overlayRoot.style.top = '0px';
            overlayRoot.style.width = Math.max(document.documentElement.scrollWidth, document.body ? document.body.scrollWidth : 0) + 'px';
            overlayRoot.style.height = Math.max(document.documentElement.scrollHeight, document.body ? document.body.scrollHeight : 0) + 'px';
            overlayRoot.style.pointerEvents = 'none';
            overlayRoot.style.zIndex = '2147483647';
            for (const rect of {mask_rects_json}) {{
              const mask = document.createElement('div');
              mask.style.position = 'absolute';
              mask.style.left = rect.left + 'px';
              mask.style.top = rect.top + 'px';
              mask.style.width = rect.width + 'px';
              mask.style.height = rect.height + 'px';
              mask.style.background = {mask_color_json};
              overlayRoot.appendChild(mask);
            }}
            if (overlayRoot.childNodes.length) document.body.appendChild(overlayRoot);
            window[{key:?}] = {{
              cleanup() {{
                styleEl.remove();
                overlayRoot.remove();
                delete window[{key:?}];
              }}
            }};
            return true;
        }})()"#,
        key = SCREENSHOT_PREPARE_STATE_KEY,
        disable_animations = if disable_animations { "true" } else { "false" },
        hide_caret = if hide_caret { "true" } else { "false" },
    );
    let mut last_error = None;
    for attempt in 0..SCREENSHOT_CONTEXT_RETRY_ATTEMPTS {
        let eval = EvaluateParams::builder()
            .expression(expression.clone())
            .await_promise(false)
            .return_by_value(true)
            .build()
            .map_err(|e| js_err(format!("screenshot override params failed: {e}")))?;
        match page.execute(eval).await {
            Ok(_) => return Ok(()),
            Err(err) => {
                let err_text = err.to_string();
                if !is_missing_execution_context_error(&err_text)
                    || attempt + 1 == SCREENSHOT_CONTEXT_RETRY_ATTEMPTS
                {
                    let message = last_error.unwrap_or(err_text);
                    return Err(js_err(format!("screenshot override failed: {message}")));
                }
                last_error = Some(err_text);
                tokio::time::sleep(std::time::Duration::from_millis(
                    SCREENSHOT_CONTEXT_RETRY_MS,
                ))
                .await;
            }
        }
    }
    Ok(())
}

async fn clear_screenshot_overrides(page: &chromiumoxide::Page) -> JsResult<()> {
    use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
    let expression = format!(
        r#"(function() {{
            const state = window[{key:?}];
            if (state && state.cleanup) state.cleanup();
            return true;
        }})()"#,
        key = SCREENSHOT_PREPARE_STATE_KEY,
    );
    for attempt in 0..SCREENSHOT_CONTEXT_RETRY_ATTEMPTS {
        let eval = EvaluateParams::builder()
            .expression(expression.clone())
            .await_promise(false)
            .return_by_value(true)
            .build()
            .map_err(|e| js_err(format!("clear screenshot override params failed: {e}")))?;
        match page.execute(eval).await {
            Ok(_) => return Ok(()),
            Err(err) => {
                let err_text = err.to_string();
                if !is_missing_execution_context_error(&err_text)
                    || attempt + 1 == SCREENSHOT_CONTEXT_RETRY_ATTEMPTS
                {
                    return Err(js_err(format!("clear screenshot override failed: {err}")));
                }
                tokio::time::sleep(std::time::Duration::from_millis(
                    SCREENSHOT_CONTEXT_RETRY_MS,
                ))
                .await;
            }
        }
    }
    Ok(())
}

pub(crate) async fn run_screenshot_capture(
    page_inner: Arc<Mutex<PageInner>>,
    options: &ParsedScreenshotOptions,
    clip_override: Option<ScreenshotClip>,
    mask_clips: &[ScreenshotClip],
    output_path: Option<PathBuf>,
) -> JsResult<Vec<u8>> {
    use chromiumoxide::cdp::browser_protocol::dom::Rgba;
    use chromiumoxide::cdp::browser_protocol::emulation::SetDefaultBackgroundColorOverrideParams;
    use chromiumoxide::cdp::browser_protocol::page::{
        CaptureScreenshotFormat, CaptureScreenshotParams, GetLayoutMetricsParams,
    };

    let (page, download_dir) = {
        let inner = page_inner.lock().await;
        (inner.page.clone(), inner.download_dir.clone())
    };
    if let Some(path) = &output_path {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| js_err(format!("screenshot mkdir failed: {e}")))?;
        } else {
            fs::create_dir_all(&download_dir)
                .map_err(|e| js_err(format!("screenshot mkdir failed: {e}")))?;
        }
    }

    install_screenshot_overrides(&page, options, mask_clips).await?;
    let mut background_overridden = false;
    if options.omit_background && options.format == ScreenshotImageFormat::Png {
        let rgba = Rgba::builder()
            .r(0)
            .g(0)
            .b(0)
            .a(0.0)
            .build()
            .map_err(|e| js_err(format!("omitBackground color failed: {e}")))?;
        page.execute(
            SetDefaultBackgroundColorOverrideParams::builder()
                .color(rgba)
                .build(),
        )
        .await
        .map_err(|e| js_err(format!("omitBackground failed: {e}")))?;
        background_overridden = true;
    }

    let capture_result = async {
        let requested_clip =
            if let Some(clip) = clip_override.clone().or_else(|| options.clip.clone()) {
                clip
            } else if options.full_page {
                let metrics = page
                    .execute(GetLayoutMetricsParams {})
                    .await
                    .map_err(|e| js_err(format!("fullPage layout metrics failed: {e}")))?;
                ScreenshotClip {
                    x: metrics.result.css_content_size.x,
                    y: metrics.result.css_content_size.y,
                    width: metrics.result.css_content_size.width,
                    height: metrics.result.css_content_size.height,
                }
            } else {
                let metrics = page
                    .execute(GetLayoutMetricsParams {})
                    .await
                    .map_err(|e| js_err(format!("layout metrics failed: {e}")))?;
                ScreenshotClip {
                    x: metrics.result.css_visual_viewport.page_x,
                    y: metrics.result.css_visual_viewport.page_y,
                    width: metrics.result.css_visual_viewport.client_width,
                    height: metrics.result.css_visual_viewport.client_height,
                }
            };
        let mut builder = CaptureScreenshotParams::builder()
            .format(match options.format {
                ScreenshotImageFormat::Png => CaptureScreenshotFormat::Png,
                ScreenshotImageFormat::Jpeg => CaptureScreenshotFormat::Jpeg,
            })
            .from_surface(true)
            .capture_beyond_viewport(
                options.full_page || clip_override.is_some() || options.clip.is_some(),
            );
        if let Some(quality) = options.quality {
            builder = builder.quality(quality);
        }
        let _scale_mode = &options.scale;
        builder = builder.clip(rect_to_viewport(&requested_clip, 1.0)?);
        let screenshot = page
            .execute(builder.build())
            .await
            .map_err(|e| js_err(format!("screenshot failed: {e}")))?;
        let bytes = decode_binary_base64(&screenshot.result.data)?;
        if let Some(path) = output_path {
            fs::write(&path, &bytes)
                .map_err(|e| js_err(format!("screenshot write failed: {e}")))?;
        }
        Ok::<Vec<u8>, rquickjs::Error>(bytes)
    }
    .await;

    if background_overridden {
        let _ = page
            .execute(SetDefaultBackgroundColorOverrideParams::builder().build())
            .await;
    }
    let _ = clear_screenshot_overrides(&page).await;
    capture_result
}

fn headers_to_map(
    headers: Option<&chromiumoxide::cdp::browser_protocol::network::Headers>,
) -> BTreeMap<String, String> {
    let Some(headers) = headers else {
        return BTreeMap::new();
    };
    let Some(map) = headers.inner().as_object() else {
        return BTreeMap::new();
    };
    map.iter()
        .map(|(name, value)| {
            let rendered = value
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string());
            (name.to_ascii_lowercase(), rendered)
        })
        .collect()
}

fn header_value(headers: &BTreeMap<String, String>, name: &str) -> Option<String> {
    headers.get(&name.to_ascii_lowercase()).cloned()
}

fn header_values(headers: &BTreeMap<String, String>, name: &str) -> Vec<String> {
    header_value(headers, name)
        .into_iter()
        .flat_map(|value| {
            value
                .split('\n')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn headers_to_json_expr(headers: &BTreeMap<String, String>) -> String {
    let json = serde_json::to_string(headers).unwrap_or_else(|_| "{}".to_string());
    format!("({json})")
}

fn headers_array_json_expr(headers: &BTreeMap<String, String>) -> String {
    let array = headers
        .iter()
        .map(|(name, value)| serde_json::json!({ "name": name, "value": value }))
        .collect::<Vec<_>>();
    serde_json::to_string(&array).unwrap_or_else(|_| "[]".to_string())
}

fn json_string_to_eval_result(json: String) -> JsResult<JsEvalResult> {
    if json.trim().is_empty() {
        return Err(js_err("empty JSON expression".to_string()));
    }
    Ok(JsEvalResult::Json(json))
}

fn serialize_to_js_eval_result<T: serde::Serialize>(value: &T) -> JsResult<JsEvalResult> {
    let json_value =
        serde_json::to_value(value).map_err(|e| js_err(format!("serialization failed: {e}")))?;
    let json = serde_json::to_string(&json_value)
        .map_err(|e| js_err(format!("serialization failed: {e}")))?;
    Ok(JsEvalResult::Json(wrap_json_for_eval(&json_value, json)))
}

fn wrap_json_for_eval(value: &serde_json::Value, json: String) -> String {
    if matches!(value, serde_json::Value::Object(_)) {
        format!("({json})")
    } else {
        json
    }
}

fn parse_form_urlencoded_simple(input: &str) -> Vec<(String, String)> {
    input
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            (key.replace('+', " "), value.replace('+', " "))
        })
        .collect()
}

fn response_timing_to_request_timing(
    timing: Option<&chromiumoxide::cdp::browser_protocol::network::ResourceTiming>,
) -> RequestTiming {
    let Some(timing) = timing else {
        return RequestTiming::default_playwright();
    };
    // Keep this mapping aligned with Playwright's ResourceTiming shape in
    // third-party/js/playwright/packages/playwright-core/src/client/network.ts.
    RequestTiming {
        start_time: timing.request_time * 1000.0,
        domain_lookup_start: timing.dns_start,
        domain_lookup_end: timing.dns_end,
        connect_start: timing.connect_start,
        secure_connection_start: timing.ssl_start,
        connect_end: timing.connect_end,
        request_start: timing.send_start,
        response_start: timing.receive_headers_end,
        response_end: timing.receive_headers_end,
    }
}

fn allocate_request_hop(
    raw_request_id: &str,
    has_redirect_response: bool,
    current_ids: &mut BTreeMap<String, String>,
    next_request_id: &AtomicU64,
) -> (String, Option<String>) {
    let previous_request_id = if has_redirect_response {
        current_ids.get(raw_request_id).cloned()
    } else {
        None
    };
    let request_id = format!(
        "request-{}",
        next_request_id.fetch_add(1, Ordering::Relaxed)
    );
    current_ids.insert(raw_request_id.to_string(), request_id.clone());
    (request_id, previous_request_id)
}

fn current_request_id_for_raw(
    current_ids: &Arc<std::sync::Mutex<BTreeMap<String, String>>>,
    raw_request_id: &str,
) -> String {
    current_ids
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .get(raw_request_id)
        .cloned()
        .unwrap_or_else(|| raw_request_id.to_string())
}

fn linked_response_for_request(
    entries: &[NetworkRequest],
    request_id: &str,
    raw_request_id: &str,
    request_url: &str,
    redirected_from: Option<&String>,
) -> Option<NetworkRequest> {
    let exact = entries
        .iter()
        .find(|entry| entry.request_id == request_id)
        .cloned();
    if exact.as_ref().is_some_and(|entry| entry.url == request_url) {
        return exact;
    }

    let raw_matches: Vec<_> = entries
        .iter()
        .filter(|entry| {
            entry
                .request_id_raw
                .as_ref()
                .is_some_and(|raw| raw.as_ref() == raw_request_id)
        })
        .cloned()
        .collect();
    if raw_matches.is_empty() {
        return None;
    }

    if let Some(found) = raw_matches.iter().find(|entry| entry.url == request_url) {
        return Some(found.clone());
    }

    if exact.is_some() {
        return exact;
    }

    if redirected_from.is_some() {
        raw_matches
            .iter()
            .rev()
            .find(|entry| !(300..400).contains(&entry.status))
            .cloned()
            .or_else(|| raw_matches.last().cloned())
    } else {
        raw_matches
            .iter()
            .find(|entry| (300..400).contains(&entry.status))
            .cloned()
            .or_else(|| raw_matches.first().cloned())
    }
}

fn linked_request_for_response(
    entries: &[RequestCaptureItem],
    request_id: &str,
    raw_request_id: Option<&chromiumoxide::cdp::browser_protocol::network::RequestId>,
    response_url: &str,
    status: i64,
) -> Option<RequestCaptureItem> {
    let exact = entries
        .iter()
        .find(|entry| entry.request_id == request_id)
        .cloned();
    if exact
        .as_ref()
        .is_some_and(|entry| entry.url == response_url)
    {
        return exact;
    }

    let Some(raw_request_id) = raw_request_id else {
        return exact;
    };
    let raw_matches: Vec<_> = entries
        .iter()
        .filter(|entry| entry.raw_request_id == raw_request_id.as_ref())
        .cloned()
        .collect();
    if raw_matches.is_empty() {
        return None;
    }

    if let Some(found) = raw_matches.iter().find(|entry| entry.url == response_url) {
        return Some(found.clone());
    }

    if exact.is_some() {
        return exact;
    }

    if (300..400).contains(&status) {
        raw_matches
            .iter()
            .find(|entry| entry.redirected_from.is_none())
            .cloned()
            .or_else(|| raw_matches.first().cloned())
    } else {
        raw_matches
            .iter()
            .rev()
            .find(|entry| entry.redirected_from.is_some())
            .cloned()
            .or_else(|| raw_matches.last().cloned())
    }
}

fn linked_redirected_from_request(
    entries: &[RequestCaptureItem],
    request_id: &str,
    raw_request_id: &str,
    redirected_from: Option<&String>,
) -> Option<RequestCaptureItem> {
    if let Some(previous_id) = redirected_from {
        if let Some(found) = entries
            .iter()
            .find(|entry| &entry.request_id == previous_id)
        {
            return Some(found.clone());
        }
    }

    let raw_matches: Vec<_> = entries
        .iter()
        .filter(|entry| entry.raw_request_id == raw_request_id && entry.request_id != request_id)
        .cloned()
        .collect();
    raw_matches
        .iter()
        .find(|entry| entry.redirected_from.is_none())
        .cloned()
        .or_else(|| raw_matches.first().cloned())
}

fn linked_redirected_to_request(
    entries: &[RequestCaptureItem],
    request_id: &str,
    raw_request_id: &str,
) -> Option<RequestCaptureItem> {
    if let Some(found) = entries
        .iter()
        .find(|entry| entry.redirected_from.as_ref() == Some(&request_id.to_string()))
    {
        return Some(found.clone());
    }

    let raw_matches: Vec<_> = entries
        .iter()
        .filter(|entry| entry.raw_request_id == raw_request_id && entry.request_id != request_id)
        .cloned()
        .collect();
    raw_matches
        .iter()
        .find(|entry| entry.redirected_from.is_some())
        .cloned()
        .or_else(|| raw_matches.last().cloned())
}

fn build_redirect_response_entry(
    previous_request_id: String,
    request_item: &RequestCaptureItem,
    redirect_response: &chromiumoxide::cdp::browser_protocol::network::Response,
    timestamp_ms: i64,
) -> NetworkRequest {
    let redirect_status = redirect_response.status;
    NetworkRequest {
        request_id: previous_request_id,
        url: redirect_response.url.clone(),
        status: redirect_status,
        ok: redirect_status == 0 || (200..300).contains(&redirect_status),
        method: redirect_response
            .request_headers
            .as_ref()
            .map(|headers| network_method_from_headers(Some(headers)))
            .unwrap_or_else(|| request_item.method.clone()),
        status_text: redirect_response.status_text.clone(),
        headers: headers_to_map(Some(&redirect_response.headers)),
        frame_id: request_item.frame_id.clone(),
        from_service_worker: redirect_response.from_service_worker.unwrap_or(false),
        ts: timestamp_ms,
        error: None,
        finished: true,
        timing: response_timing_to_request_timing(redirect_response.timing.as_ref()),
        server_addr: remote_addr_from_response(redirect_response),
        security_details: response_security_details(redirect_response),
        request_id_raw: Some(
            chromiumoxide::cdp::browser_protocol::network::RequestId::new(
                request_item.raw_request_id.clone(),
            ),
        ),
    }
}

fn is_navigation_request(
    event: &chromiumoxide::cdp::browser_protocol::network::EventRequestWillBeSent,
) -> bool {
    // Keep this aligned with Chromium Playwright's navigation-request check in
    // third-party/js/playwright/packages/playwright-core/src/server/chromium/crNetworkManager.ts.
    event.request_id.as_ref() == event.loader_id.as_ref()
        && event
            .r#type
            .as_ref()
            .is_some_and(|resource_type| resource_type.as_ref().eq_ignore_ascii_case("document"))
}

fn timing_response_end_from_timestamp(request_timing: &RequestTiming, timestamp_secs: f64) -> f64 {
    if request_timing.start_time <= 0.0 {
        return request_timing
            .response_end
            .max(request_timing.response_start);
    }
    let response_end = timestamp_secs * 1000.0 - request_timing.start_time;
    response_end.max(request_timing.response_start)
}

fn remote_addr_from_response(
    response: &chromiumoxide::cdp::browser_protocol::network::Response,
) -> Option<RemoteAddr> {
    Some(RemoteAddr {
        ip_address: response.remote_ip_address.clone()?,
        port: response.remote_port?,
    })
}

fn response_security_details(
    response: &chromiumoxide::cdp::browser_protocol::network::Response,
) -> Option<ResponseSecurityDetails> {
    let details = response.security_details.as_ref()?;
    Some(ResponseSecurityDetails {
        protocol: Some(details.protocol.clone()),
        subject_name: Some(details.subject_name.clone()),
        issuer: Some(details.issuer.clone()),
        valid_from: Some(*details.valid_from.inner()),
        valid_to: Some(*details.valid_to.inner()),
    })
}

#[derive(Debug, Clone)]
struct FrameMetadata {
    name: String,
    url: String,
    parent_id: Option<String>,
}

async fn lookup_frame_info(
    page: &chromiumoxide::Page,
    wanted_frame_id: &str,
) -> Result<Option<FrameMetadata>, String> {
    use chromiumoxide::cdp::browser_protocol::page::GetFrameTreeParams;
    let tree = page
        .execute(GetFrameTreeParams::default())
        .await
        .map_err(|e| format!("failed to get frame tree: {e}"))?;

    let mut stack = vec![tree.result.frame_tree];
    while let Some(node) = stack.pop() {
        if node.frame.id.as_ref() == wanted_frame_id {
            return Ok(Some(FrameMetadata {
                name: node.frame.name.unwrap_or_default(),
                url: node.frame.url,
                parent_id: node.frame.parent_id.map(|id| id.as_ref().to_string()),
            }));
        }
        if let Some(children) = node.child_frames {
            for child in children {
                stack.push(child);
            }
        }
    }
    Ok(None)
}

async fn get_response_body_bytes(
    page: &chromiumoxide::Page,
    request_id: chromiumoxide::cdp::browser_protocol::network::RequestId,
) -> Result<Vec<u8>, String> {
    use chromiumoxide::cdp::browser_protocol::network::GetResponseBodyParams;
    let result = page
        .execute(GetResponseBodyParams::new(request_id))
        .await
        .map_err(|e| format!("getResponseBody failed: {e}"))?;
    if result.result.base64_encoded {
        base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &result.result.body,
        )
        .map_err(|e| format!("base64 decode failed: {e}"))
    } else {
        Ok(result.result.body.into_bytes())
    }
}

async fn get_request_post_data(
    page: &chromiumoxide::Page,
    request_id: chromiumoxide::cdp::browser_protocol::network::RequestId,
) -> Result<String, String> {
    use chromiumoxide::cdp::browser_protocol::network::GetRequestPostDataParams;
    let result = page
        .execute(GetRequestPostDataParams::new(request_id))
        .await
        .map_err(|e| format!("getRequestPostData failed: {e}"))?;
    Ok(result.result.post_data)
}

fn seed_frame_entries_from_tree(
    entries: &mut BTreeMap<String, CapturedFrameInfo>,
    frame_tree: chromiumoxide::cdp::browser_protocol::page::FrameTree,
) {
    let frame = frame_tree.frame;
    entries.insert(
        frame.id.as_ref().to_string(),
        CapturedFrameInfo {
            id: frame.id.as_ref().to_string(),
            name: frame.name.unwrap_or_default(),
            url: frame.url,
            parent_id: frame.parent_id.map(|parent| parent.as_ref().to_string()),
        },
    );
    if let Some(children) = frame_tree.child_frames {
        for child in children {
            seed_frame_entries_from_tree(entries, child);
        }
    }
}

fn remove_frame_entry_and_descendants(
    entries: &mut BTreeMap<String, CapturedFrameInfo>,
    frame_id: &str,
) {
    let child_ids = entries
        .values()
        .filter(|entry| entry.parent_id.as_deref() == Some(frame_id))
        .map(|entry| entry.id.clone())
        .collect::<Vec<_>>();
    for child_id in child_ids {
        remove_frame_entry_and_descendants(entries, &child_id);
    }
    entries.remove(frame_id);
}

async fn discovered_frame_ids_from_network(page: &PageApi) -> BTreeSet<String> {
    let _ = page.ensure_request_capture().await;
    let _ = page.ensure_response_capture().await;
    let mut frame_ids = page
        .response_entries
        .lock()
        .await
        .iter()
        .filter_map(|entry| entry.frame_id.clone())
        .filter(|frame_id| !frame_id.is_empty())
        .collect::<BTreeSet<_>>();
    frame_ids.extend(
        page.request_entries
            .lock()
            .await
            .iter()
            .filter_map(|entry| entry.frame_id.clone())
            .filter(|frame_id| !frame_id.is_empty()),
    );
    frame_ids
}

pub(crate) async fn wait_for_frame_execution_context(
    page: &chromiumoxide::Page,
    frame_id: chromiumoxide::cdp::browser_protocol::page::FrameId,
) -> Result<chromiumoxide::cdp::js_protocol::runtime::ExecutionContextId, String> {
    use chromiumoxide::cdp::js_protocol::runtime::EnableParams;

    page.execute(EnableParams::default())
        .await
        .map_err(|e| format!("failed to enable runtime for frame context lookup: {e}"))?;

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

pub(crate) async fn wait_for_frame_execution_target(
    page: &chromiumoxide::Page,
    frame_id: chromiumoxide::cdp::browser_protocol::page::FrameId,
) -> Result<
    (
        chromiumoxide::cdp::js_protocol::runtime::ExecutionContextId,
        chromiumoxide::cdp::browser_protocol::target::SessionId,
    ),
    String,
> {
    use chromiumoxide::cdp::js_protocol::runtime::EnableParams;
    let mut runtime_enabled_session = None;

    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_millis(DEFAULT_TIMEOUT_MS);

    loop {
        let session = page
            .frame_session_id(frame_id.clone())
            .await
            .map_err(|e| format!("failed to query frame session: {e}"))?;
        if let Some(session_id) = session.as_ref() {
            if runtime_enabled_session.as_ref() != Some(session_id) {
                // Cross-origin iframes may be owned by child target sessions.
                // Match Playwright's OOPIF model in crPage.ts by enabling
                // Runtime on the owning session before waiting on its context.
                page.execute_with_session(EnableParams::default(), session_id.clone())
                    .await
                    .map_err(|e| {
                        format!("failed to enable runtime for frame target lookup: {e}")
                    })?;
                runtime_enabled_session = Some(session_id.clone());
            }
        }
        let context = page
            .frame_execution_context(frame_id.clone())
            .await
            .map_err(|e| format!("failed to query frame execution context: {e}"))?;

        if let (Some(context_id), Some(session_id)) = (context, session) {
            return Ok((context_id, session_id));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting for frame execution target (frame id {})",
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
                const containsComposed = (target, hit) => {
                    // 1. Walk up the normal DOM and open shadow roots
                    let cur = hit;
                    while (cur) {
                        if (cur === target) return true;
                        cur = cur.parentNode || (cur instanceof ShadowRoot ? cur.host : null);
                    }
                    
                    // 2. If the hit element has a closed shadow root, check if target is inside.
                    const checkDeepContains = (parent, node) => {
                        if (parent === node) return true;
                        const root = parent.shadowRoot || parent.openOrClosedShadowRoot;
                        if (root && checkDeepContains(root, node)) return true;
                        for (const child of parent.children || []) {
                            if (checkDeepContains(child, node)) return true;
                        }
                        return false;
                    };
                    
                    return checkDeepContains(hit, target);
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
///
/// Username-role secrets are read from kSecAttrAccount (no biometric on macOS).
/// Password-role secrets trigger biometric on macOS.
/// Legacy secrets not in the new domain-credential scheme are read via the old
/// per-(domain,name) keychain entries.
pub(crate) async fn resolve_secret_if_applicable(
    inner: &PageInner,
    value: &str,
) -> JsResult<String> {
    let referenced_name = value.trim();
    if referenced_name.is_empty() {
        return Ok(value.to_string());
    }

    let declared_domains = declared_domains_for_secret(&inner.declared_secrets, referenced_name);
    // Also check legacy store for unconfigured-but-stored names when fallback
    // is enabled during migration rollout.
    let legacy_known = if ENABLE_LEGACY_SECRET_FALLBACK {
        inner.secret_store.list_legacy_entries().unwrap_or_default()
    } else {
        Vec::new()
    };
    let configured_legacy = legacy_known.iter().any(|(_, name)| name == referenced_name);
    if declared_domains.is_empty() && !configured_legacy {
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

    // Try new domain-credential scheme first.
    let username_role =
        is_username_role(&inner.declared_secrets, &top_level_domain, referenced_name);
    if username_role {
        if let Ok(v) = inner.secret_store.get_username(&top_level_domain) {
            return Ok(v);
        }
    } else if let Ok(v) = inner.secret_store.get_password(&top_level_domain) {
        return Ok(v);
    }

    // Legacy fallback: old per-(domain, name) keychain entries.
    if ENABLE_LEGACY_SECRET_FALLBACK {
        for (domain, name) in &legacy_known {
            if name == referenced_name && domain.eq_ignore_ascii_case(&top_level_domain) {
                return inner
                    .secret_store
                    .get_legacy_value(domain, name)
                    .map_err(|e| {
                        js_err(format!(
                            "failed to read secret '{name}' for domain '{domain}': {e}"
                        ))
                    });
            }
        }
    }

    Err(js_err(format!(
        "Secret '{referenced_name}' was declared for '{top_level_domain}' but is not stored for that domain"
    )))
}

fn declared_domains_for_secret(declared: &SecretDeclarations, secret_name: &str) -> Vec<String> {
    let mut domains = declared
        .iter()
        .filter_map(|(domain, creds)| {
            let declared_here = creds.username.as_deref() == Some(secret_name)
                || creds.password.as_deref() == Some(secret_name)
                || creds.extra_names.iter().any(|n| n == secret_name);
            if declared_here {
                Some(domain.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    domains.sort();
    domains
}

/// Whether `secret_name` is the username role for `domain` in the declarations.
fn is_username_role(declared: &SecretDeclarations, domain: &str, secret_name: &str) -> bool {
    declared.get(domain).and_then(|c| c.username.as_deref()) == Some(secret_name)
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
pub type PromptUiHandler =
    Arc<dyn Fn(String) -> Result<Option<String>, String> + Send + Sync + 'static>;

pub struct RefreshmintInner {
    pub output_dir: PathBuf,
    pub prompt_overrides: PromptOverrides,
    pub prompt_requires_override: bool,
    pub script_options: ScriptOptions,
    pub debug_output_sink: Option<tokio::sync::mpsc::UnboundedSender<DebugOutputEvent>>,
    pub session_metadata: SessionMetadata,
    pub staged_resources: Vec<StagedResource>,
    pub scrape_session_id: String,
    pub extension_name: String,
    pub account_name: String,
    pub login_name: String,
    pub ledger_dir: PathBuf,
    /// When set, `prompt()` asks the host app for a response instead of
    /// reading from stdin.
    pub prompt_ui_handler: Option<PromptUiHandler>,
}

fn resolve_prompt_response(response: Option<String>) -> JsResult<String> {
    match response {
        Some(answer) => Ok(answer),
        None => Err(js_err("prompt cancelled".to_string())),
    }
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

fn parse_goto_options(options: Option<rquickjs::Value<'_>>) -> JsResult<GotoOptions> {
    let mut wait_until = "load".to_string();
    let mut timeout_ms = DEFAULT_TIMEOUT_MS;
    if let Some(opts) = options {
        let Some(obj) = opts.as_object() else {
            return Err(js_err(
                "goto options must be an object when provided".to_string(),
            ));
        };
        if let Ok(Some(wait_until_value)) = obj.get::<_, Option<String>>("waitUntil") {
            wait_until = wait_until_value;
        }
        if let Ok(Some(timeout)) = obj.get::<_, Option<u64>>("timeout") {
            timeout_ms = timeout;
        } else if let Ok(Some(timeout)) = obj.get::<_, Option<f64>>("timeout") {
            if timeout.is_sign_negative() {
                return Err(js_err(
                    "goto timeout must be a non-negative number".to_string(),
                ));
            }
            timeout_ms = timeout as u64;
        }
    }

    if wait_until == "networkidle0" {
        wait_until = "networkidle".to_string();
    }
    if wait_until != "load"
        && wait_until != "domcontentloaded"
        && wait_until != "networkidle"
        && wait_until != "commit"
    {
        return Err(js_err(format!(
            "waitUntil: expected one of (load|domcontentloaded|networkidle|commit), got {wait_until}"
        )));
    }

    Ok(GotoOptions {
        wait_until,
        timeout_ms,
    })
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
    ///
    /// In the Tauri UI context (`prompt_ui_handler` is set), asks the host app
    /// for a response and blocks until it returns one. In CLI context, reads
    /// from stdin as before.
    pub fn prompt(&self, message: String) -> JsResult<String> {
        let (override_value, require_override, prompt_ui_handler) = {
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
                inner.prompt_ui_handler.clone(),
            )
        };

        if let Some(value) = override_value {
            return Ok(value);
        }

        if require_override {
            return Err(js_err(missing_prompt_override_error(&message)));
        }

        // UI context: ask the host app to collect a response. `prompt()`
        // runs on a spawn_blocking thread so a blocking callback is safe.
        if let Some(prompt_ui_handler) = prompt_ui_handler {
            let response = prompt_ui_handler(message).map_err(js_err)?;
            return resolve_prompt_response(response);
        }

        // CLI context: read from stdin.
        eprint!("{message} ");
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .map_err(|e| js_err(format!("prompt read failed: {e}")))?;
        Ok(line.trim_end().to_string())
    }

    /// Return CLI `--option` key/value pairs as a native JS object.
    /// Returns `{}` when no options were supplied.
    #[qjs(rename = "getOptions")]
    pub fn get_options(&self) -> JsResult<JsEvalResult> {
        let inner = self
            .inner
            .try_lock()
            .map_err(|_| js_err("getOptions unavailable: state is busy".to_string()))?;
        let json = serde_json::to_string(&inner.script_options)
            .map_err(|e| js_err(format!("getOptions serialization: {e}")))?;
        // Wrap in parens so `{}` is parsed as an object literal, not a block statement.
        Ok(JsEvalResult::Json(format!("({json})")))
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
/// Convert a QuickJS `Value` (or array of values) to a list of `CallArgument`s
/// for use with `Runtime.callFunctionOn`.
///
/// `JSHandle` and `ElementHandle` instances are passed by `objectId`.
/// All other values are serialised to JSON via `serde_json`.
fn js_value_to_call_args(
    val: &rquickjs::Value<'_>,
) -> Result<Vec<chromiumoxide::cdp::js_protocol::runtime::CallArgument>, String> {
    use chromiumoxide::cdp::js_protocol::runtime::CallArgument;

    fn single_arg(v: &rquickjs::Value<'_>) -> Result<CallArgument, String> {
        // Check if it's a JSHandle instance.
        if let Ok(cls) = Class::<JsHandle>::from_value(v) {
            let object_id = cls.borrow().object_id.clone();
            return Ok(CallArgument {
                value: None,
                unserializable_value: None,
                object_id: Some(object_id.into()),
            });
        }
        // Check if it's an ElementHandle instance.
        if let Ok(cls) = Class::<ElementHandle>::from_value(v) {
            let object_id = cls.borrow().object_id.clone();
            return Ok(CallArgument {
                value: None,
                unserializable_value: None,
                object_id: Some(object_id.into()),
            });
        }
        // Fallback: serialise as JSON.
        let json_val =
            rquickjs_value_to_json(v).map_err(|e| format!("could not serialise argument: {e}"))?;
        Ok(CallArgument {
            value: Some(json_val),
            unserializable_value: None,
            object_id: None,
        })
    }

    // If the value is an Array, expand it into individual args.
    if let Some(arr) = val.as_array() {
        let len = arr.len();
        let mut out = Vec::with_capacity(len);
        for i in 0..len {
            let item: rquickjs::Value<'_> = arr
                .get(i)
                .map_err(|e| format!("failed to get arg {i}: {e}"))?;
            out.push(single_arg(&item)?);
        }
        return Ok(out);
    }

    // Single argument.
    Ok(vec![single_arg(val)?])
}

/// Best-effort serialisation of a `rquickjs::Value` to `serde_json::Value`.
fn rquickjs_value_to_json(val: &rquickjs::Value<'_>) -> Result<serde_json::Value, String> {
    if val.is_null() || val.is_undefined() {
        return Ok(serde_json::Value::Null);
    }
    if let Some(b) = val.as_bool() {
        return Ok(serde_json::Value::Bool(b));
    }
    if let Some(i) = val.as_int() {
        return Ok(serde_json::Value::Number(i.into()));
    }
    if let Some(f) = val.as_float() {
        if let Some(n) = serde_json::Number::from_f64(f) {
            return Ok(serde_json::Value::Number(n));
        }
        return Ok(serde_json::Value::Null);
    }
    if let Some(s) = val.as_string() {
        let rust_str = s
            .to_string()
            .map_err(|e| format!("string conversion failed: {e}"))?;
        return Ok(serde_json::Value::String(rust_str));
    }
    if let Some(arr) = val.as_array() {
        let len = arr.len();
        let mut out = Vec::with_capacity(len);
        for i in 0..len {
            let item: rquickjs::Value<'_> = arr.get(i).map_err(|e| format!("array[{i}]: {e}"))?;
            out.push(rquickjs_value_to_json(&item)?);
        }
        return Ok(serde_json::Value::Array(out));
    }
    if let Some(obj) = val.as_object() {
        let mut map = serde_json::Map::new();
        for key in obj.keys::<rquickjs::String<'_>>() {
            let k = key.map_err(|e| format!("object key: {e}"))?;
            let k_str = k.to_string().map_err(|e| format!("object key str: {e}"))?;
            let v: rquickjs::Value<'_> = obj
                .get(k.clone())
                .map_err(|e| format!("object[{k_str}]: {e}"))?;
            map.insert(k_str, rquickjs_value_to_json(&v)?);
        }
        return Ok(serde_json::Value::Object(map));
    }
    Err(format!("unsupported value type: {}", val.type_name()))
}

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
            "https://example.com/**"
        ));
        assert!(!url_matches_pattern(
            "https://example.com/a/b/c",
            "https://example.com/*"
        ));
        assert!(!url_matches_pattern(
            "https://example.com/a/b/c",
            "https://example.org/*"
        ));
    }

    #[test]
    fn url_matches_pattern_groups_and_escaped_literals() {
        assert!(url_matches_pattern(
            "https://example.com/login/callback",
            "https://example.com/{login,signin}/callback"
        ));
        assert!(url_matches_pattern(
            "https://example.com/file[1].csv",
            r"https://example.com/file\[1\].csv"
        ));
        assert!(!url_matches_pattern(
            "https://example.com/signout/callback",
            "https://example.com/{login,signin}/callback"
        ));
    }

    #[test]
    fn response_timing_maps_to_playwright_shape() {
        use chromiumoxide::cdp::browser_protocol::network::ResourceTiming;

        let timing = ResourceTiming::builder()
            .request_time(123.5)
            .proxy_start(-1.0)
            .proxy_end(-1.0)
            .dns_start(2.0)
            .dns_end(3.0)
            .connect_start(4.0)
            .connect_end(5.0)
            .ssl_start(4.5)
            .ssl_end(4.9)
            .worker_start(-1.0)
            .worker_ready(-1.0)
            .worker_fetch_start(-1.0)
            .worker_respond_with_settled(-1.0)
            .send_start(6.0)
            .send_end(7.0)
            .push_start(-1.0)
            .push_end(-1.0)
            .receive_headers_start(8.0)
            .receive_headers_end(9.0)
            .build()
            .unwrap_or_else(|err| panic!("timing should build: {err}"));

        let mapped = response_timing_to_request_timing(Some(&timing));
        assert_eq!(
            mapped,
            RequestTiming {
                start_time: 123_500.0,
                domain_lookup_start: 2.0,
                domain_lookup_end: 3.0,
                connect_start: 4.0,
                secure_connection_start: 4.5,
                connect_end: 5.0,
                request_start: 6.0,
                response_start: 9.0,
                response_end: 9.0,
            }
        );
    }

    #[test]
    fn timing_response_end_uses_monotonic_timestamp_relative_to_start_time() {
        let timing = RequestTiming {
            start_time: 1000.0,
            response_start: 120.0,
            response_end: 120.0,
            ..RequestTiming::default_playwright()
        };
        let response_end = timing_response_end_from_timestamp(&timing, 1.250);
        assert_eq!(response_end, 250.0);
    }

    #[test]
    fn header_values_splits_newline_joined_values() {
        let mut headers = BTreeMap::new();
        headers.insert(
            "set-cookie".to_string(),
            "a=1; Path=/\nb=2; Path=/".to_string(),
        );
        assert_eq!(
            header_values(&headers, "set-cookie"),
            vec!["a=1; Path=/".to_string(), "b=2; Path=/".to_string()]
        );
    }

    #[test]
    fn allocate_request_hop_reuses_raw_id_but_rotates_public_id_on_redirect() {
        let next_request_id = AtomicU64::new(1);
        let mut current_ids = BTreeMap::new();

        let (first_id, first_previous) =
            allocate_request_hop("raw-1", false, &mut current_ids, &next_request_id);
        assert_eq!(first_id, "request-1");
        assert_eq!(first_previous, None);

        let (second_id, second_previous) =
            allocate_request_hop("raw-1", true, &mut current_ids, &next_request_id);
        assert_eq!(second_id, "request-2");
        assert_eq!(second_previous.as_deref(), Some("request-1"));
        assert_eq!(
            current_ids.get("raw-1").map(String::as_str),
            Some("request-2")
        );
    }

    #[test]
    fn current_request_id_for_raw_uses_latest_redirect_hop() {
        let next_request_id = AtomicU64::new(1);
        let mut current_ids = BTreeMap::new();

        let _ = allocate_request_hop("raw-1", false, &mut current_ids, &next_request_id);
        let _ = allocate_request_hop("raw-1", true, &mut current_ids, &next_request_id);

        let current_ids = Arc::new(std::sync::Mutex::new(current_ids));
        assert_eq!(
            current_request_id_for_raw(&current_ids, "raw-1"),
            "request-2"
        );
        assert_eq!(
            current_request_id_for_raw(&current_ids, "raw-missing"),
            "raw-missing"
        );
    }

    #[test]
    fn build_redirect_response_entry_uses_previous_request_identity() {
        let request_item = RequestCaptureItem {
            request_id: "request-2".to_string(),
            raw_request_id: "raw-1".to_string(),
            url: "https://example.com/final".to_string(),
            method: "GET".to_string(),
            headers: std::collections::BTreeMap::new(),
            resource_type: "document".to_string(),
            post_data: None,
            frame_id: Some("frame-1".to_string()),
            is_navigation_request: true,
            redirected_from: Some("request-1".to_string()),
            error: None,
            finished: false,
            timing: RequestTiming::default_playwright(),
        };
        let redirect_response: chromiumoxide::cdp::browser_protocol::network::Response =
            serde_json::from_value(serde_json::json!({
                "url": "https://example.com/start",
                "status": 302,
                "statusText": "Found",
                "headers": {
                    "location": "https://example.com/final"
                },
                "mimeType": "text/html",
                "charset": "utf-8",
                "connectionReused": false,
                "connectionId": 1,
                "encodedDataLength": 0,
                "securityState": "neutral",
                "requestHeaders": {
                    ":method": "GET"
                }
            }))
            .unwrap_or_else(|err| panic!("redirect response should deserialize: {err}"));

        let redirect_entry = build_redirect_response_entry(
            "request-1".to_string(),
            &request_item,
            &redirect_response,
            1234,
        );
        assert_eq!(redirect_entry.request_id, "request-1");
        assert_eq!(redirect_entry.status, 302);
        assert!(!redirect_entry.ok);
        assert_eq!(redirect_entry.url, "https://example.com/start");
        assert_eq!(redirect_entry.method, "GET");
        assert!(redirect_entry.finished);
        assert_eq!(
            redirect_entry.request_id_raw.as_ref().map(|id| id.as_ref()),
            Some("raw-1")
        );
    }

    #[tokio::test]
    async fn resolve_response_request_id_prefers_previous_redirect_hop() {
        let request_entries = Arc::new(Mutex::new(vec![
            RequestCaptureItem {
                request_id: "request-1".to_string(),
                raw_request_id: "raw-1".to_string(),
                url: "https://example.com/start".to_string(),
                method: "GET".to_string(),
                headers: BTreeMap::new(),
                resource_type: "document".to_string(),
                post_data: None,
                frame_id: Some("frame-1".to_string()),
                is_navigation_request: true,
                redirected_from: None,
                error: None,
                finished: false,
                timing: RequestTiming::default_playwright(),
            },
            RequestCaptureItem {
                request_id: "request-2".to_string(),
                raw_request_id: "raw-1".to_string(),
                url: "https://example.com/final".to_string(),
                method: "GET".to_string(),
                headers: BTreeMap::new(),
                resource_type: "document".to_string(),
                post_data: None,
                frame_id: Some("frame-1".to_string()),
                is_navigation_request: true,
                redirected_from: Some("request-1".to_string()),
                error: None,
                finished: false,
                timing: RequestTiming::default_playwright(),
            },
        ]));
        let response_entries = Arc::new(Mutex::new(Vec::new()));
        let raw_request_current_ids = Arc::new(std::sync::Mutex::new(BTreeMap::from([(
            "raw-1".to_string(),
            "request-2".to_string(),
        )])));

        let resolved = PageApi::resolve_response_request_id(
            &request_entries,
            &response_entries,
            &raw_request_current_ids,
            "raw-1",
            302,
        )
        .await;
        assert_eq!(resolved.as_deref(), Some("request-1"));

        response_entries.lock().await.push(NetworkRequest {
            request_id: "request-1".to_string(),
            url: "https://example.com/start".to_string(),
            status: 302,
            ok: false,
            method: "GET".to_string(),
            status_text: "Found".to_string(),
            headers: std::collections::BTreeMap::new(),
            frame_id: Some("frame-1".to_string()),
            from_service_worker: false,
            ts: 0,
            error: None,
            finished: true,
            timing: RequestTiming::default_playwright(),
            server_addr: None,
            security_details: None,
            request_id_raw: Some(
                chromiumoxide::cdp::browser_protocol::network::RequestId::new("raw-1"),
            ),
        });

        let duplicate = PageApi::resolve_response_request_id(
            &request_entries,
            &response_entries,
            &raw_request_current_ids,
            "raw-1",
            302,
        )
        .await;
        assert!(duplicate.is_none());
    }

    #[test]
    fn linked_response_for_request_falls_back_to_raw_request_id() {
        let response = NetworkRequest {
            request_id: "84477.1".to_string(),
            url: "https://example.com/final".to_string(),
            status: 200,
            ok: true,
            method: "POST".to_string(),
            status_text: "OK".to_string(),
            headers: BTreeMap::new(),
            frame_id: Some("frame-1".to_string()),
            from_service_worker: false,
            ts: 0,
            error: None,
            finished: true,
            timing: RequestTiming::default_playwright(),
            server_addr: None,
            security_details: None,
            request_id_raw: Some(
                chromiumoxide::cdp::browser_protocol::network::RequestId::new("84477.1"),
            ),
        };
        let linked = linked_response_for_request(
            std::slice::from_ref(&response),
            "request-1",
            "84477.1",
            "https://example.com/final",
            None,
        );
        assert_eq!(linked.map(|entry| entry.status), Some(200));
    }

    #[test]
    fn linked_request_for_response_falls_back_to_raw_request_id() {
        let request = RequestCaptureItem {
            request_id: "request-1".to_string(),
            raw_request_id: "84477.1".to_string(),
            url: "https://example.com/final".to_string(),
            method: "POST".to_string(),
            headers: BTreeMap::new(),
            resource_type: "fetch".to_string(),
            post_data: None,
            frame_id: Some("frame-1".to_string()),
            is_navigation_request: false,
            redirected_from: None,
            error: None,
            finished: false,
            timing: RequestTiming::default_playwright(),
        };
        let linked = linked_request_for_response(
            std::slice::from_ref(&request),
            "84477.1",
            Some(&chromiumoxide::cdp::browser_protocol::network::RequestId::new("84477.1")),
            "https://example.com/final",
            200,
        );
        assert_eq!(
            linked.map(|entry| entry.request_id),
            Some("request-1".to_string())
        );
    }

    #[test]
    fn linked_redirected_requests_fall_back_to_raw_request_id() {
        let initial = RequestCaptureItem {
            request_id: "request-1".to_string(),
            raw_request_id: "84477.1".to_string(),
            url: "https://example.com/start".to_string(),
            method: "GET".to_string(),
            headers: BTreeMap::new(),
            resource_type: "document".to_string(),
            post_data: None,
            frame_id: Some("frame-1".to_string()),
            is_navigation_request: true,
            redirected_from: None,
            error: None,
            finished: false,
            timing: RequestTiming::default_playwright(),
        };
        let redirected = RequestCaptureItem {
            request_id: "request-2".to_string(),
            raw_request_id: "84477.1".to_string(),
            url: "https://example.com/final".to_string(),
            method: "GET".to_string(),
            headers: BTreeMap::new(),
            resource_type: "document".to_string(),
            post_data: None,
            frame_id: Some("frame-1".to_string()),
            is_navigation_request: true,
            redirected_from: Some("request-1".to_string()),
            error: None,
            finished: false,
            timing: RequestTiming::default_playwright(),
        };

        let entries = vec![initial.clone(), redirected.clone()];
        let linked_to =
            linked_redirected_to_request(&entries, "84477.1", "84477.1").map(|entry| entry.url);
        assert_eq!(linked_to.as_deref(), Some("https://example.com/final"));

        let linked_from = linked_redirected_from_request(
            &entries,
            "84477.1",
            "84477.1",
            Some(&"request-1".to_string()),
        )
        .map(|entry| entry.url);
        assert_eq!(linked_from.as_deref(), Some("https://example.com/start"));
    }

    #[test]
    fn navigation_request_matches_playwright_chromium_rule() {
        use chromiumoxide::cdp::browser_protocol::network::EventRequestWillBeSent;

        let navigation_event: EventRequestWillBeSent = serde_json::from_value(serde_json::json!({
            "requestId": "loader-1",
            "loaderId": "loader-1",
            "documentURL": "https://example.com/start",
            "request": {
                "url": "https://example.com/start",
                "method": "GET",
                "headers": {},
                "initialPriority": "Medium",
                "referrerPolicy": "strict-origin-when-cross-origin"
            },
            "timestamp": 1.0,
            "wallTime": 1.0,
            "initiator": { "type": "parser" },
            "redirectHasExtraInfo": false,
            "type": "Document"
        }))
        .unwrap_or_else(|err| panic!("navigation event should deserialize: {err}"));
        assert!(is_navigation_request(&navigation_event));

        let subresource_event: EventRequestWillBeSent = serde_json::from_value(serde_json::json!({
            "requestId": "request-2",
            "loaderId": "loader-1",
            "documentURL": "https://example.com/start",
            "request": {
                "url": "https://example.com/app.js",
                "method": "GET",
                "headers": {},
                "initialPriority": "Medium",
                "referrerPolicy": "strict-origin-when-cross-origin"
            },
            "timestamp": 2.0,
            "wallTime": 2.0,
            "initiator": { "type": "script" },
            "redirectHasExtraInfo": false,
            "type": "Script"
        }))
        .unwrap_or_else(|err| panic!("subresource event should deserialize: {err}"));
        assert!(!is_navigation_request(&subresource_event));
    }

    #[test]
    fn declared_domains_for_secret_returns_sorted_domains() {
        let mut declared = SecretDeclarations::new();
        declared.insert(
            "b.com".to_string(),
            DomainCredentials {
                password: Some("password".to_string()),
                ..Default::default()
            },
        );
        declared.insert(
            "a.com".to_string(),
            DomainCredentials {
                password: Some("password".to_string()),
                extra_names: vec!["otp".to_string()],
                ..Default::default()
            },
        );
        declared.insert(
            "c.com".to_string(),
            DomainCredentials {
                username: Some("username".to_string()),
                ..Default::default()
            },
        );

        let domains = declared_domains_for_secret(&declared, "password");
        assert_eq!(domains, vec!["a.com".to_string(), "b.com".to_string()]);

        // extra_names are also found
        let otp_domains = declared_domains_for_secret(&declared, "otp");
        assert_eq!(otp_domains, vec!["a.com".to_string()]);
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
            script_options: ScriptOptions::new(),
            debug_output_sink: None,
            session_metadata: SessionMetadata::default(),
            staged_resources: Vec::new(),
            scrape_session_id: String::new(),
            extension_name: String::new(),
            account_name: String::new(),
            login_name: String::new(),
            ledger_dir: PathBuf::new(),
            prompt_ui_handler: None,
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
    fn resolve_prompt_response_returns_submitted_empty_string() {
        let value = resolve_prompt_response(Some(String::new()))
            .unwrap_or_else(|err| panic!("expected prompt response value: {err}"));
        assert_eq!(value, "");
    }

    #[test]
    fn resolve_prompt_response_rejects_cancel() {
        let err = resolve_prompt_response(None)
            .err()
            .unwrap_or_else(|| panic!("expected prompt cancellation"));
        assert!(err.to_string().contains("prompt cancelled"));
    }

    #[test]
    fn apply_pending_request_lifecycle_marks_finished_requests() {
        let mut entry = RequestCaptureItem {
            request_id: "req-1".to_string(),
            raw_request_id: "raw-1".to_string(),
            url: "https://example.com".to_string(),
            method: "POST".to_string(),
            headers: BTreeMap::new(),
            resource_type: "fetch".to_string(),
            post_data: None,
            frame_id: None,
            is_navigation_request: false,
            redirected_from: None,
            error: Some("old".to_string()),
            finished: false,
            timing: RequestTiming::default_playwright(),
        };

        apply_pending_request_lifecycle(&mut entry, Some(&PendingRequestLifecycleState::Finished));

        assert!(entry.finished);
        assert_eq!(entry.error, None);
    }

    #[test]
    fn apply_pending_request_lifecycle_marks_failed_requests() {
        let mut entry = RequestCaptureItem {
            request_id: "req-2".to_string(),
            raw_request_id: "raw-2".to_string(),
            url: "https://example.com".to_string(),
            method: "POST".to_string(),
            headers: BTreeMap::new(),
            resource_type: "fetch".to_string(),
            post_data: None,
            frame_id: None,
            is_navigation_request: false,
            redirected_from: None,
            error: None,
            finished: false,
            timing: RequestTiming::default_playwright(),
        };

        apply_pending_request_lifecycle(
            &mut entry,
            Some(&PendingRequestLifecycleState::Failed(
                "net::ERR_ABORTED".to_string(),
            )),
        );

        assert!(entry.finished);
        assert_eq!(entry.error.as_deref(), Some("net::ERR_ABORTED"));
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

    #[test]
    fn resolve_screenshot_output_path_rejects_absolute_paths() {
        let root = PathBuf::from("/tmp/downloads");
        let err = resolve_screenshot_output_path(&root, Some("/tmp/elsewhere/out.png"))
            .err()
            .unwrap_or_else(|| panic!("absolute path should fail"));
        assert!(err.to_string().contains("relative"));
    }

    #[test]
    fn resolve_screenshot_output_path_rejects_parent_traversal() {
        let root = PathBuf::from("/tmp/downloads");
        let err = resolve_screenshot_output_path(&root, Some("../escape.png"))
            .err()
            .unwrap_or_else(|| panic!("parent traversal should fail"));
        assert!(err.to_string().contains("parent directory"));
    }

    #[test]
    fn resolve_screenshot_output_path_joins_relative_paths_under_download_dir() {
        let root = PathBuf::from("/tmp/downloads");
        let path = resolve_screenshot_output_path(&root, Some("nested/out.png"))
            .unwrap_or_else(|err| panic!("relative path should work: {err}"))
            .unwrap_or_else(|| panic!("path should be present"));
        assert_eq!(path, root.join("nested/out.png"));
    }

    #[test]
    fn url_matches_pattern_bare_star_does_not_match_http_url() {
        // Single "*" only matches strings with no slashes — real HTTP URLs always
        // contain slashes and are never matched by bare "*".  This documents why
        // waitForEvent("request") must NOT be routed through
        // wait_for_request_pattern("*"): it would silently drop all events.
        assert!(!url_matches_pattern("http://127.0.0.1:8080/api/echo", "*"));
        assert!(!url_matches_pattern("https://example.com/path", "*"));
        // A string with no slashes does match "*".
        assert!(url_matches_pattern("noslash", "*"));
    }
}
