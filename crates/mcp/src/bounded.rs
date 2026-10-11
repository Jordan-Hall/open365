//! Bounded run time for the tools whose work cannot be interrupted: layout, the PDF writer and
//! the formula engine have no cancellation hook, so the work runs on a worker thread and the
//! caller waits at most the configured deadline.
//!
//! A timeout is reported to the agent, but the worker thread is not killed: it may still run to
//! the end in the background and then drops its result. The one place where a late finish could
//! have an effect, the PDF file write, checks a cancellation flag the caller sets on timeout.

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime};

use open365_commands::{Evaluated, FormulaEngine, Target};

use crate::views;

/// Stack size of a worker thread; layout recurses through nested tables.
const WORKER_STACK: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub(crate) enum BoundedError {
    TimedOut,
    /// The worker could not start, or it panicked.
    Failed,
}

/// Runs `work` on a new thread and waits up to `deadline` for its result.
pub(crate) fn run<T, F>(deadline: Duration, work: F) -> Result<T, BoundedError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("open365-mcp-worker".to_owned())
        .stack_size(WORKER_STACK)
        .spawn(move || {
            let _ = tx.send(work());
        })
        .map_err(|_| BoundedError::Failed)?;
    match rx.recv_timeout(deadline) {
        Ok(v) => Ok(v),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(BoundedError::TimedOut),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(BoundedError::Failed),
    }
}

/// What an evaluation worker was built from. When any part differs the worker is stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stamp {
    /// The open document's edit counter.
    pub revision: u64,
    /// Modified time and size of the file on disk, when it can be read.
    pub file: Option<(Option<SystemTime>, u64)>,
}

impl Stamp {
    pub fn of(target: &Target) -> Self {
        let file = target
            .path()
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| (m.modified().ok(), m.len()));
        Self {
            revision: target.revision(),
            file,
        }
    }
}

struct Job {
    sheet: usize,
    row: u32,
    col: u32,
    formula: Option<String>,
    reply: mpsc::Sender<Result<Evaluated, String>>,
}

/// A thread that owns a copy of one workbook and its calculation engine, built once and reused
/// by every `evaluate_formula` call until the document or its file changes. The engine is not
/// `Send`, so it lives and dies on this thread. Dropping the handle ends the thread after its
/// current job.
#[derive(Debug)]
pub(crate) struct EvalWorker {
    jobs: mpsc::Sender<Job>,
    pub stamp: Stamp,
}

impl EvalWorker {
    pub fn spawn(bytes: Vec<u8>, hidden: bool, stamp: Stamp) -> Result<Self, BoundedError> {
        let (jobs, rx) = mpsc::channel::<Job>();
        thread::Builder::new()
            .name("open365-mcp-eval".to_owned())
            .stack_size(WORKER_STACK)
            .spawn(move || serve(&bytes, hidden, &rx))
            .map_err(|_| BoundedError::Failed)?;
        Ok(Self { jobs, stamp })
    }

    /// Evaluates one formula, waiting at most `deadline` (building the engine counts when this
    /// is the first call).
    pub fn evaluate(
        &self,
        deadline: Duration,
        sheet: usize,
        row: u32,
        col: u32,
        formula: Option<String>,
    ) -> Result<Result<Evaluated, String>, BoundedError> {
        let (reply, answer) = mpsc::channel();
        self.jobs
            .send(Job {
                sheet,
                row,
                col,
                formula,
                reply,
            })
            .map_err(|_| BoundedError::Failed)?;
        match answer.recv_timeout(deadline) {
            Ok(v) => Ok(v),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(BoundedError::TimedOut),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(BoundedError::Failed),
        }
    }
}

fn serve(bytes: &[u8], hidden: bool, jobs: &mpsc::Receiver<Job>) {
    let target = match Target::open_xlsx(bytes) {
        Ok(t) => t,
        Err(e) => {
            let message = e.to_string();
            for job in jobs {
                let _ = job.reply.send(Err(message.clone()));
            }
            return;
        }
    };
    let Some(wb) = target.xlsx() else { return };
    let mut engine = {
        let keep = views::visibility_filter(wb, hidden);
        FormulaEngine::build(wb, &keep)
    };
    for job in jobs {
        let keep = views::visibility_filter(wb, hidden);
        let result = engine
            .evaluate(
                wb,
                job.sheet,
                job.row,
                job.col,
                job.formula.as_deref(),
                &keep,
            )
            .map_err(|e| e.to_string());
        let _ = job.reply.send(result);
    }
}
