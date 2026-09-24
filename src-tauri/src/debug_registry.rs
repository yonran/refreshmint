//! Discovery records for independently owned debug browser workers.
//!
//! The registry contains routing metadata, never credentials. MCP clients use
//! it to find app-retained and MCP-launched workers without knowing individual
//! Unix socket paths up front.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugSessionDescriptor {
    pub session_id: String,
    pub login_name: String,
    pub kind: String,
    pub pid: u32,
    pub socket_path: PathBuf,
    pub ledger_dir: PathBuf,
    pub started_at: String,
}

pub fn registry_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("refreshmint")
        .join("debug-sessions")
}

pub struct DebugSessionRegistration {
    path: PathBuf,
}

impl DebugSessionRegistration {
    pub fn create(
        login_name: &str,
        kind: &str,
        socket_path: &Path,
        ledger_dir: &Path,
    ) -> std::io::Result<(Self, DebugSessionDescriptor)> {
        use std::io::Write;

        let dir = registry_dir();
        std::fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        }

        let session_id = format!("debug-{}", uuid::Uuid::new_v4().simple());
        let descriptor = DebugSessionDescriptor {
            session_id: session_id.clone(),
            login_name: login_name.to_string(),
            kind: kind.to_string(),
            pid: std::process::id(),
            socket_path: socket_path.to_path_buf(),
            ledger_dir: ledger_dir.to_path_buf(),
            started_at: chrono::Utc::now().to_rfc3339(),
        };
        let path = dir.join(format!("{session_id}.json"));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path)?;
        serde_json::to_writer(&mut file, &descriptor)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        Ok((Self { path }, descriptor))
    }
}

impl Drop for DebugSessionRegistration {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn list_sessions() -> Vec<DebugSessionDescriptor> {
    let Ok(entries) = std::fs::read_dir(registry_dir()) else {
        return Vec::new();
    };
    let mut sessions = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let json = std::fs::read_to_string(&path).ok()?;
            let session = serde_json::from_str::<DebugSessionDescriptor>(&json).ok()?;
            if is_reachable(&session) {
                Some(session)
            } else {
                // A worker killed without unwinding cannot drop its registry
                // guard. Remove the record once its socket has disappeared so
                // clients do not keep offering a known-dead session.
                let _ = std::fs::remove_file(path);
                None
            }
        })
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| left.started_at.cmp(&right.started_at));
    sessions
}

/// A worker that dies without unwinding (killed, crashed, machine slept
/// through a force-quit) leaves its socket special-file behind without
/// unlinking it, so merely `stat`-ing the path -- this function's old
/// behavior -- reported long-dead sessions as reachable indefinitely.
/// Connecting is the only way to tell: a live listener accepts immediately,
/// while an orphaned socket file refuses with `ECONNREFUSED`. Unix-domain
/// connects to a local path never block waiting on the network, so this
/// resolves instantly either way and needs no timeout.
fn is_reachable(session: &DebugSessionDescriptor) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::net::UnixStream::connect(&session.socket_path).is_ok()
    }
    #[cfg(not(unix))]
    {
        std::fs::metadata(&session.socket_path).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::{is_reachable, list_sessions, DebugSessionDescriptor, DebugSessionRegistration};

    #[test]
    fn descriptor_uses_camel_case_wire_names() {
        let value = serde_json::to_value(DebugSessionDescriptor {
            session_id: "debug-1".to_string(),
            login_name: "bank".to_string(),
            kind: "manual-debug".to_string(),
            pid: 1,
            socket_path: "/tmp/debug.sock".into(),
            ledger_dir: "/tmp/ledger.refreshmint".into(),
            started_at: "now".to_string(),
        })
        .unwrap_or_else(|error| panic!("failed to serialize descriptor: {error}"));
        assert_eq!(value["sessionId"], "debug-1");
        assert_eq!(value["loginName"], "bank");
    }

    fn temp_socket_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "refreshmint-debug-registry-test-{name}-{}.sock",
            std::process::id()
        ))
    }

    #[test]
    fn is_reachable_rejects_an_orphaned_socket_file_with_no_listener() {
        // A worker killed without unwinding (crash, force-quit, SIGKILL)
        // leaves its socket special-file behind without unlinking it. Bind
        // one and then drop the listener without cleaning up the path, to
        // simulate exactly that: the file still exists and is still a
        // socket, but nothing is listening on it any more.
        let socket_path = temp_socket_path("orphaned");
        let _ = std::fs::remove_file(&socket_path);
        {
            let listener = std::os::unix::net::UnixListener::bind(&socket_path)
                .unwrap_or_else(|error| panic!("failed to bind test socket: {error}"));
            drop(listener);
        }
        assert!(
            socket_path.exists(),
            "test setup: socket file should remain on disk after the listener is dropped"
        );

        let descriptor = DebugSessionDescriptor {
            session_id: "debug-orphaned".to_string(),
            login_name: "chase".to_string(),
            kind: "failed-scrape".to_string(),
            pid: 1,
            socket_path: socket_path.clone(),
            ledger_dir: "/tmp/ledger.refreshmint".into(),
            started_at: "now".to_string(),
        };

        assert!(
            !is_reachable(&descriptor),
            "an orphaned socket file with no listener must not be reported reachable"
        );

        let _ = std::fs::remove_file(&socket_path);
    }

    #[test]
    fn is_reachable_accepts_a_socket_with_a_live_listener() {
        let socket_path = temp_socket_path("live");
        let _ = std::fs::remove_file(&socket_path);
        let listener = std::os::unix::net::UnixListener::bind(&socket_path)
            .unwrap_or_else(|error| panic!("failed to bind test socket: {error}"));

        let descriptor = DebugSessionDescriptor {
            session_id: "debug-live".to_string(),
            login_name: "chase".to_string(),
            kind: "failed-scrape".to_string(),
            pid: 1,
            socket_path: socket_path.clone(),
            ledger_dir: "/tmp/ledger.refreshmint".into(),
            started_at: "now".to_string(),
        };

        assert!(is_reachable(&descriptor));

        drop(listener);
        let _ = std::fs::remove_file(&socket_path);
    }

    #[test]
    fn list_sessions_prunes_an_orphaned_session_and_its_registry_file() {
        let socket_path = temp_socket_path("pruned");
        let _ = std::fs::remove_file(&socket_path);
        {
            let listener = std::os::unix::net::UnixListener::bind(&socket_path)
                .unwrap_or_else(|error| panic!("failed to bind test socket: {error}"));
            drop(listener);
        }

        let ledger_dir = std::env::temp_dir().join(format!(
            "refreshmint-debug-registry-test-ledger-{}",
            std::process::id()
        ));
        let (_registration, descriptor) =
            DebugSessionRegistration::create("chase", "failed-scrape", &socket_path, &ledger_dir)
                .unwrap_or_else(|error| panic!("failed to register test session: {error}"));
        let registry_path = super::registry_dir().join(format!("{}.json", descriptor.session_id));
        assert!(registry_path.exists());

        let sessions = list_sessions();

        assert!(
            !sessions
                .iter()
                .any(|s| s.session_id == descriptor.session_id),
            "an orphaned session must be pruned from list_sessions()"
        );
        assert!(
            !registry_path.exists(),
            "list_sessions() must remove the stale registry file for an orphaned session"
        );

        let _ = std::fs::remove_file(&socket_path);
    }
}
