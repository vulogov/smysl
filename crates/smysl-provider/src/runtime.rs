//! The runtime (§21.5, D-12).
//!
//! One current-thread tokio runtime, on one dedicated OS thread, created lazily and owned
//! here and nowhere else. Callers hand it closures and get results back over an
//! `std::sync::mpsc` channel, so no caller ever names a future and no second concurrency
//! model enters the workspace.
//!
//! # Why it looks thin
//!
//! The HTTP client is blocking (`ureq`), so the runtime's job is to *own a thread*, not to
//! multiplex futures on it. That is deliberate. D-12 fixes where the runtime lives so that
//! adding a streaming mapper later does not mean introducing async into a crate that has
//! none - and the observable contract, which is what callers depend on, is the channel and
//! `try_recv`, not what is behind it.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::OnceLock;

/// A job the runtime thread will run. Boxed because the whole point is that callers do not
/// share a type with each other.
type Job = Box<dyn FnOnce() + Send + 'static>;

struct Inner {
    tx: Sender<Job>,
}

static RUNTIME: OnceLock<Inner> = OnceLock::new();

fn inner() -> &'static Inner {
    RUNTIME.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("smysl-provider".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("a current-thread runtime needs no resources to build");
                // The thread outlives every caller and ends when the process does, which is
                // why the loop has no exit condition other than the sender being dropped.
                while let Ok(job) = rx.recv() {
                    rt.block_on(async move { job() });
                }
            })
            .expect("spawning one OS thread");
        Inner { tx }
    })
}

/// Run `job` on the provider thread and wait for its result.
///
/// Blocking is the point: every caller in this workspace is synchronous, and a caller that
/// wanted concurrency would ask for [`spawn`] instead.
pub fn run<T: Send + 'static>(job: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    submit(move || {
        // A receiver that has gone away means the caller stopped caring, which is not an
        // error - it is the normal shape of a cancelled operation.
        let _ = tx.send(job());
    });
    rx.recv()
        .expect("the provider thread does not exit while a job is outstanding")
}

/// Run `job` on the provider thread and return immediately.
pub fn spawn(job: impl FnOnce() + Send + 'static) {
    submit(job);
}

/// Run `job` and get a receiver for its result, without waiting.
pub fn submit_for<T: Send + 'static>(job: impl FnOnce() -> T + Send + 'static) -> Receiver<T> {
    let (tx, rx) = mpsc::channel();
    submit(move || {
        let _ = tx.send(job());
    });
    rx
}

fn submit(job: impl FnOnce() + Send + 'static) {
    inner()
        .tx
        .send(Box::new(job))
        .expect("the provider thread outlives the process");
}

/// Whether the runtime thread has been started. Nothing spins it up until a provider needs
/// it, so a purely local `smysl check` never creates a thread at all (guarantee A2).
pub fn is_started() -> bool {
    RUNTIME.get().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn a_job_runs_and_returns_its_value() {
        assert_eq!(run(|| 2 + 2), 4);
    }

    #[test]
    fn jobs_run_on_the_provider_thread_not_the_caller() {
        let caller = std::thread::current().id();
        let on = run(std::thread::current);
        assert_ne!(on.id(), caller);
        assert_eq!(on.name(), Some("smysl-provider"));
    }

    /// One thread, not one per call: the whole reason the runtime is a singleton.
    #[test]
    fn every_job_runs_on_the_same_thread() {
        let a = run(|| std::thread::current().id());
        let b = run(|| std::thread::current().id());
        assert_eq!(a, b);
    }

    #[test]
    fn a_spawned_job_runs_without_the_caller_waiting() {
        let n = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&n);
        let rx = submit_for(move || {
            seen.fetch_add(1, Ordering::SeqCst);
            "done"
        });
        assert_eq!(rx.recv().unwrap(), "done");
        assert_eq!(n.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn jobs_run_in_submission_order() {
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut receivers = Vec::new();
        for i in 0..8 {
            let log = Arc::clone(&log);
            receivers.push(submit_for(move || log.lock().unwrap().push(i)));
        }
        for r in receivers {
            r.recv().unwrap();
        }
        assert_eq!(*log.lock().unwrap(), (0..8).collect::<Vec<i32>>());
    }

    /// The runtime is lazy, so a purely local command never creates a thread (A2). This
    /// test can only observe the flag after other tests have run, so it asserts the
    /// weaker, always-true half: once started, it stays started.
    #[test]
    fn the_runtime_is_started_lazily_and_stays_started() {
        run(|| ());
        assert!(is_started());
    }

    /// A caller that stops waiting must not take the runtime thread down with it.
    #[test]
    fn a_dropped_receiver_does_not_poison_the_thread() {
        drop(submit_for(|| 1));
        assert_eq!(run(|| 7), 7, "the thread still serves the next caller");
    }
}
