//! Waiting for the socket, a deadline or another thread, whichever comes first.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use polling::{Events, Poller};

/// Wakes [`Client::poll`](crate::Client::poll) from another thread, such as one with something to send.
#[derive(Clone)]
pub struct Waker(pub(crate) Arc<Signal>);

impl Waker {
    pub fn wake(&self) {
        self.0.woken.store(true, Ordering::Release);
        if let Err(error) = self.0.poller.notify() {
            log::warn!("Could not wake the Socket.IO client: {error}");
        }
    }
}

/// The client's poller, which the connection's socket is registered with, and whether a [`Waker`] fired.
pub(crate) struct Signal {
    pub poller: Poller,
    woken: AtomicBool,
}

impl Signal {
    pub fn new() -> std::io::Result<Arc<Signal>> {
        Ok(Arc::new(Signal {
            poller: Poller::new()?,
            woken: AtomicBool::new(false),
        }))
    }

    /// Blocks until a registered socket is ready, a [`Waker`] fires, or `until`.
    pub fn wait(&self, until: Option<Instant>) {
        let timeout = until.map(|until| until.saturating_duration_since(Instant::now()));
        if timeout == Some(Duration::ZERO) {
            return;
        }
        let mut events = Events::with_capacity(NonZeroUsize::MIN);
        if let Err(error) = self.poller.wait(&mut events, timeout) {
            log::warn!("Waiting on the socket failed: {error}");
        }
    }

    /// Whether a [`Waker`] fired since the last call.
    pub fn take_woken(&self) -> bool {
        self.woken.swap(false, Ordering::Acquire)
    }
}
