use app_lib::scrape::{self, ScrapeConfig};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const EXTENSION_NAME: &str = "smoke";
const LOGIN_NAME: &str = "smoke-account";
const DRIVER_SOURCE: &str = r##"
try {
  refreshmint.log("integration smoke start");
  const url = await page.url();
  refreshmint.reportValue("url", String(url));
  const evalResult = await page.evaluate("1 + 1");
  refreshmint.reportValue("eval", String(evalResult));
  await refreshmint.saveResource("smoke.bin", [111, 107]);
  refreshmint.log("integration smoke done");
} catch (e) {
  const msg = (e && (e.stack || e.message)) ? (e.stack || e.message) : String(e);
  refreshmint.log("integration smoke error: " + msg);
  throw e;
}
"##;

const POPUP_DRIVER_SOURCE: &str = r##"
try {
  refreshmint.log("integration popup start");
  await page.goto(__OPENER_URL__);
  const opener = page;
  const openerBefore = await opener.url();
  const popupPromise = opener.waitForEvent("popup", 10000);
  await page.evaluate(`(() => {
    document.getElementById('open').click();
    return "clicked";
  })()`);
  const popup = await popupPromise;
  await popup.waitForLoadState("domcontentloaded", 10000);
  const pages = await browser.pages();
  if (!Array.isArray(pages) || pages.length < 2) {
    throw new Error(`expected at least 2 pages, got ${Array.isArray(pages) ? pages.length : 'non-array'}`);
  }
  const marker = await popup.evaluate("document.getElementById('popup-marker') ? 'yes' : 'no'");
  if (marker !== "yes") {
    throw new Error("popup page did not contain expected marker");
  }
  const openerAfter = await opener.url();
  if (openerAfter !== openerBefore) {
    throw new Error(`opener page changed unexpectedly: ${openerAfter}`);
  }
  await refreshmint.saveResource("popup.bin", [111, 107]);
  refreshmint.log("integration popup done");
} catch (e) {
  const msg = (e && (e.stack || e.message)) ? (e.stack || e.message) : String(e);
  refreshmint.log("integration popup error: " + msg);
  throw e;
}
"##;

const OVERLAY_DRIVER_SOURCE: &str = r##"
try {
  refreshmint.log("integration overlay start");
  const blockedHtml = encodeURIComponent(`
    <style>
      #target {
        position: fixed;
        top: 40px;
        left: 40px;
        z-index: 1;
      }
      #overlay {
        position: fixed;
        inset: 0;
        z-index: 2;
        background: rgba(0, 0, 0, 0.1);
      }
    </style>
    <button id="target">Target</button>
    <div id="overlay"></div>
  `);
  await page.goto(`data:text/html,${blockedHtml}`);
  let sawInterceptError = false;
  try {
    await page.click("#target");
  } catch (e) {
    const msg = String(e && e.message ? e.message : e);
    if (msg.includes("intercepts pointer events")) {
      sawInterceptError = true;
    }
  }
  if (!sawInterceptError) {
    throw new Error("expected click to fail with overlay interception error");
  }
  await refreshmint.saveResource("overlay.bin", [111, 107]);
  refreshmint.log("integration overlay done");
} catch (e) {
  const msg = (e && (e.stack || e.message)) ? (e.stack || e.message) : String(e);
  refreshmint.log("integration overlay error: " + msg);
  throw e;
}
"##;

const FRAME_DRIVER_SOURCE: &str = r##"
try {
  refreshmint.log("frame test start");
  await page.goto(__FRAME_URL__);
  await page.waitForLoadState("domcontentloaded", undefined);

  // 1. frames() should return the main frame and the iframe.
  const framesJson = await page.frames();
  const frames = JSON.parse(framesJson);
  refreshmint.reportValue("frame_count", String(frames.length));
  if (frames.length < 2) {
    throw new Error("Expected at least 2 frames, got " + frames.length + ": " + framesJson);
  }
  const iframeFrame = frames.find(f => f.name === "logonbox");
  if (!iframeFrame) {
    throw new Error("Could not find logonbox frame. Frames: " + framesJson);
  }

  // 2. isVisible in main frame should NOT see #user (it lives in the iframe).
  const visibleInMain = await page.isVisible("#user");
  refreshmint.reportValue("visible_in_main", String(visibleInMain));
  if (visibleInMain) {
    throw new Error("isVisible('#user') should be false in main frame");
  }

  // 3. Switch to frame by name and verify element methods see iframe content.
  await page.switchToFrame("logonbox");

  const visibleInFrame = await page.isVisible("#user");
  refreshmint.reportValue("visible_in_frame", String(visibleInFrame));
  if (!visibleInFrame) {
    throw new Error("isVisible('#user') should be true in logonbox frame");
  }

  const evalInFrame = await page.evaluate("document.getElementById('user') ? 'found' : 'missing'");
  refreshmint.reportValue("eval_in_frame", evalInFrame);
  if (evalInFrame !== "found") {
    throw new Error("evaluate in frame returned: " + evalInFrame);
  }

  await page.fill("#user", "testuser");
  const filledValue = await page.evaluate("document.getElementById('user').value");
  refreshmint.reportValue("filled_value", filledValue);
  if (filledValue !== "testuser") {
    throw new Error("fill in frame failed: value is " + filledValue);
  }

  // 4. switchToMainFrame restores the main-frame context.
  await page.switchToMainFrame();

  const visibleAfter = await page.isVisible("#user");
  refreshmint.reportValue("visible_after_switch", String(visibleAfter));
  if (visibleAfter) {
    throw new Error("isVisible('#user') should be false after switchToMainFrame");
  }

  const mainVisible = await page.isVisible("#main");
  refreshmint.reportValue("main_visible", String(mainVisible));
  if (!mainVisible) {
    throw new Error("isVisible('#main') should be true in main frame");
  }

  await refreshmint.saveResource("frame_test.bin", [111, 107]);
  refreshmint.log("frame test done");
} catch (e) {
  const msg = (e && (e.stack || e.message)) ? (e.stack || e.message) : String(e);
  refreshmint.log("frame test error: " + msg);
  throw e;
}
"##;

const GOTO_DRIVER_SOURCE: &str = r##"
try {
  refreshmint.log("integration goto start");
  const baseUrl = __GOTO_URL__;

  await page.goto(baseUrl);
  await page.goto(baseUrl);
  const afterSame = await page.url();
  if (afterSame !== baseUrl) {
    throw new Error(`same-url goto landed at unexpected URL: ${afterSame}`);
  }

  const hashFoo = `${baseUrl}#foo`;
  const hashBar = `${baseUrl}#bar`;
  await page.goto(hashFoo);
  const afterHashFoo = await page.url();
  if (afterHashFoo !== hashFoo) {
    throw new Error(`hash goto to #foo landed at unexpected URL: ${afterHashFoo}`);
  }

  await page.goto(hashBar);
  const afterHashBar = await page.url();
  if (afterHashBar !== hashBar) {
    throw new Error(`hash goto to #bar landed at unexpected URL: ${afterHashBar}`);
  }

  await refreshmint.saveResource("goto.bin", [111, 107]);
  refreshmint.log("integration goto done");
} catch (e) {
  const msg = (e && (e.stack || e.message)) ? (e.stack || e.message) : String(e);
  refreshmint.log("integration goto error: " + msg);
  throw e;
}
"##;

struct TestSandbox {
    root: PathBuf,
}

impl TestSandbox {
    fn new(prefix: &str) -> Result<Self, Box<dyn Error>> {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root = std::env::temp_dir().join(format!(
            "refreshmint-{prefix}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

fn write_fixture_file(
    sandbox: &TestSandbox,
    name: &str,
    contents: &str,
) -> Result<String, Box<dyn Error>> {
    let path = sandbox.path().join(name);
    fs::write(&path, contents)?;
    file_url(&path)
}

fn file_url(path: &Path) -> Result<String, Box<dyn Error>> {
    let absolute = path.canonicalize()?;
    #[cfg(windows)]
    {
        let normalized = absolute.to_string_lossy().replace('\\', "/");
        Ok(format!("file:///{normalized}"))
    }
    #[cfg(not(windows))]
    {
        Ok(format!("file://{}", absolute.to_string_lossy()))
    }
}

impl Drop for TestSandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
#[ignore = "requires a local Chrome/Edge install; run periodically with --ignored"]
fn scrape_smoke_driver_writes_output() -> Result<(), Box<dyn Error>> {
    if scrape::browser::find_chrome_binary().is_err() {
        eprintln!("skipping scrape smoke test: Chrome/Edge binary not found");
        return Ok(());
    }

    let sandbox = TestSandbox::new("scrape")?;
    let ledger_dir = sandbox.path().join("ledger.refreshmint");
    let driver_path = ledger_dir
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("driver.mjs");
    let driver_parent = match driver_path.parent() {
        Some(parent) => parent,
        None => return Err("driver path has no parent".into()),
    };
    fs::create_dir_all(driver_parent)?;
    fs::write(
        driver_parent.join("manifest.json"),
        format!("{{\"name\":\"{EXTENSION_NAME}\"}}"),
    )?;
    fs::write(&driver_path, DRIVER_SOURCE)?;

    let profile_dir = sandbox.path().join("profile");
    let config = ScrapeConfig {
        login_name: LOGIN_NAME.to_string(),
        extension_name: EXTENSION_NAME.to_string(),
        ledger_dir: ledger_dir.clone(),
        profile_override: Some(profile_dir),
        prompt_overrides: app_lib::scrape::js_api::PromptOverrides::new(),
        prompt_requires_override: false,
    };

    scrape::run_scrape(config)?;

    let output_file = ledger_dir
        .join("cache")
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("output")
        .join("smoke.bin");
    let bytes = fs::read(&output_file)?;
    assert_eq!(bytes, b"ok");

    Ok(())
}

#[test]
#[ignore = "requires a local Chrome/Edge install; run periodically with --ignored"]
fn scrape_popup_wait_for_event_switches_tab() -> Result<(), Box<dyn Error>> {
    if scrape::browser::find_chrome_binary().is_err() {
        eprintln!("skipping popup scrape test: Chrome/Edge binary not found");
        return Ok(());
    }

    let sandbox = TestSandbox::new("scrape-popup")?;
    let ledger_dir = sandbox.path().join("ledger.refreshmint");
    let driver_path = ledger_dir
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("driver.mjs");
    let driver_parent = match driver_path.parent() {
        Some(parent) => parent,
        None => return Err("driver path has no parent".into()),
    };
    fs::create_dir_all(driver_parent)?;
    let popup_url = write_fixture_file(
        &sandbox,
        "popup.html",
        "<!doctype html><html><body><div id=\"popup-marker\">popup</div></body></html>",
    )?;
    let opener_html = format!(
        "<!doctype html><html><body><button id=\"open\" type=\"button\">Open Popup</button><script>document.getElementById('open').addEventListener('click', () => window.open({}, '_blank'));</script></body></html>",
        serde_json::to_string(&popup_url)?,
    );
    let opener_url = write_fixture_file(&sandbox, "popup-opener.html", &opener_html)?;
    fs::write(
        driver_parent.join("manifest.json"),
        format!("{{\"name\":\"{EXTENSION_NAME}\"}}"),
    )?;
    let popup_driver =
        POPUP_DRIVER_SOURCE.replace("__OPENER_URL__", &serde_json::to_string(&opener_url)?);
    fs::write(&driver_path, popup_driver)?;

    let profile_dir = sandbox.path().join("profile");
    let config = ScrapeConfig {
        login_name: LOGIN_NAME.to_string(),
        extension_name: EXTENSION_NAME.to_string(),
        ledger_dir: ledger_dir.clone(),
        profile_override: Some(profile_dir),
        prompt_overrides: app_lib::scrape::js_api::PromptOverrides::new(),
        prompt_requires_override: false,
    };

    scrape::run_scrape(config)?;

    let output_file = ledger_dir
        .join("cache")
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("output")
        .join("popup.bin");
    let bytes = fs::read(&output_file)?;
    assert_eq!(bytes, b"ok");

    Ok(())
}

#[test]
#[ignore = "requires a local Chrome/Edge install; run periodically with --ignored"]
fn scrape_click_reports_overlay_interception() -> Result<(), Box<dyn Error>> {
    if scrape::browser::find_chrome_binary().is_err() {
        eprintln!("skipping overlay scrape test: Chrome/Edge binary not found");
        return Ok(());
    }

    let sandbox = TestSandbox::new("scrape-overlay")?;
    let ledger_dir = sandbox.path().join("ledger.refreshmint");
    let driver_path = ledger_dir
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("driver.mjs");
    let driver_parent = match driver_path.parent() {
        Some(parent) => parent,
        None => return Err("driver path has no parent".into()),
    };
    fs::create_dir_all(driver_parent)?;
    fs::write(
        driver_parent.join("manifest.json"),
        format!("{{\"name\":\"{EXTENSION_NAME}\"}}"),
    )?;
    fs::write(&driver_path, OVERLAY_DRIVER_SOURCE)?;

    let profile_dir = sandbox.path().join("profile");
    let config = ScrapeConfig {
        login_name: LOGIN_NAME.to_string(),
        extension_name: EXTENSION_NAME.to_string(),
        ledger_dir: ledger_dir.clone(),
        profile_override: Some(profile_dir),
        prompt_overrides: app_lib::scrape::js_api::PromptOverrides::new(),
        prompt_requires_override: false,
    };

    scrape::run_scrape(config)?;

    let output_file = ledger_dir
        .join("cache")
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("output")
        .join("overlay.bin");
    let bytes = fs::read(&output_file)?;
    assert_eq!(bytes, b"ok");

    Ok(())
}

#[test]
#[ignore = "requires a local Chrome/Edge install; run periodically with --ignored"]
fn scrape_goto_handles_same_url_and_hash_navigation() -> Result<(), Box<dyn Error>> {
    if scrape::browser::find_chrome_binary().is_err() {
        eprintln!("skipping goto scrape test: Chrome/Edge binary not found");
        return Ok(());
    }

    let sandbox = TestSandbox::new("scrape-goto")?;
    let ledger_dir = sandbox.path().join("ledger.refreshmint");
    let driver_path = ledger_dir
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("driver.mjs");
    let driver_parent = match driver_path.parent() {
        Some(parent) => parent,
        None => return Err("driver path has no parent".into()),
    };
    fs::create_dir_all(driver_parent)?;
    let goto_url = write_fixture_file(
        &sandbox,
        "goto.html",
        "<!doctype html><title>goto</title><h1>ok</h1>",
    )?;
    fs::write(
        driver_parent.join("manifest.json"),
        format!("{{\"name\":\"{EXTENSION_NAME}\"}}"),
    )?;
    fs::write(
        &driver_path,
        GOTO_DRIVER_SOURCE.replace("__GOTO_URL__", &serde_json::to_string(&goto_url)?),
    )?;

    let profile_dir = sandbox.path().join("profile");
    let config = ScrapeConfig {
        login_name: LOGIN_NAME.to_string(),
        extension_name: EXTENSION_NAME.to_string(),
        ledger_dir: ledger_dir.clone(),
        profile_override: Some(profile_dir),
        prompt_overrides: app_lib::scrape::js_api::PromptOverrides::new(),
        prompt_requires_override: false,
    };

    scrape::run_scrape(config)?;

    let output_file = ledger_dir
        .join("cache")
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("output")
        .join("goto.bin");
    let bytes = fs::read(&output_file)?;
    assert_eq!(bytes, b"ok");

    Ok(())
}

#[test]
#[ignore = "requires a local Chrome/Edge install; run periodically with --ignored"]
fn scrape_frame_methods_switch_context() -> Result<(), Box<dyn Error>> {
    if scrape::browser::find_chrome_binary().is_err() {
        eprintln!("skipping frame scrape test: Chrome/Edge binary not found");
        return Ok(());
    }

    let sandbox = TestSandbox::new("scrape-frame")?;
    let ledger_dir = sandbox.path().join("ledger.refreshmint");
    let driver_path = ledger_dir
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("driver.mjs");
    let driver_parent = match driver_path.parent() {
        Some(parent) => parent,
        None => return Err("driver path has no parent".into()),
    };
    fs::create_dir_all(driver_parent)?;
    let frame_child_url = write_fixture_file(
        &sandbox,
        "frame-child.html",
        "<!doctype html><html><body><input id=\"user\"><input id=\"pass\"><button id=\"submit\">OK</button></body></html>",
    )?;
    let frame_html = format!(
        "<!doctype html><html><body><div id=\"main\">Main</div><iframe name=\"logonbox\" src={}></iframe></body></html>",
        serde_json::to_string(&frame_child_url)?,
    );
    let frame_url = write_fixture_file(&sandbox, "frame.html", &frame_html)?;
    fs::write(
        driver_parent.join("manifest.json"),
        format!("{{\"name\":\"{EXTENSION_NAME}\"}}"),
    )?;
    fs::write(
        &driver_path,
        FRAME_DRIVER_SOURCE.replace("__FRAME_URL__", &serde_json::to_string(&frame_url)?),
    )?;

    let profile_dir = sandbox.path().join("profile");
    let config = ScrapeConfig {
        login_name: LOGIN_NAME.to_string(),
        extension_name: EXTENSION_NAME.to_string(),
        ledger_dir: ledger_dir.clone(),
        profile_override: Some(profile_dir),
        prompt_overrides: app_lib::scrape::js_api::PromptOverrides::new(),
        prompt_requires_override: false,
    };

    scrape::run_scrape(config)?;

    let output_file = ledger_dir
        .join("cache")
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("output")
        .join("frame_test.bin");
    let bytes = fs::read(&output_file)?;
    assert_eq!(bytes, b"ok");

    Ok(())
}
