use std::{io, os::unix::io::RawFd};

#[cfg(target_os = "linux")]
mod epoll;
#[cfg(target_os = "linux")]
use epoll::Epoll as Poller;

#[cfg(any(
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
))]
mod kqueue;
#[cfg(any(
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
))]
use kqueue::Kqueue as Poller;

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
pub struct TokenFactory {
    id: u32,
    sub_id: u32,
}

impl TokenFactory {
    pub(crate) fn new(id: u32) -> TokenFactory {
        TokenFactory { id, sub_id: 0 }
    }

    /// Produce a new unique token
    pub fn token(&mut self) -> Token {
        let token = Token {
            id: self.id,
            sub_id: self.sub_id,
        };
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
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Token {
    pub(crate) id: u32,
    pub(crate) sub_id: u32,
}

#[allow(dead_code)]
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

    pub(crate) fn to_u32(self) -> u32 {
        (self.id << 16) + self.sub_id
    }

    pub(crate) fn from_u32(i: u32) -> Token {
        Token {
            id: (i >> 16),
            sub_id: (i & std::u16::MAX as u32),
        }
    }

    /// Produces an invalid Token, that is not recognized by the
    /// event loop.
    ///
    /// Can be used as a placeholder to avoid `Option<Token>`
    pub fn invalid() -> Token {
        Token {
            id: std::u32::MAX,
            sub_id: std::u32::MAX,
        }
    }

    /// Check if this token is the invalid token
    pub fn is_invalid(self) -> bool {
        self.id == std::u32::MAX && self.sub_id == std::u32::MAX
    }

    /// Provide a copy of the Token with the given `sub_id`.
    pub fn with_sub_id(self, sub_id: u32) -> Token {
        Self { sub_id, ..self }
    }

    #[cfg(target_pointer_width = "64")]
    pub(crate) fn to_usize(self) -> usize {
        self.to_u64() as usize
    }

    #[cfg(target_pointer_width = "32")]
    pub(crate) fn to_usize(self) -> usize {
        self.to_u32() as usize
    }

    #[cfg(target_pointer_width = "64")]
    pub(crate) fn from_usize(i: usize) -> Token {
        Self::from_u64(i as u64)
    }

    #[cfg(target_pointer_width = "32")]
    pub(crate) fn from_usize(i: usize) -> Token {
        Self::from_u64(i as u64)
    }
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
        if token.is_invalid() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Invalid Token provided to register().",
            ));
        }
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
        if token.is_invalid() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Invalid Token provided to reregister().",
            ));
        }
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
    fn token_convert_u64() {
        let t = Token {
            id: 0x1337,
            sub_id: 0x42,
        };
        assert_eq!(t.to_u64(), 0x0000133700000042);
        let t2 = Token::from_u64(0x0000133700000042);
        assert_eq!(t, t2);
    }

    #[test]
    fn token_convert_u32() {
        let t = Token {
            id: 0x1337,
            sub_id: 0x42,
        };
        assert_eq!(t.to_u32(), 0x13370042);
        let t2 = Token::from_u32(0x13370042);
        assert_eq!(t, t2);
    }

    #[test]
    fn token_with_sub_id() {
        let t1 = Token {
            id: 0x1234,
            sub_id: 0,
        };
        let t2 = t1.with_sub_id(0xABCD);
        assert_eq!(
            t1,
            Token {
                id: 0x1234,
                sub_id: 0
            }
        );
        assert_eq!(
            t2,
            Token {
                id: 0x1234,
                sub_id: 0xABCD
            }
        );
    }
}
