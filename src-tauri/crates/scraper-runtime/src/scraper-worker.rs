//! Stable scraper execution process.
//!
//! This binary deliberately does not link `app_lib`: only the scraper runtime
//! modules below are compilation inputs.  Consequently, rebuilding unrelated
//! Tauri/backend code does not relink this executable or change the code
//! identity macOS Keychain associates with scraper credential reads.

#![allow(dead_code, clippy::wrong_self_convention)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

#[path = "../../../src/account_config.rs"]
mod account_config;
#[path = "../../../src/builtin_extensions.rs"]
mod builtin_extensions;
#[path = "../../../src/debug_registry.rs"]
mod debug_registry;
#[path = "../../../src/js_module_loader.rs"]
mod js_module_loader;
#[path = "../../../src/login_config.rs"]
mod login_config;
#[path = "../../../src/scrape.rs"]
mod scrape;
#[path = "../../../src/scraper_protocol.rs"]
mod scraper_protocol;
#[path = "../../../src/secret.rs"]
mod secret;
#[path = "../../../src/ts_strip.rs"]
mod ts_strip;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "scraper-worker")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Own one persistent browser until the debug session is stopped.
    DebugStart {
        #[arg(long)]
        ledger: PathBuf,
        #[arg(long)]
        login: String,
        #[arg(long)]
        extension: String,
        #[arg(long)]
        socket: PathBuf,
        #[arg(long)]
        profile: Option<PathBuf>,
        #[arg(long)]
        headless: bool,
        #[arg(long)]
        prompt_requires_override: bool,
    },
    /// Run one scrape and exchange typed events/commands over stdout/stdin.
    RunScrape {
        #[arg(long)]
        ledger: PathBuf,
        #[arg(long)]
        login: String,
        #[arg(long)]
        extension: String,
        #[arg(long)]
        profile: Option<PathBuf>,
        #[arg(long)]
        headless: bool,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    match Args::parse().command {
        Command::DebugStart {
            ledger,
            login,
            extension,
            socket,
            profile,
            headless,
            prompt_requires_override,
        } => scrape::debug::run_debug_session(scrape::debug::DebugStartConfig {
            login_name: login,
            extension_name: extension,
            ledger_dir: ledger,
            profile_override: profile,
            headless,
            socket_path: Some(socket),
            prompt_requires_override,
        }),
        Command::RunScrape {
            ledger,
            login,
            extension,
            profile,
            headless,
        } => run_scrape_worker(ledger, login, extension, profile, headless),
    }
}

fn write_event(output: &std::sync::Mutex<std::io::Stdout>, event: &scraper_protocol::WorkerEvent) {
    use std::io::Write;

    if let Ok(mut output) = output.lock() {
        if serde_json::to_writer(&mut *output, event).is_ok() {
            let _ = output.write_all(b"\n");
            let _ = output.flush();
        }
    }
}

async fn run_human_challenge_stream(
    request_id: u64,
    message: String,
    page: chromiumoxide::Page,
    output: std::sync::Arc<std::sync::Mutex<std::io::Stdout>>,
    mut inputs: tokio::sync::mpsc::UnboundedReceiver<scraper_protocol::HumanChallengeInput>,
    timeout: std::time::Duration,
) -> Result<(), String> {
    use chromiumoxide::cdp::browser_protocol::input::{
        DispatchMouseEventParams, DispatchMouseEventType, MouseButton,
    };
    use chromiumoxide::cdp::browser_protocol::page::{
        EventScreencastFrame, ScreencastFrameAckParams, StartScreencastFormat,
        StartScreencastParams, StopScreencastParams,
    };
    use futures::StreamExt;

    let mut frames = page
        .event_listener::<EventScreencastFrame>()
        .await
        .map_err(|error| format!("failed to listen for challenge frames: {error}"))?;
    page.start_screencast(
        StartScreencastParams::builder()
            .format(StartScreencastFormat::Jpeg)
            .quality(80)
            .max_width(1280)
            .max_height(900)
            .every_nth_frame(1)
            .build(),
    )
    .await
    .map_err(|error| format!("failed to start challenge stream: {error}"))?;

    write_event(
        &output,
        &scraper_protocol::WorkerEvent::HumanChallengeRequested {
            request_id,
            message,
        },
    );

    let timeout = tokio::time::sleep(timeout);
    tokio::pin!(timeout);
    let result = loop {
        tokio::select! {
            () = &mut timeout => {
                break Err("human challenge timed out".to_string());
            }
            frame = frames.next() => {
                let Some(frame) = frame else {
                    break Err("human challenge frame stream closed".to_string());
                };
                write_event(
                    &output,
                    &scraper_protocol::WorkerEvent::HumanChallengeFrame {
                        request_id,
                        data: frame.data.0.clone(),
                        device_width: frame.metadata.device_width.max(0.0).round() as u32,
                        device_height: frame.metadata.device_height.max(0.0).round() as u32,
                    },
                );
                if let Err(error) = page
                    .ack_screencast(ScreencastFrameAckParams::new(frame.session_id))
                    .await
                {
                    break Err(format!("failed to acknowledge challenge frame: {error}"));
                }
            }
            input = inputs.recv() => {
                let Some(input) = input else {
                    break Err("human challenge input channel closed".to_string());
                };
                let params = match input {
                    scraper_protocol::HumanChallengeInput::PointerMove { x, y, buttons } => {
                        DispatchMouseEventParams::builder()
                            .r#type(DispatchMouseEventType::MouseMoved)
                            .x(x)
                            .y(y)
                            .buttons(buttons)
                            .build()
                    }
                    scraper_protocol::HumanChallengeInput::PointerDown { x, y } => {
                        DispatchMouseEventParams::builder()
                            .r#type(DispatchMouseEventType::MousePressed)
                            .x(x)
                            .y(y)
                            .button(MouseButton::Left)
                            .buttons(1)
                            .click_count(1)
                            .build()
                    }
                    scraper_protocol::HumanChallengeInput::PointerUp { x, y } => {
                        DispatchMouseEventParams::builder()
                            .r#type(DispatchMouseEventType::MouseReleased)
                            .x(x)
                            .y(y)
                            .button(MouseButton::Left)
                            .buttons(0)
                            .click_count(1)
                            .build()
                    }
                    scraper_protocol::HumanChallengeInput::Complete => break Ok(()),
                    scraper_protocol::HumanChallengeInput::Cancel => {
                        break Err("human challenge cancelled".to_string());
                    }
                };
                let params = match params {
                    Ok(params) => params,
                    Err(error) => {
                        break Err(format!("invalid challenge pointer input: {error}"));
                    }
                };
                if let Err(error) = page.execute(params).await {
                    break Err(format!("failed to relay challenge pointer input: {error}"));
                }
            }
        }
    };

    let _ = page.stop_screencast(StopScreencastParams::default()).await;
    write_event(
        &output,
        &scraper_protocol::WorkerEvent::HumanChallengeClosed { request_id },
    );
    result
}

fn run_scrape_worker(
    ledger_dir: PathBuf,
    login_name: String,
    extension_name: String,
    profile_override: Option<PathBuf>,
    headless: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::BufRead;
    use std::sync::{Arc, Mutex};

    type PromptResult = Result<Option<String>, String>;
    type PendingPrompts =
        Arc<Mutex<std::collections::HashMap<u64, std::sync::mpsc::Sender<PromptResult>>>>;
    type PendingChallenges = Arc<
        Mutex<
            std::collections::HashMap<
                u64,
                tokio::sync::mpsc::UnboundedSender<scraper_protocol::HumanChallengeInput>,
            >,
        >,
    >;

    let start = {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        match serde_json::from_str::<scraper_protocol::WorkerCommand>(line.trim())? {
            scraper_protocol::WorkerCommand::Start {
                prompt_overrides,
                prompt_requires_override,
                retain_failure_debug,
                enable_human_challenges,
                human_challenge_timeout_secs,
            } => (
                prompt_overrides,
                prompt_requires_override,
                retain_failure_debug,
                enable_human_challenges,
                human_challenge_timeout_secs,
            ),
            _ => return Err("first scraper worker command must be start".into()),
        }
    };
    let (
        prompt_overrides,
        prompt_requires_override,
        retain_failure_debug,
        enable_human_challenges,
        human_challenge_timeout_secs,
    ) = start;

    let output = Arc::new(Mutex::new(std::io::stdout()));
    let pending_prompts: PendingPrompts = Arc::new(Mutex::new(std::collections::HashMap::new()));
    let pending_challenges: PendingChallenges =
        Arc::new(Mutex::new(std::collections::HashMap::new()));
    let (cancel_sender, cancel_receiver) = tokio::sync::watch::channel(false);

    {
        let pending_prompts = pending_prompts.clone();
        let pending_challenges = pending_challenges.clone();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                let Ok(command) = serde_json::from_str::<scraper_protocol::WorkerCommand>(&line)
                else {
                    eprintln!("ignoring invalid scraper worker command");
                    continue;
                };
                match command {
                    scraper_protocol::WorkerCommand::Start { .. } => {
                        eprintln!("ignoring duplicate scraper worker start command");
                    }
                    scraper_protocol::WorkerCommand::PromptAnswer {
                        request_id,
                        answer,
                        error,
                    } => {
                        if let Ok(mut pending) = pending_prompts.lock() {
                            if let Some(sender) = pending.remove(&request_id) {
                                let response = error.map_or(Ok(answer), Err);
                                let _ = sender.send(response);
                            }
                        }
                    }
                    scraper_protocol::WorkerCommand::HumanChallengeInput { request_id, input } => {
                        let cancel_requested =
                            matches!(input, scraper_protocol::HumanChallengeInput::Cancel);
                        if let Ok(pending) = pending_challenges.lock() {
                            if let Some(sender) = pending.get(&request_id) {
                                let _ = sender.send(input);
                            }
                        }
                        if cancel_requested {
                            let _ = cancel_sender.send(true);
                        }
                    }
                    scraper_protocol::WorkerCommand::Cancel => {
                        let _ = cancel_sender.send(true);
                        if let Ok(mut pending) = pending_prompts.lock() {
                            for (_, sender) in pending.drain() {
                                let _ = sender.send(Err("prompt cancelled".to_string()));
                            }
                        }
                        if let Ok(mut pending) = pending_challenges.lock() {
                            for (_, sender) in pending.drain() {
                                let _ = sender.send(scraper_protocol::HumanChallengeInput::Cancel);
                            }
                        }
                    }
                }
            }
            // Losing the parent control pipe has the same semantics as cancel.
            let _ = cancel_sender.send(true);
            if let Ok(mut pending) = pending_prompts.lock() {
                for (_, sender) in pending.drain() {
                    let _ = sender.send(Err("prompt cancelled".to_string()));
                }
            }
            if let Ok(mut pending) = pending_challenges.lock() {
                for (_, sender) in pending.drain() {
                    let _ = sender.send(scraper_protocol::HumanChallengeInput::Cancel);
                }
            }
        });
    }

    let prompt_sequence = Arc::new(std::sync::atomic::AtomicU64::new(1));
    let prompt_ui_handler = {
        let output = output.clone();
        let pending_prompts = pending_prompts.clone();
        let prompt_sequence = prompt_sequence.clone();
        Arc::new(move |message: String, choices: Option<Vec<String>>| {
            let request_id = prompt_sequence.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let (sender, receiver) = std::sync::mpsc::channel();
            pending_prompts
                .lock()
                .map_err(|err| err.to_string())?
                .insert(request_id, sender);
            write_event(
                &output,
                &scraper_protocol::WorkerEvent::PromptRequested {
                    request_id,
                    message,
                    choices,
                },
            );
            receiver
                .recv()
                .map_err(|_| "prompt cancelled".to_string())?
        })
    };

    let human_challenge_ui_handler: Option<scrape::js_api::HumanChallengeUiHandler> =
        if enable_human_challenges {
            let output = output.clone();
            let pending_challenges = pending_challenges.clone();
            let prompt_sequence = prompt_sequence.clone();
            let timeout = std::time::Duration::from_secs(human_challenge_timeout_secs);
            Some(Arc::new(
                move |message: String, page: chromiumoxide::Page| {
                    let output = output.clone();
                    let pending_challenges = pending_challenges.clone();
                    let request_id =
                        prompt_sequence.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    Box::pin(async move {
                        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
                        pending_challenges
                            .lock()
                            .map_err(|error| error.to_string())?
                            .insert(request_id, sender);
                        let result = run_human_challenge_stream(
                            request_id, message, page, output, receiver, timeout,
                        )
                        .await;
                        if let Ok(mut pending) = pending_challenges.lock() {
                            pending.remove(&request_id);
                        }
                        result
                    })
                        as std::pin::Pin<
                            Box<dyn std::future::Future<Output = Result<(), String>> + Send>,
                        >
                },
            ))
        } else {
            None
        };

    let log_listener: scrape::ScrapeLogListener = {
        let output = output.clone();
        Arc::new(move |event: &scrape::js_api::DebugOutputEvent| {
            write_event(
                &output,
                &scraper_protocol::WorkerEvent::Log {
                    stream: event.stream,
                    line: event.line.clone(),
                },
            );
        })
    };

    #[cfg(unix)]
    let failure_debug = if retain_failure_debug {
        let socket_path = scrape::debug::default_debug_socket_path(&login_name)?;
        let output = output.clone();
        Some(scrape::FailureDebugConfig {
            socket_path,
            max_duration: std::time::Duration::from_secs(30 * 60),
            ready_listener: Some(Arc::new(move |descriptor, error| {
                write_event(
                    &output,
                    &scraper_protocol::WorkerEvent::FailureRetained {
                        error: error.message.clone(),
                        artifacts_dir: error
                            .artifacts_dir
                            .as_ref()
                            .map(|path| path.to_string_lossy().into_owned()),
                        debug_session: descriptor.clone(),
                    },
                );
            })),
        })
    } else {
        None
    };

    #[cfg(not(unix))]
    let _ = retain_failure_debug;

    let result = scrape::run_scrape(scrape::ScrapeConfig {
        login_name,
        extension_name,
        ledger_dir,
        profile_override,
        headless,
        prompt_overrides,
        prompt_requires_override,
        prompt_ui_handler: Some(prompt_ui_handler),
        human_challenge_ui_handler,
        log_listener: Some(log_listener),
        cancel: Some(cancel_receiver),
        #[cfg(unix)]
        failure_debug,
    });
    let artifacts_dir = result
        .as_ref()
        .err()
        .and_then(|error| error.artifacts_dir.as_ref())
        .map(|path| path.to_string_lossy().into_owned());
    let error = result.as_ref().err().map(|error| error.message.clone());
    write_event(
        &output,
        &scraper_protocol::WorkerEvent::Result {
            ok: result.is_ok(),
            error,
            artifacts_dir,
        },
    );
    Ok(())
}
