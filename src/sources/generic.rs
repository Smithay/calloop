//! A generic event source wrapping an `Evented` type

use std::cell::RefCell;
use std::io;
#[cfg(unix)]
use std::os::unix::io::{AsRawFd, RawFd};
use std::rc::Rc;

use mio::{Evented, Poll, PollOpt, Ready, Token};

use {EventDispatcher, EventSource};

/// A generic event source wrapping an `Evented` type
///
/// It will simply forward the readiness and an acces to
/// the wrapped `Evented` type to the suer callback. See
/// the `Event` type in this module.
pub struct Generic<E: Evented + 'static> {
    inner: Rc<RefCell<E>>,
    interest: Ready,
    pollopts: PollOpt,
}

impl<E: Evented + 'static> Generic<E> {
    /// Wrap a `Evented` type into a `Generic` event source
    ///
    /// It is initialized with no interest nor poll options,
    /// as such you should set them using the `set_interest`
    /// and `set_pollopts` methods before inserting it in the
    /// event loop.
    pub fn new(source: E) -> Generic<E> {
        Generic {
            inner: Rc::new(RefCell::new(source)),
            interest: Ready::empty(),
            pollopts: PollOpt::empty(),
        }
    }

    /// Change the interest for this evented source
    ///
    /// If the source was already inserted in an event loop,
    /// it needs to be re-registered for the change to take
    /// effect.
    pub fn set_interest(&mut self, interest: Ready) {
        self.interest = interest;
    }

    /// Change the poll options for this evented source
    ///
    /// If the source was already inserted in an event loop,
    /// it needs to be re-registered for the change to take
    /// effect.
    pub fn set_pollopts(&mut self, pollopts: PollOpt) {
        self.pollopts = pollopts;
    }

    /// Get a clone of the inner `Rc` wrapping your event source
    pub fn clone_inner(&self) -> Rc<RefCell<E>> {
        self.inner.clone()
    }

    /// Unwrap the `Generic` source to retrieve the underlying `Evented`.
    ///
    /// If you didn't clone the `Rc<RefCell<E>>` from the `Event<E>` you received,
    /// the returned `Rc` should be unique.
    pub fn unwrap(self) -> Rc<RefCell<E>> {
        self.inner
    }
}

impl<Fd: AsRawFd> Generic<EventedFd<Fd>> {
    /// Wrap a file descriptor based source into a `Generic` event source.
    ///
    /// This will only work with poll-compatible file descriptors, which typically
    /// not include basic files.
    #[cfg(unix)]
    pub fn from_fd_source(source: Fd) -> Generic<EventedFd<Fd>> {
        Generic::new(EventedFd(source))
    }
}

impl Generic<EventedRawFd> {
    /// Wrap a raw file descriptor into a `Generic` event source.
    ///
    /// This will only work with poll-compatible file descriptors, which typically
    /// not include basic files.
    ///
    /// This does _not_ take ownership of the file descriptor, hence you are responsible
    /// of its correct lifetime.
    #[cfg(unix)]
    pub fn from_raw_fd(fd: RawFd) -> Generic<EventedRawFd> {
        Generic::new(EventedRawFd(fd))
    }
}

/// An event generated by the `Generic` source
pub struct Event<E: Evented + 'static> {
    /// An access to the source that generated this event
    pub source: Rc<RefCell<E>>,
    /// The associated rediness
    pub readiness: Ready,
}

/// An owning wrapper implementing Evented for any file descriptor based type in Unix
#[cfg(unix)]
pub struct EventedFd<F: AsRawFd>(pub F);

impl<F: AsRawFd> Evented for EventedFd<F> {
    fn register(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        ::mio::unix::EventedFd(&self.0.as_raw_fd()).register(poll, token, interest, opts)
    }

    fn reregister(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        ::mio::unix::EventedFd(&self.0.as_raw_fd()).reregister(poll, token, interest, opts)
    }

    fn deregister(&self, poll: &Poll) -> io::Result<()> {
        ::mio::unix::EventedFd(&self.0.as_raw_fd()).deregister(poll)
    }
}

/// A wrapper implementing Evented for any raw file descriptor.
///
/// It does _not_ take ownership of the file descriptor, you are
/// responsible for ensuring its correct lifetime.
#[cfg(unix)]
pub struct EventedRawFd(pub RawFd);

impl Evented for EventedRawFd {
    fn register(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        ::mio::unix::EventedFd(&self.0).register(poll, token, interest, opts)
    }

    fn reregister(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        ::mio::unix::EventedFd(&self.0).reregister(poll, token, interest, opts)
    }

    fn deregister(&self, poll: &Poll) -> io::Result<()> {
        ::mio::unix::EventedFd(&self.0).deregister(poll)
    }
}

impl<E: Evented + 'static> Evented for Generic<E> {
    fn register(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        self.inner.borrow().register(poll, token, interest, opts)
    }

    fn reregister(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        self.inner.borrow().reregister(poll, token, interest, opts)
    }

    fn deregister(&self, poll: &Poll) -> io::Result<()> {
        self.inner.borrow().deregister(poll)
    }
}

impl<E: Evented + 'static> EventSource for Generic<E> {
    type Event = Event<E>;

    fn interest(&self) -> Ready {
        self.interest
    }

    fn pollopts(&self) -> PollOpt {
        self.pollopts
    }

    fn make_dispatcher<Data: 'static, F: FnMut(Event<E>, &mut Data) + 'static>(
        &self,
        callback: F,
    ) -> Rc<RefCell<EventDispatcher<Data>>> {
        Rc::new(RefCell::new(Dispatcher {
            _data: ::std::marker::PhantomData,
            inner: self.inner.clone(),
            callback,
        }))
    }
}

struct Dispatcher<Data, E: Evented + 'static, F: FnMut(Event<E>, &mut Data)> {
    _data: ::std::marker::PhantomData<fn(&mut Data)>,
    inner: Rc<RefCell<E>>,
    callback: F,
}

impl<Data, E: Evented + 'static, F: FnMut(Event<E>, &mut Data)> EventDispatcher<Data>
    for Dispatcher<Data, E, F>
{
    fn ready(&mut self, ready: Ready, data: &mut Data) {
        (self.callback)(
            Event {
                source: self.inner.clone(),
                readiness: ready,
            },
            data,
        )
    }
}

#[cfg(test)]
mod test {
    use std::io::{self, Read, Write};

    use super::{Event, Generic};
    #[cfg(unix)]
    #[test]
    fn dispatch_unix() {
        use std::os::unix::net::UnixStream;

        let mut event_loop = ::EventLoop::new().unwrap();

        let handle = event_loop.handle();

        let (mut tx, rx) = UnixStream::pair().unwrap();

        let mut generic = Generic::from_fd_source(rx);
        generic.set_interest(::mio::Ready::readable());

        let mut dispached = false;

        handle
            .insert_source(generic, move |Event { source, readiness }, d| {
                assert!(readiness.is_readable());
                // we have not registered for writability
                assert!(!readiness.is_writable());
                let mut buffer = vec![0; 10];
                let ret = source.borrow_mut().0.read(&mut buffer).unwrap();
                assert_eq!(ret, 6);
                assert_eq!(&buffer[..6], &[1, 2, 3, 4, 5, 6]);

                *d = true;
            }).map_err(Into::<io::Error>::into)
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
}
