//! Ping to the event loop
//!
//! This is an event source that just produces `()` events whevener the associated
//! [`Ping::ping`](Ping#method.ping) method is called. If the event source is pinged multiple times
//! between a single dispatching, it'll only generate one event.
//!
//! This event loop is a simple way of waking up the event loop from an other part of your program
//! (and is what backs the [`LoopSignal`](crate::LoopSignal)). It can also be used as a building
//! block to construct event sources whose source of event is not file descriptor, but rather an
//! userspace source (like an other thread).

use std::{os::unix::io::RawFd, sync::Arc};

use nix::{
    fcntl::OFlag,
    unistd::{close, pipe2, read, write},
};

use super::generic::{Fd, Generic};
use crate::{
    no_nix_err, EventSource, Interest, Mode, Poll, PostAction, Readiness, Token, TokenFactory,
};

/// Create a new ping event source
///
/// you are given a [`Ping`] instance, which can be cloned and used to ping the
/// event loop, and a [`PingSource`], which you can insert in your event loop to
/// receive the pings.
pub fn make_ping() -> std::io::Result<(Ping, PingSource)> {
    let (read, write) = pipe2(OFlag::O_CLOEXEC | OFlag::O_NONBLOCK).map_err(no_nix_err)?;
    let source = PingSource {
        pipe: Generic::from_fd(read, Interest::READ, Mode::Level),
    };
    let ping = Ping {
        pipe: Arc::new(CloseOnDrop(write)),
    };
    Ok((ping, source))
}

/// The ping event source
///
/// You can insert it in your event loop to receive pings.
///
/// If you use it directly, it will automatically remove itself from the event loop
/// once all [`Ping`] instances are dropped.
#[derive(Debug)]
pub struct PingSource {
    pipe: Generic<Fd>,
}

impl EventSource for PingSource {
    type Event = ();
    type Metadata = ();
    type Ret = ();

    fn process_events<C>(
        &mut self,
        readiness: Readiness,
        token: Token,
        mut callback: C,
    ) -> std::io::Result<PostAction>
    where
        C: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
    {
        self.pipe.process_events(readiness, token, |_, fd| {
            let mut buf = [0u8; 32];
            let mut read_something = false;
            let mut action = PostAction::Continue;
            loop {
                match read(fd.0, &mut buf) {
                    Ok(0) => {
                        // The other end of the pipe was closed, mark ourselved to for removal
                        action = PostAction::Remove;
                        break;
                    }
                    Ok(_) => read_something = true,
                    Err(e) => {
                        let e = no_nix_err(e);
                        if e.kind() == std::io::ErrorKind::WouldBlock {
                            break;
                        // nothing more to read
                        } else {
                            // propagate error
                            return Err(e);
                        }
                    }
                }
            }
            if read_something {
                callback((), &mut ());
            }
            Ok(action)
        })
    }

    fn register(
        &mut self,
        poll: &mut Poll,
        token_factory: &mut TokenFactory,
    ) -> std::io::Result<()> {
        self.pipe.register(poll, token_factory)
    }

    fn reregister(
        &mut self,
        poll: &mut Poll,
        token_factory: &mut TokenFactory,
    ) -> std::io::Result<()> {
        self.pipe.reregister(poll, token_factory)
    }

    fn unregister(&mut self, poll: &mut Poll) -> std::io::Result<()> {
        self.pipe.unregister(poll)
    }
}

impl Drop for PingSource {
    fn drop(&mut self) {
        if let Err(e) = close(self.pipe.file.0) {
            log::warn!("[calloop] Failed to close read ping: {:?}", e);
        }
    }
}

/// The Ping handle
///
/// This handle can be cloned and sent accross threads. It can be used to
/// send pings to the `PingSource`.
#[derive(Clone, Debug)]
pub struct Ping {
    pipe: Arc<CloseOnDrop>,
}

impl Ping {
    /// Send a ping to the `PingSource`
    pub fn ping(&self) {
        if let Err(e) = write(self.pipe.0, &[0u8]) {
            log::warn!("[calloop] Failed to write a ping: {:?}", e);
        }
    }
}

#[derive(Debug)]
struct CloseOnDrop(RawFd);

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        if let Err(e) = close(self.0) {
            log::warn!("[calloop] Failed to close write ping: {:?}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping() {
        let mut event_loop = crate::EventLoop::<bool>::try_new().unwrap();

        let (ping, source) = make_ping().unwrap();

        event_loop
            .handle()
            .insert_source(source, |(), &mut (), dispatched| *dispatched = true)
            .unwrap();

        ping.ping();

        let mut dispatched = false;
        event_loop
            .dispatch(std::time::Duration::from_millis(0), &mut dispatched)
            .unwrap();
        assert!(dispatched);

        // Ping has been drained an no longer generates events
        let mut dispatched = false;
        event_loop
            .dispatch(std::time::Duration::from_millis(0), &mut dispatched)
            .unwrap();
        assert!(!dispatched);
    }

    #[test]
    fn ping_closed() {
        let mut event_loop = crate::EventLoop::<bool>::try_new().unwrap();

        let (_, source) = make_ping().unwrap();
        event_loop
            .handle()
            .insert_source(source, |(), &mut (), dispatched| *dispatched = true)
            .unwrap();

        let mut dispatched = false;

        // If the sender is closed from the start, the ping should first trigger
        // once, disabling itself but not invoking the callback
        event_loop
            .dispatch(std::time::Duration::from_millis(0), &mut dispatched)
            .unwrap();
        assert!(!dispatched);

        // Then it should not trigger any more, so this dispatch should wait the whole 100ms
        let now = std::time::Instant::now();
        event_loop
            .dispatch(std::time::Duration::from_millis(100), &mut dispatched)
            .unwrap();
        assert!(now.elapsed() >= std::time::Duration::from_millis(100));
    }
}
