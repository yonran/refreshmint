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
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .filter_map(|json| serde_json::from_str::<DebugSessionDescriptor>(&json).ok())
        .filter(is_reachable)
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| left.started_at.cmp(&right.started_at));
    sessions
}

fn is_reachable(session: &DebugSessionDescriptor) -> bool {
    session.socket_path.exists()
}

#[cfg(test)]
mod tests {
    use super::DebugSessionDescriptor;

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
}
