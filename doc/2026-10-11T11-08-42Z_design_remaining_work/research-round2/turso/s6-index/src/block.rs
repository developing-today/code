//! Runs one turso future to completion on the calling thread. turso's API is async and
//! the redb and rusqlite stores are sync, so this adapter lets all three sit behind the
//! same `Store` trait. Polls that return Pending are counted, so the report shows how
//! often the adapter had to wait for turso.

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};

/// Pending polls across every `block_on` call in this process.
pub static PENDING_POLLS: AtomicU64 = AtomicU64::new(0);

struct Unpark(Thread);

impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

/// Polls `fut` on this thread until it is ready. A Pending poll yields the thread and
/// polls again: turso's IO completes inside `poll`, so a waker is not always needed.
pub fn block_on<F: Future>(fut: F) -> F::Output {
    let waker = Waker::from(Arc::new(Unpark(thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut fut = pin!(fut);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(out) => return out,
            Poll::Pending => {
                PENDING_POLLS.fetch_add(1, Ordering::Relaxed);
                thread::yield_now();
            }
        }
    }
}
