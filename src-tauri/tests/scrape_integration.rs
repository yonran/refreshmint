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
  const popupHtml = encodeURIComponent("<div id='popup-marker'>popup</div>");
  const openerHtml = encodeURIComponent(`
    <button id="open">Open Popup</button>
    <script>
      document.getElementById("open").addEventListener("click", () => {
        window.open("data:text/html,${popupHtml}", "_blank");
      });
    </script>
  `);
  await page.goto(`data:text/html,${openerHtml}`);
  const popupPromise = page.waitForEvent("popup", 10000);
  await page.click("#open");
  const popup = JSON.parse(await popupPromise);
  if (!popup || popup.current !== true || typeof popup.targetId !== "string" || popup.targetId.length === 0) {
    throw new Error("invalid popup summary");
  }
  await page.waitForLoadState("domcontentloaded", 10000);
  const marker = await page.evaluate("document.getElementById('popup-marker') ? 'yes' : 'no'");
  if (marker !== "yes") {
    throw new Error("did not switch to popup tab");
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

const GOTO_DRIVER_SOURCE: &str = r##"
try {
  refreshmint.log("integration goto start");
  const html = encodeURIComponent("<!doctype html><title>goto</title><h1>ok</h1>");
  const baseUrl = `data:text/html,${html}`;

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
    fs::write(
        driver_parent.join("manifest.json"),
        format!("{{\"name\":\"{EXTENSION_NAME}\"}}"),
    )?;
    fs::write(&driver_path, POPUP_DRIVER_SOURCE)?;

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
    fs::write(
        driver_parent.join("manifest.json"),
        format!("{{\"name\":\"{EXTENSION_NAME}\"}}"),
    )?;
    fs::write(&driver_path, GOTO_DRIVER_SOURCE)?;

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
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("output")
        .join("goto.bin");
    let bytes = fs::read(&output_file)?;
    assert_eq!(bytes, b"ok");

    Ok(())
}
