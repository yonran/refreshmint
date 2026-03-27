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

#[test]
fn extension_build_script_produces_runtime_ready_artifact() -> Result<(), Box<dyn Error>> {
    let sandbox = TestSandbox::new("extension-build")?;
    let source_dir = sandbox.path().join("extension-src");
    let built_dir = sandbox.path().join("extension-built");
    fs::create_dir_all(
        source_dir
            .join("node_modules")
            .join("demo-pkg")
            .join("dist"),
    )?;
    fs::write(
        source_dir.join("manifest.json"),
        r#"{"name":"build-ext","driver":"driver.mts","extract":"extract.mts"}"#,
    )?;
    fs::write(
        source_dir.join("package.json"),
        r#"{"name":"build-ext-src","private":true}"#,
    )?;
    fs::write(
        source_dir
            .join("node_modules")
            .join("demo-pkg")
            .join("package.json"),
        r#"{"name":"demo-pkg","module":"./dist/index.js"}"#,
    )?;
    fs::write(
        source_dir
            .join("node_modules")
            .join("demo-pkg")
            .join("dist")
            .join("index.js"),
        "export const value = 'ok';\n",
    )?;
    fs::write(
        source_dir.join("driver.mts"),
        "import { value } from 'demo-pkg';\nif (value !== 'ok') { throw new Error('bad value'); }\n",
    )?;
    fs::write(
        source_dir.join("extract.mts"),
        "import { value } from 'demo-pkg';\nexport async function extract(context) { return [{ tdate: '2024-01-01', tstatus: 'Cleared', tdescription: value, tcomment: '', ttags: [[ 'evidence', `${context.document.name}:1:1` ]] }]; }\n",
    )?;

    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("src-tauri should have repo parent")?
        .to_path_buf();
    let builder = repo_root.join("scripts").join("build-extensions.mjs");
    let output = Command::new("node")
        .arg(&builder)
        .arg("--extension-dir")
        .arg(&source_dir)
        .arg("--out-dir")
        .arg(&built_dir)
        .current_dir(&repo_root)
        .output()?;
    assert!(
        output.status.success(),
        "builder failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let built_manifest = fs::read_to_string(built_dir.join("manifest.json"))?;
    assert!(
        built_manifest.contains(r#""driver": "dist/driver.mjs""#),
        "unexpected built manifest:\n{built_manifest}"
    );
    assert!(
        built_manifest.contains(r#""extract": "dist/extract.mjs""#),
        "unexpected built manifest:\n{built_manifest}"
    );
    assert!(built_dir.join("dist").join("driver.mjs").is_file());
    assert!(built_dir.join("dist").join("extract.mjs").is_file());
    assert!(
        !built_dir.join("package.json").exists(),
        "built artifact should not include package.json"
    );
    let built_extract = fs::read_to_string(built_dir.join("dist").join("extract.mjs"))?;
    assert!(
        !built_extract.contains("from 'demo-pkg'"),
        "built extractor still has bare package import:\n{built_extract}"
    );

    Ok(())
}
