use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

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
fn extension_load_smoke_from_directory() -> Result<(), Box<dyn Error>> {
    let sandbox = TestSandbox::new("extension-load")?;
    let ledger_dir = sandbox.path().join("ledger.refreshmint");
    let extension_source = sandbox.path().join("extension-src");
    fs::create_dir_all(&ledger_dir)?;
    fs::create_dir_all(&extension_source)?;
    fs::write(
        extension_source.join("manifest.json"),
        r#"{"name":"smoke-ext"}"#,
    )?;
    fs::write(extension_source.join("driver.mjs"), "// smoke-v1\n")?;

    let binary = env!("CARGO_BIN_EXE_app");
    let first_load = Command::new(binary)
        .args(["extension", "load"])
        .arg(&extension_source)
        .arg("--ledger")
        .arg(&ledger_dir)
        .output()?;
    assert!(
        first_load.status.success(),
        "first load failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&first_load.stdout),
        String::from_utf8_lossy(&first_load.stderr)
    );
    let loaded_driver = ledger_dir
        .join("extensions")
        .join("smoke-ext")
        .join("driver.mjs");
    assert!(loaded_driver.is_file());

    let second_load = Command::new(binary)
        .args(["extension", "load"])
        .arg(&extension_source)
        .arg("--ledger")
        .arg(&ledger_dir)
        .output()?;
    assert!(!second_load.status.success());
    assert!(
        String::from_utf8_lossy(&second_load.stderr).contains("already exists"),
        "unexpected error message:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&second_load.stdout),
        String::from_utf8_lossy(&second_load.stderr)
    );

    fs::write(extension_source.join("driver.mjs"), "// smoke-v2\n")?;
    let replace_load = Command::new(binary)
        .args(["extension", "load"])
        .arg(&extension_source)
        .arg("--ledger")
        .arg(&ledger_dir)
        .arg("--replace")
        .output()?;
    assert!(
        replace_load.status.success(),
        "replace load failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&replace_load.stdout),
        String::from_utf8_lossy(&replace_load.stderr)
    );
    let contents = fs::read_to_string(&loaded_driver)?;
    assert_eq!(contents, "// smoke-v2\n");

    Ok(())
}
