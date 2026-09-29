//! Runs blocking Tauri command bodies off the main thread, one at a time.
//!
//! A plain `#[tauri::command] fn` runs on the main thread, so a slow one (an
//! `hledger print`, a big journal rewrite) freezes the whole window. Running
//! it on a blocking thread fixes that, but the main thread also ran those
//! commands strictly one after another, and code relies on that: GL
//! mutations take `.gl.lock` try-only (see docs/locking.md), so two of them
//! running concurrently would make the second fail with "currently in use"
//! instead of waiting. The lane keeps that one-at-a-time behavior.

use tokio::sync::Mutex;

static LANE: Mutex<()> = Mutex::const_new(());

/// Runs `body` on a blocking thread once every previously queued body has
/// finished. Waiting is async, so queued commands don't tie up the runtime's
/// worker threads that scrapes and other async commands need.
pub(crate) async fn run_serialized<T, F>(body: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let _turn = LANE.lock().await;
    match tokio::task::spawn_blocking(body).await {
        Ok(value) => value,
        // Re-raise the body's panic, as it would have on the main thread.
        Err(err) => std::panic::resume_unwind(err.into_panic()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn run_serialized_runs_bodies_one_at_a_time_off_the_caller_thread() {
        let caller = std::thread::current().id();
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let tasks: Vec<_> = (0..8)
            .map(|_| {
                let active = Arc::clone(&active);
                let max_active = Arc::clone(&max_active);
                tokio::spawn(run_serialized(move || {
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    max_active.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(10));
                    active.fetch_sub(1, Ordering::SeqCst);
                    std::thread::current().id()
                }))
            })
            .collect();
        for task in tasks {
            match task.await {
                Ok(thread) => assert_ne!(thread, caller),
                Err(err) => panic!("lane task failed: {err}"),
            }
        }
        assert_eq!(max_active.load(Ordering::SeqCst), 1);
    }
}
