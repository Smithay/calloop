//! Socket event source.
//!
//! Only available on Linux.
//!
//! This module provides a Calloop event source for Unix domain sockets.
//!
//! ```
//! # extern crate calloop;
//! use std::os::unix::net::UnixListener;
//!
//! use calloop::{Interest, Mode, PostAction};
//! use calloop::socket::SocketSource;
//!
//! # fn main() {
//! # let mut event_loop = calloop::EventLoop::<()>::try_new()
//! #                .expect("Failed to initialize the event loop!");
//! # let handle = event_loop.handle();
//! let socket = UnixListener::bind("/tmp/calloop.sock")
//!     .expect("Failed to create socket");
//! let source = SocketSource::new(socket)
//!     .expect("Failed to create calloop socket source");
//! handle.insert_source(
//!     source,
//!     |stream, _, shared_data| {
//!         // The first argument of the callback is a new socket stream.
//!     }
//! );
//! # }
//! ```

use std::io::{self, ErrorKind};
use std::os::unix::net::{UnixListener, UnixStream};

use crate::generic::Generic;
use crate::{EventSource, Interest, Mode, Poll, PostAction, Readiness, Token, TokenFactory};

/// Unix domain socket source.
#[derive(Debug)]
pub struct SocketSource {
    socket: Generic<UnixListener>,
}

impl SocketSource {
    /// Create a new socket event source.
    ///
    /// This will always call [`UnixListener::set_nonblocking`] on the socket automatically, to
    /// prevent it from blocking up the calloop event loop.
    pub fn new(socket: UnixListener) -> crate::Result<Self> {
        // Ensure we'll get `WouldBlock` when reading from an empty socket.
        socket.set_nonblocking(true)?;

        Ok(Self {
            socket: Generic::new(socket, Interest::READ, Mode::Level),
        })
    }
}

impl EventSource for SocketSource {
    type Event = UnixStream;
    type Error = io::Error;
    type Metadata = ();
    type Ret = ();

    fn process_events<F>(
        &mut self,
        readiness: Readiness,
        token: Token,
        mut callback: F,
    ) -> io::Result<PostAction>
    where
        F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
    {
        self.socket.process_events(readiness, token, |_, socket| {
            // Accept next connection, separating `WouldBlock` from other errors.
            let accept_next = || match socket.accept() {
                Ok((stream, _)) => Ok(Some(stream)),
                Err(err) if err.kind() == ErrorKind::WouldBlock => Ok(None),
                Err(err) => Err(err),
            };

            // Read from the socket until it would block.
            while let Some(stream) = accept_next()? {
                callback(stream, &mut ());
            }

            Ok(PostAction::Continue)
        })
    }

    fn register(&mut self, poll: &mut Poll, token_factory: &mut TokenFactory) -> crate::Result<()> {
        self.socket.register(poll, token_factory)
    }

    fn reregister(
        &mut self,
        poll: &mut Poll,
        token_factory: &mut TokenFactory,
    ) -> crate::Result<()> {
        self.socket.reregister(poll, token_factory)
    }

    fn unregister(&mut self, poll: &mut Poll) -> crate::Result<()> {
        self.socket.unregister(poll)
    }
}
