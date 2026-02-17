use app_lib::scrape::{self, ScrapeConfig};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const EXTENSION_NAME: &str = "smoke";
const ACCOUNT_NAME: &str = "smoke-account";
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
        account: ACCOUNT_NAME.to_string(),
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
