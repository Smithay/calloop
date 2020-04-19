use std::{io, os::unix::io::RawFd};

#[cfg(target_os = "linux")]
mod epoll;
#[cfg(target_os = "linux")]
use epoll::Epoll as Poller;

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
    /// it must thus be fully processed befor it'll generate events again. If the same
    /// FD is inserted on multiple event loops, it may be that not all of them are notified
    /// of readiness, and not necessarily always the same(s) (at least one is notified).
    Edge,
}

/// Interest to register regarding the file descriptor
#[derive(Copy, Clone, Debug)]
pub enum Interest {
    /// Will generate events when readable
    Readable,
    /// Will generate events when writable
    Writable,
    /// Will generate events when readable or writable
    Both,
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

pub(crate) struct PollEvent {
    pub(crate) readiness: Readiness,
    pub(crate) token: Token,
}

/// A Token for registration
///
/// This token is given to you by the event loop, and you should
/// forward it to the `Poll` when registering your file descriptors.
///
/// It also contains a publc field that you can change. In case your event
/// source needs to register more than one FD, you can register each with a
/// different value of the `sub_id` field, to differentiate them. You can then
/// know which of these FD si ready by reading the `sub_id` field of the
/// token you'll be given in the `process_events` method of your source.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Token {
    pub(crate) id: u32,
    /// The source-internal ID
    pub sub_id: u32,
}

impl Token {
    pub(crate) fn to_u64(self) -> u64 {
        ((self.id as u64) << 32) + self.sub_id as u64
    }

    pub(crate) fn from_u64(i: u64) -> Token {
        Token {
            id: (i >> 32) as u32,
            sub_id: (i & std::u32::MAX as u64) as u32,
        }
    }
}

/// The polling system
///
/// This type represents the polling system of calloop, on which you
/// can register your file descriptors. This interface is only accessible in
/// implementations of the `EventSource` trait.
///
/// You only need to interact with this type if you are implementing you
/// own event sources, while implementing the `EventSource` trait. And even in this case,
/// you can often just use the `Generic` event source and delegate the implementations to it.
pub struct Poll {
    poller: Poller,
}

impl Poll {
    pub(crate) fn new() -> io::Result<Poll> {
        Ok(Poll {
            poller: Poller::new()?,
        })
    }

    pub(crate) fn poll(
        &mut self,
        timeout: Option<std::time::Duration>,
    ) -> io::Result<Vec<PollEvent>> {
        self.poller.poll(timeout)
    }

    /// Register a new file descriptor for polling
    ///
    /// The file descriptor will be registered with given interest,
    /// mode and token. This function will fail if given a
    /// bad file descriptor or if the provided file descriptor is already
    /// registered.
    pub fn register(
        &mut self,
        fd: RawFd,
        interest: Interest,
        mode: Mode,
        token: Token,
    ) -> io::Result<()> {
        self.poller.register(fd, interest, mode, token)
    }

    /// Update the registration for a file descriptor
    ///
    /// This allows you to change the interest, mode or token of a file
    /// descriptor. Fails if the provided fd is not currently registered.
    pub fn reregister(
        &mut self,
        fd: RawFd,
        interest: Interest,
        mode: Mode,
        token: Token,
    ) -> io::Result<()> {
        self.poller.reregister(fd, interest, mode, token)
    }

    /// Unregister a file descriptor
    ///
    /// This file descriptor will no longer generate events. Fails if the
    /// provided file descriptor is not currently registered.
    pub fn unregister(&mut self, fd: RawFd) -> io::Result<()> {
        self.poller.unregister(fd)
    }
}

#[cfg(test)]
mod tests {
    use super::Token;
    #[test]
    fn token_convert() {
        let t = Token {
            id: 0x1337,
            sub_id: 0x42,
        };
        assert_eq!(t.to_u64(), 0x0000133700000042);
        let t2 = Token::from_u64(0x0000133700000042);
        assert_eq!(t, t2);
    }
}
