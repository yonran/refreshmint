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
        enable_human_challenges: bool,
        human_challenge_timeout_secs: u64,
    },
    PromptAnswer {
        request_id: u64,
        answer: Option<String>,
        error: Option<String>,
    },
    HumanChallengeInput {
        request_id: u64,
        input: HumanChallengeInput,
    },
    Cancel,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HumanChallengeInput {
    PointerMove { x: u32, y: u32, buttons: i64 },
    PointerDown { x: u32, y: u32 },
    PointerUp { x: u32, y: u32 },
    Complete,
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
    HumanChallengeRequested {
        request_id: u64,
        message: String,
    },
    HumanChallengeFrame {
        request_id: u64,
        data: String,
        device_width: u32,
        device_height: u32,
    },
    HumanChallengeClosed {
        request_id: u64,
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

#[cfg(test)]
mod tests {
    use super::{HumanChallengeInput, WorkerCommand, WorkerEvent};

    #[test]
    fn human_challenge_pointer_command_uses_camel_case_wire_format() {
        let command = WorkerCommand::HumanChallengeInput {
            request_id: 7,
            input: HumanChallengeInput::PointerMove {
                x: 13,
                y: 24,
                buttons: 1,
            },
        };
        let value = serde_json::to_value(&command)
            .unwrap_or_else(|error| panic!("command should serialize: {error}"));
        assert_eq!(value["type"], "humanChallengeInput");
        assert_eq!(value["request_id"], 7);
        assert_eq!(value["input"]["kind"], "pointerMove");
        let json = serde_json::to_string(&command)
            .unwrap_or_else(|error| panic!("command should serialize to JSON: {error}"));
        let _: WorkerCommand = serde_json::from_str(&json)
            .unwrap_or_else(|error| panic!("command should deserialize from JSON: {error}"));
    }

    #[test]
    fn human_challenge_frame_event_round_trips() {
        let event = WorkerEvent::HumanChallengeFrame {
            request_id: 9,
            data: "jpeg-data".to_string(),
            device_width: 1280,
            device_height: 900,
        };
        let json = serde_json::to_string(&event)
            .unwrap_or_else(|error| panic!("event should serialize: {error}"));
        let decoded: WorkerEvent = serde_json::from_str(&json)
            .unwrap_or_else(|error| panic!("event should deserialize: {error}"));
        match decoded {
            WorkerEvent::HumanChallengeFrame {
                request_id,
                device_width,
                device_height,
                ..
            } => {
                assert_eq!(request_id, 9);
                assert_eq!(device_width, 1280);
                assert_eq!(device_height, 900);
            }
            _ => panic!("wrong event variant"),
        }
    }
}
