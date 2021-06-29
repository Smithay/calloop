//! A generic event source wrapping an IO objects or file descriptor
//!
//! You can use this general purpose adapter around file-descriptor backed objects to
//! insert into an [`EventLoop`](crate::EventLoop).
//!
//! The event generated by this [`Generic`] event source are the [`Readiness`](crate::Readiness)
//! notification itself, and the monitored object is provided to your callback as the second
//! argument.
//!
//! ```
//! # extern crate calloop;
//! use calloop::{generic::Generic, Interest, Mode, PostAction};
//!
//! # fn main() {
//! # let mut event_loop = calloop::EventLoop::<()>::try_new()
//! #                .expect("Failed to initialize the event loop!");
//! # let handle = event_loop.handle();
//! # let io_object = calloop::generic::Fd(0);
//! handle.insert_source(
//!     // wrap your IO object in a Generic, here we register for read readiness
//!     // in level-triggering mode
//!     Generic::new(io_object, Interest::READ, Mode::Level),
//!     |readiness, io_object, shared_data| {
//!         // The first argument of the callback is a Readiness
//!         // The second is a &mut reference to your object
//!
//!         // your callback needs to return a Result<PostAction, std::io::Error>
//!         // if it returns an error, the event loop will consider this event
//!         // event source as erroring and report it to the user.
//!         Ok(PostAction::Continue)
//!     }
//! );
//! # }
//! ```
//!
//! It can also help you implementing your own event sources: just have
//! these `Generic<_>` as fields of your event source, and delegate the
//! [`EventSource`](crate::EventSource) implementation to them.
//!
//! If you need to directly work with a [`RawFd`](std::os::unix::io::RawFd), rather than an
//! FD-backed object, see [`Generic::from_fd`](Generic#method.from_fd).

use std::io;
#[cfg(unix)]
use std::os::unix::io::{AsRawFd, RawFd};

use crate::{EventSource, Interest, Mode, Poll, PostAction, Readiness, Token, TokenFactory};

/// A generic event source wrapping a FD-backed type
#[derive(Debug)]
pub struct Generic<F: AsRawFd> {
    /// The wrapped FD-backed type
    pub file: F,
    /// The programmed interest
    pub interest: Interest,
    /// The programmed mode
    pub mode: Mode,
    token: Token,
}

/// A wrapper to insert a raw file descriptor into a `Generic` event source
#[derive(Debug)]
pub struct Fd(pub RawFd);

impl AsRawFd for Fd {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

impl<F: AsRawFd> Generic<F> {
    /// Wrap a FD-backed type into a `Generic` event source
    pub fn new(file: F, interest: Interest, mode: Mode) -> Generic<F> {
        Generic {
            file,
            interest,
            mode,
            token: Token::invalid(),
        }
    }

    /// Unwrap the `Generic` source to retrieve the underlying type
    pub fn unwrap(self) -> F {
        self.file
    }
}

impl Generic<Fd> {
    /// Wrap a raw file descriptor into a `Generic` event source
    pub fn from_fd(fd: RawFd, interest: Interest, mode: Mode) -> Generic<Fd> {
        Self::new(Fd(fd), interest, mode)
    }
}

impl<F: AsRawFd> EventSource for Generic<F> {
    type Event = Readiness;
    type Metadata = F;
    type Ret = io::Result<PostAction>;

    fn process_events<C>(
        &mut self,
        readiness: Readiness,
        token: Token,
        mut callback: C,
    ) -> std::io::Result<PostAction>
    where
        C: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
    {
        if token != self.token {
            return Ok(PostAction::Continue);
        }
        callback(readiness, &mut self.file)
    }

    fn register(
        &mut self,
        poll: &mut Poll,
        token_factory: &mut TokenFactory,
    ) -> std::io::Result<()> {
        let token = token_factory.token();
        poll.register(self.file.as_raw_fd(), self.interest, self.mode, token)?;
        self.token = token;
        Ok(())
    }

    fn reregister(
        &mut self,
        poll: &mut Poll,
        token_factory: &mut TokenFactory,
    ) -> std::io::Result<()> {
        let token = token_factory.token();
        poll.reregister(self.file.as_raw_fd(), self.interest, self.mode, token)?;
        self.token = token;
        Ok(())
    }

    fn unregister(&mut self, poll: &mut Poll) -> std::io::Result<()> {
        poll.unregister(self.file.as_raw_fd())?;
        self.token = Token::invalid();
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::io::{self, Read, Write};

    use super::Generic;
    use crate::{Dispatcher, Interest, Mode, PostAction};
    #[cfg(unix)]
    #[test]
    fn dispatch_unix() {
        use std::os::unix::net::UnixStream;

        let mut event_loop = crate::EventLoop::try_new().unwrap();

        let handle = event_loop.handle();

        let (mut tx, rx) = UnixStream::pair().unwrap();

        let generic = Generic::new(rx, Interest::READ, Mode::Level);

        let mut dispached = false;

        let _generic_token = handle
            .insert_source(generic, move |readiness, file, d| {
                assert!(readiness.readable);
                // we have not registered for writability
                assert!(!readiness.writable);
                let mut buffer = vec![0; 10];
                let ret = file.read(&mut buffer).unwrap();
                assert_eq!(ret, 6);
                assert_eq!(&buffer[..6], &[1, 2, 3, 4, 5, 6]);

                *d = true;
                Ok(PostAction::Continue)
            })
            .map_err(Into::<io::Error>::into)
            .unwrap();

        event_loop
            .dispatch(Some(::std::time::Duration::from_millis(0)), &mut dispached)
            .unwrap();

        assert!(!dispached);

        tx.write(&[1, 2, 3, 4, 5, 6]).unwrap();
        tx.flush().unwrap();

        event_loop
            .dispatch(Some(::std::time::Duration::from_millis(0)), &mut dispached)
            .unwrap();

        assert!(dispached);
    }

    #[test]
    fn register_deregister_unix() {
        use std::os::unix::net::UnixStream;

        let mut event_loop = crate::EventLoop::try_new().unwrap();

        let handle = event_loop.handle();

        let (mut tx, rx) = UnixStream::pair().unwrap();

        let generic = Generic::new(rx, Interest::READ, Mode::Level);
        let dispatcher = Dispatcher::new(generic, move |_, _, d| {
            *d = true;
            Ok(PostAction::Continue)
        });

        let mut dispached = false;

        let generic_token = handle.register_dispatcher(dispatcher.clone()).unwrap();

        event_loop
            .dispatch(Some(::std::time::Duration::from_millis(0)), &mut dispached)
            .unwrap();

        assert!(!dispached);

        // remove the source, and then write something

        event_loop.handle().remove(generic_token);

        tx.write(&[1, 2, 3, 4, 5, 6]).unwrap();
        tx.flush().unwrap();

        event_loop
            .dispatch(Some(::std::time::Duration::from_millis(0)), &mut dispached)
            .unwrap();

        // the source has not been dispatched, as the source is no longer here
        assert!(!dispached);

        // insert it again
        let generic = dispatcher.into_source_inner();
        let _generic_token = handle
            .insert_source(generic, move |readiness, file, d| {
                assert!(readiness.readable);
                // we have not registered for writability
                assert!(!readiness.writable);
                let mut buffer = vec![0; 10];
                let ret = file.read(&mut buffer).unwrap();
                assert_eq!(ret, 6);
                assert_eq!(&buffer[..6], &[1, 2, 3, 4, 5, 6]);

                *d = true;
                Ok(PostAction::Continue)
            })
            .map_err(Into::<io::Error>::into)
            .unwrap();

        event_loop
            .dispatch(Some(::std::time::Duration::from_millis(0)), &mut dispached)
            .unwrap();

        // the has now been properly dispatched
        assert!(dispached);
    }
}
