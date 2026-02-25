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

const GET_BY_ROLE_DRIVER_SOURCE: &str = r##"
try {
  refreshmint.log("getByRole test start");
  const html = encodeURIComponent(`
    <h1>Page Title</h1>
    <h2 id="s1">Section One</h2>
    <h2 id="s2">Section Two</h2>
    <button id="submit">Submit</button>
    <button aria-label="Close dialog">X</button>
    <a href="#">Home link</a>
    <input type="text" placeholder="Email address" />
    <input type="text" aria-label="Username" />
    <input type="checkbox" id="chk" />
    <label for="chk">Accept terms</label>
    <div id="form-area">
      <button id="inner-btn">Log In</button>
    </div>
  `);
  await page.goto(`data:text/html,${html}`);

  // 1. Count all buttons (implicit role from <button> + aria-label button)
  const allButtons = page.getByRole('button');
  const btnCount = await allButtons.count();
  if (btnCount !== 3) throw new Error(`Expected 3 buttons, got ${btnCount}`);

  // 2. getByRole with exact name (case-insensitive substring, default)
  const submitBtn = page.getByRole('button', { name: 'Submit' });
  const submitText = await submitBtn.innerText();
  if (submitText !== 'Submit') throw new Error(`Expected Submit, got ${submitText}`);

  // 3. getByRole with exact:true (full match, case-insensitive)
  const exactBtn = page.getByRole('button', { name: 'submit', exact: true });
  const exactCount = await exactBtn.count();
  if (exactCount !== 1) throw new Error(`Expected 1 exact button match, got ${exactCount}`);

  // 4. Name matched via aria-label
  const closeBtn = page.getByRole('button', { name: 'Close dialog' });
  const closeCount = await closeBtn.count();
  if (closeCount !== 1) throw new Error(`Expected 1 close button, got ${closeCount}`);

  // 5. Link via implicit role
  const links = page.getByRole('link');
  const linkCount = await links.count();
  if (linkCount !== 1) throw new Error(`Expected 1 link, got ${linkCount}`);

  // 6. Headings - count
  const headings = page.getByRole('heading');
  const headingCount = await headings.count();
  if (headingCount !== 3) throw new Error(`Expected 3 headings (h1+h2+h2), got ${headingCount}`);

  // 7. Headings - filter by level
  const h2s = page.getByRole('heading', { level: 2 });
  const h2Count = await h2s.count();
  if (h2Count !== 2) throw new Error(`Expected 2 h2 headings, got ${h2Count}`);

  // 8. Textboxes
  const textboxes = page.getByRole('textbox');
  const tbCount = await textboxes.count();
  if (tbCount !== 2) throw new Error(`Expected 2 textboxes, got ${tbCount}`);

  // 9. Textbox by accessible name via placeholder
  const emailInput = page.getByRole('textbox', { name: 'Email' });
  const emailCount = await emailInput.count();
  if (emailCount !== 1) throw new Error(`Expected 1 email input, got ${emailCount}`);

  // 10. Textbox by aria-label
  const usernameInput = page.getByRole('textbox', { name: 'Username' });
  const usernameCount = await usernameInput.count();
  if (usernameCount !== 1) throw new Error(`Expected 1 username input, got ${usernameCount}`);

  // 11. Checkbox via implicit role
  const checkboxes = page.getByRole('checkbox');
  const cbCount = await checkboxes.count();
  if (cbCount !== 1) throw new Error(`Expected 1 checkbox, got ${cbCount}`);

  // 12. Chaining: form area -> button
  const innerBtn = page.locator('#form-area').getByRole('button', { name: 'Log In' });
  const innerBtnText = await innerBtn.innerText();
  if (innerBtnText !== 'Log In') throw new Error(`Expected Log In, got ${innerBtnText}`);

  // 13. locator('role=...') string syntax also works
  const roleStrBtn = page.locator('role=button[name="Submit"i]');
  const roleStrText = await roleStrBtn.innerText();
  if (roleStrText !== 'Submit') throw new Error(`Expected Submit via role= string, got ${roleStrText}`);

  // 14. nth on getByRole
  const firstH2 = page.getByRole('heading', { level: 2 }).first();
  const firstH2Text = await firstH2.innerText();
  if (firstH2Text !== 'Section One') throw new Error(`Expected Section One, got ${firstH2Text}`);

  await refreshmint.saveResource("get_by_role.bin", [111, 107]);
  refreshmint.log("getByRole test done");
} catch (e) {
  const msg = (e && (e.stack || e.message)) ? (e.stack || e.message) : String(e);
  refreshmint.log("getByRole test error: " + msg);
  try {
     const body = await page.evaluate("document.body.innerHTML");
     refreshmint.log("Body: " + body);
  } catch (_) {}
  throw e;
}
"##;

const SHADOW_DOM_DRIVER_SOURCE: &str = r##"
try {
  refreshmint.log("shadow DOM test start");
  const html = encodeURIComponent(`
    <div id="host"></div>
    <script>
      const shadow = document.getElementById('host').attachShadow({ mode: 'open' });
      shadow.innerHTML = '<button id="shadow-btn">Shadow Button</button>';
    </script>
  `);
  await page.goto(`data:text/html,${html}`);

  // 1. locator('button').count() should find the button inside the shadow root
  const count = await page.locator('button').count();
  if (count !== 1) throw new Error(`Expected 1 shadow button, got ${count}`);

  // 2. getByRole finds the shadow button by name
  const shadowBtn = page.getByRole('button', { name: 'Shadow Button' });
  const text = await shadowBtn.innerText();
  if (text !== 'Shadow Button') throw new Error(`Expected Shadow Button, got ${text}`);

  // 3. Click the shadow button via CSS locator
  await page.locator('button').click();

  await refreshmint.saveResource("shadow_dom.bin", [111, 107]);
  refreshmint.log("shadow DOM test done");
} catch (e) {
  const msg = (e && (e.stack || e.message)) ? (e.stack || e.message) : String(e);
  refreshmint.log("shadow DOM test error: " + msg);
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

#[test]
#[ignore = "requires a local Chrome/Edge install; run periodically with --ignored"]
fn get_by_role_api_works() -> Result<(), Box<dyn Error>> {
    if scrape::browser::find_chrome_binary().is_err() {
        eprintln!("skipping getByRole test: Chrome/Edge binary not found");
        return Ok(());
    }

    let sandbox = TestSandbox::new("get-by-role")?;
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
    fs::write(&driver_path, GET_BY_ROLE_DRIVER_SOURCE)?;

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
        .join("get_by_role.bin");

    if !output_file.exists() {
        return Err("get_by_role.bin not found, test likely failed".into());
    }
    let bytes = fs::read(&output_file)?;
    assert_eq!(bytes, b"ok");

    Ok(())
}

#[test]
#[ignore = "requires a local Chrome/Edge install; run periodically with --ignored"]
fn shadow_dom_api_works() -> Result<(), Box<dyn Error>> {
    if scrape::browser::find_chrome_binary().is_err() {
        eprintln!("skipping shadow DOM test: Chrome/Edge binary not found");
        return Ok(());
    }

    let sandbox = TestSandbox::new("shadow-dom")?;
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
    fs::write(&driver_path, SHADOW_DOM_DRIVER_SOURCE)?;

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
        .join("shadow_dom.bin");

    if !output_file.exists() {
        return Err("shadow_dom.bin not found, test likely failed".into());
    }
    let bytes = fs::read(&output_file)?;
    assert_eq!(bytes, b"ok");

    Ok(())
}
