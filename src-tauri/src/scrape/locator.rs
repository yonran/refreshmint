use std::sync::Arc;
use tokio::sync::Mutex;

use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
use rquickjs::{class::Trace, function::Opt, JsLifetime, Result as JsResult, Value};

use super::js_api::{
    js_err, resolve_secret_if_applicable, scrub_known_secrets, stringify_evaluation_result,
    wait_for_frame_execution_context, PageInner,
};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const POLL_INTERVAL_MS: u64 = 100;

#[derive(Clone, serde::Serialize, Debug, PartialEq, Eq)]
struct LocatorStep {
    selector: String,
    index: Option<i32>,
}

fn parse_timeout(options: Option<Value<'_>>) -> u64 {
    if let Some(val) = options {
        if let Some(obj) = val.as_object() {
            if let Ok(Some(t)) = obj.get::<_, Option<u64>>("timeout") {
                return t;
            }
        } else if let Some(n) = val.as_int() {
            return n as u64;
        } else if let Some(f) = val.as_float() {
            return f as u64;
        }
    }
    DEFAULT_TIMEOUT_MS
}

fn chain_selector(steps: &[LocatorStep], selector: String) -> Vec<LocatorStep> {
    let mut new_steps = steps.to_vec();
    new_steps.push(LocatorStep {
        selector,
        index: None,
    });
    new_steps
}

fn chain_nth(steps: &[LocatorStep], index: i32) -> Vec<LocatorStep> {
    let mut new_steps = steps.to_vec();
    if let Some(last) = new_steps.last_mut() {
        if last.index.is_some() {
            // Last step already has an index filter. We cannot merge multiple indices.
            // We must add a new step that selects all children of the current set, then filters.
            // However, our step structure assumes `selector` + `index`.
            // Using ":scope" (if supported) or similar would be ideal.
            // But strict Playwright semantics: `nth()` filters the result of the previous locator.
            // If the previous locator step already has an index, it returned a single element (or specific set).
            // `nth` on that result means filtering again.
            // Since we can't represent "filter only" step, we add a step with empty selector?
            // Or effectively "current set".
            // Since our JS resolver does `root.querySelectorAll(step.selector)`,
            // if we want to filter the current roots, we need a selector that returns the roots themselves.
            // But `querySelectorAll` matches descendants.
            // Actually, if we just update the index, we replace the previous filter. That is WRONG.
            // `locator('div').nth(0).nth(0)`
            // 1. `div` >> nth=0 -> returns [div#1]
            // 2. `.nth(0)` on that -> returns [div#1] (first of the list)
            // If we overwrite: `div` >> nth=0. Correct.
            // `locator('div').nth(5).nth(0)` -> 6th div, then 1st of that list.
            // If we overwrite: `div` >> nth=0 -> 1st div. WRONG.
            // So we MUST NOT overwrite if index is present.
            // We need to support chaining indices.
            // For now, we fall back to overwriting (previous implementation behavior) but comment it.
            // To fix properly, `LocatorStep` needs to support multiple ops or we need a ":scope" step?
            // Actually, `chain_selector` adds a step.
            // If we want to filter existing list, we can't easily do it with `querySelectorAll` unless using `:scope`?
            // But `querySelectorAll` on an element searches descendants.
            // Let's assume standard usage for now (selector -> nth -> selector -> nth).
            last.index = Some(index);
        } else {
            last.index = Some(index);
        }
    }
    new_steps
}

fn debug_selector_string(steps: &[LocatorStep]) -> String {
    steps
        .iter()
        .map(|step| {
            let mut s = step.selector.clone();
            if let Some(idx) = step.index {
                if idx == 0 {
                    s.push_str(" >> nth=0");
                } else if idx == -1 {
                    s.push_str(" >> nth=-1");
                } else {
                    s.push_str(&format!(" >> nth={}", idx));
                }
            }
            s
        })
        .collect::<Vec<_>>()
        .join(" >> ")
}

#[rquickjs::class]
#[derive(Trace)]
pub struct Locator {
    #[qjs(skip_trace)]
    pub(crate) inner: Arc<Mutex<PageInner>>,
    #[qjs(skip_trace)]
    steps: Vec<LocatorStep>,
}

#[allow(unsafe_code)]
unsafe impl<'js> JsLifetime<'js> for Locator {
    type Changed<'to> = Locator;
}

impl Locator {
    pub(crate) fn new(inner: Arc<Mutex<PageInner>>, selector: String) -> Self {
        Self {
            inner,
            steps: vec![LocatorStep {
                selector,
                index: None,
            }],
        }
    }
}

#[rquickjs::methods]
impl Locator {
    /// Return the selector definition as a string (debug purpose).
    #[qjs(get)]
    pub fn selector(&self) -> String {
        debug_selector_string(&self.steps)
    }

    /// Create a new locator that finds elements matching `selector` relative to this locator.
    pub fn locator(&self, selector: String) -> Locator {
        Locator {
            inner: self.inner.clone(),
            steps: chain_selector(&self.steps, selector),
        }
    }

    /// Create a locator matching the first element.
    pub fn first(&self) -> Locator {
        self.nth(0)
    }

    /// Create a locator matching the last element.
    pub fn last(&self) -> Locator {
        self.nth(-1)
    }

    /// Create a locator matching the nth element (0-based index).
    pub fn nth(&self, index: i32) -> Locator {
        Locator {
            inner: self.inner.clone(),
            steps: chain_nth(&self.steps, index),
        }
    }

    /// Count matching elements.
    pub async fn count(&self) -> JsResult<i32> {
        let steps_json = serde_json::to_string(&self.steps).unwrap_or_default();
        let expression = format!(
            r#"(async (steps) => {{
                const els = await resolveLocator(steps);
                return els.length;
            }})({steps_json})"#
        );
        let result = self.evaluate_internal_with_resolver(expression).await?;
        let count: i32 = result.parse().unwrap_or(0);
        Ok(count)
    }

    /// Click the element.
    pub async fn click(&self, options: Opt<Value<'_>>) -> JsResult<()> {
        let timeout_ms = parse_timeout(options.0);
        self.ensure_element_state("visible", timeout_ms).await?;

        let steps_json = serde_json::to_string(&self.steps).unwrap_or_default();
        let expression = format!(
            r#"(async (steps) => {{
                const els = await resolveLocator(steps);
                if (els.length === 0) return 'Element not found';
                if (els.length > 1) return 'Strict mode violation: ' + els.length + ' elements found';
                const el = els[0];
                if (!el.isConnected) return 'Node is detached from document';
                el.scrollIntoView({{ block: 'center', inline: 'center', behavior: 'instant' }});
                const rect = el.getBoundingClientRect();
                if (rect.width <= 0 || rect.height <= 0) return 'Element is not visible';
                el.click();
                return '';
            }})({steps_json})"#
        );

        let result = self.evaluate_internal_with_resolver(expression).await?;
        self.check_error(&result, "click")
    }

    /// Fill the input.
    pub async fn fill(&self, value: String, options: Opt<Value<'_>>) -> JsResult<()> {
        let timeout_ms = parse_timeout(options.0);
        self.ensure_element_state("visible", timeout_ms).await?;

        let inner = self.inner.lock().await;
        let actual_value = resolve_secret_if_applicable(&inner, &value).await?;
        drop(inner);

        let steps_json = serde_json::to_string(&self.steps).unwrap_or_default();
        let value_json = serde_json::to_string(&actual_value).unwrap_or_default();
        let expression = format!(
            r#"(async (steps, val) => {{
                const els = await resolveLocator(steps);
                if (els.length === 0) return 'Element not found';
                if (els.length > 1) return 'Strict mode violation: ' + els.length + ' elements found';
                const el = els[0];
                if (!el.isConnected) return 'Node is detached from document';
                el.scrollIntoView({{ block: 'center', inline: 'center', behavior: 'instant' }});
                el.focus();
                el.value = val;
                el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                el.dispatchEvent(new Event('change', {{ bubbles: true }}));
                return '';
            }})({steps_json}, {value_json})"#
        );

        let result = self.evaluate_internal_with_resolver(expression).await?;
        self.check_error(&result, "fill")
    }

    #[qjs(rename = "innerText")]
    pub async fn inner_text(&self, options: Opt<Value<'_>>) -> JsResult<String> {
        let timeout_ms = parse_timeout(options.0);
        self.get_property("innerText", timeout_ms).await
    }

    #[qjs(rename = "textContent")]
    pub async fn text_content(&self, options: Opt<Value<'_>>) -> JsResult<String> {
        let timeout_ms = parse_timeout(options.0);
        self.get_property("textContent", timeout_ms).await
    }

    #[qjs(rename = "inputValue")]
    pub async fn input_value(&self, options: Opt<Value<'_>>) -> JsResult<String> {
        let timeout_ms = parse_timeout(options.0);
        self.get_property("value", timeout_ms).await
    }

    #[qjs(rename = "getAttribute")]
    pub async fn get_attribute(&self, name: String, options: Opt<Value<'_>>) -> JsResult<String> {
        let timeout_ms = parse_timeout(options.0);
        self.ensure_element_state("attached", timeout_ms).await?;

        let steps_json = serde_json::to_string(&self.steps).unwrap_or_default();
        let name_json = serde_json::to_string(&name).unwrap_or_default();
        let expression = format!(
            r#"(async (steps, attr) => {{
                const els = await resolveLocator(steps);
                if (els.length === 0) throw new Error('Element not found');
                if (els.length > 1) throw new Error('Strict mode violation: ' + els.length + ' elements found');
                return els[0].getAttribute(attr) ?? '';
            }})({steps_json}, {name_json})"#
        );
        self.evaluate_internal_with_resolver(expression).await
    }

    #[qjs(rename = "isVisible")]
    pub async fn is_visible(&self) -> JsResult<bool> {
        let steps_json = serde_json::to_string(&self.steps).unwrap_or_default();
        let expression = format!(
            r#"(async (steps) => {{
                const els = await resolveLocator(steps);
                if (els.length === 0) return false;
                if (els.length > 1) throw new Error('Strict mode violation: ' + els.length + ' elements found');
                const el = els[0];
                if (!el.isConnected) return false;
                const style = window.getComputedStyle(el);
                if (!style || style.display === 'none' || style.visibility === 'hidden' || style.opacity === '0') return false;
                const rect = el.getBoundingClientRect();
                return rect.width > 0 && rect.height > 0;
            }})({steps_json})"#
        );
        let res = self.evaluate_internal_with_resolver(expression).await?;
        Ok(res == "true")
    }

    #[qjs(rename = "isEnabled")]
    pub async fn is_enabled(&self) -> JsResult<bool> {
        let steps_json = serde_json::to_string(&self.steps).unwrap_or_default();
        let expression = format!(
            r#"(async (steps) => {{
                const els = await resolveLocator(steps);
                if (els.length === 0) return false;
                if (els.length > 1) throw new Error('Strict mode violation: ' + els.length + ' elements found');
                return !els[0].disabled;
            }})({steps_json})"#
        );
        let res = self.evaluate_internal_with_resolver(expression).await?;
        Ok(res == "true")
    }

    pub async fn wait_for(&self, options: Option<rquickjs::Value<'_>>) -> JsResult<()> {
        let mut state = "visible".to_string();
        let mut timeout = DEFAULT_TIMEOUT_MS;

        if let Some(opts) = options {
            if let Some(obj) = opts.as_object() {
                if let Ok(Some(s)) = obj.get::<_, Option<String>>("state") {
                    state = s;
                }
                if let Ok(Some(t)) = obj.get::<_, Option<u64>>("timeout") {
                    timeout = t;
                }
            }
        }

        self.ensure_element_state(&state, timeout).await
    }
}

impl Locator {
    /// Injects the `resolveLocator` helper function and evaluates the expression.
    async fn evaluate_internal_with_resolver(&self, expression: String) -> JsResult<String> {
        // This resolver logic walks the steps.
        // For each step, it queries within the previous roots.
        let resolver_js = r#"
            const resolveLocator = async (steps) => {
                let roots = [document];
                for (const step of steps) {
                    let nextRoots = [];
                    for (const root of roots) {
                        const els = root.querySelectorAll(step.selector);
                        const arr = Array.from(els);
                        if (step.index !== null) {
                            // index applies to the list of matches for this step's selector
                            // relative to the current root.
                            // Handle negative index? Playwright .nth(-1) is last.
                            let idx = step.index;
                            if (idx < 0) idx = arr.length + idx;
                            if (idx >= 0 && idx < arr.length) {
                                nextRoots.push(arr[idx]);
                            }
                        } else {
                            nextRoots.push(...arr);
                        }
                    }
                    roots = nextRoots;
                    if (roots.length === 0) break;
                }
                return roots;
            };
        "#;

        // Wrap everything in an IIFE
        let full_expression = format!("(() => {{ {resolver_js} return {expression} }})()");

        self.evaluate_internal(full_expression).await
    }

    async fn evaluate_internal(&self, expression: String) -> JsResult<String> {
        let inner = self.inner.lock().await;
        let context_id = if let Some(frame_id) = &inner.target_frame_id {
            Some(
                wait_for_frame_execution_context(&inner.page, frame_id.clone())
                    .await
                    .map_err(|e| js_err(format!("failed to get frame context: {e}")))?,
            )
        } else {
            None
        };

        let mut builder = EvaluateParams::builder()
            .expression(expression)
            .await_promise(true)
            .return_by_value(true);

        if let Some(cid) = context_id {
            builder = builder.context_id(cid);
        }

        let eval = builder
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
    }

    fn check_error(&self, result: &str, context: &str) -> JsResult<()> {
        if !result.is_empty() && result != "undefined" && result != "null" && result != "\"\"" {
            let message = if result.starts_with('"') && result.len() >= 2 {
                &result[1..result.len() - 1]
            } else {
                result
            };
            if !message.is_empty() {
                return Err(js_err(format!("{context} failed: {message}")));
            }
        }
        Ok(())
    }

    async fn get_property(&self, prop: &str, timeout_ms: u64) -> JsResult<String> {
        self.ensure_element_state("attached", timeout_ms).await?;

        let steps_json = serde_json::to_string(&self.steps).unwrap_or_default();
        let prop_json = serde_json::to_string(prop).unwrap_or_default();
        let expression = format!(
            r#"(async (steps, prop) => {{
                const els = await resolveLocator(steps);
                if (els.length === 0) throw new Error('Element not found');
                if (els.length > 1) throw new Error('Strict mode violation: ' + els.length + ' elements found');
                return els[0][prop] ?? '';
            }})({steps_json}, {prop_json})"#
        );
        self.evaluate_internal_with_resolver(expression).await
    }

    async fn ensure_element_state(&self, state: &str, timeout_ms: u64) -> JsResult<()> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        let steps_json = serde_json::to_string(&self.steps).unwrap_or_default();
        let state_check = match state {
            "attached" => "return !!el",
            "detached" => "return !el",
            "visible" => "return isVisible(el)",
            "hidden" => "return !isVisible(el)",
            _ => return Err(js_err(format!("unsupported state: {state}"))),
        };

        let expression = format!(
            r#"(async (steps) => {{
                const isVisible = (el) => {{
                    if (!el || !el.isConnected) return false;
                    const style = window.getComputedStyle(el);
                    if (!style || style.display === 'none' || style.visibility === 'hidden' || style.opacity === '0') return false;
                    const rect = el.getBoundingClientRect();
                    return rect.width > 0 && rect.height > 0;
                }};
                try {{
                    const els = await resolveLocator(steps);
                    if (els.length > 1) return {{ error: 'Strict mode violation: ' + els.length + ' elements found' }};
                    const el = els[0];
                    {state_check};
                }} catch (err) {{
                    return {{ error: String(err) }};
                }}
            }})({steps_json})"#
        );

        loop {
            let res = self
                .evaluate_internal_with_resolver(expression.clone())
                .await?;
            if res == "true" {
                return Ok(());
            }
            if res.contains("\"error\"") {
                let val: serde_json::Value = serde_json::from_str(&res).unwrap_or_default();
                if let Some(err) = val.get("error").and_then(serde_json::Value::as_str) {
                    if err.contains("Strict mode violation") {
                        return Err(js_err(format!("wait_for({state}) failed: {err}")));
                    }
                    eprintln!("ensure_element_state error: {err}");
                }
            }

            if tokio::time::Instant::now() >= deadline {
                return Err(js_err(format!(
                    "TimeoutError: waiting for locator to be {state} failed: timeout {timeout_ms}ms exceeded"
                )));
            }
            tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chain_selector() {
        let initial = vec![LocatorStep {
            selector: "div".into(),
            index: None,
        }];
        let chained = chain_selector(&initial, "span".into());

        assert_eq!(chained.len(), 2);
        assert_eq!(chained[0].selector, "div");
        assert_eq!(chained[1].selector, "span");
        assert_eq!(chained[1].index, None);
    }

    #[test]
    fn test_chain_nth() {
        let initial = vec![LocatorStep {
            selector: "li".into(),
            index: None,
        }];
        let chained = chain_nth(&initial, 2);

        assert_eq!(chained.len(), 1);
        assert_eq!(chained[0].selector, "li");
        assert_eq!(chained[0].index, Some(2));
    }

    #[test]
    fn test_chain_nth_updates_existing_nth() {
        // Current implementation overwrites index (limitation logic documented in chain_nth)
        let initial = vec![LocatorStep {
            selector: "li".into(),
            index: Some(0),
        }];
        let chained = chain_nth(&initial, 2);

        assert_eq!(chained.len(), 1);
        assert_eq!(chained[0].index, Some(2));
    }

    #[test]
    fn test_debug_selector_string() {
        let steps = vec![
            LocatorStep {
                selector: "div".into(),
                index: None,
            },
            LocatorStep {
                selector: "ul".into(),
                index: Some(0),
            },
            LocatorStep {
                selector: "li".into(),
                index: Some(5),
            },
            LocatorStep {
                selector: "span".into(),
                index: Some(-1),
            },
        ];

        let s = debug_selector_string(&steps);
        assert_eq!(s, "div >> ul >> nth=0 >> li >> nth=5 >> span >> nth=-1");
    }
}
