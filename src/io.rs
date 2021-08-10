//! Adapters for async IO objects
//!
//! This module mainly hosts the [`Async`] adapter for making IO objects async with readiness
//! monitoring backed by an [`EventLoop`](crate::EventLoop). See [`LoopHandle::adapt_io`] for
//! how to create them.
//!
//! [`LoopHandle::adapt_io`]: crate::LoopHandle#method.adapt_io

use std::cell::RefCell;
use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll as TaskPoll, Waker};

use nix::fcntl::{fcntl, FcntlArg, OFlag};

#[cfg(feature = "futures-io")]
use futures_io::{AsyncRead, AsyncWrite, IoSlice, IoSliceMut};

use crate::{
    loop_logic::LoopInner, sources::EventDispatcher, Interest, Mode, Poll, PostAction, Readiness,
    Token, TokenFactory,
};

/// Adapter for async IO manipulations
///
/// This type wraps an IO object, providing methods to create futures waiting for its
/// readiness.
///
/// If the `futures-io` cargo feature is enabled, it also implements `AsyncRead` and/or
/// `AsyncWrite` if the underlying type implements `Read` and/or `Write`.
///
/// Note that this adapter and the futures procuded from it and *not* threadsafe.
pub struct Async<'l, F: AsRawFd> {
    fd: Option<F>,
    dispatcher: Rc<RefCell<IoDispatcher>>,
    inner: Rc<dyn IoLoopInner + 'l>,
    old_flags: OFlag,
}

impl<'l, F: AsRawFd + std::fmt::Debug> std::fmt::Debug for Async<'l, F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Async").field("fd", &self.fd).finish()
    }
}

impl<'l, F: AsRawFd> Async<'l, F> {
    pub(crate) fn new<Data>(
        inner: Rc<LoopInner<'l, Data>>,
        fd: F,
    ) -> std::io::Result<Async<'l, F>> {
        let rawfd = fd.as_raw_fd();
        // set non-blocking
        let old_flags = fcntl(rawfd, FcntlArg::F_GETFL)?;
        let old_flags = unsafe { OFlag::from_bits_unchecked(old_flags) };
        fcntl(rawfd, FcntlArg::F_SETFL(old_flags | OFlag::O_NONBLOCK))?;
        // register in the loop
        let dispatcher = Rc::new(RefCell::new(IoDispatcher {
            fd: rawfd,
            id: 0,
            waker: None,
            is_registered: false,
            interest: Interest::EMPTY,
            last_readiness: Readiness::EMPTY,
        }));
        let id = inner.sources.borrow_mut().add_source(dispatcher.clone());
        dispatcher.borrow_mut().id = id;
        inner.register(&dispatcher)?;

        // Straightforward casting would require us to add the bound `Data: 'l` but we don't actually need it
        // as this module never accesses the dispatch data, so we use transmute to erase it
        let inner: Rc<dyn IoLoopInner + 'l> =
            unsafe { std::mem::transmute(inner as Rc<dyn IoLoopInner>) };

        Ok(Async {
            fd: Some(fd),
            dispatcher,
            inner,
            old_flags,
        })
    }

    /// Mutably access the underlying IO object
    pub fn get_mut(&mut self) -> &mut F {
        self.fd.as_mut().unwrap()
    }

    /// A future that resolves once the object becomes ready for reading
    pub fn readable<'s>(&'s mut self) -> Readable<'s, 'l, F> {
        Readable { io: self }
    }

    /// A future that resolves once the object becomes ready for writing
    pub fn writable<'s>(&'s mut self) -> Writable<'s, 'l, F> {
        Writable { io: self }
    }

    /// Remove the async adapter and retrieve the underlying object
    pub fn into_inner(mut self) -> F {
        self.fd.take().unwrap()
    }

    fn readiness(&self) -> Readiness {
        self.dispatcher.borrow_mut().readiness()
    }

    fn register_waker(&self, interest: Interest, waker: Waker) -> std::io::Result<()> {
        {
            let mut disp = self.dispatcher.borrow_mut();
            disp.interest = interest;
            disp.waker = Some(waker);
        }
        self.inner.reregister(&self.dispatcher)
    }
}

/// A future that resolves once the associated object becomes ready for reading
#[derive(Debug)]
pub struct Readable<'s, 'l, F: AsRawFd> {
    io: &'s mut Async<'l, F>,
}

impl<'s, 'l, F: AsRawFd> std::future::Future for Readable<'s, 'l, F> {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> TaskPoll<()> {
        let io = &mut self.as_mut().io;
        let readiness = io.readiness();
        if readiness.readable || readiness.error {
            TaskPoll::Ready(())
        } else {
            let _ = io.register_waker(Interest::READ, cx.waker().clone());
            TaskPoll::Pending
        }
    }
}

/// A future that resolves once the associated object becomes ready for writing
#[derive(Debug)]
pub struct Writable<'s, 'l, F: AsRawFd> {
    io: &'s mut Async<'l, F>,
}

impl<'s, 'l, F: AsRawFd> std::future::Future for Writable<'s, 'l, F> {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> TaskPoll<()> {
        let io = &mut self.as_mut().io;
        let readiness = io.readiness();
        if readiness.writable || readiness.error {
            TaskPoll::Ready(())
        } else {
            let _ = io.register_waker(Interest::WRITE, cx.waker().clone());
            TaskPoll::Pending
        }
    }
}

impl<'l, F: AsRawFd> Drop for Async<'l, F> {
    fn drop(&mut self) {
        self.inner.kill(&self.dispatcher);
        // restore flags
        let _ = fcntl(
            self.dispatcher.borrow().fd,
            FcntlArg::F_SETFL(self.old_flags),
        );
    }
}

impl<'l, F: AsRawFd> Unpin for Async<'l, F> {}

trait IoLoopInner {
    fn register(&self, dispatcher: &RefCell<IoDispatcher>) -> io::Result<()>;
    fn reregister(&self, dispatcher: &RefCell<IoDispatcher>) -> io::Result<()>;
    fn kill(&self, dispatcher: &RefCell<IoDispatcher>);
}

impl<'l, Data> IoLoopInner for LoopInner<'l, Data> {
    fn register(&self, dispatcher: &RefCell<IoDispatcher>) -> io::Result<()> {
        let disp = dispatcher.borrow();
        self.poll.borrow_mut().register(
            disp.fd,
            Interest::EMPTY,
            Mode::OneShot,
            Token {
                id: disp.id,
                sub_id: 0,
            },
        )
    }

    fn reregister(&self, dispatcher: &RefCell<IoDispatcher>) -> io::Result<()> {
        let disp = dispatcher.borrow();
        self.poll.borrow_mut().reregister(
            disp.fd,
            disp.interest,
            Mode::OneShot,
            Token {
                id: disp.id,
                sub_id: 0,
            },
        )
    }

    fn kill(&self, dispatcher: &RefCell<IoDispatcher>) {
        let id = dispatcher.borrow().id;
        let _source = self
            .sources
            .borrow_mut()
            .del_source(id)
            .expect("Attempting to remove a non-existent source?!");
    }
}

struct IoDispatcher {
    fd: RawFd,
    id: u32,
    waker: Option<Waker>,
    is_registered: bool,
    interest: Interest,
    last_readiness: Readiness,
}

impl IoDispatcher {
    fn readiness(&mut self) -> Readiness {
        std::mem::replace(&mut self.last_readiness, Readiness::EMPTY)
    }
}

impl<Data> EventDispatcher<Data> for RefCell<IoDispatcher> {
    fn process_events(
        &self,
        readiness: Readiness,
        _token: Token,
        _data: &mut Data,
    ) -> std::io::Result<PostAction> {
        let mut disp = self.borrow_mut();
        disp.last_readiness = readiness;
        if let Some(waker) = disp.waker.take() {
            waker.wake();
        }
        Ok(PostAction::Continue)
    }

    fn register(&self, _: &mut Poll, _: &mut TokenFactory) -> std::io::Result<()> {
        // registration is handled by IoLoopInner
        unreachable!()
    }

    fn reregister(&self, _: &mut Poll, _: &mut TokenFactory) -> std::io::Result<()> {
        // registration is handled by IoLoopInner
        unreachable!()
    }

    fn unregister(&self, poll: &mut Poll) -> std::io::Result<()> {
        let disp = self.borrow();
        if disp.is_registered {
            poll.unregister(disp.fd)?;
        }
        Ok(())
    }
}

/*
 * Async IO trait implementations
 */

#[cfg(feature = "futures-io")]
impl<'l, F: AsRawFd + io::Read> AsyncRead for Async<'l, F> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> TaskPoll<io::Result<usize>> {
        match (*self).get_mut().read(buf) {
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
            res => return TaskPoll::Ready(res),
        }
        self.register_waker(Interest::READ, cx.waker().clone())?;
        TaskPoll::Pending
    }

    fn poll_read_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &mut [IoSliceMut<'_>],
    ) -> TaskPoll<io::Result<usize>> {
        match (*self).get_mut().read_vectored(bufs) {
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
            res => return TaskPoll::Ready(res),
        }
        self.register_waker(Interest::READ, cx.waker().clone())?;
        TaskPoll::Pending
    }
}

#[cfg(feature = "futures-io")]
impl<'l, F: AsRawFd + io::Write> AsyncWrite for Async<'l, F> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> TaskPoll<io::Result<usize>> {
        match (*self).get_mut().write(buf) {
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
            res => return TaskPoll::Ready(res),
        }
        self.register_waker(Interest::WRITE, cx.waker().clone())?;
        TaskPoll::Pending
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> TaskPoll<io::Result<usize>> {
        match (*self).get_mut().write_vectored(bufs) {
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
            res => return TaskPoll::Ready(res),
        }
        self.register_waker(Interest::WRITE, cx.waker().clone())?;
        TaskPoll::Pending
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> TaskPoll<io::Result<()>> {
        match (*self).get_mut().flush() {
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
            res => return TaskPoll::Ready(res),
        }
        self.register_waker(Interest::WRITE, cx.waker().clone())?;
        TaskPoll::Pending
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> TaskPoll<io::Result<()>> {
        self.poll_flush(cx)
    }
}

#[cfg(all(test, feature = "executor", feature = "futures-io"))]
mod tests {
    use futures::io::{AsyncReadExt, AsyncWriteExt};

    use crate::sources::futures::executor;

    #[test]
    fn read_write() {
        let mut event_loop = crate::EventLoop::try_new().unwrap();
        let handle = event_loop.handle();
        let (exec, sched) = executor().unwrap();
        handle
            .insert_source(exec, move |ret, &mut (), got| {
                *got = ret;
            })
            .map_err(Into::<std::io::Error>::into)
            .unwrap();

        let (tx, rx) = std::os::unix::net::UnixStream::pair().unwrap();
        let mut tx = handle.adapt_io(tx).unwrap();
        let mut rx = handle.adapt_io(rx).unwrap();
        let received = std::rc::Rc::new(std::cell::Cell::new(false));
        let fut_received = received.clone();

        sched
            .schedule(async move {
                let mut buf = [0; 12];
                rx.read_exact(&mut buf).await.unwrap();
                assert_eq!(&buf, b"Hello World!");
                fut_received.set(true);
            })
            .unwrap();

        // The receiving future alone cannot advance
        event_loop
            .dispatch(Some(std::time::Duration::from_millis(10)), &mut ())
            .unwrap();
        assert!(!received.get());

        // schedule the writing future as well and wait until finish
        sched
            .schedule(async move {
                tx.write_all(b"Hello World!").await.unwrap();
                tx.flush().await.unwrap();
            })
            .unwrap();

        while !received.get() {
            event_loop.dispatch(None, &mut ()).unwrap();
        }
    }

    #[test]
    fn read_write_vectored() {
        let mut event_loop = crate::EventLoop::try_new().unwrap();
        let handle = event_loop.handle();
        let (exec, sched) = executor().unwrap();
        handle
            .insert_source(exec, move |ret, &mut (), got| {
                *got = ret;
            })
            .map_err(Into::<std::io::Error>::into)
            .unwrap();

        let (tx, rx) = std::os::unix::net::UnixStream::pair().unwrap();
        let mut tx = handle.adapt_io(tx).unwrap();
        let mut rx = handle.adapt_io(rx).unwrap();
        let received = std::rc::Rc::new(std::cell::Cell::new(false));
        let fut_received = received.clone();

        sched
            .schedule(async move {
                let mut buf = [0; 12];
                let mut ioslices = buf
                    .chunks_mut(2)
                    .map(|chunk| std::io::IoSliceMut::new(chunk))
                    .collect::<Vec<_>>();
                let count = rx.read_vectored(&mut ioslices).await.unwrap();
                assert_eq!(count, 12);
                assert_eq!(&buf, b"Hello World!");
                fut_received.set(true);
            })
            .unwrap();

        // The receiving future alone cannot advance
        event_loop
            .dispatch(Some(std::time::Duration::from_millis(10)), &mut ())
            .unwrap();
        assert!(!received.get());

        // schedule the writing future as well and wait until finish
        sched
            .schedule(async move {
                let buf = b"Hello World!";
                let ioslices = buf
                    .chunks(2)
                    .map(|chunk| std::io::IoSlice::new(chunk))
                    .collect::<Vec<_>>();
                let count = tx.write_vectored(&ioslices).await.unwrap();
                assert_eq!(count, 12);
                tx.flush().await.unwrap();
            })
            .unwrap();

        while !received.get() {
            event_loop.dispatch(None, &mut ()).unwrap();
        }
    }
}
