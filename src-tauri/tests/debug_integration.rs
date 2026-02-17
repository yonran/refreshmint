#[cfg(unix)]
mod unix_only {
    use app_lib::scrape::{
        browser,
        debug::{self, DebugStartConfig},
    };
    use std::error::Error;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
    #[ignore = "requires a local Chrome/Edge install and browser permissions; run periodically with --ignored"]
    fn debug_session_exec_and_stop_smoke() -> Result<(), Box<dyn Error>> {
        if browser::find_chrome_binary().is_err() {
            eprintln!("skipping debug smoke test: Chrome/Edge binary not found");
            return Ok(());
        }

        let sandbox = TestSandbox::new("debug")?;
        let ledger_dir = sandbox.path().join("ledger.refreshmint");
        fs::create_dir_all(&ledger_dir)?;

        let socket_path = sandbox.path().join("debug.sock");
        let profile_dir = sandbox.path().join("profile");
        let config = DebugStartConfig {
            account: "smoke-account".to_string(),
            extension_name: "smoke-ext".to_string(),
            ledger_dir: ledger_dir.clone(),
            profile_override: Some(profile_dir),
            socket_path: Some(socket_path.clone()),
        };

        let session_thread =
            thread::spawn(move || debug::run_debug_session(config).map_err(|err| err.to_string()));

        let mut socket_ready = false;
        for _ in 0..120 {
            if socket_path.exists() {
                socket_ready = true;
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        if !socket_ready {
            let _ = debug::stop_debug_session(&socket_path);
            let result = session_thread.join().map_err(|_| "debug thread panicked")?;
            if let Err(err) = result {
                return Err(format!("debug session failed before socket was ready: {err}").into());
            }
            return Err("debug socket did not become ready".into());
        }

        let script = r#"
refreshmint.log("debug integration smoke start");
const url = await page.url();
refreshmint.reportValue("debug_url", String(url));
await refreshmint.saveResource("debug-smoke.bin", [111, 107]);
"#;
        debug::exec_debug_script(&socket_path, script)?;
        debug::stop_debug_session(&socket_path)?;

        let session_result = session_thread.join().map_err(|_| "debug thread panicked")?;
        session_result?;

        let output_file = ledger_dir
            .join("extensions")
            .join("smoke-ext")
            .join("output")
            .join("debug-smoke.bin");
        let bytes = fs::read(&output_file)?;
        assert_eq!(bytes, b"ok");

        Ok(())
    }
}
