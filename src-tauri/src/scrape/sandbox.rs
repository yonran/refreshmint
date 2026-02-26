use std::path::Path;
use std::sync::Arc;

use rquickjs::loader::{BuiltinLoader, BuiltinResolver};
use rquickjs::{
    AsyncContext, AsyncRuntime, CatchResultExt, CaughtError, Exception, Module, Promise,
};
use tokio::sync::Mutex;

use super::js_api::{self, PageInner, RefreshmintInner};

const REFRESHMINT_UTIL_MODULE_NAME: &str = "refreshmint:util";
const REFRESHMINT_UTIL_MODULE_SOURCE: &str =
    include_str!("../../../builtin-extensions/_shared/refreshmint-util.mjs");

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

fn format_caught_js_error(caught: CaughtError<'_>) -> String {
    match caught {
        CaughtError::Exception(exception) => exception
            .message()
            .filter(|message| !message.trim().is_empty())
            .unwrap_or_else(|| "JavaScript exception".to_string()),
        CaughtError::Value(_) => "JavaScript exception (non-Error value thrown)".to_string(),
        CaughtError::Error(error) => error.to_string(),
    }
}

fn source_uses_static_module_syntax(source: &str) -> bool {
    source.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("import ") || trimmed.starts_with("export ")
    })
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
    run_script_source_internal(source, Some((page_inner, refreshmint_inner)), options).await
}

type SandboxGlobals = (Arc<Mutex<PageInner>>, Arc<Mutex<RefreshmintInner>>);

async fn run_script_source_internal(
    source: &str,
    globals: Option<SandboxGlobals>,
    options: SandboxRunOptions,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let driver_source = source.to_string();

    maybe_diag(options, "[sandbox] Creating QuickJS runtime...");
    let runtime = AsyncRuntime::new()?;
    runtime
        .set_loader(
            BuiltinResolver::default().with_module(REFRESHMINT_UTIL_MODULE_NAME),
            BuiltinLoader::default()
                .with_module(REFRESHMINT_UTIL_MODULE_NAME, REFRESHMINT_UTIL_MODULE_SOURCE),
        )
        .await;
    let context = AsyncContext::full(&runtime).await?;
    maybe_diag(options, "[sandbox] Runtime created.");

    // Register globals and execute the driver module
    maybe_diag(
        options,
        "[sandbox] Registering globals and evaluating driver...",
    );
    let setup_result: Result<(), String> = context
        .with(|ctx| {
            if let Some((page_inner, refreshmint_inner)) = globals {
                js_api::register_globals(&ctx, page_inner, refreshmint_inner)
                    .map_err(|e| format!("failed to register globals: {e}"))?;
            }
            maybe_diag(options, "[sandbox] Globals registered.");

            let promise = if source_uses_static_module_syntax(&driver_source) {
                maybe_diag(options, "[sandbox] Evaluating script as module...");
                let module = Module::declare(ctx.clone(), "__driver__.mjs", driver_source.as_str())
                    .catch(&ctx)
                    .map_err(|e| format!("failed to compile driver module: {e}"))?;
                let (_module, module_promise) = module
                    .eval()
                    .catch(&ctx)
                    .map_err(|e| format!("failed to eval driver module: {e}"))?;
                maybe_diag(options, "[sandbox] Module evaluated, got promise.");
                module_promise
            } else {
                // Wrap script source in an async IIFE so top-level await works
                let wrapped = format!(
                    "(async () => {{\n{source}\n}})();\n",
                    source = driver_source
                );
                maybe_diag(options, "[sandbox] Evaluating wrapped script...");
                let result = ctx.eval::<Promise, _>(wrapped).catch(&ctx);
                match result {
                    Ok(p) => {
                        maybe_diag(options, "[sandbox] Script evaluated, got promise.");
                        p
                    }
                    Err(e) => return Err(format!("failed to eval driver: {e}")),
                }
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

    // Drive the QuickJS event loop until all jobs are done.
    // Avoid AsyncRuntime::idle() because it only updates the JS stack top once,
    // which can trigger intermittent stack overflows when jobs run on deeper stacks.
    maybe_diag(
        options,
        "[sandbox] Driving event loop (runtime.execute_pending_job)...",
    );
    drive_runtime(&runtime, &options).await;
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
                        Err(caught) => format_caught_js_error(caught),
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

async fn drive_runtime(runtime: &AsyncRuntime, options: &SandboxRunOptions) {
    loop {
        if !runtime.is_job_pending().await {
            break;
        }

        match runtime.execute_pending_job().await {
            Ok(true) => {}
            Ok(false) => {
                tokio::task::yield_now().await;
            }
            Err(err) => {
                if options.emit_diagnostics {
                    let _ = err
                        .0
                        .with(|ctx| {
                            let err = ctx.catch();
                            if let Some(exc) =
                                err.clone().into_object().and_then(Exception::from_object)
                            {
                                eprintln!("[sandbox] error executing job: {exc}");
                            } else {
                                eprintln!("[sandbox] error executing job: {err:?}");
                            }
                        })
                        .await;
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rquickjs::function::Async;
    use rquickjs::prelude::Func;
    use rquickjs::{CaughtError, Ctx, Function, Promise, Value};
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use std::time::Duration;

    fn format_caught_js_error_for_test(caught: CaughtError<'_>) -> String {
        match caught {
            CaughtError::Exception(exception) => exception
                .message()
                .filter(|message| !message.trim().is_empty())
                .unwrap_or_else(|| "JavaScript exception".to_string()),
            CaughtError::Value(_) => "JavaScript exception (non-Error value thrown)".to_string(),
            CaughtError::Error(error) => error.to_string(),
        }
    }

    async fn drive_runtime_idle(runtime: &AsyncRuntime) {
        runtime.idle().await;
    }

    async fn drive_runtime_with_fix(runtime: &AsyncRuntime) {
        drive_runtime(runtime, &SandboxRunOptions::default()).await;
    }

    /// Repro test for "Maximum call stack size exceeded".
    /// This test uses AsyncRuntime::idle() which only records the JS stack top
    /// once, so re-polling from a deep stack frame results in overflow.
    #[tokio::test]
    async fn runtime_idle_stack_overflow_fail_repro() {
        let runtime =
            AsyncRuntime::new().unwrap_or_else(|err| panic!("failed to create runtime: {err}"));
        // Set a small stack cap (32 KiB)
        runtime.set_max_stack_size(32 * 1024).await;
        let context = AsyncContext::full(&runtime)
            .await
            .unwrap_or_else(|err| panic!("failed to create context: {err}"));

        context
            .with(|ctx| {
                ctx.globals()
                    .set(
                        "hostYield",
                        Func::from(Async(move || async {
                            tokio::task::yield_now().await;
                        })),
                    )
                    .unwrap();
            })
            .await;

        eval_script_as_promise(
            &context,
            r#"
for (let i = 0; i < 5; i++) {
    await hostYield();
}
"#,
        )
        .await
        .unwrap_or_else(|err| panic!("{err}"));

        let mut idle_fut = Box::pin(runtime.idle());
        let mut first = true;

        let _result = tokio::time::timeout(
            Duration::from_secs(2),
            futures::future::poll_fn(|cx| {
                if first {
                    first = false;
                    idle_fut.as_mut().poll(cx)
                } else {
                    fn recurse_and_poll<F: Future<Output = ()> + ?Sized>(
                        f: Pin<&mut F>,
                        cx: &mut Context<'_>,
                        depth: usize,
                    ) -> Poll<()> {
                        if depth == 0 {
                            f.poll(cx)
                        } else {
                            let mut arr = [0u8; 1024];
                            std::hint::black_box(&mut arr);
                            recurse_and_poll(f, cx, depth - 1)
                        }
                    }
                    recurse_and_poll(idle_fut.as_mut(), cx, 40)
                }
            }),
        )
        .await;

        // This is expected to fail with Maximum call stack size exceeded
        // because we are re-polling idle() from a depth of 40KB while max stack is 32KB.
        let promise_result: Result<(), String> = context
            .with(|ctx| {
                let promise: Promise = ctx
                    .globals()
                    .get("__test_promise__")
                    .map_err(|e| e.to_string())?;
                match promise.result::<Value>() {
                    Some(Err(err)) => {
                        let msg = match Err::<(), _>(err).catch(&ctx) {
                            Err(caught) => format_caught_js_error_for_test(caught),
                            Ok(()) => "unknown JavaScript exception".to_string(),
                        };
                        Err(msg)
                    }
                    _ => Ok(()),
                }
            })
            .await;

        assert!(
            promise_result.is_err(),
            "Expected promise to fail with stack overflow"
        );
        assert!(promise_result
            .unwrap_err()
            .contains("Maximum call stack size exceeded"));
    }

    /// Verification test for the fix.
    /// This test uses the new drive_runtime() loop which uses execute_pending_job(),
    /// which updates the JS stack top on every job, making it immune to re-poll depth drift.
    #[tokio::test]
    async fn runtime_idle_stack_overflow_fix_verification() {
        let runtime =
            AsyncRuntime::new().unwrap_or_else(|err| panic!("failed to create runtime: {err}"));
        runtime.set_max_stack_size(32 * 1024).await;
        let context = AsyncContext::full(&runtime)
            .await
            .unwrap_or_else(|err| panic!("failed to create context: {err}"));

        context
            .with(|ctx| {
                ctx.globals()
                    .set(
                        "hostYield",
                        Func::from(Async(move || async {
                            tokio::task::yield_now().await;
                        })),
                    )
                    .unwrap();
            })
            .await;

        eval_script_as_promise(
            &context,
            r#"
for (let i = 0; i < 5; i++) {
    await hostYield();
}
"#,
        )
        .await
        .unwrap_or_else(|err| panic!("{err}"));

        let mut drive_fut = Box::pin(drive_runtime_with_fix(&runtime));
        let mut first = true;

        tokio::time::timeout(
            Duration::from_secs(2),
            futures::future::poll_fn(|cx| {
                if first {
                    first = false;
                    drive_fut.as_mut().poll(cx)
                } else {
                    fn recurse_and_poll<F: Future<Output = ()> + ?Sized>(
                        f: Pin<&mut F>,
                        cx: &mut Context<'_>,
                        depth: usize,
                    ) -> Poll<()> {
                        if depth == 0 {
                            f.poll(cx)
                        } else {
                            let mut arr = [0u8; 1024];
                            std::hint::black_box(&mut arr);
                            recurse_and_poll(f, cx, depth - 1)
                        }
                    }
                    // Even when re-polled from 40KB deep, it should pass because
                    // drive_runtime calls execute_pending_job which updates stack top.
                    recurse_and_poll(drive_fut.as_mut(), cx, 40)
                }
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("drive_runtime timed out"));

        assert_promise_resolved(&context).await;
    }

    /// Verification test for "Maximum call stack size exceeded" using the real
    /// `run_script_source_internal` entry point.
    /// This test should now PASS with the fix.
    #[tokio::test]
    async fn run_script_stack_overflow_depth_fix_verification() {
        // Use a script that needs some stack
        let source = r#"
            function recurse(n) {
                if (n === 0) return 0;
                return 1 + recurse(n - 1);
            }
            recurse(10);
        "#;

        async fn recurse_and_run(
            source: &str,
            depth: usize,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            if depth == 0 {
                // Set a small stack cap (32 KiB)
                let options = SandboxRunOptions {
                    emit_diagnostics: false,
                };
                run_script_source_internal(source, None, options).await
            } else {
                let mut arr = [0u8; 1024];
                std::hint::black_box(&mut arr);
                Box::pin(recurse_and_run(source, depth - 1)).await
            }
        }

        // Run from 200KB deep.
        // With the fix, this should SUCCEED because drive_runtime updates stack top.
        let result = recurse_and_run(source, 200).await;

        if let Err(ref e) = result {
            panic!("Verification failed: {}", e);
        }

        assert!(
            result.is_ok(),
            "Expected script to succeed with fix even when called from deep stack"
        );
    }

    async fn eval_script_as_promise(context: &AsyncContext, script: &str) -> Result<(), String> {
        context
            .with(|ctx| {
                let wrapped = format!("(async () => {{\n{script}\n}})();\n");
                let promise = ctx
                    .eval::<Promise, _>(wrapped)
                    .catch(&ctx)
                    .map_err(|e| format!("failed to eval script: {e}"))?;
                ctx.globals()
                    .set("__test_promise__", promise)
                    .map_err(|e| format!("failed to store promise: {e}"))?;
                Ok(())
            })
            .await
    }

    async fn assert_promise_resolved(context: &AsyncContext) {
        let result: Result<(), String> = context
            .with(|ctx| {
                let promise: Promise = ctx
                    .globals()
                    .get("__test_promise__")
                    .map_err(|e| format!("failed to get promise: {e}"))?;
                match promise.result::<Value>() {
                    None => Ok(()),
                    Some(Ok(_)) => Ok(()),
                    Some(Err(err)) => match Err::<(), _>(err).catch(&ctx) {
                        Err(caught) => Err(format_caught_js_error_for_test(caught)),
                        Ok(()) => Err("unknown JavaScript exception".to_string()),
                    },
                }
            })
            .await;
        if let Err(err) = result {
            panic!("script promise rejected: {err}");
        }
    }

    fn set_timeout_spawn<'js>(
        ctx: Ctx<'js>,
        callback: Function<'js>,
        ms: u64,
    ) -> rquickjs::Result<()> {
        ctx.spawn(async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            let _ = callback.call::<_, ()>(());
        });
        Ok(())
    }

    fn register_test_timers(ctx: Ctx<'_>) -> rquickjs::Result<()> {
        let globals = ctx.globals();
        globals.set("setTimeout", Func::from(set_timeout_spawn))?;
        Ok(())
    }

    #[tokio::test]
    async fn drive_runtime_runs_spawned_jobs() {
        let runtime =
            AsyncRuntime::new().unwrap_or_else(|err| panic!("failed to create runtime: {err}"));
        let context = AsyncContext::full(&runtime)
            .await
            .unwrap_or_else(|err| panic!("failed to create context: {err}"));

        let setup_result: Result<(), String> = context
            .with(|ctx| register_test_timers(ctx).map_err(|e| e.to_string()))
            .await;
        assert!(setup_result.is_ok(), "failed to register test globals");

        eval_script_as_promise(
            &context,
            r#"
globalThis.__count = 0;
for (let i = 0; i < 5; i += 1) {
  setTimeout(() => { globalThis.__count += 1; }, 1);
}
"#,
        )
        .await
        .unwrap_or_else(|err| panic!("{err}"));

        tokio::time::timeout(Duration::from_secs(2), drive_runtime_idle(&runtime))
            .await
            .unwrap_or_else(|err| panic!("drive_runtime timed out: {err}"));

        assert_promise_resolved(&context).await;
        let count: Result<i32, String> = context
            .with(|ctx| ctx.globals().get("__count").map_err(|e| e.to_string()))
            .await;
        assert_eq!(count.unwrap_or_default(), 5);
    }

    #[tokio::test]
    #[ignore]
    async fn drive_runtime_stack_overflow_regression_stress() {
        let runtime =
            AsyncRuntime::new().unwrap_or_else(|err| panic!("failed to create runtime: {err}"));
        runtime.set_max_stack_size(64 * 1024).await;
        let context = AsyncContext::full(&runtime)
            .await
            .unwrap_or_else(|err| panic!("failed to create context: {err}"));

        let setup_result: Result<(), String> = context
            .with(|ctx| register_test_timers(ctx).map_err(|e| e.to_string()))
            .await;
        assert!(setup_result.is_ok(), "failed to register test globals");

        eval_script_as_promise(
            &context,
            r#"
globalThis.__count = 0;
async function run() {
  for (let i = 0; i < 2000; i += 1) {
    await new Promise(resolve => setTimeout(resolve, 0));
  }
  for (let i = 0; i < 2000; i += 1) {
    Promise.resolve().then(() => { globalThis.__count += 1; });
  }
}
await run();
"#,
        )
        .await
        .unwrap_or_else(|err| panic!("{err}"));

        tokio::time::timeout(Duration::from_secs(5), drive_runtime_idle(&runtime))
            .await
            .unwrap_or_else(|err| panic!("drive_runtime timed out: {err}"));

        assert_promise_resolved(&context).await;
        let count: Result<i32, String> = context
            .with(|ctx| ctx.globals().get("__count").map_err(|e| e.to_string()))
            .await;
        assert_eq!(count.unwrap_or_default(), 2000);
    }

    #[tokio::test]
    async fn module_import_refreshmint_util_inspect_works() {
        let source = r#"
import { inspect } from 'refreshmint:util';
const root = { answer: 42 };
root.self = root;
const cause = new TypeError('bad mfa code');
const err = new Error('top level failure');
err.cause = cause;
err.details = root;

const out = inspect(err);
if (!out.includes('Error: top level failure')) {
  throw new Error('inspect output missing error header: ' + out);
}
if (!out.includes('cause')) {
  throw new Error('inspect output missing cause: ' + out);
}
if (!out.includes('[Circular]')) {
  throw new Error('inspect output missing circular marker: ' + out);
}
"#;
        let options = SandboxRunOptions {
            emit_diagnostics: false,
        };
        let result = run_script_source_internal(source, None, options).await;
        assert!(
            result.is_ok(),
            "expected module import script to pass: {result:?}"
        );
    }
}
