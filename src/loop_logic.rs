use std::cell::RefCell;
use std::fmt::{self, Debug, Formatter};
use std::io;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::list::SourceList;
use crate::sources::{EventSource, Idle, IdleDispatcher, Source};
use crate::Poll;

type IdleCallback<Data> = Rc<RefCell<dyn IdleDispatcher<Data>>>;

struct LoopInner<Data> {
    poll: RefCell<Poll>,
    sources: RefCell<SourceList<Data>>,
    idles: RefCell<Vec<IdleCallback<Data>>>,
}

/// An handle to an event loop
///
/// This handle allows you to insert new sources and idles in this event loop,
/// it can be cloned, and it is possible to insert new sources from within a source
/// callback.
pub struct LoopHandle<Data> {
    inner: Rc<LoopInner<Data>>,
}

#[cfg(not(tarpaulin_include))]
impl<Data> Clone for LoopHandle<Data> {
    fn clone(&self) -> LoopHandle<Data> {
        LoopHandle {
            inner: self.inner.clone(),
        }
    }
}

/// An error generated when trying to insert an event source
pub struct InsertError<E> {
    /// The source that could not be inserted
    pub source: E,
    /// The generated error
    pub error: io::Error,
}

#[cfg(not(tarpaulin_include))]
impl<E> Debug for InsertError<E> {
    fn fmt(&self, formatter: &mut Formatter) -> Result<(), fmt::Error> {
        write!(formatter, "{:?}", self.error)
    }
}

#[cfg(not(tarpaulin_include))]
impl<E> std::fmt::Display for InsertError<E> {
    fn fmt(&self, formatter: &mut Formatter) -> Result<(), fmt::Error> {
        write!(formatter, "{}", self.error)
    }
}

#[cfg(not(tarpaulin_include))]
impl<E> From<InsertError<E>> for io::Error {
    fn from(e: InsertError<E>) -> io::Error {
        e.error
    }
}

#[cfg(not(tarpaulin_include))]
impl<E> std::error::Error for InsertError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.error.source()
    }
}

impl<Data> LoopHandle<Data> {
    /// Insert an new event source in the loop
    ///
    /// The provided callback will be called during the dispatching cycles whenever the
    /// associated source generates events, see `EventLoop::dispatch(..)` for details.
    pub fn insert_source<S, F>(&self, source: S, callback: F) -> Result<Source<S>, InsertError<S>>
    where
        S: EventSource + 'static,
        F: FnMut(S::Event, &mut S::Metadata, &mut Data) -> S::Ret + 'static,
    {
        let mut sources = self.inner.sources.borrow_mut();
        let mut poll = self.inner.poll.borrow_mut();

        let dispatcher = Rc::new(RefCell::new(crate::sources::Dispatcher::<S, F>::new(
            source, callback,
        )));
        let token = sources.add_source(dispatcher);

        let ret = sources
            .get_dispatcher(token)
            .unwrap()
            .register(&mut *poll, token);

        if let Err(error) = ret {
            let source = sources
                .del_source(token)
                .expect("Source was just inserted?!")
                .into_source_any()
                .downcast::<S>()
                .map(|s| *s)
                .unwrap_or_else(|_| panic!("Downcasting failed!"));
            return Err(InsertError { error, source });
        }

        Ok(Source {
            token,
            _type: std::marker::PhantomData,
        })
    }

    /// Insert an idle callback
    ///
    /// This callback will be called during a dispatching cycle when the event loop has
    /// finished processing all pending events from the sources and becomes idle.
    pub fn insert_idle<F: FnOnce(&mut Data) + 'static>(&self, callback: F) -> Idle {
        let mut opt_cb = Some(callback);
        let callback = Rc::new(RefCell::new(Some(move |data: &mut Data| {
            if let Some(cb) = opt_cb.take() {
                cb(data);
            }
        })));
        self.inner.idles.borrow_mut().push(callback.clone());
        Idle { callback }
    }

    /// Access this event source
    ///
    /// This allows you to modify the event source without removing it from the event
    /// loop if it allows it.
    ///
    /// Note that replacing it with an other using `mem::replace` or equivalent would not
    /// correctly update its registration and generate errors. Instead you should remove
    /// the source using the `remove()` method and then insert a new one.
    ///
    /// **Note**: This cannot be done from within the callback of the same source.
    pub fn with_source<S: EventSource + 'static, T, F: FnOnce(&mut S) -> T>(
        &self,
        source: &Source<S>,
        f: F,
    ) -> T {
        let disp = self
            .inner
            .sources
            .borrow()
            .get_dispatcher(source.token)
            .expect("Attempting to access a non-existent source?!");
        let mut source_any = disp.as_source_any();
        let source_mut = source_any.downcast_mut::<S>().expect("Downcasting failed!");
        f(source_mut)
    }

    /// Enable this previously disabled event source
    ///
    /// This previously disabled source will start generating events again.
    ///
    /// **Note**: This cannot be done from within the callback of the same source.
    pub fn enable<E: EventSource>(&self, source: &Source<E>) -> io::Result<()> {
        self.inner
            .sources
            .borrow()
            .get_dispatcher(source.token)
            .expect("Attempting to access a non-existent source?!")
            .register(&mut *self.inner.poll.borrow_mut(), source.token)
    }

    /// Make this source update its registration
    ///
    /// If after accessing the source you changed its parameters in a way that requires
    /// updating its registration.
    ///
    /// **Note**: This cannot be done from within the callback of the same source.
    pub fn update<E: EventSource>(&self, source: &Source<E>) -> io::Result<()> {
        self.inner
            .sources
            .borrow()
            .get_dispatcher(source.token)
            .expect("Attempting to access a non-existent source?!")
            .reregister(&mut *self.inner.poll.borrow_mut(), source.token)
    }

    /// Disable this event source
    ///
    /// The source remains in the event loop, but it'll no longer generate events
    ///
    /// **Note**: This cannot be done from within the callback of the same source.
    pub fn disable<E: EventSource>(&self, source: &Source<E>) -> io::Result<()> {
        self.inner
            .sources
            .borrow()
            .get_dispatcher(source.token)
            .expect("Attempting to access a non-existent source?!")
            .unregister(&mut *self.inner.poll.borrow_mut())
    }

    /// Remove this source from the event loop
    ///
    /// You are givent the `EventSource` back.
    ///
    /// **Note**: This cannot be done from within the callback of the same source.
    pub fn remove<S: EventSource + 'static>(&self, source: Source<S>) -> S {
        let source = self
            .inner
            .sources
            .borrow_mut()
            .del_source(source.token)
            .expect("Attempting to remove a non-existent source?!");
        if let Err(e) = source.unregister(&mut *self.inner.poll.borrow_mut()) {
            log::warn!(
                "[calloop] Failed to unregister source from the polling system: {:?}",
                e
            );
        }
        source
            .into_source_any()
            .downcast::<S>()
            .map(|s| *s)
            .unwrap_or_else(|_| panic!("Downcasting failed!"))
    }

    /// Remove this event source from the event loop and drop it
    ///
    /// The source is not given back to you but instead dropped
    ///
    /// **Note**: This *can* be done from within the callback of the same source
    pub fn kill<S: EventSource + 'static>(&self, source: Source<S>) {
        self.inner.sources.borrow_mut().del_source(source.token);
    }
}

/// An event loop
///
/// This loop can host several event sources, that can be dynamically added or removed.
pub struct EventLoop<Data> {
    handle: LoopHandle<Data>,
    stop_signal: Arc<AtomicBool>,
    ping: crate::sources::ping::Ping,
}

impl<Data> EventLoop<Data> {
    /// Create a new event loop
    ///
    /// Fails if the initialization of the polling system failed.
    pub fn new() -> io::Result<EventLoop<Data>> {
        let poll = Poll::new()?;
        let handle = LoopHandle {
            inner: Rc::new(LoopInner {
                poll: RefCell::new(poll),
                sources: RefCell::new(SourceList::new()),
                idles: RefCell::new(Vec::new()),
            }),
        };
        let (ping, ping_source) = crate::sources::ping::make_ping()?;
        handle.insert_source(ping_source, |_, _, _| {})?;
        Ok(EventLoop {
            handle,
            stop_signal: Arc::new(AtomicBool::new(false)),
            ping,
        })
    }

    /// Retrieve a loop handle
    pub fn handle(&self) -> LoopHandle<Data> {
        self.handle.clone()
    }

    fn dispatch_events(
        &mut self,
        mut timeout: Option<Duration>,
        data: &mut Data,
    ) -> io::Result<()> {
        let events = {
            let mut poll = self.handle.inner.poll.borrow_mut();
            loop {
                let now = std::time::Instant::now();
                let result = poll.poll(timeout);

                match result {
                    Ok(events) => break events,
                    Err(ref err) if err.kind() == io::ErrorKind::Interrupted => {
                        // Interrupted by a signal. Update timeout and retry.
                        if let Some(to) = timeout {
                            let elapsed = now.elapsed();
                            if elapsed >= to {
                                return Ok(());
                            } else {
                                timeout = Some(to - elapsed);
                            }
                        }
                    }
                    Err(err) => return Err(err),
                };
            }
        };

        for event in events {
            let opt_disp = self
                .handle
                .inner
                .sources
                .borrow()
                .get_dispatcher(event.token);

            if let Some(disp) = opt_disp {
                disp.process_events(event.readiness, event.token, data)?;

                if !self.handle.inner.sources.borrow().contains(event.token) {
                    // the source has been removed from within its callback, unregister it
                    let mut poll = self.handle.inner.poll.borrow_mut();
                    if let Err(e) = disp.unregister(&mut *poll) {
                        log::warn!(
                            "[calloop] Failed to unregister source from the polling system: {:?}",
                            e
                        );
                    }
                }
            } else {
                log::warn!(
                    "[calloop] Received an event for non-existence source: {}",
                    event.token.id
                );
            }
        }

        Ok(())
    }

    fn dispatch_idles(&mut self, data: &mut Data) {
        let idles = ::std::mem::replace(&mut *self.handle.inner.idles.borrow_mut(), Vec::new());
        for idle in idles {
            idle.borrow_mut().dispatch(data);
        }
    }

    /// Dispatch pending events to their callbacks
    ///
    /// Some source have events available, their callbacks will be immediatly called.
    /// Otherwise this will wait until an event is receive or the provided `timeout`
    /// is reached. If `timeout` is `None`, it will wait without a duration limit.
    ///
    /// Once pending events have been processed or the timeout is reached, all pending
    /// idle callbacks will be fired before this method returns.
    pub fn dispatch<D: Into<Option<Duration>>>(
        &mut self,
        timeout: D,
        data: &mut Data,
    ) -> io::Result<()> {
        self.dispatch_events(timeout.into(), data)?;

        self.dispatch_idles(data);

        Ok(())
    }

    /// Get a signal to stop this event loop from running
    ///
    /// To be used in conjunction with the `run()` method.
    pub fn get_signal(&self) -> LoopSignal {
        LoopSignal {
            signal: self.stop_signal.clone(),
            ping: self.ping.clone(),
        }
    }

    /// Run this event loop
    ///
    /// This will repeatedly try to dispatch events (see the `dispatch()` method) on
    /// this event loop, waiting at most `timeout` every time.
    ///
    /// Between each dispatch wait, your provided callback will be called.
    ///
    /// You can use the `get_signal()` method to retrieve a way to stop or wakeup
    /// the event loop from anywhere.
    pub fn run<F>(
        &mut self,
        timeout: Option<Duration>,
        data: &mut Data,
        mut cb: F,
    ) -> io::Result<()>
    where
        F: FnMut(&mut Data),
    {
        self.stop_signal.store(false, Ordering::Release);
        while !self.stop_signal.load(Ordering::Acquire) {
            self.dispatch(timeout, data)?;
            cb(data);
        }
        Ok(())
    }
}

/// A signal that can be shared between thread to stop or wakeup a running
/// event loop
#[derive(Clone)]
pub struct LoopSignal {
    signal: Arc<AtomicBool>,
    ping: crate::sources::ping::Ping,
}

impl LoopSignal {
    /// Stop the event loop
    ///
    /// Once this method is called, the next time the event loop has finished
    /// waiting for events, it will return rather than starting to wait again.
    ///
    /// This is only usefull if you are using the `EventLoop::run()` method.
    pub fn stop(&self) {
        self.signal.store(true, Ordering::Release);
    }

    /// Wake up the event loop
    ///
    /// This sends a dummy event to the event loop to simulate the reception
    /// of an event, making the wait return early. Called after `stop()`, this
    /// ensures the event loop will terminate quickly if you specified a long
    /// timeout (or no timeout at all) to the `dispatch` or `run` method.
    pub fn wakeup(&self) {
        self.ping.ping();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{
        generic::Generic, sources::ping::*, Interest, Mode, Poll, Readiness, Source, Token,
    };

    use super::EventLoop;

    #[test]
    fn dispatch_idle() {
        let mut event_loop = EventLoop::new().unwrap();

        let mut dispatched = false;

        event_loop.handle().insert_idle(|d| {
            *d = true;
        });

        event_loop
            .dispatch(Some(Duration::from_millis(0)), &mut dispatched)
            .unwrap();

        assert!(dispatched);
    }

    #[test]
    fn cancel_idle() {
        let mut event_loop = EventLoop::new().unwrap();

        let mut dispatched = false;

        let idle = event_loop.handle().insert_idle(move |d| {
            *d = true;
        });

        idle.cancel();

        event_loop
            .dispatch(Duration::from_millis(0), &mut dispatched)
            .unwrap();

        assert!(!dispatched);
    }

    #[test]
    fn wakeup() {
        let mut event_loop = EventLoop::new().unwrap();

        let signal = event_loop.get_signal();

        ::std::thread::spawn(move || {
            ::std::thread::sleep(Duration::from_millis(500));
            signal.wakeup();
        });

        // the test should return
        event_loop.dispatch(None, &mut ()).unwrap();
    }

    #[test]
    fn wakeup_stop() {
        let mut event_loop = EventLoop::new().unwrap();

        let signal = event_loop.get_signal();

        ::std::thread::spawn(move || {
            ::std::thread::sleep(Duration::from_millis(500));
            signal.stop();
            signal.wakeup();
        });

        // the test should return
        event_loop.run(None, &mut (), |_| {}).unwrap();
    }

    #[test]
    fn insert_remove() {
        let mut event_loop = EventLoop::<()>::new().unwrap();
        let source_1 = event_loop
            .handle()
            .insert_source(DummySource, |_, _, _| {})
            .unwrap();
        assert_eq!(source_1.token.id, 1); // numer 0 is the internal ping
        let source_2 = event_loop
            .handle()
            .insert_source(DummySource, |_, _, _| {})
            .unwrap();
        assert_eq!(source_2.token.id, 2);
        // ensure token reuse on source removal
        event_loop.handle().remove(source_1);

        event_loop
            .dispatch(Duration::from_millis(0), &mut ())
            .unwrap();

        let source_3 = event_loop
            .handle()
            .insert_source(DummySource, |_, _, _| {})
            .unwrap();
        assert_eq!(source_3.token.id, 1);
    }

    #[test]
    fn insert_bad_source() {
        let event_loop = EventLoop::<()>::new().unwrap();
        let ret = event_loop.handle().insert_source(
            crate::sources::generic::Generic::from_fd(42, Interest::Readable, Mode::Level),
            |_, _, _| Ok(()),
        );
        assert!(ret.is_err());
    }

    #[test]
    fn disarm_rearm() {
        let mut event_loop = EventLoop::<bool>::new().unwrap();
        let (ping, ping_source) = make_ping().unwrap();

        let ping_source = event_loop
            .handle()
            .insert_source(ping_source, |(), &mut (), dispatched| {
                *dispatched = true;
            })
            .unwrap();

        ping.ping();
        let mut dispatched = false;
        event_loop
            .dispatch(Duration::from_millis(0), &mut dispatched)
            .unwrap();
        assert_eq!(dispatched, true);

        // disable the source
        ping.ping();
        event_loop.handle().disable(&ping_source).unwrap();
        let mut dispatched = false;
        event_loop
            .dispatch(Duration::from_millis(0), &mut dispatched)
            .unwrap();
        assert_eq!(dispatched, false);

        // disabling it again is an error
        event_loop.handle().disable(&ping_source).unwrap_err();

        // reenable it, the previous ping now gets dispatched
        event_loop.handle().enable(&ping_source).unwrap();
        let mut dispatched = false;
        event_loop
            .dispatch(Duration::from_millis(0), &mut dispatched)
            .unwrap();
        assert_eq!(dispatched, true);
    }

    #[test]
    fn multiple_tokens() {
        struct DoubleSource {
            ping1: PingSource,
            ping2: PingSource,
        }

        impl crate::EventSource for DoubleSource {
            type Event = u32;
            type Metadata = ();
            type Ret = ();

            fn process_events<F>(
                &mut self,
                readiness: Readiness,
                token: Token,
                mut callback: F,
            ) -> std::io::Result<()>
            where
                F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
            {
                if token.sub_id == 1 {
                    self.ping1
                        .process_events(readiness, token, |(), &mut ()| callback(1, &mut ()))
                } else if token.sub_id == 2 {
                    self.ping2
                        .process_events(readiness, token, |(), &mut ()| callback(2, &mut ()))
                } else {
                    Ok(())
                }
            }

            fn register(&mut self, poll: &mut Poll, token: Token) -> std::io::Result<()> {
                self.ping1.register(poll, Token { sub_id: 1, ..token })?;
                self.ping2.register(poll, Token { sub_id: 2, ..token })?;
                Ok(())
            }

            fn reregister(&mut self, poll: &mut Poll, token: Token) -> std::io::Result<()> {
                self.ping1.reregister(poll, Token { sub_id: 1, ..token })?;
                self.ping2.reregister(poll, Token { sub_id: 2, ..token })?;
                Ok(())
            }

            fn unregister(&mut self, poll: &mut Poll) -> std::io::Result<()> {
                self.ping1.unregister(poll)?;
                self.ping2.unregister(poll)?;
                Ok(())
            }
        }

        let mut event_loop = EventLoop::<u32>::new().unwrap();

        let (ping1, source1) = make_ping().unwrap();
        let (ping2, source2) = make_ping().unwrap();

        let source = DoubleSource {
            ping1: source1,
            ping2: source2,
        };

        event_loop
            .handle()
            .insert_source(source, |i, _, d| {
                eprintln!("Dispatching {}", i);
                *d += i
            })
            .unwrap();

        let mut dispatched = 0;
        ping1.ping();
        event_loop
            .dispatch(Duration::from_millis(0), &mut dispatched)
            .unwrap();
        assert_eq!(dispatched, 1);

        dispatched = 0;
        ping2.ping();
        event_loop
            .dispatch(Duration::from_millis(0), &mut dispatched)
            .unwrap();
        assert_eq!(dispatched, 2);

        dispatched = 0;
        ping1.ping();
        ping2.ping();
        event_loop
            .dispatch(Duration::from_millis(0), &mut dispatched)
            .unwrap();
        assert_eq!(dispatched, 3);
    }

    #[test]
    fn change_interests() {
        use nix::sys::socket::{socketpair, AddressFamily, SockFlag, SockType};
        use nix::unistd::{read, write};
        let mut event_loop = EventLoop::<bool>::new().unwrap();

        let (sock1, sock2) = socketpair(
            AddressFamily::Unix,
            SockType::Stream,
            None,
            SockFlag::SOCK_NONBLOCK,
        )
        .unwrap();

        let source = Generic::from_fd(sock1, Interest::Readable, Mode::Level);

        let sock1 = event_loop
            .handle()
            .insert_source(source, |_, fd, dispatched| {
                *dispatched = true;
                // read all contents available to drain the socket
                let mut buf = [0u8; 32];
                loop {
                    match read(fd.0, &mut buf) {
                        Ok(0) => break, // closed pipe, we are now inert
                        Ok(_) => {}
                        Err(e) => {
                            let e = crate::no_nix_err(e);
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
                Ok(())
            })
            .unwrap();

        // first dispatch, nothing is readable
        let mut dispatched = false;
        event_loop
            .dispatch(Duration::from_millis(0), &mut dispatched)
            .unwrap();
        assert!(!dispatched);

        // write something, the socket becomes readable
        write(sock2, &[1, 2, 3]).unwrap();
        dispatched = false;
        event_loop
            .dispatch(Duration::from_millis(0), &mut dispatched)
            .unwrap();
        assert!(dispatched);

        // All has been read, no longer readable
        dispatched = false;
        event_loop
            .dispatch(Duration::from_millis(0), &mut dispatched)
            .unwrap();
        assert!(!dispatched);

        // change the interests for writability instead
        event_loop
            .handle()
            .with_source(&sock1, |g| g.interest = Interest::Writable);
        event_loop.handle().update(&sock1).unwrap();

        // the socket is writable
        dispatched = false;
        event_loop
            .dispatch(Duration::from_millis(0), &mut dispatched)
            .unwrap();
        assert!(dispatched);

        // change back to readable
        event_loop
            .handle()
            .with_source(&sock1, |g| g.interest = Interest::Readable);
        event_loop.handle().update(&sock1).unwrap();

        // the socket is not readable
        dispatched = false;
        event_loop
            .dispatch(Duration::from_millis(0), &mut dispatched)
            .unwrap();
        assert!(!dispatched);
    }

    #[test]
    fn kill_source() {
        let mut event_loop = EventLoop::<Option<Source<PingSource>>>::new().unwrap();

        let handle = event_loop.handle();
        let (ping, ping_source) = make_ping().unwrap();
        let source = event_loop
            .handle()
            .insert_source(ping_source, move |(), &mut (), opt_src| {
                if let Some(src) = opt_src.take() {
                    handle.kill(src);
                }
            })
            .unwrap();

        ping.ping();

        let mut opt_src = Some(source);

        event_loop
            .dispatch(Duration::from_millis(0), &mut opt_src)
            .unwrap();

        assert!(opt_src.is_none());
    }

    #[test]
    fn non_static_data() {
        use std::sync::mpsc;

        let (sender, receiver) = mpsc::channel();

        {
            struct RefSender<'a>(&'a mpsc::Sender<()>);
            let mut ref_sender = RefSender(&sender);

            let mut event_loop = EventLoop::<RefSender<'_>>::new().unwrap();
            let (ping, ping_source) = make_ping().unwrap();
            let _source = event_loop
                .handle()
                .insert_source(ping_source, |_, _, ref_sender| {
                    ref_sender.0.send(()).unwrap();
                })
                .unwrap();

            ping.ping();

            event_loop
                .dispatch(Duration::from_millis(0), &mut ref_sender)
                .unwrap();
        }

        receiver.recv().unwrap();
        // sender still usable (e.g. for another EventLoop)
        drop(sender);
    }

    // A dummy EventSource to test insertion and removal of sources
    struct DummySource;

    impl crate::EventSource for DummySource {
        type Event = ();
        type Metadata = ();
        type Ret = ();

        fn process_events<F>(
            &mut self,
            _: Readiness,
            _: Token,
            mut callback: F,
        ) -> std::io::Result<()>
        where
            F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
        {
            callback((), &mut ());
            Ok(())
        }

        fn register(&mut self, _: &mut Poll, _: Token) -> std::io::Result<()> {
            Ok(())
        }

        fn reregister(&mut self, _: &mut Poll, _: Token) -> std::io::Result<()> {
            Ok(())
        }

        fn unregister(&mut self, _: &mut Poll) -> std::io::Result<()> {
            Ok(())
        }
    }
}
