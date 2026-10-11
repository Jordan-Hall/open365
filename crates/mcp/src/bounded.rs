//! Bounded run time for the slow tools: layout, the PDF writer and the formula engine run on a
//! worker thread and the caller waits at most the configured deadline.
//!
//! Each request owns a cancellation flag. On a timeout the caller sets it, and the long loops of
//! layout pagination, the PDF writer and recalculation check it and stop. The PDF file write
//! removes a file the request created. The thread itself is not killed: it ends at its next
//! check, so a single step that never checks can still run to its end, and its result is dropped.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, SystemTime};

use open365_commands::{CommandError, Evaluated, FormulaEngine, Target};

use crate::views;

/// Stack size of a worker thread; layout recurses through nested tables.
const WORKER_STACK: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub(crate) enum BoundedError {
    TimedOut,
    /// The worker could not start, or it panicked.
    Failed,
}

/// Runs `work` on a new thread and waits up to `deadline` for its result. `work` gets the
/// request's cancellation flag, which is set when the deadline passes.
pub(crate) fn run<T, F>(deadline: Duration, work: F) -> Result<T, BoundedError>
where
    T: Send + 'static,
    F: FnOnce(Arc<AtomicBool>) -> T + Send + 'static,
{
    let cancel = Arc::new(AtomicBool::new(false));
    let token = Arc::clone(&cancel);
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("open365-mcp-worker".to_owned())
        .stack_size(WORKER_STACK)
        .spawn(move || {
            let _ = tx.send(work(token));
        })
        .map_err(|_| BoundedError::Failed)?;
    match rx.recv_timeout(deadline) {
        Ok(v) => Ok(v),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            cancel.store(true, Ordering::Relaxed);
            Err(BoundedError::TimedOut)
        }
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
    cancel: Arc<AtomicBool>,
    reply: mpsc::Sender<Result<Evaluated, CommandError>>,
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
    ) -> Result<Result<Evaluated, CommandError>, BoundedError> {
        let (reply, answer) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.jobs
            .send(Job {
                sheet,
                row,
                col,
                formula,
                cancel: Arc::clone(&cancel),
                reply,
            })
            .map_err(|_| BoundedError::Failed)?;
        match answer.recv_timeout(deadline) {
            Ok(v) => Ok(v),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // The job stops at its next check and the worker keeps its engine.
                cancel.store(true, Ordering::Relaxed);
                Err(BoundedError::TimedOut)
            }
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
                let _ = job.reply.send(Err(CommandError::Formula(message.clone())));
            }
            return;
        }
    };
    let Some(wb) = target.xlsx() else { return };
    // Built by the first job that gets to run, under that job's cancellation flag. A build
    // that is cancelled leaves `None` and the next job builds again.
    let mut engine: Option<FormulaEngine> = None;
    for job in jobs {
        if job.cancel.load(Ordering::Relaxed) {
            let _ = job.reply.send(Err(CommandError::Cancelled));
            continue;
        }
        let keep = views::visibility_filter(wb, hidden);
        if engine.is_none() {
            match FormulaEngine::build_cancellable(wb, &keep, &job.cancel) {
                Ok(e) => engine = Some(e),
                Err(e) => {
                    let _ = job.reply.send(Err(e));
                    continue;
                }
            }
        }
        let Some(engine) = engine.as_mut() else {
            continue;
        };
        let result = engine.evaluate_cancellable(
            wb,
            (job.sheet, job.row, job.col),
            job.formula.as_deref(),
            &keep,
            &job.cancel,
        );
        let _ = job.reply.send(result);
    }
}
