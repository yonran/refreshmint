use std::sync::Arc;
use tokio::sync::Mutex;

use chromiumoxide::cdp::browser_protocol::dom::GetContentQuadsParams;
use chromiumoxide::cdp::js_protocol::runtime::{CallFunctionOnParams, EvaluateParams};
use chromiumoxide::layout::ElementQuad;
use rquickjs::{class::Trace, function::Opt, JsLifetime, Result as JsResult, Value};

use super::js_api::{
    js_err, resolve_secret_if_applicable, scrub_known_secrets, stringify_evaluation_result,
    wait_for_frame_execution_context, PageInner,
};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const POLL_INTERVAL_MS: u64 = 100;

const RESOLVER_JS: &str = r#"
    // Shadow-piercing querySelectorAll: matches selector in root then recurses
    // into every open shadow root found in root's subtree. Mirrors Playwright's
    // _queryCSS implementation.
    const queryAllDeep = (root, selector) => {
        const result = [];
        const query = (r) => {
            for (const el of r.querySelectorAll(selector)) result.push(el);
            if (r.shadowRoot) query(r.shadowRoot);
            for (const el of r.querySelectorAll('*')) {
                if (el.shadowRoot) query(el.shadowRoot);
            }
        };
        query(root);
        return result;
    };

    // Collect all descendant elements including those inside shadow roots.
    // Includes `root` itself when it is an Element (not document).
    const collectAllDeep = (root) => {
        const result = root === document ? [] : [root];
        const collect = (r) => {
            if (r.shadowRoot) collect(r.shadowRoot);
            for (const el of r.querySelectorAll('*')) {
                result.push(el);
                if (el.shadowRoot) collect(el.shadowRoot);
            }
        };
        collect(root);
        return result;
    };

    const IMPLICIT_ROLE = (el) => {
        const explicit = el.getAttribute('role');
        if (explicit) return explicit.trim().split(/\s+/)[0].toLowerCase();
        const tag = el.tagName.toLowerCase();
        const type = (el.getAttribute('type') || 'text').toLowerCase();
        if (tag === 'button') return 'button';
        if (tag === 'a' && el.hasAttribute('href')) return 'link';
        if (tag === 'input') {
            if (type === 'button' || type === 'submit' || type === 'reset' || type === 'image') return 'button';
            if (type === 'checkbox') return 'checkbox';
            if (type === 'radio') return 'radio';
            if (type === 'range') return 'slider';
            if (type === 'number') return 'spinbutton';
            if (type === 'search') return 'searchbox';
            return 'textbox';
        }
        if (tag === 'textarea') return 'textbox';
        if (tag === 'select') return el.hasAttribute('multiple') ? 'listbox' : 'combobox';
        if (tag === 'option') return 'option';
        if (tag === 'img') return 'img';
        if (/^h[1-6]$/.test(tag)) return 'heading';
        if (tag === 'summary') return 'button';
        if (tag === 'meter') return 'meter';
        if (tag === 'progress') return 'progressbar';
        if (tag === 'table') return 'table';
        if (tag === 'tr') return 'row';
        if (tag === 'td') return 'cell';
        if (tag === 'th') return 'columnheader';
        if (tag === 'li') return 'listitem';
        if (tag === 'nav') return 'navigation';
        if (tag === 'section' && (el.hasAttribute('aria-label') || el.hasAttribute('aria-labelledby'))) return 'region';
        return '';
    };

    const ACCESSIBLE_NAME = (el) => {
        const ariaLabel = (el.getAttribute('aria-label') || '').trim();
        if (ariaLabel) return ariaLabel;
        const lbIds = (el.getAttribute('aria-labelledby') || '').trim().split(/\s+/).filter(Boolean);
        if (lbIds.length) {
            const text = lbIds.map(id => { const r = document.getElementById(id); return r ? (r.innerText || r.textContent || '').trim() : ''; }).filter(Boolean).join(' ');
            if (text) return text;
        }
        if (el.labels && el.labels.length) {
            const text = Array.from(el.labels).map(l => (l.innerText || l.textContent || '').trim()).filter(Boolean).join(' ');
            if (text) return text;
        }
        const tag = el.tagName.toLowerCase();
        if (['button','a','h1','h2','h3','h4','h5','h6','summary'].includes(tag))
            return (el.innerText || el.textContent || '').trim();
        if (tag === 'img') return (el.getAttribute('alt') || '').trim();
        return (el.getAttribute('placeholder') || el.getAttribute('title') || '').trim();
    };

    const IS_ARIA_HIDDEN = (el) => {
        if (el.getAttribute('aria-hidden') === 'true') return true;
        const s = window.getComputedStyle(el);
        return s.display === 'none' || s.visibility === 'hidden';
    };

    const resolveLocator = async (steps) => {
        let roots = [document];
        for (const step of steps) {
            let nextRoots = [];
            if (step.type === 'role') {
                for (const root of roots) {
                    const candidates = collectAllDeep(root);
                    const matched = candidates.filter(el => {
                        if (IMPLICIT_ROLE(el) !== step.role) return false;
                        if (!step.includeHidden && IS_ARIA_HIDDEN(el)) return false;
                        if (step.name !== null && step.name !== undefined) {
                            const accName = ACCESSIBLE_NAME(el);
                            if (step.namePattern !== null && step.namePattern !== undefined) {
                                if (!new RegExp(step.namePattern, step.nameFlags || '').test(accName)) return false;
                            } else {
                                const a = accName.toLowerCase();
                                const b = step.name.toLowerCase();
                                if (step.exact ? a !== b : !a.includes(b)) return false;
                            }
                        }
                        if (step.checked !== null && step.checked !== undefined) {
                            const v = el.getAttribute('aria-checked');
                            const actual = v !== null ? v === 'true' : (typeof el.checked === 'boolean' ? el.checked : null);
                            if (actual !== step.checked) return false;
                        }
                        if (step.disabled !== null && step.disabled !== undefined) {
                            const dis = !!el.disabled || el.getAttribute('aria-disabled') === 'true';
                            if (dis !== step.disabled) return false;
                        }
                        if (step.expanded !== null && step.expanded !== undefined) {
                            const exp = el.getAttribute('aria-expanded');
                            if (exp === null || (exp === 'true') !== step.expanded) return false;
                        }
                        if (step.level !== null && step.level !== undefined) {
                            const m = el.tagName.match(/^H([1-6])$/i);
                            const lv = m ? parseInt(m[1]) : (el.getAttribute('aria-level') ? parseInt(el.getAttribute('aria-level')) : null);
                            if (lv !== step.level) return false;
                        }
                        if (step.pressed !== null && step.pressed !== undefined) {
                            const pr = el.getAttribute('aria-pressed');
                            if (pr === null || (pr === 'true') !== step.pressed) return false;
                        }
                        if (step.selected !== null && step.selected !== undefined) {
                            const v = el.getAttribute('aria-selected');
                            const actual = v !== null ? v === 'true' : (typeof el.selected === 'boolean' ? el.selected : null);
                            if (actual !== step.selected) return false;
                        }
                        return true;
                    });
                    if (step.index !== null && step.index !== undefined) {
                        let idx = step.index;
                        if (idx < 0) idx = matched.length + idx;
                        if (idx >= 0 && idx < matched.length) {
                            nextRoots.push(matched[idx]);
                        }
                    } else {
                        nextRoots.push(...matched);
                    }
                }
            } else {
                for (const root of roots) {
                    const arr = queryAllDeep(root, step.selector);
                    if (step.index !== null) {
                        let idx = step.index;
                        if (idx < 0) idx = arr.length + idx;
                        if (idx >= 0 && idx < arr.length) {
                            nextRoots.push(arr[idx]);
                        }
                    } else {
                        nextRoots.push(...arr);
                    }
                }
            }
            roots = nextRoots;
            if (roots.length === 0) break;
        }
        return roots;
    };
"#;

#[derive(Clone, serde::Serialize, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
enum LocatorStep {
    Css {
        selector: String,
        index: Option<i32>,
    },
    Role {
        role: String,
        /// Plain string name filter (used when name_pattern is None)
        name: Option<String>,
        /// Regex source (when name is a regex)
        name_pattern: Option<String>,
        /// Regex flags (when name is a regex)
        name_flags: Option<String>,
        /// true = case-sensitive full match for string names; false = case-insensitive substring
        exact: bool,
        checked: Option<bool>,
        disabled: Option<bool>,
        expanded: Option<bool>,
        include_hidden: bool,
        level: Option<u32>,
        pressed: Option<bool>,
        selected: Option<bool>,
        index: Option<i32>,
    },
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

/// Parse a `role=button[name="Log In"i][checked=true]` selector into a `LocatorStep::Role`.
/// Returns `None` if the selector does not start with `role=`.
fn parse_role_selector(s: &str) -> Option<LocatorStep> {
    let rest = s.strip_prefix("role=")?;

    // role name is everything up to the first '[' or end of string
    let (role_name, mut attrs_str) = match rest.find('[') {
        Some(pos) => (&rest[..pos], &rest[pos..]),
        None => (rest, ""),
    };
    let role = role_name.trim().to_string();
    if role.is_empty() {
        return None;
    }

    let mut name: Option<String> = None;
    let mut name_pattern: Option<String> = None;
    let mut name_flags: Option<String> = None;
    let mut exact = false;
    let mut checked: Option<bool> = None;
    let mut disabled: Option<bool> = None;
    let mut expanded: Option<bool> = None;
    let mut include_hidden = false;
    let mut level: Option<u32> = None;
    let mut pressed: Option<bool> = None;
    let mut selected: Option<bool> = None;

    // Parse attribute list: each attr is `[name=value]`
    while let Some(rest2) = attrs_str.strip_prefix('[') {
        // find the closing ']' — must account for quoted strings and regex
        let close = find_attr_close(rest2)?;
        let attr_content = &rest2[..close];
        attrs_str = &rest2[close + 1..];

        // split on first '='
        let eq = attr_content.find('=')?;
        let attr_name = attr_content[..eq].trim();
        let attr_val = attr_content[eq + 1..].trim();

        match attr_name {
            "name" => {
                if let Some(regex_str) = attr_val.strip_prefix('/') {
                    // regex: /pattern/flags
                    let (src, flags) = parse_regex_literal(regex_str);
                    name_pattern = Some(src.to_string());
                    name_flags = Some(flags.to_string());
                } else if attr_val.starts_with('"') || attr_val.starts_with('\'') {
                    // string with optional trailing 'i' or 's'
                    let (string_val, suffix) = parse_quoted_string(attr_val);
                    name = Some(string_val);
                    exact = suffix == "s";
                }
            }
            "checked" => checked = parse_bool(attr_val),
            "disabled" => disabled = parse_bool(attr_val),
            "expanded" => expanded = parse_bool(attr_val),
            "include-hidden" => include_hidden = attr_val == "true",
            "level" => level = attr_val.parse().ok(),
            "pressed" => pressed = parse_bool(attr_val),
            "selected" => selected = parse_bool(attr_val),
            _ => {} // unknown attr: ignore
        }
    }

    Some(LocatorStep::Role {
        role,
        name,
        name_pattern,
        name_flags,
        exact,
        checked,
        disabled,
        expanded,
        include_hidden,
        level,
        pressed,
        selected,
        index: None,
    })
}

/// Find the position of the closing `]` for an attribute value that may contain
/// quoted strings (`"..."` or `'...'`) or regex literals (`/.../`).
fn find_attr_close(s: &str) -> Option<usize> {
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            ']' => return Some(i),
            '"' | '\'' => {
                let q = chars[i];
                i += 1;
                while i < chars.len() && chars[i] != q {
                    if chars[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1; // skip closing quote
                        // skip optional suffix 'i' or 's'
                if i < chars.len() && (chars[i] == 'i' || chars[i] == 's') {
                    i += 1;
                }
            }
            '/' => {
                // regex literal
                i += 1;
                while i < chars.len() && chars[i] != '/' {
                    if chars[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1; // skip closing '/'
                        // skip flags
                while i < chars.len() && chars[i].is_alphabetic() {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    None
}

/// Parse a regex literal `pattern/flags` (the leading `/` has already been stripped).
fn parse_regex_literal(s: &str) -> (&str, &str) {
    // find closing `/` not preceded by `\`
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
        } else if bytes[i] == b'/' {
            let src = &s[..i];
            let flags = &s[i + 1..];
            return (src, flags);
        } else {
            i += 1;
        }
    }
    (s, "")
}

/// Parse a quoted string `"value"i` or `'value'i` and return (unescaped value, suffix).
fn parse_quoted_string(s: &str) -> (String, &str) {
    if s.is_empty() {
        return (String::new(), "");
    }
    let Some(quote) = s.chars().next() else {
        return (String::new(), "");
    };
    if quote != '"' && quote != '\'' {
        return (s.to_string(), "");
    }
    let inner = &s[1..];
    let mut result = String::new();
    let mut iter = inner.char_indices();
    let mut suffix_start = inner.len();
    while let Some((i, c)) = iter.next() {
        if c == '\\' {
            // take next char
            if let Some((_, nc)) = iter.next() {
                result.push(nc);
            }
        } else if c == quote {
            suffix_start = i + 1;
            break;
        } else {
            result.push(c);
        }
    }
    let suffix = &inner[suffix_start..];
    (result, suffix)
}

fn parse_bool(s: &str) -> Option<bool> {
    match s {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Build a `role=...` selector string from a role name and JS options object.
pub(crate) fn build_role_selector(role: &str, options: Option<Value<'_>>) -> String {
    let mut s = format!("role={role}");
    let Some(val) = options else {
        return s;
    };
    let Some(obj) = val.as_object() else {
        return s;
    };

    // name: string | RegExp
    // QuickJS RegExp objects have `source` and `flags` string properties.
    if let Ok(name_val) = obj.get::<_, Value<'_>>("name") {
        if let Some(name_str) = name_val.as_string() {
            if let Ok(name_string) = name_str.to_string() {
                // Check exact option
                let exact = obj.get::<_, bool>("exact").unwrap_or(false);
                let suffix = if exact { "s" } else { "i" };
                // Escape double quotes in the name
                let escaped = name_string.replace('\\', "\\\\").replace('"', "\\\"");
                s.push_str(&format!("[name=\"{escaped}\"{suffix}]"));
            }
        } else if name_val.is_object() {
            // Could be a RegExp: try to extract source and flags
            if let Some(name_obj) = name_val.as_object() {
                let source = name_obj.get::<_, String>("source").unwrap_or_default();
                let flags = name_obj.get::<_, String>("flags").unwrap_or_default();
                if !source.is_empty() {
                    s.push_str(&format!("[name=/{source}/{flags}]"));
                }
            }
        }
    }

    // exact is already consumed above for string names; no separate attr needed
    // checked
    if let Ok(v) = obj.get::<_, bool>("checked") {
        s.push_str(&format!("[checked={v}]"));
    }
    // disabled
    if let Ok(v) = obj.get::<_, bool>("disabled") {
        s.push_str(&format!("[disabled={v}]"));
    }
    // expanded
    if let Ok(v) = obj.get::<_, bool>("expanded") {
        s.push_str(&format!("[expanded={v}]"));
    }
    // includeHidden
    if let Ok(v) = obj.get::<_, bool>("includeHidden") {
        if v {
            s.push_str("[include-hidden=true]");
        }
    }
    // level
    if let Ok(v) = obj.get::<_, u32>("level") {
        s.push_str(&format!("[level={v}]"));
    }
    // pressed
    if let Ok(v) = obj.get::<_, bool>("pressed") {
        s.push_str(&format!("[pressed={v}]"));
    }
    // selected
    if let Ok(v) = obj.get::<_, bool>("selected") {
        s.push_str(&format!("[selected={v}]"));
    }

    s
}

fn chain_selector(steps: &[LocatorStep], selector: String) -> Vec<LocatorStep> {
    let mut new_steps = steps.to_vec();
    new_steps.push(LocatorStep::Css {
        selector,
        index: None,
    });
    new_steps
}

fn chain_nth(steps: &[LocatorStep], index: i32) -> Vec<LocatorStep> {
    let mut new_steps = steps.to_vec();
    if let Some(last) = new_steps.last_mut() {
        match last {
            LocatorStep::Css { index: idx, .. } => *idx = Some(index),
            LocatorStep::Role { index: idx, .. } => *idx = Some(index),
        }
    }
    new_steps
}

fn debug_selector_string(steps: &[LocatorStep]) -> String {
    steps
        .iter()
        .map(|step| {
            let (label, index) = match step {
                LocatorStep::Css { selector, index } => (selector.clone(), *index),
                LocatorStep::Role { role, index, .. } => (format!("role={role}"), *index),
            };
            let mut s = label;
            if let Some(idx) = index {
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
        let step = if let Some(role_step) = parse_role_selector(&selector) {
            role_step
        } else {
            LocatorStep::Css {
                selector,
                index: None,
            }
        };
        Self {
            inner,
            steps: vec![step],
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

    /// Create a new locator that finds elements by ARIA role relative to this locator.
    #[qjs(rename = "getByRole")]
    pub fn get_by_role(&self, role: String, options: Opt<Value<'_>>) -> Locator {
        let selector = build_role_selector(&role, options.0);
        let step = parse_role_selector(&selector).unwrap_or(LocatorStep::Css {
            selector,
            index: None,
        });
        let mut steps = self.steps.clone();
        steps.push(step);
        Locator {
            inner: self.inner.clone(),
            steps,
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

    /// Click the element using a trusted CDP mouse click (Input.dispatchMouseEvent).
    ///
    /// Unlike `el.click()` via Runtime.evaluate, this produces `isTrusted: true` events,
    /// which is required for sites that check event.isTrusted (e.g. login flows).
    pub async fn click(&self, options: Opt<Value<'_>>) -> JsResult<()> {
        let timeout_ms = parse_timeout(options.0);
        self.ensure_element_state("visible", timeout_ms).await?;

        // Hold the lock for the entire click sequence.
        let inner = self.inner.lock().await;

        // A. Determine frame execution context.
        let context_id = if let Some(frame_id) = &inner.target_frame_id {
            Some(
                wait_for_frame_execution_context(&inner.page, frame_id.clone())
                    .await
                    .map_err(|e| js_err(format!("click: frame context: {e}")))?,
            )
        } else {
            None
        };

        // B. Resolve element → RemoteObjectId via evaluate with return_by_value=false.
        let object_id = {
            let steps_json = serde_json::to_string(&self.steps).unwrap_or_default();
            let expression = format!(
                r#"(async (steps) => {{
                    const els = await resolveLocator(steps);
                    if (els.length === 0) throw new Error('Element not found');
                    if (els.length > 1) throw new Error('Strict mode violation: ' + els.length + ' elements found');
                    return els[0];
                }})({steps_json})"#
            );
            let full_expression = format!("(() => {{ {RESOLVER_JS} return {expression} }})()");

            let mut builder = EvaluateParams::builder()
                .expression(full_expression)
                .await_promise(true)
                .return_by_value(false);
            if let Some(cid) = context_id {
                builder = builder.context_id(cid);
            }
            let eval = builder
                .build()
                .map_err(|e| js_err(format!("click: params: {e}")))?;

            let result = inner
                .page
                .evaluate_expression(eval)
                .await
                .map_err(|e| js_err(format!("click: resolve element: {e}")))?;

            result
                .object()
                .object_id
                .clone()
                .ok_or_else(|| js_err("click: element resolved to null".to_string()))?
        };

        // C. Scroll into view and check actionability (detached, visible, not occluded).
        //    Uses shadow-aware elementFromPoint traversal to detect occlusion.
        let scroll = inner
            .page
            .execute(
                CallFunctionOnParams::builder()
                    .function_declaration(
                        r#"function() {
                        if (!this.isConnected) return 'Node is detached from document';
                        this.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
                        const rect = this.getBoundingClientRect();
                        if (rect.width <= 0 || rect.height <= 0) return 'Element is not visible';
                        const x = rect.left + rect.width / 2;
                        const y = rect.top + rect.height / 2;
                        const hit = document.elementFromPoint(x, y);
                        if (!hit) return 'Element is outside of the viewport';
                        const containsComposed = (root, node) => {
                            let cur = node;
                            while (cur) {
                                if (cur === root) return true;
                                cur = cur.parentNode || (cur instanceof ShadowRoot ? cur.host : null);
                            }
                            return false;
                        };
                        if (!containsComposed(this, hit)) {
                            const el = hit instanceof Element ? hit : hit.parentElement;
                            const tag = el ? el.tagName.toLowerCase() : 'unknown';
                            return tag + ' intercepts pointer events';
                        }
                        return '';
                    }"#,
                    )
                    .object_id(object_id.clone())
                    .await_promise(false)
                    .return_by_value(true)
                    .build()
                    .map_err(|e| js_err(format!("click: scroll params: {e}")))?,
            )
            .await
            .map_err(|e| js_err(format!("click: scroll: {e}")))?;
        let msg = scroll
            .result
            .result
            .value
            .as_ref()
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if !msg.is_empty() {
            return Err(js_err(format!("click: {msg}")));
        }

        // D. Get clickable coordinates — DOM.getContentQuads returns top-level viewport coords,
        //    so this works correctly even inside iframes.
        let quads = inner
            .page
            .execute(
                GetContentQuadsParams::builder()
                    .object_id(object_id)
                    .build(),
            )
            .await
            .map_err(|e| js_err(format!("click: getContentQuads: {e}")))?;
        let point = quads
            .quads
            .iter()
            .filter(|q| q.inner().len() == 8)
            .map(ElementQuad::from_quad)
            .filter(|q| q.quad_area() > 1.)
            .map(|q| q.quad_center())
            .next()
            .ok_or_else(|| js_err("click: element not visible in viewport".to_string()))?;

        // E. Trusted mouse click via Input.dispatchMouseEvent.
        inner
            .page
            .click(point)
            .await
            .map_err(|e| js_err(format!("click: dispatch: {e}")))?;

        Ok(())
    }

    /// Fill the input.
    pub async fn fill(&self, value: String, options: Opt<Value<'_>>) -> JsResult<()> {
        let timeout_ms = parse_timeout(options.0);
        self.ensure_element_state("visible", timeout_ms).await?;

        let inner = self.inner.lock().await;
        let actual_value: String = resolve_secret_if_applicable(&inner, &value).await?;
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
        let full_expression = format!("(() => {{ {RESOLVER_JS} return {expression} }})()");
        self.evaluate_internal(full_expression).await
    }

    async fn evaluate_internal(&self, expression: String) -> JsResult<String> {
        let inner = self.inner.lock().await;
        let context_id: Option<chromiumoxide::cdp::js_protocol::runtime::ExecutionContextId> =
            if let Some(frame_id) = &inner.target_frame_id {
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
        let initial = vec![LocatorStep::Css {
            selector: "div".into(),
            index: None,
        }];
        let chained = chain_selector(&initial, "span".into());

        assert_eq!(chained.len(), 2);
        assert!(matches!(&chained[0], LocatorStep::Css { selector, .. } if selector == "div"));
        assert!(matches!(&chained[1], LocatorStep::Css { selector, .. } if selector == "span"));
        assert!(matches!(&chained[1], LocatorStep::Css { index: None, .. }));
    }

    #[test]
    fn test_chain_nth() {
        let initial = vec![LocatorStep::Css {
            selector: "li".into(),
            index: None,
        }];
        let chained = chain_nth(&initial, 2);

        assert_eq!(chained.len(), 1);
        assert!(matches!(&chained[0], LocatorStep::Css { selector, .. } if selector == "li"));
        assert!(matches!(
            &chained[0],
            LocatorStep::Css { index: Some(2), .. }
        ));
    }

    #[test]
    fn test_chain_nth_updates_existing_nth() {
        // Current implementation overwrites index (limitation logic documented in chain_nth)
        let initial = vec![LocatorStep::Css {
            selector: "li".into(),
            index: Some(0),
        }];
        let chained = chain_nth(&initial, 2);

        assert_eq!(chained.len(), 1);
        assert!(matches!(
            &chained[0],
            LocatorStep::Css { index: Some(2), .. }
        ));
    }

    #[test]
    fn test_debug_selector_string() {
        let steps = vec![
            LocatorStep::Css {
                selector: "div".into(),
                index: None,
            },
            LocatorStep::Css {
                selector: "ul".into(),
                index: Some(0),
            },
            LocatorStep::Css {
                selector: "li".into(),
                index: Some(5),
            },
            LocatorStep::Css {
                selector: "span".into(),
                index: Some(-1),
            },
        ];

        let s = debug_selector_string(&steps);
        assert_eq!(s, "div >> ul >> nth=0 >> li >> nth=5 >> span >> nth=-1");
    }

    #[test]
    fn test_parse_role_selector_simple() {
        let Some(step) = parse_role_selector("role=button") else {
            panic!("expected role selector to parse");
        };
        assert!(matches!(&step, LocatorStep::Role { role, name: None, .. } if role == "button"));
    }

    #[test]
    fn test_parse_role_selector_with_name() {
        let Some(step) = parse_role_selector(r#"role=button[name="Log In"i]"#) else {
            panic!("expected role selector to parse");
        };
        match step {
            LocatorStep::Role {
                role, name, exact, ..
            } => {
                assert_eq!(role, "button");
                assert_eq!(name.as_deref(), Some("Log In"));
                assert!(!exact);
            }
            _ => panic!("expected Role step"),
        }
    }

    #[test]
    fn test_parse_role_selector_exact() {
        let Some(step) = parse_role_selector(r#"role=button[name="Submit"s]"#) else {
            panic!("expected role selector to parse");
        };
        match step {
            LocatorStep::Role { name, exact, .. } => {
                assert_eq!(name.as_deref(), Some("Submit"));
                assert!(exact);
            }
            _ => panic!("expected Role step"),
        }
    }

    #[test]
    fn test_parse_role_selector_regex() {
        let Some(step) = parse_role_selector(r#"role=heading[name=/^Log/i]"#) else {
            panic!("expected role selector to parse");
        };
        match step {
            LocatorStep::Role {
                role,
                name_pattern,
                name_flags,
                ..
            } => {
                assert_eq!(role, "heading");
                assert_eq!(name_pattern.as_deref(), Some("^Log"));
                assert_eq!(name_flags.as_deref(), Some("i"));
            }
            _ => panic!("expected Role step"),
        }
    }

    #[test]
    fn test_parse_role_selector_with_level() {
        let Some(step) = parse_role_selector("role=heading[level=2]") else {
            panic!("expected role selector to parse");
        };
        match step {
            LocatorStep::Role { role, level, .. } => {
                assert_eq!(role, "heading");
                assert_eq!(level, Some(2));
            }
            _ => panic!("expected Role step"),
        }
    }

    #[test]
    fn test_parse_role_selector_not_role() {
        assert!(parse_role_selector("button").is_none());
        assert!(parse_role_selector("[role=button]").is_none());
    }

    #[test]
    fn test_parse_role_selector_checked() {
        let Some(step) = parse_role_selector("role=checkbox[checked=true]") else {
            panic!("expected role selector to parse");
        };
        match step {
            LocatorStep::Role { checked, .. } => {
                assert_eq!(checked, Some(true));
            }
            _ => panic!("expected Role step"),
        }
    }

    #[test]
    fn test_parse_role_selector_multiple_attrs() {
        let Some(step) =
            parse_role_selector(r#"role=checkbox[name="Accept"i][checked=true][disabled=false]"#)
        else {
            panic!("expected role selector to parse");
        };
        match step {
            LocatorStep::Role {
                role,
                name,
                exact,
                checked,
                disabled,
                ..
            } => {
                assert_eq!(role, "checkbox");
                assert_eq!(name.as_deref(), Some("Accept"));
                assert!(!exact);
                assert_eq!(checked, Some(true));
                assert_eq!(disabled, Some(false));
            }
            _ => panic!("expected Role step"),
        }
    }

    #[test]
    fn test_parse_role_selector_bool_attrs() {
        let Some(step) = parse_role_selector(
            "role=button[disabled=true][expanded=false][pressed=true][selected=false]",
        ) else {
            panic!("expected role selector to parse");
        };
        match step {
            LocatorStep::Role {
                disabled,
                expanded,
                pressed,
                selected,
                ..
            } => {
                assert_eq!(disabled, Some(true));
                assert_eq!(expanded, Some(false));
                assert_eq!(pressed, Some(true));
                assert_eq!(selected, Some(false));
            }
            _ => panic!("expected Role step"),
        }
    }

    #[test]
    fn test_parse_role_selector_include_hidden() {
        let Some(step) = parse_role_selector("role=button[include-hidden=true]") else {
            panic!("expected role selector to parse");
        };
        match step {
            LocatorStep::Role { include_hidden, .. } => {
                assert!(include_hidden);
            }
            _ => panic!("expected Role step"),
        }
    }

    #[test]
    fn test_parse_role_selector_name_with_escaped_quote() {
        let Some(step) = parse_role_selector(r#"role=button[name="Say \"hi\""i]"#) else {
            panic!("expected role selector to parse");
        };
        match step {
            LocatorStep::Role { name, .. } => {
                assert_eq!(name.as_deref(), Some(r#"Say "hi""#));
            }
            _ => panic!("expected Role step"),
        }
    }

    #[test]
    fn test_build_role_selector_no_options() {
        let s = build_role_selector("button", None);
        assert_eq!(s, "role=button");
        // Should round-trip through parse
        let Some(step) = parse_role_selector(&s) else {
            panic!("expected role selector to parse");
        };
        assert!(matches!(&step, LocatorStep::Role { role, name: None, .. } if role == "button"));
    }

    #[test]
    fn test_chain_nth_on_role_step() {
        let initial = vec![LocatorStep::Role {
            role: "button".into(),
            name: None,
            name_pattern: None,
            name_flags: None,
            exact: false,
            checked: None,
            disabled: None,
            expanded: None,
            include_hidden: false,
            level: None,
            pressed: None,
            selected: None,
            index: None,
        }];
        let chained = chain_nth(&initial, 1);
        assert!(matches!(
            &chained[0],
            LocatorStep::Role { index: Some(1), .. }
        ));
    }

    #[test]
    fn test_debug_selector_string_with_role_step() {
        let steps = vec![
            LocatorStep::Css {
                selector: "form".into(),
                index: None,
            },
            LocatorStep::Role {
                role: "textbox".into(),
                name: None,
                name_pattern: None,
                name_flags: None,
                exact: false,
                checked: None,
                disabled: None,
                expanded: None,
                include_hidden: false,
                level: None,
                pressed: None,
                selected: None,
                index: Some(0),
            },
        ];
        let s = debug_selector_string(&steps);
        assert_eq!(s, "form >> role=textbox >> nth=0");
    }

    #[test]
    fn test_locator_new_role_selector() {
        // Verify Locator::new parses role= selectors into a Role step
        // (we test serialization instead of constructing a real Locator since that requires Arc<Mutex<PageInner>>)
        let Some(step) = parse_role_selector("role=textbox[name=\"Email\"i]") else {
            panic!("expected role selector to parse");
        };
        let json = match serde_json::to_string(&step) {
            Ok(json) => json,
            Err(err) => panic!("failed to serialize role step: {err}"),
        };
        assert!(json.contains("\"type\":\"role\""));
        assert!(json.contains("\"role\":\"textbox\""));
        assert!(json.contains("\"name\":\"Email\""));
    }

    #[test]
    fn test_css_step_serialization() {
        let step = LocatorStep::Css {
            selector: "button.primary".into(),
            index: Some(0),
        };
        let json = match serde_json::to_string(&step) {
            Ok(json) => json,
            Err(err) => panic!("failed to serialize css step: {err}"),
        };
        assert!(json.contains("\"type\":\"css\""));
        assert!(json.contains("\"selector\":\"button.primary\""));
        assert!(json.contains("\"index\":0"));
    }
}
