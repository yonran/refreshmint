//! Line-delimited control protocol between the Tauri host and one scrape worker.
//!
//! Keep this protocol private to local child processes. MCP uses its own
//! JSON-RPC facade and never receives resolved credential values.

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum WorkerCommand {
    Start {
        prompt_overrides: crate::scrape::js_api::PromptOverrides,
        prompt_requires_override: bool,
        retain_failure_debug: bool,
    },
    PromptAnswer {
        request_id: u64,
        answer: Option<String>,
        error: Option<String>,
    },
    Cancel,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum WorkerEvent {
    Log {
        stream: crate::scrape::js_api::DebugOutputStream,
        line: String,
    },
    PromptRequested {
        request_id: u64,
        message: String,
        choices: Option<Vec<String>>,
    },
    FailureRetained {
        error: String,
        artifacts_dir: Option<String>,
        debug_session: crate::debug_registry::DebugSessionDescriptor,
    },
    Result {
        ok: bool,
        error: Option<String>,
        artifacts_dir: Option<String>,
    },
}
