use app_lib::scrape::{self, ScrapeConfig};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const EXTENSION_NAME: &str = "locator-test";
const LOGIN_NAME: &str = "locator-account";

const LOCATOR_DRIVER_SOURCE: &str = r##"
try {
  refreshmint.log("locator test start");
  const html = encodeURIComponent(`
    <div id="container">
      <button id="b1">Button 1</button>
      <button id="b2">Button 2</button>
      <input type="text" value="initial" />
      <span class="label">Label 1</span>
      <span class="label">Label 2</span>
      <div id="nested">
        <span class="child">Child</span>
      </div>
    </div>
  `);
  await page.goto(`data:text/html,${html}`);

  // 1. Basic locator click (strict mode check: should fail if multiple)
  try {
    await page.locator("button").click({ timeout: 500 });
    throw new Error("Expected strict mode violation for button click");
  } catch (e) {
    if (!String(e).includes("Strict mode violation")) {
        throw new Error("Expected strict violation, got: " + e);
    }
  }

  // 2. nth / first / last
  const buttons = page.locator("button");
  
  // click first (b1)
  await buttons.first().click();
  
  // click second (b2) via nth(1)
  const secondButton = buttons.nth(1);
  const text = await secondButton.innerText();
  if (text !== "Button 2") throw new Error(`Expected Button 2, got ${text}`);

  // click last (b2)
  const lastButton = buttons.last();
  const lastText = await lastButton.innerText();
  if (lastText !== "Button 2") throw new Error(`Expected Button 2 (last), got ${lastText}`);

  // 3. Chaining
  const nestedChild = page.locator("#nested").locator(".child");
  const childText = await nestedChild.textContent();
  if (childText !== "Child") throw new Error(`Expected Child, got ${childText}`);

  // 4. Fill
  const input = page.locator("input");
  await input.fill("filled value");
  const val = await input.inputValue();
  if (val !== "filled value") throw new Error(`Expected filled value, got ${val}`);

  // 5. Count
  const count = await page.locator(".label").count();
  if (count !== 2) throw new Error(`Expected 2 labels, got ${count}`);

  // 6. isVisible / isEnabled
  const visible = await page.locator("#container").isVisible();
  if (!visible) throw new Error("Container should be visible");

  const enabled = await page.locator("input").isEnabled();
  if (!enabled) throw new Error("Input should be enabled");

  // 7. Strict mode pass
  await page.locator("#nested").click(); // Single element, should pass

  await refreshmint.saveResource("locator.bin", [111, 107]);
  refreshmint.log("locator test done");
} catch (e) {
  const msg = (e && (e.stack || e.message)) ? (e.stack || e.message) : String(e);
  refreshmint.log("locator test error: " + msg);
  try {
     const body = await page.evaluate("document.body.innerHTML");
     refreshmint.log("Body: " + body);
  } catch (_) {}
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
fn locator_api_works() -> Result<(), Box<dyn Error>> {
    if scrape::browser::find_chrome_binary().is_err() {
        eprintln!("skipping locator test: Chrome/Edge binary not found");
        return Ok(());
    }

    let sandbox = TestSandbox::new("locator-api")?;
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
        format!("{{\"name\":\"{}\"}}", EXTENSION_NAME),
    )?;
    fs::write(&driver_path, LOCATOR_DRIVER_SOURCE)?;

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
        .join("locator.bin");

    // If test failed, this file won't exist or won't have "ok"
    if !output_file.exists() {
        return Err("locator.bin not found, test likely failed".into());
    }
    let bytes = fs::read(&output_file)?;
    assert_eq!(bytes, b"ok");

    Ok(())
}
