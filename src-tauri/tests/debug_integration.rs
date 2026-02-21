#[cfg(unix)]
mod unix_only {
    use app_lib::scrape::{
        browser,
        debug::{self, DebugStartConfig},
    };
    use std::error::Error;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
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

    type DebugSessionJoin = std::thread::JoinHandle<Result<(), String>>;
    type DebugFixture = (PathBuf, PathBuf, DebugSessionJoin);

    fn create_debug_fixture(sandbox: &TestSandbox) -> Result<DebugFixture, Box<dyn Error>> {
        let ledger_dir = sandbox.path().join("ledger.refreshmint");
        fs::create_dir_all(&ledger_dir)?;
        let extension_dir = ledger_dir.join("extensions").join("smoke-ext");
        fs::create_dir_all(&extension_dir)?;
        fs::write(
            extension_dir.join("manifest.json"),
            r#"{"name":"smoke-ext","secrets":{"example.com":["bank_password"]}}"#,
        )?;
        fs::write(extension_dir.join("driver.mjs"), "// smoke\n")?;

        let socket_path = sandbox.path().join("debug.sock");
        let profile_dir = sandbox.path().join("profile");
        let config = DebugStartConfig {
            login_name: "smoke-account".to_string(),
            extension_name: "smoke-ext".to_string(),
            ledger_dir: ledger_dir.clone(),
            profile_override: Some(profile_dir),
            socket_path: Some(socket_path.clone()),
            prompt_requires_override: false,
        };
        let session_thread =
            thread::spawn(move || debug::run_debug_session(config).map_err(|err| err.to_string()));
        Ok((ledger_dir, socket_path, session_thread))
    }

    fn wait_for_socket_or_fail(
        socket_path: &Path,
        session_thread: DebugSessionJoin,
    ) -> Result<DebugSessionJoin, Box<dyn Error>> {
        let mut socket_ready = false;
        for _ in 0..120 {
            if socket_path.exists() {
                socket_ready = true;
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        if socket_ready {
            return Ok(session_thread);
        }

        let _ = debug::stop_debug_session(socket_path);
        let result = session_thread.join().map_err(|_| "debug thread panicked")?;
        if let Err(err) = result {
            return Err(format!("debug session failed before socket was ready: {err}").into());
        }
        Err("debug socket did not become ready".into())
    }

    #[test]
    #[ignore = "requires a local Chrome/Edge install and browser permissions; run periodically with --ignored"]
    fn debug_session_exec_and_stop_smoke() -> Result<(), Box<dyn Error>> {
        if browser::find_chrome_binary().is_err() {
            eprintln!("skipping debug smoke test: Chrome/Edge binary not found");
            return Ok(());
        }

        let sandbox = TestSandbox::new("debug")?;
        let (ledger_dir, socket_path, session_thread) = create_debug_fixture(&sandbox)?;
        let session_thread = wait_for_socket_or_fail(&socket_path, session_thread)?;

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

    #[test]
    #[ignore = "requires a local Chrome/Edge install and browser permissions; run periodically with --ignored"]
    fn debug_exec_cli_streams_script_output_to_exec_stdio() -> Result<(), Box<dyn Error>> {
        if browser::find_chrome_binary().is_err() {
            eprintln!("skipping debug stream test: Chrome/Edge binary not found");
            return Ok(());
        }

        let sandbox = TestSandbox::new("debug-exec-stream")?;
        let (_ledger_dir, socket_path, session_thread) = create_debug_fixture(&sandbox)?;
        let session_thread = wait_for_socket_or_fail(&socket_path, session_thread)?;

        let script_path = sandbox.path().join("stream-smoke.mjs");
        fs::write(
            &script_path,
            r#"
refreshmint.log("stream stderr line");
refreshmint.reportValue("stream_key", "stream_value");
"#,
        )?;

        let app_binary = std::env::var("CARGO_BIN_EXE_app")
            .map_err(|_| "missing CARGO_BIN_EXE_app for debug exec stream test")?;
        let exec_output = Command::new(app_binary)
            .arg("debug")
            .arg("exec")
            .arg("--socket")
            .arg(&socket_path)
            .arg("--script")
            .arg(&script_path)
            .output()?;

        let _ = debug::stop_debug_session(&socket_path);
        let session_result = session_thread.join().map_err(|_| "debug thread panicked")?;
        session_result?;

        assert!(
            exec_output.status.success(),
            "debug exec failed: {}",
            String::from_utf8_lossy(&exec_output.stderr)
        );
        let stdout = String::from_utf8_lossy(&exec_output.stdout);
        let stderr = String::from_utf8_lossy(&exec_output.stderr);
        assert!(
            stdout.contains("stream_key: stream_value"),
            "expected reportValue output in stdout, got: {stdout}"
        );
        assert!(
            stderr.contains("stream stderr line"),
            "expected log output in stderr, got: {stderr}"
        );

        Ok(())
    }
}
