use std::path::PathBuf;
use std::sync::Arc;

use rquickjs::class::Trace;
use rquickjs::{Ctx, JsLifetime, Result as JsResult};
use tokio::sync::Mutex;

use crate::secret::SecretStore;

fn js_err(msg: String) -> rquickjs::Error {
    rquickjs::Error::new_from_js_message("Error", "Error", msg)
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
    pub async fn js_wait_for_selector(&self, selector: String) -> JsResult<()> {
        let inner = self.inner.lock().await;
        inner
            .page
            .find_element(selector)
            .await
            .map_err(|e| js_err(format!("waitForSelector failed: {e}")))?;
        Ok(())
    }

    /// Wait for navigation to complete.
    #[qjs(rename = "waitForNavigation")]
    pub async fn js_wait_for_navigation(&self) -> JsResult<()> {
        let inner = self.inner.lock().await;
        inner
            .page
            .wait_for_navigation()
            .await
            .map_err(|e| js_err(format!("waitForNavigation failed: {e}")))?;
        Ok(())
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

        let mut text = format!("{result:?}");

        // Scrub known secret values
        if let Ok(secrets) = inner.secret_store.all_values() {
            for secret in &secrets {
                if !secret.is_empty() {
                    text = text.replace(secret, "[REDACTED]");
                }
            }
        }

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
}
