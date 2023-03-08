use std::{cell::RefCell, rc::Rc, sync::Arc, time::Duration};

#[cfg(unix)]
use std::os::unix::io::AsRawFd;

#[cfg(windows)]
use std::os::windows::io::AsRawSocket;

#[cfg(unix)]
use io_lifetimes::AsFd;

#[cfg(windows)]
use io_lifetimes::AsSocket;

use polling::{Event, PollMode, Poller};

use crate::loop_logic::{MAX_SOURCES, MAX_SUBSOURCES_TOTAL};
use crate::sources::timer::TimerWheel;

/// Possible modes for registering a file descriptor
#[derive(Copy, Clone, Debug)]
pub enum Mode {
    /// Single event generation
    ///
    /// This FD will be disabled as soon as it has generated one event.
    ///
    /// The user will need to use `LoopHandle::update()` to re-enable it if
    /// desired.
    OneShot,

    /// Level-triggering
    ///
    /// This FD will report events on every poll as long as the requested interests
    /// are available. If the same FD is inserted in multiple event loops, all of
    /// them are notified of readiness.
    Level,

    /// Edge-triggering
    ///
    /// This FD will report events only when it *gains* one of the requested interests.
    /// it must thus be fully processed before it'll generate events again. If the same
    /// FD is inserted on multiple event loops, it may be that not all of them are notified
    /// of readiness, and not necessarily always the same(s) (at least one is notified).
    Edge,
}

/// Interest to register regarding the file descriptor
#[derive(Copy, Clone, Debug)]
pub struct Interest {
    /// Wait for the FD to be readable
    pub readable: bool,

    /// Wait for the FD to be writable
    pub writable: bool,
}

impl Interest {
    /// Shorthand for empty interest
    pub const EMPTY: Interest = Interest {
        readable: false,
        writable: false,
    };

    /// Shorthand for read interest
    pub const READ: Interest = Interest {
        readable: true,
        writable: false,
    };

    /// Shorthand for write interest
    pub const WRITE: Interest = Interest {
        readable: false,
        writable: true,
    };

    /// Shorthand for read and write interest
    pub const BOTH: Interest = Interest {
        readable: true,
        writable: true,
    };
}

/// Readiness for a file descriptor notification
#[derive(Copy, Clone, Debug)]
pub struct Readiness {
    /// Is the FD readable
    pub readable: bool,

    /// Is the FD writable
    pub writable: bool,

    /// Is the FD in an error state
    pub error: bool,
}

impl Readiness {
    /// Shorthand for empty readiness
    pub const EMPTY: Readiness = Readiness {
        readable: false,
        writable: false,
        error: false,
    };
}

#[derive(Debug)]
pub(crate) struct PollEvent {
    pub(crate) readiness: Readiness,
    pub(crate) token: Token,
}

/// Factory for creating tokens in your registrations
///
/// When composing event sources, each sub-source needs to
/// have its own token to identify itself. This factory is
/// provided to produce such unique tokens.

#[derive(Debug)]
pub struct TokenFactory {
    /// The key of the source this factory is associated with.
    key: usize,

    /// The next sub-id to use.
    sub_id: u32,
}

impl TokenFactory {
    pub(crate) fn new(key: usize) -> TokenFactory {
        TokenFactory { key, sub_id: 0 }
    }

    /// Produce a new unique token
    pub fn token(&mut self) -> Token {
        // Ensure we don't overflow the sub-id.
        if self.sub_id >= MAX_SUBSOURCES_TOTAL as _ {
            panic!("Too many sub-sources for this source");
        }

        // Compose the key and the sub-key together.
        let mut key = self.key;
        key |= (self.sub_id as usize) << MAX_SOURCES;

        let token = Token { key };

        self.sub_id += 1;

        token
    }
}

/// A token (for implementation of the [`EventSource`](crate::EventSource) trait)
///
/// This token is produced by the [`TokenFactory`] and is used when calling the
/// [`EventSource`](crate::EventSource) implementations to process event, in order
/// to identify which sub-source produced them.
///
/// You should forward it to the [`Poll`] when registering your file descriptors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub(crate) key: usize,
}

/// The polling system
///
/// This type represents the polling system of calloop, on which you
/// can register your file descriptors. This interface is only accessible in
/// implementations of the [`EventSource`](crate::EventSource) trait.
///
/// You only need to interact with this type if you are implementing your
/// own event sources, while implementing the [`EventSource`](crate::EventSource) trait.
/// And even in this case, you can often just use the [`Generic`](crate::generic::Generic) event
/// source and delegate the implementations to it.
pub struct Poll {
    /// The handle to wepoll/epoll/kqueue/... used to poll for events.
    poller: Arc<Poller>,

    /// The buffer of events returned by the poller.
    events: RefCell<Vec<Event>>,

    pub(crate) timers: Rc<RefCell<TimerWheel>>,
}

impl std::fmt::Debug for Poll {
    #[cfg_attr(coverage, no_coverage)]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Poll { ... }")
    }
}

impl Poll {
    pub(crate) fn new(_high_precision: bool) -> crate::Result<Poll> {
        Ok(Poll {
            poller: Arc::new(Poller::new()?),
            events: RefCell::new(Vec::new()),
            timers: Rc::new(RefCell::new(TimerWheel::new())),
        })
    }

    pub(crate) fn poll(&self, mut timeout: Option<Duration>) -> crate::Result<Vec<PollEvent>> {
        let now = std::time::Instant::now();

        // Adjust the timeout for the timers.
        if let Some(next_timeout) = self.timers.borrow().next_deadline() {
            if next_timeout <= now {
                timeout = Some(Duration::ZERO);
            } else if let Some(deadline) = timeout {
                timeout = Some(std::cmp::min(deadline, next_timeout - now));
            } else {
                timeout = Some(next_timeout - now);
            }
        };

        let mut events = self.events.borrow_mut();
        self.poller.wait(&mut events, timeout)?;

        // Convert `polling` events to `calloop` events.
        let mut poll_events = events
            .drain(..)
            .map(|ev| PollEvent {
                readiness: Readiness {
                    readable: ev.readable,
                    writable: ev.writable,
                    error: false,
                },
                token: Token { key: ev.key },
            })
            .collect::<Vec<_>>();

        drop(events);

        // Update 'now' as some time may have elapsed in poll()
        let now = std::time::Instant::now();
        let mut timers = self.timers.borrow_mut();
        while let Some((_, token)) = timers.next_expired(now) {
            poll_events.push(PollEvent {
                readiness: Readiness {
                    readable: true,
                    writable: false,
                    error: false,
                },
                token,
            });
        }

        Ok(poll_events)
    }

    /// Register a new file descriptor for polling
    ///
    /// The file descriptor will be registered with given interest,
    /// mode and token. This function will fail if given a
    /// bad file descriptor or if the provided file descriptor is already
    /// registered.
    ///
    /// # Leaking tokens
    ///
    /// If your event source is dropped without being unregistered, the token
    /// passed in here will remain on the heap and continue to be used by the
    /// polling system even though no event source will match it.
    pub fn register(
        &self,
        #[cfg(unix)] fd: impl AsFd,
        #[cfg(windows)] fd: impl AsSocket,
        interest: Interest,
        mode: Mode,
        token: Token,
    ) -> crate::Result<()> {
        let raw = {
            #[cfg(unix)]
            {
                fd.as_fd().as_raw_fd()
            }

            #[cfg(windows)]
            {
                fd.as_socket().as_raw_socket()
            }
        };

        self.poller
            .add_with_mode(raw, cvt_interest(interest, token), cvt_mode(mode))?;

        Ok(())
    }

    /// Update the registration for a file descriptor
    ///
    /// This allows you to change the interest, mode or token of a file
    /// descriptor. Fails if the provided fd is not currently registered.
    ///
    /// See note on [`register()`](Self::register()) regarding leaking.
    pub fn reregister(
        &self,
        #[cfg(unix)] fd: impl AsFd,
        #[cfg(windows)] fd: impl AsSocket,
        interest: Interest,
        mode: Mode,
        token: Token,
    ) -> crate::Result<()> {
        let raw = {
            #[cfg(unix)]
            {
                fd.as_fd().as_raw_fd()
            }

            #[cfg(windows)]
            {
                fd.as_socket().as_raw_socket()
            }
        };

        self.poller
            .modify_with_mode(raw, cvt_interest(interest, token), cvt_mode(mode))?;

        Ok(())
    }

    /// Unregister a file descriptor
    ///
    /// This file descriptor will no longer generate events. Fails if the
    /// provided file descriptor is not currently registered.
    pub fn unregister(
        &self,
        #[cfg(unix)] fd: impl AsFd,
        #[cfg(windows)] fd: impl AsSocket,
    ) -> crate::Result<()> {
        let raw = {
            #[cfg(unix)]
            {
                fd.as_fd().as_raw_fd()
            }

            #[cfg(windows)]
            {
                fd.as_socket().as_raw_socket()
            }
        };
        self.poller.delete(raw)?;

        Ok(())
    }

    /// Get a thread-safe handle which can be used to wake up the `Poll`.
    pub(crate) fn notifier(&self) -> Notifier {
        Notifier(self.poller.clone())
    }
}

/// Thread-safe handle which can be used to wake up the `Poll`.
#[derive(Clone)]
pub(crate) struct Notifier(Arc<Poller>);

impl Notifier {
    pub(crate) fn notify(&self) -> crate::Result<()> {
        self.0.notify()?;

        Ok(())
    }
}

fn cvt_interest(interest: Interest, tok: Token) -> Event {
    Event {
        readable: interest.readable,
        writable: interest.writable,
        key: tok.key,
    }
}

fn cvt_mode(mode: Mode) -> PollMode {
    match mode {
        Mode::Edge => PollMode::Edge,
        Mode::Level => PollMode::Level,
        Mode::OneShot => PollMode::Oneshot,
    }
}
