//! Cooperative cancellation of a layout run.
//!
//! Layout is a single-threaded function of its inputs, so the flag lives in a thread-local that
//! [`crate::layout_document_cancellable`] installs for the duration of one call. The long loops
//! (laying out blocks, placing items on pages) ask [`cancelled`] once per item. Outside such a
//! call the answer is always `false`.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

thread_local! {
    static CURRENT: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
}

/// True when the layout running on this thread has been asked to stop.
pub(crate) fn cancelled() -> bool {
    CURRENT.with(|c| {
        c.borrow()
            .as_ref()
            .is_some_and(|f| f.load(Ordering::Relaxed))
    })
}

/// Restores the previous flag on drop, so nested or panicking runs leave no stale token.
#[derive(Debug)]
pub(crate) struct Scope(Option<Arc<AtomicBool>>);

impl Scope {
    pub(crate) fn enter(flag: &Arc<AtomicBool>) -> Self {
        Self(CURRENT.with(|c| c.replace(Some(Arc::clone(flag)))))
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        let prev = self.0.take();
        CURRENT.with(|c| *c.borrow_mut() = prev);
    }
}
