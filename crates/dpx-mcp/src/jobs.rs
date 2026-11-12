//! Jobs for the thread that owns the [`Workspace`] (ADR-43). The server never touches the
//! editor state itself: it sends a [`Job`] over a channel, wakes the owner (the GUI passes
//! `egui::Context::request_repaint`) and awaits a oneshot reply.
//!
//! The owner runs the jobs in the order they arrive: the headless server on a thread of its
//! own ([`spawn_headless`]), the GUI once per pass. The GUI may also *hold* a job (it stays
//! first in the queue, while a mouse button is held on the picture) or *refuse* it (the
//! call fails with the owner's error, while a requester is open) without running it; for
//! that every job says whether its call reads, changes or asks ([`JobKind`]).

use std::collections::VecDeque;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use tokio::sync::oneshot;

use crate::error::ToolError;
use crate::workspace::{HeadlessWorkspace, Question, Workspace};

/// How long a job may wait for the owner before the call fails. The GUI runs jobs every
/// pass (and holds a changing job at most a few seconds), so only a stuck editor gets here.
pub const JOB_TIMEOUT: Duration = Duration::from_secs(60);

/// How long the user has to answer a question ([`Executor::ask`]). The GUI closes its
/// requester after this time and answers [`NO_ANSWER`]; the server side waits
/// [`ASK_MARGIN`] longer as a safety net (the GUI may hold the question a few seconds
/// while a mouse button is held before it shows it).
pub const ASK_TIMEOUT: Duration = Duration::from_secs(45);
pub const ASK_MARGIN: Duration = Duration::from_secs(15);

/// The error code of a question nobody answered in time.
pub const NO_ANSWER: &str = "NO_ANSWER";

/// What a job's call does, so the owner can hold or refuse it before it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    /// A call that only reads the editor state (or keeps books: saved state, log).
    Read,
    /// A call that changes a document, the pages or the brush.
    Edit,
    /// A question to the user (it opens a requester).
    Ask,
}

/// The type-erased work of a job: run it on a workspace, or refuse it with an error.
trait Task: Send {
    fn run(self: Box<Self>, ws: &mut dyn Workspace);
    fn refuse(self: Box<Self>, error: ToolError);
    /// The caller stopped waiting (timed out or cancelled): running it would be invisible.
    fn abandoned(&self) -> bool;
}

struct Call<F, R> {
    f: F,
    tx: oneshot::Sender<Result<R, ToolError>>,
}

impl<F, R> Task for Call<F, R>
where
    F: FnOnce(&mut dyn Workspace) -> R + Send,
    R: Send,
{
    fn run(self: Box<Self>, ws: &mut dyn Workspace) {
        let Call { f, tx } = *self;
        let _ = tx.send(Ok(f(ws)));
    }

    fn refuse(self: Box<Self>, error: ToolError) {
        let _ = self.tx.send(Err(error));
    }

    fn abandoned(&self) -> bool {
        self.tx.is_closed()
    }
}

struct AskTask {
    question: Question,
    tx: oneshot::Sender<Result<bool, ToolError>>,
}

impl Task for AskTask {
    fn run(self: Box<Self>, ws: &mut dyn Workspace) {
        let AskTask { question, tx } = *self;
        ws.ask(
            question,
            Box::new(move |answer| {
                let _ = tx.send(answer);
            }),
        );
    }

    fn refuse(self: Box<Self>, error: ToolError) {
        let _ = self.tx.send(Err(error));
    }

    fn abandoned(&self) -> bool {
        self.tx.is_closed()
    }
}

/// One unit of work on the editor state.
pub struct Job {
    kind: JobKind,
    queued: Instant,
    task: Box<dyn Task>,
}

impl std::fmt::Debug for Job {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Job")
            .field("kind", &self.kind)
            .field("queued", &self.queued)
            .finish_non_exhaustive()
    }
}

impl Job {
    pub fn kind(&self) -> JobKind {
        self.kind
    }

    /// How long it has waited since it was queued.
    pub fn waited(&self) -> Duration {
        self.queued.elapsed()
    }

    /// Runs it on `ws`. A panic is caught (the call fails with `INTERNAL_ERROR`, the owner
    /// keeps going); a job whose caller stopped waiting is dropped without running, so a
    /// timed-out call never changes the picture later.
    pub fn run(self, ws: &mut dyn Workspace) {
        if self.task.abandoned() {
            return;
        }
        let task = self.task;
        if std::panic::catch_unwind(AssertUnwindSafe(|| task.run(ws))).is_err() {
            eprintln!("deluxepainter mcp: a tool call panicked; the editor state is kept");
        }
    }

    /// Fails its call with `error` without running it.
    pub fn refuse(self, error: ToolError) {
        self.task.refuse(error);
    }
}

/// The server's side: sends jobs and awaits their replies.
#[derive(Clone)]
pub struct Executor {
    tx: mpsc::Sender<Job>,
    wake: Arc<dyn Fn() + Send + Sync>,
    /// The kind of the jobs [`Executor::run`] sends.
    kind: JobKind,
}

impl std::fmt::Debug for Executor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Executor({:?})", self.kind)
    }
}

/// The owner's side: receives jobs (and keeps the ones it holds back, in order).
pub struct JobQueue {
    rx: mpsc::Receiver<Job>,
    held: VecDeque<Job>,
}

impl std::fmt::Debug for JobQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JobQueue({} held)", self.held.len())
    }
}

/// A connected pair. `wake` is called after every job is queued (the GUI asks for a repaint,
/// so an idle window still runs the job).
pub fn channel(wake: impl Fn() + Send + Sync + 'static) -> (Executor, JobQueue) {
    let (tx, rx) = mpsc::channel();
    (
        Executor {
            tx,
            wake: Arc::new(wake),
            kind: JobKind::Read,
        },
        JobQueue {
            rx,
            held: VecDeque::new(),
        },
    )
}

fn unavailable() -> ToolError {
    ToolError::new(
        "APP_UNAVAILABLE",
        "the editor stopped before it could run the call",
    )
}

impl Executor {
    /// The same executor whose jobs are marked as changing the editor state ([`JobKind::Edit`]).
    pub fn editing(&self) -> Executor {
        Executor {
            kind: JobKind::Edit,
            ..self.clone()
        }
    }

    pub fn kind(&self) -> JobKind {
        self.kind
    }

    fn send(&self, kind: JobKind, task: Box<dyn Task>) -> Result<(), ToolError> {
        let job = Job {
            kind,
            queued: Instant::now(),
            task,
        };
        self.tx.send(job).map_err(|_| unavailable())?;
        (self.wake)();
        Ok(())
    }

    /// Runs `f` on the owner's thread and returns its result. The owner may refuse it (its
    /// error is returned, e.g. `EDITOR_BUSY`).
    pub async fn run<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut dyn Workspace) -> R + Send + 'static,
    ) -> Result<R, ToolError> {
        let (tx, rx) = oneshot::channel();
        self.send(self.kind, Box::new(Call { f, tx }))?;
        match tokio::time::timeout(JOB_TIMEOUT, rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err(ToolError::internal(
                "the editor could not finish the call (see its log)",
            )),
            Err(_) => Err(ToolError::new(
                "APP_BUSY",
                format!(
                    "the editor did not run the call within {} s; it was dropped",
                    JOB_TIMEOUT.as_secs()
                ),
            )),
        }
    }

    /// Asks the user through the workspace and waits for the answer (at most [`ASK_TIMEOUT`]
    /// plus [`ASK_MARGIN`]; then the error code is [`NO_ANSWER`]).
    pub async fn ask(&self, question: Question) -> Result<bool, ToolError> {
        let (tx, rx) = oneshot::channel();
        self.send(JobKind::Ask, Box::new(AskTask { question, tx }))?;
        let limit = ASK_TIMEOUT + ASK_MARGIN;
        match tokio::time::timeout(limit, rx).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) => Err(unavailable()),
            Err(_) => Err(no_answer()),
        }
    }
}

/// The error of a question nobody answered within [`ASK_TIMEOUT`].
pub fn no_answer() -> ToolError {
    ToolError::new(
        NO_ANSWER,
        format!("nobody answered within {} s", ASK_TIMEOUT.as_secs()),
    )
}

impl JobQueue {
    /// The next job: a held one first, else one that arrived (without waiting).
    pub fn take(&mut self) -> Option<Job> {
        self.held.pop_front().or_else(|| self.rx.try_recv().ok())
    }

    /// Puts `job` back at the front; [`JobQueue::take`] returns it again first.
    pub fn hold(&mut self, job: Job) {
        self.held.push_front(job);
    }

    /// True while a job is held back.
    pub fn holding(&self) -> bool {
        !self.held.is_empty()
    }

    /// Runs every queued job now. Returns how many ran.
    pub fn run_pending(&mut self, ws: &mut dyn Workspace) -> usize {
        let mut n = 0;
        while let Some(job) = self.take() {
            job.run(ws);
            n += 1;
        }
        n
    }

    /// Fails every queued and held job with `error` (the owner stops serving).
    pub fn refuse_all(&mut self, error: &ToolError) {
        while let Some(job) = self.take() {
            job.refuse(error.clone());
        }
    }

    /// Runs jobs until every [`Executor`] is dropped (the headless thread).
    pub fn run_until_closed(&mut self, ws: &mut dyn Workspace) {
        while let Some(job) = self.held.pop_front() {
            job.run(ws);
        }
        while let Ok(job) = self.rx.recv() {
            job.run(ws);
        }
    }
}

/// A thread owning `ws` that runs jobs until every executor is dropped.
pub fn spawn_headless(ws: HeadlessWorkspace) -> Executor {
    let (exec, mut queue) = channel(|| {});
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

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn jobs_run_in_order_on_the_owner_and_survive_panics() {
        let (exec, mut queue) = channel(|| {});
        // The owner drains the queue a little later, from another thread.
        let owner = std::thread::spawn(move || {
            let mut ws = HeadlessWorkspace::default();
            std::thread::sleep(Duration::from_millis(50));
            queue.run_pending(&mut ws)
        });
        rt().block_on(async {
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
        rt().block_on(async {
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

    #[test]
    fn owners_hold_and_refuse_jobs_by_kind() {
        let (exec, mut queue) = channel(|| {});
        let owner = std::thread::spawn(move || {
            let mut ws = HeadlessWorkspace::default();
            let mut kinds = Vec::new();
            // Wait for both jobs, hold the edit once, then refuse it and run the read.
            let mut got = Vec::new();
            while got.len() < 2 {
                match queue.take() {
                    Some(j) => got.push(j),
                    None => std::thread::sleep(Duration::from_millis(1)),
                }
            }
            for j in got.into_iter().rev() {
                queue.hold(j);
            }
            assert!(queue.holding());
            while let Some(j) = queue.take() {
                kinds.push(j.kind());
                match j.kind() {
                    JobKind::Edit => j.refuse(ToolError::new("EDITOR_BUSY", "busy")),
                    _ => j.run(&mut ws),
                }
            }
            kinds
        });
        rt().block_on(async {
            let edit = exec.editing();
            assert_eq!(edit.kind(), JobKind::Edit);
            let a = edit.run(|_| 1);
            let b = exec.run(|_| 2);
            let (a, b) = tokio::join!(a, b);
            assert_eq!(a.unwrap_err().code, "EDITOR_BUSY");
            assert_eq!(b.unwrap(), 2);
        });
        assert_eq!(
            owner.join().unwrap(),
            vec![JobKind::Edit, JobKind::Read],
            "held jobs keep their order"
        );
    }

    #[test]
    fn abandoned_jobs_do_not_run() {
        let (exec, mut queue) = channel(|| {});
        let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let r = ran.clone();
        rt().block_on(async {
            // The call is dropped before the owner gets to it.
            let call = exec.run(move |_| r.store(true, std::sync::atomic::Ordering::SeqCst));
            let _ = tokio::time::timeout(Duration::from_millis(10), call).await;
        });
        let mut ws = HeadlessWorkspace::default();
        assert_eq!(queue.run_pending(&mut ws), 1);
        assert!(!ran.load(std::sync::atomic::Ordering::SeqCst));
    }
}
