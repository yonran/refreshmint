use std::path::Path;
use std::sync::Arc;

use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Promise};
use tokio::sync::Mutex;

use super::js_api::{self, PageInner, RefreshmintInner};

#[derive(Clone, Copy)]
pub struct SandboxRunOptions {
    pub emit_diagnostics: bool,
}

impl Default for SandboxRunOptions {
    fn default() -> Self {
        Self {
            emit_diagnostics: true,
        }
    }
}

fn maybe_diag(options: SandboxRunOptions, message: &str) {
    if options.emit_diagnostics {
        eprintln!("{message}");
    }
}

/// Run a driver script inside a QuickJS sandbox with the given page and config.
pub async fn run_driver(
    driver_path: &Path,
    page_inner: Arc<Mutex<PageInner>>,
    refreshmint_inner: Arc<Mutex<RefreshmintInner>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let options = SandboxRunOptions::default();
    maybe_diag(options, "[sandbox] Reading driver source...");
    let driver_source = tokio::fs::read_to_string(driver_path).await?;
    if options.emit_diagnostics {
        eprintln!("[sandbox] Driver source: {} bytes", driver_source.len());
    }
    run_script_source_with_options(&driver_source, page_inner, refreshmint_inner, options).await
}

/// Run arbitrary JS source inside the same QuickJS sandbox used by drivers.
pub async fn run_script_source(
    source: &str,
    page_inner: Arc<Mutex<PageInner>>,
    refreshmint_inner: Arc<Mutex<RefreshmintInner>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    run_script_source_with_options(
        source,
        page_inner,
        refreshmint_inner,
        SandboxRunOptions::default(),
    )
    .await
}

pub async fn run_script_source_with_options(
    source: &str,
    page_inner: Arc<Mutex<PageInner>>,
    refreshmint_inner: Arc<Mutex<RefreshmintInner>>,
    options: SandboxRunOptions,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let driver_source = source.to_string();

    maybe_diag(options, "[sandbox] Creating QuickJS runtime...");
    let runtime = AsyncRuntime::new()?;
    let context = AsyncContext::full(&runtime).await?;
    maybe_diag(options, "[sandbox] Runtime created.");

    // Register globals and execute the driver module
    maybe_diag(
        options,
        "[sandbox] Registering globals and evaluating driver...",
    );
    let setup_result: Result<(), String> = context
        .with(|ctx| {
            js_api::register_globals(&ctx, page_inner, refreshmint_inner)
                .map_err(|e| format!("failed to register globals: {e}"))?;
            maybe_diag(options, "[sandbox] Globals registered.");

            // Wrap the driver source in an async IIFE so top-level await works
            let wrapped = format!(
                "(async () => {{\n{source}\n}})();\n",
                source = driver_source
            );

            maybe_diag(options, "[sandbox] Evaluating wrapped script...");
            let result = ctx.eval::<Promise, _>(wrapped).catch(&ctx);
            let promise = match result {
                Ok(p) => {
                    maybe_diag(options, "[sandbox] Script evaluated, got promise.");
                    p
                }
                Err(e) => return Err(format!("failed to eval driver: {e}")),
            };

            ctx.globals()
                .set("__driver_promise__", promise)
                .map_err(|e| format!("failed to store promise: {e}"))?;

            Ok(())
        })
        .await;

    if let Err(msg) = setup_result {
        return Err(msg.into());
    }

    // Drive the QuickJS event loop until all jobs are done
    maybe_diag(options, "[sandbox] Driving event loop (runtime.idle)...");
    runtime.idle().await;
    maybe_diag(options, "[sandbox] Event loop done.");

    // Check if the promise resolved or rejected
    let result: Result<(), String> = context
        .with(|ctx| {
            let promise_result: Result<Promise, _> = ctx.globals().get("__driver_promise__");
            let promise = match promise_result {
                Ok(p) => p,
                Err(e) => return Err(format!("failed to get promise: {e}")),
            };
            match promise.result::<rquickjs::Value>() {
                None => {
                    maybe_diag(options, "[sandbox] Promise still pending after idle.");
                    Ok(())
                }
                Some(Ok(_)) => {
                    maybe_diag(options, "[sandbox] Promise resolved successfully.");
                    Ok(())
                }
                Some(Err(err)) => {
                    let msg = match Err::<(), _>(err).catch(&ctx) {
                        Err(caught) => caught.to_string(),
                        Ok(()) => "unknown JavaScript exception".to_string(),
                    };
                    if options.emit_diagnostics {
                        eprintln!("[sandbox] Promise rejected: {msg}");
                    }
                    Err(msg)
                }
            }
        })
        .await;

    match result {
        Ok(()) => Ok(()),
        Err(msg) => Err(format!("driver script failed: {msg}").into()),
    }
}
