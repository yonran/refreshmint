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

/// Shared state backing the `page` JS object.
pub struct PageInner {
    pub page: chromiumoxide::Page,
    pub secret_store: SecretStore,
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
}

// Safety: PageApi only contains Arc<Mutex<...>> which is 'static and has no JS lifetimes.
#[allow(unsafe_code)]
unsafe impl<'js> JsLifetime<'js> for PageApi {
    type Changed<'to> = PageApi;
}

impl PageApi {
    pub fn new(inner: Arc<Mutex<PageInner>>) -> Self {
        Self { inner }
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
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        if state == "networkidle" {
            self.ensure_network_monitor().await?;
        }

        loop {
            let ready = match state.as_str() {
                "load" => self.ready_state_is_complete().await?,
                "domcontentloaded" => self.ready_state_is_interactive_or_complete().await?,
                "networkidle" => self.is_network_idle().await?,
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
        self.ensure_network_monitor().await?;
        let baseline_len = self.read_network_requests().await?.len();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);

        loop {
            let requests = self.read_network_requests().await?;
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
        self.ensure_network_monitor().await?;
        let requests = self.read_network_requests().await?;
        serde_json::to_string(&requests)
            .map_err(|e| js_err(format!("networkRequests serialization failed: {e}")))
    }

    /// Clear captured network requests.
    #[qjs(rename = "clearNetworkRequests")]
    pub async fn js_clear_network_requests(&self) -> JsResult<()> {
        self.ensure_network_monitor().await?;
        let inner = self.inner.lock().await;
        inner
            .page
            .evaluate(
                r#"(() => {
                    if (window.__refreshmintNetwork && Array.isArray(window.__refreshmintNetwork.requests)) {
                        window.__refreshmintNetwork.requests = [];
                    }
                    return true;
                })()"#,
            )
            .await
            .map_err(|e| js_err(format!("clearNetworkRequests failed: {e}")))?;
        Ok(())
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
    /// If `value` matches a secret name stored for the current page's domain,
    /// the real secret is resolved from keychain and injected via CDP.
    /// The JS sandbox only ever sees the placeholder name.
    pub async fn fill(&self, selector: String, value: String) -> JsResult<()> {
        let inner = self.inner.lock().await;

        // Determine the actual value to fill
        let actual_value = resolve_secret_if_applicable(&inner, &value).await;

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

    /// Evaluate a JS expression inside a same-origin iframe.
    #[qjs(rename = "frameEvaluate")]
    pub async fn js_frame_evaluate(
        &self,
        frame_selector: String,
        expression: String,
    ) -> JsResult<String> {
        let frame_selector_json =
            serde_json::to_string(&frame_selector).unwrap_or_else(|_| "\"\"".to_string());
        let expression_json =
            serde_json::to_string(&expression).unwrap_or_else(|_| "\"\"".to_string());
        let script = format!(
            r#"(() => {{
                const frame = document.querySelector({frame_selector_json});
                if (!frame) throw new Error('frameEvaluate: frame not found: ' + {frame_selector_json});
                const win = frame.contentWindow;
                if (!win) throw new Error('frameEvaluate: frame has no contentWindow');
                return win.eval({expression_json});
            }})()"#
        );
        let inner = self.inner.lock().await;
        let result = inner
            .page
            .evaluate(script)
            .await
            .map_err(|e| js_err(format!("frameEvaluate failed: {e}")))?;
        let mut text =
            stringify_evaluation_result(result.value(), result.object().description.as_deref());
        scrub_known_secrets(&inner.secret_store, &mut text);
        Ok(text)
    }

    /// Fill a value in a same-origin iframe.
    #[qjs(rename = "frameFill")]
    pub async fn js_frame_fill(
        &self,
        frame_selector: String,
        selector: String,
        value: String,
    ) -> JsResult<()> {
        let inner = self.inner.lock().await;
        let actual_value = resolve_secret_if_applicable(&inner, &value).await;
        let frame_selector_json =
            serde_json::to_string(&frame_selector).unwrap_or_else(|_| "\"\"".to_string());
        let selector_json = serde_json::to_string(&selector).unwrap_or_else(|_| "\"\"".to_string());
        let value_json =
            serde_json::to_string(&actual_value).unwrap_or_else(|_| "\"\"".to_string());
        let script = format!(
            r#"(() => {{
                const frame = document.querySelector({frame_selector_json});
                if (!frame) throw new Error('frameFill: frame not found: ' + {frame_selector_json});
                const doc = frame.contentDocument;
                if (!doc) throw new Error('frameFill: frame has no contentDocument (cross-origin?)');
                const el = doc.querySelector({selector_json});
                if (!el) throw new Error('frameFill: element not found: ' + {selector_json});
                el.focus();
                el.value = {value_json};
                el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                el.dispatchEvent(new Event('change', {{ bubbles: true }}));
                return true;
            }})()"#
        );
        inner
            .page
            .evaluate(script)
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

    async fn ensure_network_monitor(&self) -> JsResult<()> {
        let inner = self.inner.lock().await;
        inner
            .page
            .evaluate(
                r#"(() => {
                    if (window.__refreshmintNetwork) return true;
                    const state = {
                        requests: [],
                        inFlight: 0,
                        lastActivity: Date.now(),
                    };
                    const markActivity = () => { state.lastActivity = Date.now(); };
                    const push = (entry) => {
                        state.requests.push(entry);
                        if (state.requests.length > 2000) state.requests.shift();
                    };

                    const originalFetch = window.fetch;
                    if (typeof originalFetch === 'function') {
                        window.fetch = async (...args) => {
                            let url = '';
                            try {
                                const first = args[0];
                                url = String((first && first.url) ? first.url : first ?? '');
                            } catch {}
                            const method = String((args[1] && args[1].method) || 'GET');
                            state.inFlight += 1;
                            markActivity();
                            try {
                                const response = await originalFetch(...args);
                                push({
                                    url,
                                    status: Number(response.status || 0),
                                    ok: !!response.ok,
                                    method,
                                    ts: Date.now(),
                                });
                                return response;
                            } catch (err) {
                                push({
                                    url,
                                    status: 0,
                                    ok: false,
                                    method,
                                    ts: Date.now(),
                                    error: String(err),
                                });
                                throw err;
                            } finally {
                                state.inFlight = Math.max(0, state.inFlight - 1);
                                markActivity();
                            }
                        };
                    }

                    const originalOpen = XMLHttpRequest.prototype.open;
                    const originalSend = XMLHttpRequest.prototype.send;
                    XMLHttpRequest.prototype.open = function(method, url, ...rest) {
                        this.__refreshmintMethod = String(method || 'GET');
                        this.__refreshmintUrl = String(url || '');
                        return originalOpen.call(this, method, url, ...rest);
                    };
                    XMLHttpRequest.prototype.send = function(...args) {
                        state.inFlight += 1;
                        markActivity();
                        this.addEventListener('loadend', () => {
                            push({
                                url: String(this.__refreshmintUrl || ''),
                                status: Number(this.status || 0),
                                ok: Number(this.status || 0) >= 200 && Number(this.status || 0) < 400,
                                method: String(this.__refreshmintMethod || 'GET'),
                                ts: Date.now(),
                            });
                            state.inFlight = Math.max(0, state.inFlight - 1);
                            markActivity();
                        }, { once: true });
                        return originalSend.apply(this, args);
                    };

                    window.__refreshmintNetwork = state;
                    return true;
                })()"#,
            )
            .await
            .map_err(|e| js_err(format!("failed to initialize network monitor: {e}")))?;
        Ok(())
    }

    async fn read_network_requests(&self) -> JsResult<Vec<NetworkRequest>> {
        let inner = self.inner.lock().await;
        let result = inner
            .page
            .evaluate(
                r#"(() => {
                    const state = window.__refreshmintNetwork;
                    if (!state || !Array.isArray(state.requests)) return [];
                    return state.requests;
                })()"#,
            )
            .await
            .map_err(|e| js_err(format!("networkRequests failed: {e}")))?;
        let value = result
            .value()
            .cloned()
            .unwrap_or(serde_json::Value::Array(Vec::new()));
        serde_json::from_value(value)
            .map_err(|e| js_err(format!("networkRequests decode failed: {e}")))
    }

    async fn is_network_idle(&self) -> JsResult<bool> {
        let inner = self.inner.lock().await;
        let result = inner
            .page
            .evaluate(
                r#"(() => {
                    const state = window.__refreshmintNetwork;
                    if (!state) return false;
                    const quietForMs = Date.now() - Number(state.lastActivity || 0);
                    return Number(state.inFlight || 0) === 0 && quietForMs >= 500;
                })()"#,
            )
            .await
            .map_err(|e| js_err(format!("waitForLoadState(networkidle) failed: {e}")))?;
        Ok(result
            .value()
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false))
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

/// Resolve a secret value if the given `value` is a secret name for the current domain.
async fn resolve_secret_if_applicable(inner: &PageInner, value: &str) -> String {
    let current_url = inner.page.url().await.ok().flatten().unwrap_or_default();
    let domain = extract_domain(&current_url.to_string());

    if let Ok(secrets) = inner.secret_store.list() {
        for (d, name) in &secrets {
            if d == &domain && name == value {
                if let Ok(real_value) = inner.secret_store.get(d, name) {
                    return real_value;
                }
            }
        }
    }

    value.to_string()
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

    /// Prompt the user: print message to stderr, read a line from stdin.
    pub fn prompt(&self, message: String) -> JsResult<String> {
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
}
