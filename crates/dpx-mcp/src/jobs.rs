//! Jobs for the thread that owns the [`Workspace`] (ADR-43). The server never touches the
//! editor state itself: it sends a boxed `FnOnce(&mut dyn Workspace)` over a channel, wakes
//! the owner (the GUI passes `egui::Context::request_repaint`) and awaits a oneshot reply.
//! The owner runs the jobs in the order they arrive: the headless server on a thread of its
//! own ([`spawn_headless`]), the GUI once per frame ([`JobQueue::run_pending`]).

use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use tokio::sync::oneshot;

use crate::error::ToolError;
use crate::workspace::{HeadlessWorkspace, Question, Workspace};

/// One unit of work on the editor state.
pub type Job = Box<dyn FnOnce(&mut dyn Workspace) + Send>;

/// How long a job may wait for the owner before the call fails. The GUI runs jobs every
/// frame, so only a stuck editor gets here; questions to the user have no limit.
pub const JOB_TIMEOUT: Duration = Duration::from_secs(60);

/// The server's side: sends jobs and awaits their replies.
#[derive(Clone)]
pub struct Executor {
    tx: mpsc::Sender<Job>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl std::fmt::Debug for Executor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Executor")
    }
}

/// The owner's side: receives jobs and runs them on a workspace.
pub struct JobQueue {
    rx: mpsc::Receiver<Job>,
}

/// A connected pair. `wake` is called after every job is queued (the GUI asks for a repaint,
/// so an idle window still runs the job).
pub fn channel(wake: impl Fn() + Send + Sync + 'static) -> (Executor, JobQueue) {
    let (tx, rx) = mpsc::channel();
    (
        Executor {
            tx,
            wake: Arc::new(wake),
        },
        JobQueue { rx },
    )
}

fn unavailable() -> ToolError {
    ToolError::new(
        "APP_UNAVAILABLE",
        "the editor stopped before it could run the call",
    )
}

impl Executor {
    fn send(&self, job: Job) -> Result<(), ToolError> {
        self.tx.send(job).map_err(|_| unavailable())?;
        (self.wake)();
        Ok(())
    }

    /// Runs `f` on the owner's thread and returns its result.
    pub async fn run<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut dyn Workspace) -> R + Send + 'static,
    ) -> Result<R, ToolError> {
        let (tx, rx) = oneshot::channel();
        self.send(Box::new(move |ws| {
            let _ = tx.send(f(ws));
        }))?;
        match tokio::time::timeout(JOB_TIMEOUT, rx).await {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(_)) => Err(ToolError::internal(
                "the editor could not finish the call (see its log)",
            )),
            Err(_) => Err(ToolError::new(
                "APP_BUSY",
                format!(
                    "the editor did not run the call within {} s; it may still run later",
                    JOB_TIMEOUT.as_secs()
                ),
            )),
        }
    }

    /// Asks the user through the workspace and waits for the answer (no time limit).
    pub async fn ask(&self, question: Question) -> Result<bool, ToolError> {
        let (tx, rx) = oneshot::channel();
        self.send(Box::new(move |ws| {
            ws.ask(
                question,
                Box::new(move |yes| {
                    let _ = tx.send(yes);
                }),
            );
        }))?;
        rx.await.map_err(|_| unavailable())
    }
}

impl JobQueue {
    /// Runs every queued job now (the GUI calls this once per frame, before its requesters
    /// handle input). A panicking job is caught and its call fails; the editor keeps going.
    /// Returns how many jobs ran.
    pub fn run_pending(&self, ws: &mut dyn Workspace) -> usize {
        let mut n = 0;
        while let Ok(job) = self.rx.try_recv() {
            run_one(job, ws);
            n += 1;
        }
        n
    }

    /// Runs jobs until every [`Executor`] is dropped (the headless thread).
    pub fn run_until_closed(&self, ws: &mut dyn Workspace) {
        while let Ok(job) = self.rx.recv() {
            run_one(job, ws);
        }
    }
}

fn run_one(job: Job, ws: &mut dyn Workspace) {
    if std::panic::catch_unwind(AssertUnwindSafe(|| job(ws))).is_err() {
        eprintln!("deluxepainter mcp: a tool call panicked; the editor state is kept");
    }
}

/// A thread owning `ws` that runs jobs until every executor is dropped.
pub fn spawn_headless(ws: HeadlessWorkspace) -> Executor {
    let (exec, queue) = channel(|| {});
    std::thread::Builder::new()
        .name("dpx-mcp-workspace".into())
        .spawn(move || {
            let mut ws = ws;
            queue.run_until_closed(&mut ws);
        })
        .expect("spawn the workspace thread");
    exec
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_run_in_order_on_the_owner_and_survive_panics() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (exec, queue) = channel(|| {});
        // The owner drains the queue a little later, from another thread.
        let owner = std::thread::spawn(move || {
            let mut ws = HeadlessWorkspace::default();
            std::thread::sleep(Duration::from_millis(50));
            queue.run_pending(&mut ws)
        });
        rt.block_on(async {
            let a = exec.run(|ws| ws.engine().document().revision());
            let b = exec.run(|_| -> u8 { panic!("boom") });
            let c = exec.run(|ws| ws.kind());
            let (a, b, c) = tokio::join!(a, b, c);
            assert_eq!(a.unwrap(), 0);
            assert_eq!(b.unwrap_err().code, "INTERNAL_ERROR");
            assert_eq!(c.unwrap(), "headless");
        });
        assert_eq!(owner.join().unwrap(), 3);
    }

    #[test]
    fn headless_asks_answer_no_and_a_gone_owner_fails_calls() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let exec = spawn_headless(HeadlessWorkspace::default());
            let q = Question {
                title: "t".into(),
                text: "x".into(),
                yes: "Yes".into(),
                no: "No".into(),
            };
            assert!(!exec.ask(q).await.unwrap());
            let (exec, queue) = channel(|| {});
            drop(queue);
            assert_eq!(exec.run(|_| ()).await.unwrap_err().code, "APP_UNAVAILABLE");
        });
    }
}
