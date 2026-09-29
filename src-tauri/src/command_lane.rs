//! Runs blocking Tauri command bodies off the main thread.
//!
//! A plain `#[tauri::command] fn` runs on the main thread, so a slow one (an
//! `hledger print`, a big journal rewrite) freezes the whole window. Running
//! it on a blocking thread fixes that, but the main thread also ran those
//! commands strictly one after another, and code relies on that:
//! - GL mutations take `.gl.lock` try-only (see docs/locking.md), so two of
//!   them running concurrently would make the second fail with "currently in
//!   use" instead of waiting.
//! - A post rewrites general.journal and an account journal in turn; a read
//!   landing in between would see a half-applied change (and a consistency
//!   check would report it as a real problem).
//!
//! So commands that change anything run exclusively, and commands that only
//! read run alongside each other (never alongside a writer). Without that,
//! quick reads like `get_login_config` sat behind a multi-second
//! `suggest_gl_categories` in the queue.

use tokio::sync::RwLock;

static LANE: RwLock<()> = RwLock::const_new(());

/// For commands that may change ledger files, lock files or other state:
/// runs `body` on a blocking thread once no other command body is running.
pub(crate) async fn run_exclusive<T, F>(body: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    run_exclusive_on(&LANE, body).await
}

/// For commands that only read files: runs `body` on a blocking thread
/// alongside other readers, once no exclusive body is running. The lane is
/// fair, so a queued writer isn't starved by a stream of readers.
pub(crate) async fn run_shared<T, F>(body: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    run_shared_on(&LANE, body).await
}

async fn run_exclusive_on<T, F>(lane: &RwLock<()>, body: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let _turn = lane.write().await;
    run_blocking(body).await
}

async fn run_shared_on<T, F>(lane: &RwLock<()>, body: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let _turn = lane.read().await;
    run_blocking(body).await
}

/// Waiting for the lane above is async, so queued commands don't tie up the
/// runtime's worker threads that scrapes and other async commands need.
async fn run_blocking<T, F>(body: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
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

    #[derive(Default)]
    struct Activity {
        readers: AtomicUsize,
        writers: AtomicUsize,
        max_readers: AtomicUsize,
        max_writers: AtomicUsize,
        overlaps: AtomicUsize,
    }

    fn shared_body(activity: Arc<Activity>) -> impl FnOnce() -> std::thread::ThreadId {
        move || {
            let now = activity.readers.fetch_add(1, Ordering::SeqCst) + 1;
            activity.max_readers.fetch_max(now, Ordering::SeqCst);
            if activity.writers.load(Ordering::SeqCst) != 0 {
                activity.overlaps.fetch_add(1, Ordering::SeqCst);
            }
            std::thread::sleep(Duration::from_millis(50));
            activity.readers.fetch_sub(1, Ordering::SeqCst);
            std::thread::current().id()
        }
    }

    fn exclusive_body(activity: Arc<Activity>) -> impl FnOnce() -> std::thread::ThreadId {
        move || {
            let now = activity.writers.fetch_add(1, Ordering::SeqCst) + 1;
            activity.max_writers.fetch_max(now, Ordering::SeqCst);
            if activity.readers.load(Ordering::SeqCst) != 0 {
                activity.overlaps.fetch_add(1, Ordering::SeqCst);
            }
            std::thread::sleep(Duration::from_millis(10));
            activity.writers.fetch_sub(1, Ordering::SeqCst);
            std::thread::current().id()
        }
    }

    // A lane per test, so other tests' commands can't interleave with this one.
    static TEST_LANE: RwLock<()> = RwLock::const_new(());

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn shared_bodies_overlap_but_exclusive_ones_run_alone_off_the_caller_thread() {
        let caller = std::thread::current().id();
        let activity = Arc::new(Activity::default());
        let mut tasks = Vec::new();
        for i in 0..12 {
            let activity = Arc::clone(&activity);
            tasks.push(if i % 3 == 0 {
                tokio::spawn(run_exclusive_on(&TEST_LANE, exclusive_body(activity)))
            } else {
                tokio::spawn(run_shared_on(&TEST_LANE, shared_body(activity)))
            });
        }
        for task in tasks {
            match task.await {
                Ok(thread) => assert_ne!(thread, caller),
                Err(err) => panic!("lane task failed: {err}"),
            }
        }
        assert_eq!(activity.max_writers.load(Ordering::SeqCst), 1);
        assert_eq!(activity.overlaps.load(Ordering::SeqCst), 0);
        assert!(
            activity.max_readers.load(Ordering::SeqCst) >= 2,
            "shared bodies should run concurrently"
        );
    }
}
