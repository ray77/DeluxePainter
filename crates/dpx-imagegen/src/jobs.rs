//! Generation jobs (ADR-45): each generation runs on its own `std::thread` worker, so the GUI
//! thread never blocks; the GUI polls ([`JobManager::status`], [`JobManager::revision`]) and
//! MCP tools (`generate_image`, `get_generation_status`, `apply_generated_image`,
//! `cancel_generation`) will use the same calls. [`JobManager`] is `Clone + Send + Sync`.
//!
//! - States: running, done (pictures ready), failed (typed [`GenError`]), cancelled.
//! - Cancelling marks the job cancelled at once and sets its [`CancelToken`]; a blocking
//!   request already sent cannot be stopped, so its answer is thrown away when it comes
//!   (a cloud provider may still bill it).
//! - At most [`JobManager::MAX_RUNNING`] jobs run at once (more is `BUSY`): every
//!   generation may cost money, and MCP clients must not start them without limit.
//! - Finished jobs keep their pictures until [`JobManager::forget`] or until more than
//!   [`JobManager::KEEP_FINISHED`] jobs have finished (the oldest go first).
//! - A provider that panics fails its job; it does not take the editor down.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use dpx_core::RgbaImage;

use crate::provider::{
    CancelToken, GenError, GenerationRequest, ImageGenerationProvider, ProviderStatus,
};

/// A job's id, unique for the manager's lifetime (from 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JobId(pub u64);

impl std::fmt::Display for JobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "gen-{}", self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobState {
    Running,
    Done,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn as_str(self) -> &'static str {
        match self {
            JobState::Running => "running",
            JobState::Done => "done",
            JobState::Failed => "failed",
            JobState::Cancelled => "cancelled",
        }
    }
}

/// A snapshot of one job.
#[derive(Clone, Debug, PartialEq)]
pub struct JobStatus {
    pub id: JobId,
    pub state: JobState,
    /// The provider's id.
    pub provider: String,
    pub request: GenerationRequest,
    /// Time since the start (until the end once finished).
    pub elapsed: Duration,
    /// Why it failed.
    pub error: Option<GenError>,
    /// Pictures ready (done jobs).
    pub images: usize,
}

struct Job {
    id: JobId,
    provider: String,
    request: GenerationRequest,
    started: Instant,
    finished: Option<Instant>,
    state: JobState,
    error: Option<GenError>,
    images: Option<Arc<Vec<RgbaImage>>>,
    cancel: CancelToken,
}

impl Job {
    fn status(&self) -> JobStatus {
        JobStatus {
            id: self.id,
            state: self.state,
            provider: self.provider.clone(),
            request: self.request.clone(),
            elapsed: self.finished.unwrap_or_else(Instant::now) - self.started,
            error: self.error.clone(),
            images: self.images.as_ref().map_or(0, |i| i.len()),
        }
    }
}

/// Called after every change of a job's state (e.g. egui's `request_repaint`).
pub type Waker = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct State {
    jobs: Vec<Job>,
    next: u64,
    /// Increases with every change.
    revision: u64,
    waker: Option<Waker>,
}

#[derive(Default)]
struct Inner {
    state: Mutex<State>,
    changed: Condvar,
}

/// The generation jobs of one app (GUI and MCP share it).
#[derive(Clone, Default)]
pub struct JobManager {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for JobManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self.lock();
        f.debug_struct("JobManager")
            .field("jobs", &s.jobs.len())
            .field("revision", &s.revision)
            .finish()
    }
}

impl JobManager {
    /// Most jobs running at once.
    pub const MAX_RUNNING: usize = 2;
    /// Most finished jobs kept with their pictures.
    pub const KEEP_FINISHED: usize = 8;

    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.inner.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Calls `waker` after every state change from now on (on the thread that made it).
    pub fn set_waker(&self, waker: Waker) {
        self.lock().waker = Some(waker);
    }

    /// Starts generating `request` with `provider` on a worker thread. Refused at once, with
    /// nothing sent, when the provider is not set up, the request does not fit its
    /// capabilities or [`Self::MAX_RUNNING`] jobs are running.
    pub fn submit(
        &self,
        provider: Arc<dyn ImageGenerationProvider>,
        request: GenerationRequest,
    ) -> Result<JobId, GenError> {
        if let ProviderStatus::NotConfigured(m) = provider.status() {
            return Err(GenError::NotConfigured(m));
        }
        provider.capabilities().check(&request)?;
        let cancel = CancelToken::new();
        let id = {
            let mut s = self.lock();
            let running = s
                .jobs
                .iter()
                .filter(|j| j.state == JobState::Running)
                .count();
            if running >= Self::MAX_RUNNING {
                return Err(GenError::Busy { running });
            }
            s.next += 1;
            let id = JobId(s.next);
            s.jobs.push(Job {
                id,
                provider: provider.id().to_string(),
                request: request.clone(),
                started: Instant::now(),
                finished: None,
                state: JobState::Running,
                error: None,
                images: None,
                cancel: cancel.clone(),
            });
            s.revision += 1;
            id
        };
        let manager = self.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("dpx-imagegen-{id}"))
            .spawn(move || {
                let result =
                    catch_unwind(AssertUnwindSafe(|| provider.generate(&request, &cancel)));
                let result = result.unwrap_or_else(|_| {
                    Err(GenError::BadResponse(
                        "the image generator stopped unexpectedly".into(),
                    ))
                });
                manager.finish(id, result);
            });
        if let Err(e) = spawned {
            let mut s = self.lock();
            s.jobs.retain(|j| j.id != id);
            return Err(GenError::Network(format!(
                "could not start a worker thread: {e}"
            )));
        }
        self.notify();
        Ok(id)
    }

    fn finish(&self, id: JobId, result: Result<Vec<RgbaImage>, GenError>) {
        {
            let mut s = self.lock();
            let Some(job) = s.jobs.iter_mut().find(|j| j.id == id) else {
                return;
            };
            if job.state != JobState::Running {
                // Cancelled (or forgotten) meanwhile: the answer is thrown away.
                return;
            }
            job.finished = Some(Instant::now());
            match result {
                Ok(images) if !images.is_empty() => {
                    job.state = JobState::Done;
                    job.images = Some(Arc::new(images));
                }
                Ok(_) => {
                    job.state = JobState::Failed;
                    job.error = Some(GenError::BadResponse("no picture came back".into()));
                }
                Err(GenError::Cancelled) => job.state = JobState::Cancelled,
                Err(e) => {
                    job.state = JobState::Failed;
                    job.error = Some(e);
                }
            }
            s.revision += 1;
            prune(&mut s.jobs);
        }
        self.notify();
    }

    fn notify(&self) {
        self.inner.changed.notify_all();
        let waker = self.lock().waker.clone();
        if let Some(w) = waker {
            w();
        }
    }

    /// The job's state, or `None` for an unknown (or forgotten) id.
    pub fn status(&self, id: JobId) -> Option<JobStatus> {
        self.lock()
            .jobs
            .iter()
            .find(|j| j.id == id)
            .map(Job::status)
    }

    /// Every job kept, oldest first.
    pub fn list(&self) -> Vec<JobStatus> {
        self.lock().jobs.iter().map(Job::status).collect()
    }

    /// Increases with every change of any job (cheap to poll every frame).
    pub fn revision(&self) -> u64 {
        self.lock().revision
    }

    /// The pictures of a done job.
    pub fn images(&self, id: JobId) -> Option<Arc<Vec<RgbaImage>>> {
        self.lock()
            .jobs
            .iter()
            .find(|j| j.id == id && j.state == JobState::Done)
            .and_then(|j| j.images.clone())
    }

    /// Cancels a running job (true), else does nothing (false).
    pub fn cancel(&self, id: JobId) -> bool {
        let cancelled = {
            let mut s = self.lock();
            match s
                .jobs
                .iter_mut()
                .find(|j| j.id == id && j.state == JobState::Running)
            {
                Some(job) => {
                    job.cancel.cancel();
                    job.state = JobState::Cancelled;
                    job.finished = Some(Instant::now());
                    s.revision += 1;
                    true
                }
                None => false,
            }
        };
        if cancelled {
            self.notify();
        }
        cancelled
    }

    /// Drops a finished job and its pictures (true); a running job must be cancelled first.
    pub fn forget(&self, id: JobId) -> bool {
        let mut s = self.lock();
        let before = s.jobs.len();
        s.jobs
            .retain(|j| j.id != id || j.state == JobState::Running);
        let gone = s.jobs.len() < before;
        if gone {
            s.revision += 1;
        }
        gone
    }

    /// Waits until the job is no longer running or `timeout` has passed; its state then.
    /// For tests and MCP calls, never the GUI thread.
    pub fn wait(&self, id: JobId, timeout: Duration) -> Option<JobStatus> {
        let deadline = Instant::now() + timeout;
        let mut s = self.lock();
        loop {
            let job = s.jobs.iter().find(|j| j.id == id)?;
            let now = Instant::now();
            if job.state != JobState::Running || now >= deadline {
                return Some(job.status());
            }
            s = self
                .inner
                .changed
                .wait_timeout(s, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}

/// Keeps at most [`JobManager::KEEP_FINISHED`] finished jobs (the newest).
fn prune(jobs: &mut Vec<Job>) {
    let finished = jobs.iter().filter(|j| j.state != JobState::Running).count();
    let mut excess = finished.saturating_sub(JobManager::KEEP_FINISHED);
    jobs.retain(|j| {
        if excess > 0 && j.state != JobState::Running {
            excess -= 1;
            false
        } else {
            true
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::Capabilities;
    use crate::size::{CustomSizes, SizeRule};
    use dpx_core::Rgba8;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;

    /// A provider whose generations wait for a go from the test.
    struct Gated {
        go: Mutex<mpsc::Receiver<Result<usize, GenError>>>,
        configured: bool,
        panics: bool,
    }

    impl Gated {
        fn new() -> (Arc<Self>, mpsc::Sender<Result<usize, GenError>>) {
            let (tx, rx) = mpsc::channel();
            (
                Arc::new(Self {
                    go: Mutex::new(rx),
                    configured: true,
                    panics: false,
                }),
                tx,
            )
        }
    }

    impl ImageGenerationProvider for Gated {
        fn id(&self) -> &str {
            "gated"
        }
        fn display_name(&self) -> &str {
            "Gated"
        }
        fn needs_network(&self) -> bool {
            false
        }
        fn cost_note(&self) -> &str {
            "test"
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities {
                sizes: SizeRule::Custom(CustomSizes::OPENAI),
                transparent_background: false,
                max_variants: 4,
                seed: false,
                max_prompt_chars: 100,
            }
        }
        fn status(&self) -> ProviderStatus {
            if self.configured {
                ProviderStatus::Ready
            } else {
                ProviderStatus::NotConfigured("no key".into())
            }
        }
        fn generate(
            &self,
            request: &GenerationRequest,
            _cancel: &CancelToken,
        ) -> Result<Vec<RgbaImage>, GenError> {
            if self.panics {
                panic!("provider bug");
            }
            // Blocks like a request in flight; ignores the token on purpose.
            let n = self.go.lock().unwrap().recv().unwrap()?;
            Ok((0..n)
                .map(|i| {
                    RgbaImage::new(request.size.0, request.size.1, Rgba8::opaque(i as u8, 0, 0))
                })
                .collect())
        }
    }

    fn request() -> GenerationRequest {
        GenerationRequest::new("castle", (1024, 768))
    }

    const WAIT: Duration = Duration::from_secs(10);

    #[test]
    fn a_job_runs_on_a_worker_and_delivers_its_pictures() {
        let jobs = JobManager::new();
        let wakes = Arc::new(AtomicUsize::new(0));
        let w = wakes.clone();
        jobs.set_waker(Arc::new(move || {
            w.fetch_add(1, Ordering::SeqCst);
        }));
        let (p, go) = Gated::new();
        let id = jobs.submit(p, request()).unwrap();
        let s = jobs.status(id).unwrap();
        assert_eq!(s.state, JobState::Running);
        assert_eq!(s.provider, "gated");
        assert!(jobs.images(id).is_none());
        let rev = jobs.revision();
        go.send(Ok(2)).unwrap();
        let s = jobs.wait(id, WAIT).unwrap();
        assert_eq!(s.state, JobState::Done);
        assert_eq!(s.images, 2);
        assert!(jobs.revision() > rev);
        let images = jobs.images(id).unwrap();
        assert_eq!(images.len(), 2);
        assert_eq!((images[0].width(), images[0].height()), (1024, 768));
        assert!(wakes.load(Ordering::SeqCst) >= 2);
        assert_eq!(id.to_string(), "gen-1");
        assert!(jobs.forget(id));
        assert!(jobs.status(id).is_none());
    }

    #[test]
    fn failures_are_kept_typed() {
        let jobs = JobManager::new();
        let (p, go) = Gated::new();
        let id = jobs.submit(p.clone(), request()).unwrap();
        go.send(Err(GenError::Timeout)).unwrap();
        let s = jobs.wait(id, WAIT).unwrap();
        assert_eq!(s.state, JobState::Failed);
        assert_eq!(s.error, Some(GenError::Timeout));
        assert!(jobs.images(id).is_none());
        // No picture at all is a failure too.
        let id = jobs.submit(p, request()).unwrap();
        go.send(Ok(0)).unwrap();
        let s = jobs.wait(id, WAIT).unwrap();
        assert!(matches!(s.error, Some(GenError::BadResponse(_))));
    }

    #[test]
    fn cancelling_drops_the_late_answer() {
        let jobs = JobManager::new();
        let (p, go) = Gated::new();
        let id = jobs.submit(p, request()).unwrap();
        assert!(jobs.cancel(id));
        assert_eq!(jobs.status(id).unwrap().state, JobState::Cancelled);
        assert!(!jobs.cancel(id));
        // The worker's answer arrives afterwards and is thrown away.
        go.send(Ok(1)).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let s = jobs.status(id).unwrap();
        assert_eq!(s.state, JobState::Cancelled);
        assert_eq!(s.images, 0);
        assert!(jobs.images(id).is_none());
    }

    #[test]
    fn refusals_happen_before_anything_runs() {
        let jobs = JobManager::new();
        let (tx, rx) = mpsc::channel();
        drop(tx);
        let unconfigured = Arc::new(Gated {
            go: Mutex::new(rx),
            configured: false,
            panics: false,
        });
        assert_eq!(
            jobs.submit(unconfigured, request()),
            Err(GenError::NotConfigured("no key".into()))
        );
        let (p, go) = Gated::new();
        let odd = GenerationRequest::new("castle", (320, 240));
        assert!(matches!(
            jobs.submit(p.clone(), odd),
            Err(GenError::InvalidRequest(_))
        ));
        assert!(jobs.list().is_empty());
        // At most two at once.
        let a = jobs.submit(p.clone(), request()).unwrap();
        let b = jobs.submit(p.clone(), request()).unwrap();
        assert_eq!(
            jobs.submit(p.clone(), request()),
            Err(GenError::Busy { running: 2 })
        );
        assert!(!jobs.forget(a), "a running job is not forgotten");
        go.send(Ok(1)).unwrap();
        go.send(Ok(1)).unwrap();
        jobs.wait(a, WAIT).unwrap();
        jobs.wait(b, WAIT).unwrap();
        let c = jobs.submit(p, request()).unwrap();
        assert_eq!(c, JobId(3));
        go.send(Ok(1)).unwrap();
        assert_eq!(jobs.wait(c, WAIT).unwrap().state, JobState::Done);
    }

    #[test]
    fn a_panicking_provider_fails_its_job_only() {
        let jobs = JobManager::new();
        let (tx, rx) = mpsc::channel();
        drop(tx);
        let p = Arc::new(Gated {
            go: Mutex::new(rx),
            configured: true,
            panics: true,
        });
        let id = jobs.submit(p, request()).unwrap();
        let s = jobs.wait(id, WAIT).unwrap();
        assert_eq!(s.state, JobState::Failed);
        assert!(matches!(s.error, Some(GenError::BadResponse(_))));
    }

    #[test]
    fn old_finished_jobs_are_pruned() {
        let jobs = JobManager::new();
        let (p, go) = Gated::new();
        let mut ids = Vec::new();
        for _ in 0..JobManager::KEEP_FINISHED + 3 {
            let id = jobs.submit(p.clone(), request()).unwrap();
            go.send(Ok(1)).unwrap();
            jobs.wait(id, WAIT).unwrap();
            ids.push(id);
        }
        let kept: Vec<JobId> = jobs.list().iter().map(|s| s.id).collect();
        assert_eq!(kept, ids[3..].to_vec());
        assert!(jobs.status(ids[0]).is_none());
    }

    #[test]
    fn the_manager_is_send_and_sync() {
        fn check<T: Send + Sync + Clone>() {}
        check::<JobManager>();
    }
}
