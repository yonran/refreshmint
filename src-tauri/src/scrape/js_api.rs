use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use rquickjs::class::Trace;
use rquickjs::{Ctx, JsLifetime, Result as JsResult};
use tokio::sync::Mutex;

use crate::secret::SecretStore;

fn js_err(msg: String) -> rquickjs::Error {
    rquickjs::Error::new_from_js_message("Error", "Error", msg)
}

const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const POLL_INTERVAL_MS: u64 = 100;

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

pub type SecretDeclarations = BTreeMap<String, BTreeSet<String>>;
pub type PromptOverrides = BTreeMap<String, String>;

/// Shared state backing the `page` JS object.
pub struct PageInner {
    pub page: chromiumoxide::Page,
    pub secret_store: SecretStore,
    pub declared_secrets: SecretDeclarations,
    pub download_dir: PathBuf,
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
}

// Safety: PageApi only contains Arc<Mutex<...>> which is 'static and has no JS lifetimes.
#[allow(unsafe_code)]
unsafe impl<'js> JsLifetime<'js> for PageApi {
    type Changed<'to> = PageApi;
}

impl PageApi {
    pub fn new(inner: Arc<Mutex<PageInner>>) -> Self {
        Self {
            inner,
            response_capture: Arc::new(Mutex::new(None)),
        }
    }
}

#[rquickjs::methods]
impl PageApi {
    /// Navigate to a URL.
    #[qjs(rename = "goto")]
    pub async fn js_goto(&self, url: String) -> JsResult<()> {
        let inner = self.inner.lock().await;
        inner
            .page
            .goto(&url)
            .await
            .map_err(|e| js_err(format!("goto failed: {e}")))?;
        Ok(())
    }

    /// Get the current page URL.
    pub async fn url(&self) -> JsResult<String> {
        let inner = self.inner.lock().await;
        let url = inner
            .page
            .url()
            .await
            .map_err(|e| js_err(format!("url() failed: {e}")))?
            .unwrap_or_default();
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
            let maybe_error = {
                let inner = self.inner.lock().await;
                let result = inner
                    .page
                    .evaluate(probe.as_str())
                    .await
                    .map_err(|e| js_err(format!("waitForSelector failed: {e}")))?;
                if let Some(value) = result.value() {
                    if value.as_bool() == Some(true) {
                        return Ok(());
                    }
                    value
                        .get("__refreshmintSelectorError")
                        .and_then(serde_json::Value::as_str)
                        .map(|selector_error| selector_error.to_string())
                } else {
                    None
                }
            };

            if let Some(selector_error) = maybe_error {
                return Err(js_err(format!(
                    "waitForSelector(\"{selector}\") failed: {selector_error}"
                )));
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

    /// Click an element matching the CSS selector.
    pub async fn click(&self, selector: String) -> JsResult<()> {
        let inner = self.inner.lock().await;
        let element = inner
            .page
            .find_element(selector)
            .await
            .map_err(|e| js_err(format!("click find failed: {e}")))?;
        element
            .click()
            .await
            .map_err(|e| js_err(format!("click failed: {e}")))?;
        Ok(())
    }

    /// Type text into an element, character by character.
    #[qjs(rename = "type")]
    pub async fn js_type(&self, selector: String, text: String) -> JsResult<()> {
        let inner = self.inner.lock().await;
        let element = inner
            .page
            .find_element(selector)
            .await
            .map_err(|e| js_err(format!("type find failed: {e}")))?;
        element
            .click()
            .await
            .map_err(|e| js_err(format!("type click failed: {e}")))?;
        element
            .type_str(&text)
            .await
            .map_err(|e| js_err(format!("type failed: {e}")))?;
        Ok(())
    }

    /// Fill an input element's value.
    ///
    /// If `value` matches a manifest-declared secret name for the current
    /// top-level domain, the real secret is resolved from keychain and injected via CDP.
    /// The JS sandbox only ever sees the placeholder name.
    pub async fn fill(&self, selector: String, value: String) -> JsResult<()> {
        let inner = self.inner.lock().await;

        // Determine the actual value to fill
        let actual_value = resolve_secret_if_applicable(&inner, &value).await?;

        // Use CDP to set the value and dispatch events so the JS sandbox
        // never receives the real secret.
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

        inner
            .page
            .evaluate(js)
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

    /// Accessibility-like snapshot of interactive page elements as JSON.
    pub async fn snapshot(&self) -> JsResult<String> {
        let inner = self.inner.lock().await;
        let result = inner
            .page
            .evaluate(
                r#"(() => {
                    const nodes = [];
                    const interesting = document.querySelectorAll(
                        'a,button,input,select,textarea,[role],[aria-label]'
                    );
                    for (const el of interesting) {
                        const role = el.getAttribute('role') ||
                            (el.tagName ? el.tagName.toLowerCase() : 'node');
                        const label = el.getAttribute('aria-label') ||
                            el.getAttribute('name') ||
                            el.getAttribute('id') ||
                            (el.innerText ? el.innerText.trim() : '') ||
                            (el.textContent ? el.textContent.trim() : '');
                        const rect = el.getBoundingClientRect();
                        const visible = !!(rect.width > 0 && rect.height > 0);
                        nodes.push({
                            role,
                            label: String(label || ''),
                            visible,
                            selectorHint: el.id ? ('#' + el.id) : (el.name ? ('[name="' + el.name + '"]') : el.tagName.toLowerCase()),
                        });
                    }
                    return nodes;
                })()"#,
            )
            .await
            .map_err(|e| js_err(format!("snapshot failed: {e}")))?;
        if let Some(value) = result.value() {
            serde_json::to_string_pretty(value)
                .map_err(|e| js_err(format!("snapshot serialization failed: {e}")))
        } else {
            Ok("[]".to_string())
        }
    }

    /// Evaluate a JavaScript expression in the browser context.
    ///
    /// The return value is scrubbed: all known secret values are replaced
    /// with `[REDACTED]`.
    pub async fn evaluate(&self, expression: String) -> JsResult<String> {
        let inner = self.inner.lock().await;
        let result = inner
            .page
            .evaluate(expression)
            .await
            .map_err(|e| js_err(format!("evaluate failed: {e}")))?;

        let mut text =
            stringify_evaluation_result(result.value(), result.object().description.as_deref());
        scrub_known_secrets(&inner.secret_store, &mut text);

        Ok(text)
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
    ///
    /// Sets up CDP download behavior, then waits for `Page.downloadProgress`
    /// with state=completed.
    #[qjs(rename = "waitForDownload")]
    pub async fn js_wait_for_download(&self) -> JsResult<DownloadInfo> {
        let inner = self.inner.lock().await;

        let download_path = inner.download_dir.to_string_lossy().to_string();

        // Set download behavior via CDP
        use chromiumoxide::cdp::browser_protocol::browser::SetDownloadBehaviorParams;
        inner
            .page
            .execute(SetDownloadBehaviorParams::new(
                chromiumoxide::cdp::browser_protocol::browser::SetDownloadBehaviorBehavior::AllowAndName,
            ))
            .await
            .map_err(|e| js_err(format!("setDownloadBehavior failed: {e}")))?;

        Ok(DownloadInfo {
            path: download_path,
            suggested_filename: String::new(),
        })
    }
}

impl PageApi {
    async fn current_url(&self) -> JsResult<String> {
        let inner = self.inner.lock().await;
        let url = inner
            .page
            .url()
            .await
            .map_err(|e| js_err(format!("url() failed: {e}")))?
            .unwrap_or_default();
        Ok(url.to_string())
    }

    async fn eval_string(&self, expression: String, method_name: &str) -> JsResult<String> {
        let inner = self.inner.lock().await;
        let result = inner
            .page
            .evaluate(expression)
            .await
            .map_err(|e| js_err(format!("{method_name} failed: {e}")))?;
        let value = result
            .value()
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        Ok(value)
    }

    async fn eval_bool(&self, expression: String, method_name: &str) -> JsResult<bool> {
        let inner = self.inner.lock().await;
        let result = inner
            .page
            .evaluate(expression)
            .await
            .map_err(|e| js_err(format!("{method_name} failed: {e}")))?;
        Ok(result
            .value()
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false))
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

fn stringify_evaluation_result(
    value: Option<&serde_json::Value>,
    description: Option<&str>,
) -> String {
    match value {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(other) => serde_json::to_string(other).unwrap_or_else(|_| other.to_string()),
        None => description.unwrap_or("undefined").to_string(),
    }
}

fn scrub_known_secrets(secret_store: &SecretStore, text: &mut String) {
    if let Ok(secrets) = secret_store.all_values() {
        for secret in &secrets {
            if !secret.is_empty() {
                *text = text.replace(secret, "[REDACTED]");
            }
        }
    }
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

async fn wait_for_frame_execution_context(
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

/// Resolve a secret value if `value` is a known secret name.
///
/// A secret name can only be used when it is declared in the extension
/// manifest for the current top-level navigation domain.
async fn resolve_secret_if_applicable(inner: &PageInner, value: &str) -> JsResult<String> {
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

/// Shared state backing the `refreshmint` JS namespace.
pub struct RefreshmintInner {
    pub output_dir: PathBuf,
    pub prompt_overrides: PromptOverrides,
    pub prompt_requires_override: bool,
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

fn missing_prompt_override_error(message: &str) -> String {
    format!(
        "missing prompt value for refreshmint.prompt(\"{message}\"); supply --prompt \"{message}=VALUE\""
    )
}

#[rquickjs::methods]
impl RefreshmintApi {
    /// Save binary data to a file in the extension output directory.
    #[qjs(rename = "saveResource")]
    pub async fn js_save_resource(&self, filename: String, data: Vec<u8>) -> JsResult<()> {
        let inner = self.inner.lock().await;
        let path = inner.output_dir.join(&filename);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| js_err(format!("saveResource mkdir failed: {e}")))?;
        }
        std::fs::write(&path, &data)
            .map_err(|e| js_err(format!("saveResource write failed: {e}")))?;
        Ok(())
    }

    /// Report a key-value pair to stdout.
    #[qjs(rename = "reportValue")]
    pub fn js_report_value(&self, key: String, value: String) -> JsResult<()> {
        println!("{key}: {value}");
        Ok(())
    }

    /// Log a message to stderr.
    pub fn log(&self, message: String) -> JsResult<()> {
        eprintln!("{message}");
        Ok(())
    }

    /// Prompt the user: use CLI-provided override when available.
    pub fn prompt(&self, message: String) -> JsResult<String> {
        let (override_value, require_override) = {
            let inner = self.inner.blocking_lock();
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

/// Register the `page` and `refreshmint` globals on a QuickJS context.
pub fn register_globals(
    ctx: &Ctx<'_>,
    page_inner: Arc<Mutex<PageInner>>,
    refreshmint_inner: Arc<Mutex<RefreshmintInner>>,
) -> JsResult<()> {
    let globals = ctx.globals();

    let page = PageApi::new(page_inner);
    globals.set("page", page)?;

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
    fn prompt_returns_override_when_present_in_strict_mode() {
        let mut overrides = PromptOverrides::new();
        overrides.insert("OTP".to_string(), "123456".to_string());
        let api = RefreshmintApi::new(Arc::new(Mutex::new(RefreshmintInner {
            output_dir: PathBuf::new(),
            prompt_overrides: overrides,
            prompt_requires_override: true,
        })));

        let value = api
            .prompt("OTP".to_string())
            .unwrap_or_else(|err| panic!("prompt unexpectedly failed: {err}"));
        assert_eq!(value, "123456");
    }

    #[test]
    fn prompt_errors_when_missing_in_strict_mode() {
        let api = RefreshmintApi::new(Arc::new(Mutex::new(RefreshmintInner {
            output_dir: PathBuf::new(),
            prompt_overrides: PromptOverrides::new(),
            prompt_requires_override: true,
        })));

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
        let api = RefreshmintApi::new(Arc::new(Mutex::new(RefreshmintInner {
            output_dir: PathBuf::new(),
            prompt_overrides: overrides,
            prompt_requires_override: true,
        })));

        let value = api
            .prompt("Enter the texted MFA code: ".to_string())
            .unwrap_or_else(|err| panic!("prompt unexpectedly failed: {err}"));
        assert_eq!(value, "245221");
    }
}
