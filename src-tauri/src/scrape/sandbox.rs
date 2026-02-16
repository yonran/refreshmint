use std::path::Path;
use std::sync::Arc;

use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Promise};
use tokio::sync::Mutex;

use super::js_api::{self, PageInner, RefreshmintInner};

/// Run a driver script inside a QuickJS sandbox with the given page and config.
pub async fn run_driver(
    driver_path: &Path,
    page_inner: Arc<Mutex<PageInner>>,
    refreshmint_inner: Arc<Mutex<RefreshmintInner>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    eprintln!("[sandbox] Reading driver source...");
    let driver_source = tokio::fs::read_to_string(driver_path).await?;
    eprintln!("[sandbox] Driver source: {} bytes", driver_source.len());

    eprintln!("[sandbox] Creating QuickJS runtime...");
    let runtime = AsyncRuntime::new()?;
    let context = AsyncContext::full(&runtime).await?;
    eprintln!("[sandbox] Runtime created.");

    // Register globals and execute the driver module
    eprintln!("[sandbox] Registering globals and evaluating driver...");
    let setup_result: Result<(), String> = context
        .with(|ctx| {
            js_api::register_globals(&ctx, page_inner, refreshmint_inner)
                .map_err(|e| format!("failed to register globals: {e}"))?;
            eprintln!("[sandbox] Globals registered.");

            // Wrap the driver source in an async IIFE so top-level await works
            let wrapped = format!(
                "(async () => {{\n{source}\n}})();\n",
                source = driver_source
            );

            eprintln!("[sandbox] Evaluating wrapped script...");
            let result = ctx.eval::<Promise, _>(wrapped).catch(&ctx);
            let promise = match result {
                Ok(p) => {
                    eprintln!("[sandbox] Script evaluated, got promise.");
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
    eprintln!("[sandbox] Driving event loop (runtime.idle)...");
    runtime.idle().await;
    eprintln!("[sandbox] Event loop done.");

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
                    eprintln!("[sandbox] Promise still pending after idle.");
                    Ok(())
                }
                Some(Ok(_)) => {
                    eprintln!("[sandbox] Promise resolved successfully.");
                    Ok(())
                }
                Some(Err(err)) => {
                    let msg = format!("{err:?}");
                    eprintln!("[sandbox] Promise rejected: {msg}");
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
