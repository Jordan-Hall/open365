//! Paragraph-level result cache and a background worker, for the UI's squiggle pipeline.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{Builder, JoinHandle};
use std::time::Duration;

use crate::proof::{LangSpan, ParagraphReport, Proofing};

struct Entry {
    text: String,
    spans: Vec<LangSpan>,
    generation: u64,
    report: Arc<ParagraphReport>,
}

/// Remembers the last result for each paragraph, so unchanged paragraphs cost a string compare.
///
/// Edited paragraphs are re-tokenised, but every word's dictionary verdict is memoised in
/// [`Proofing`], so only words that are new since the last check reach the dictionary. Results are
/// shared: when nothing changed, `check` returns the same [`Arc`] as before, which callers can
/// compare with [`Arc::ptr_eq`] to skip repainting.
pub struct ProofSession {
    proofing: Proofing,
    paragraphs: HashMap<u64, Entry>,
}

impl fmt::Debug for ProofSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProofSession")
            .field("paragraphs", &self.paragraphs.len())
            .finish_non_exhaustive()
    }
}

impl ProofSession {
    /// A session over `proofing`.
    #[must_use]
    pub fn new(proofing: Proofing) -> Self {
        Self {
            proofing,
            paragraphs: HashMap::new(),
        }
    }

    /// The service this session checks with.
    #[must_use]
    pub fn proofing(&self) -> &Proofing {
        &self.proofing
    }

    /// Checks paragraph `id` (any caller-chosen number) written in one language.
    pub fn check(&mut self, id: u64, text: &str, lang: &str) -> Arc<ParagraphReport> {
        self.check_spans(id, text, &[LangSpan::new(0..text.len(), lang)])
    }

    /// Checks paragraph `id` whose runs have different languages.
    pub fn check_spans(&mut self, id: u64, text: &str, spans: &[LangSpan]) -> Arc<ParagraphReport> {
        let generation = self.proofing.generation();
        if let Some(entry) = self.paragraphs.get(&id)
            && entry.generation == generation
            && entry.text == text
            && entry.spans == spans
        {
            return Arc::clone(&entry.report);
        }
        let report = self.proofing.check_spans(text, spans);
        // Keep the old allocation when the new result is identical, so the UI sees no change.
        let report = match self.paragraphs.get(&id) {
            Some(entry) if *entry.report == report => Arc::clone(&entry.report),
            _ => Arc::new(report),
        };
        self.paragraphs.insert(
            id,
            Entry {
                text: text.to_owned(),
                spans: spans.to_vec(),
                generation,
                report: Arc::clone(&report),
            },
        );
        report
    }

    /// Drops the cached result for a deleted paragraph.
    pub fn forget(&mut self, id: u64) {
        self.paragraphs.remove(&id);
    }

    /// Drops every cached result.
    pub fn clear(&mut self) {
        self.paragraphs.clear();
    }

    /// Number of paragraphs held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.paragraphs.len()
    }

    /// True when no paragraph is cached.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.paragraphs.is_empty()
    }
}

/// A paragraph to check on the worker thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofRequest {
    /// Caller-chosen paragraph identity.
    pub id: u64,
    /// Caller-chosen version of the paragraph; echoed back so stale answers can be dropped.
    pub revision: u64,
    /// The paragraph text.
    pub text: String,
    /// Language runs.
    pub spans: Vec<LangSpan>,
}

/// The answer to a [`ProofRequest`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofResponse {
    /// The paragraph that was checked.
    pub id: u64,
    /// The revision that was checked; compare with the paragraph's current revision.
    pub revision: u64,
    /// The result.
    pub report: Arc<ParagraphReport>,
}

enum Command {
    Check(ProofRequest),
    Forget(u64),
    Shutdown,
}

/// A dedicated thread that checks paragraphs off the UI thread.
///
/// Requests for the same paragraph that are still queued are coalesced: only the newest is
/// checked. Dropping the worker stops the thread after the paragraph in progress.
pub struct ProofWorker {
    commands: Sender<Command>,
    responses: Receiver<ProofResponse>,
    handle: Option<JoinHandle<()>>,
}

impl fmt::Debug for ProofWorker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProofWorker").finish_non_exhaustive()
    }
}

impl ProofWorker {
    /// Starts the worker thread.
    ///
    /// # Errors
    /// Returns the OS error when the thread cannot be created.
    pub fn spawn(proofing: Proofing) -> std::io::Result<Self> {
        Self::spawn_with_notify(proofing, || {})
    }

    /// Starts the worker thread; `notify` runs on the worker after each response is queued, for
    /// example to request a repaint.
    ///
    /// # Errors
    /// Returns the OS error when the thread cannot be created.
    pub fn spawn_with_notify(
        proofing: Proofing,
        notify: impl Fn() + Send + 'static,
    ) -> std::io::Result<Self> {
        let (commands, command_rx) = mpsc::channel();
        let (response_tx, responses) = mpsc::channel();
        let handle = Builder::new()
            .name("open365-spell".to_owned())
            .spawn(move || run(proofing, &command_rx, &response_tx, &notify))?;
        Ok(Self {
            commands,
            responses,
            handle: Some(handle),
        })
    }

    /// Queues a paragraph. Returns false if the worker has stopped.
    pub fn submit(&self, request: ProofRequest) -> bool {
        self.commands.send(Command::Check(request)).is_ok()
    }

    /// Drops a deleted paragraph's cache and any queued request for it.
    pub fn forget(&self, id: u64) {
        let _ = self.commands.send(Command::Forget(id));
    }

    /// The next finished response, if any.
    #[must_use]
    pub fn try_recv(&self) -> Option<ProofResponse> {
        self.responses.try_recv().ok()
    }

    /// Waits up to `timeout` for the next response.
    #[must_use]
    pub fn recv_timeout(&self, timeout: Duration) -> Option<ProofResponse> {
        self.responses.recv_timeout(timeout).ok()
    }

    /// Every response finished so far.
    #[must_use]
    pub fn drain(&self) -> Vec<ProofResponse> {
        self.responses.try_iter().collect()
    }
}

impl Drop for ProofWorker {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn run(
    proofing: Proofing,
    commands: &Receiver<Command>,
    responses: &Sender<ProofResponse>,
    notify: &dyn Fn(),
) {
    let mut session = ProofSession::new(proofing);
    while let Ok(first) = commands.recv() {
        // Gather everything already queued so superseded requests are skipped.
        let mut pending: Vec<ProofRequest> = Vec::new();
        let mut next = Some(first);
        while let Some(command) = next.take().or_else(|| commands.try_recv().ok()) {
            match command {
                Command::Shutdown => return,
                Command::Forget(id) => {
                    pending.retain(|r| r.id != id);
                    session.forget(id);
                }
                Command::Check(request) => {
                    pending.retain(|r| r.id != request.id);
                    pending.push(request);
                }
            }
        }
        for request in pending {
            let report = session.check_spans(request.id, &request.text, &request.spans);
            let response = ProofResponse {
                id: request.id,
                revision: request.revision,
                report,
            };
            if responses.send(response).is_err() {
                return;
            }
            notify();
        }
    }
}
