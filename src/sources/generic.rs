//! A generic event source wrapping a file descriptor
//!
//! You can use this general purpose adapter around file descriptor
//! to insert your own file descriptors into an event loop.
//!
//! It can also help you implementing your own event sources: just have
//! these `Generic<_>` as fields of your event source, and delegate the
//! `EventSource` implementation to them.

use std::io;
#[cfg(unix)]
use std::os::unix::io::{AsRawFd, RawFd};

use crate::{EventSource, Interest, Mode, Poll, Readiness, Token};

/// A generic event source wrapping a FD-backed type
pub struct Generic<F: AsRawFd> {
    /// The wrapped FD-backed type
    pub file: F,
    /// The programmed interest
    pub interest: Interest,
    /// The programmed mode
    pub mode: Mode,
}

/// A wrapper to insert a raw file descriptor into a `Generic` event source
pub struct Fd(pub RawFd);

impl AsRawFd for Fd {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

impl<F: AsRawFd + 'static> Generic<F> {
    /// Wrap a FD-backed type into a `Generic` event source
    pub fn new(file: F, interest: Interest, mode: Mode) -> Generic<F> {
        Generic {
            file,
            interest,
            mode,
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
    type Ret = io::Result<()>;

    fn process_events<C>(
        &mut self,
        readiness: Readiness,
        _: Token,
        mut callback: C,
    ) -> std::io::Result<()>
    where
        C: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
    {
        callback(readiness, &mut self.file)
    }

    fn register(&mut self, poll: &mut Poll, token: Token) -> std::io::Result<()> {
        poll.register(self.file.as_raw_fd(), self.interest, self.mode, token)
    }

    fn reregister(&mut self, poll: &mut Poll, token: Token) -> std::io::Result<()> {
        poll.reregister(self.file.as_raw_fd(), self.interest, self.mode, token)
    }

    fn unregister(&mut self, poll: &mut Poll) -> std::io::Result<()> {
        poll.unregister(self.file.as_raw_fd())
    }
}

#[cfg(test)]
mod test {
    use std::io::{self, Read, Write};

    use super::Generic;
    use crate::{Interest, Mode};
    #[cfg(unix)]
    #[test]
    fn dispatch_unix() {
        use std::os::unix::net::UnixStream;

        let mut event_loop = crate::EventLoop::new().unwrap();

        let handle = event_loop.handle();

        let (mut tx, rx) = UnixStream::pair().unwrap();

        let generic = Generic::new(rx, Interest::Readable, Mode::Level);

        let mut dispached = false;

        let _source = handle
            .insert_source(generic, move |readiness, file, d| {
                assert!(readiness.readable);
                // we have not registered for writability
                assert!(!readiness.writable);
                let mut buffer = vec![0; 10];
                let ret = file.read(&mut buffer).unwrap();
                assert_eq!(ret, 6);
                assert_eq!(&buffer[..6], &[1, 2, 3, 4, 5, 6]);

                *d = true;
                Ok(())
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

        let mut event_loop = crate::EventLoop::new().unwrap();

        let handle = event_loop.handle();

        let (mut tx, rx) = UnixStream::pair().unwrap();

        let generic = Generic::new(rx, Interest::Readable, Mode::Level);

        let mut dispached = false;

        let source = handle
            .insert_source(generic, move |_, _, d| {
                *d = true;
                Ok(())
            })
            .map_err(Into::<io::Error>::into)
            .unwrap();

        event_loop
            .dispatch(Some(::std::time::Duration::from_millis(0)), &mut dispached)
            .unwrap();

        assert!(!dispached);

        // remove the source, and then write something

        let generic = event_loop.handle().remove(source);

        tx.write(&[1, 2, 3, 4, 5, 6]).unwrap();
        tx.flush().unwrap();

        event_loop
            .dispatch(Some(::std::time::Duration::from_millis(0)), &mut dispached)
            .unwrap();

        // the source has not been dispatched, as the source is no longer here
        assert!(!dispached);

        // insert it again
        let _source = handle
            .insert_source(generic, move |readiness, file, d| {
                assert!(readiness.readable);
                // we have not registered for writability
                assert!(!readiness.writable);
                let mut buffer = vec![0; 10];
                let ret = file.read(&mut buffer).unwrap();
                assert_eq!(ret, 6);
                assert_eq!(&buffer[..6], &[1, 2, 3, 4, 5, 6]);

                *d = true;
                Ok(())
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
