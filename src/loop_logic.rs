use std::cell::{Cell, RefCell};
use std::fmt::Debug;
use std::io;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(feature = "block_on")]
use std::future::Future;

use io_lifetimes::AsFd;
use slab::Slab;

use crate::sources::{Dispatcher, EventSource, Idle, IdleDispatcher};
use crate::sys::{Notifier, PollEvent};
use crate::{
    AdditionalLifetimeEventsSet, EventDispatcher, InsertError, Poll, PostAction, TokenFactory,
};

type IdleCallback<'i, Data> = Rc<RefCell<dyn IdleDispatcher<Data> + 'i>>;

// The number of bits used to store the source ID.
//
// This plus `MAX_SUBSOURCES` must equal the number of bits in `usize`.
#[cfg(target_pointer_width = "64")]
pub(crate) const MAX_SOURCES: u32 = 44;
#[cfg(target_pointer_width = "32")]
pub(crate) const MAX_SOURCES: u32 = 22;
#[cfg(target_pointer_width = "16")]
pub(crate) const MAX_SOURCES: u32 = 10;

// The number of bits used to store the sub-source ID.
//
// This plus `MAX_SOURCES` must equal the number of bits in `usize`.
#[cfg(target_pointer_width = "64")]
pub(crate) const MAX_SUBSOURCES: u32 = 20;
#[cfg(target_pointer_width = "32")]
pub(crate) const MAX_SUBSOURCES: u32 = 10;
#[cfg(target_pointer_width = "16")]
pub(crate) const MAX_SUBSOURCES: u32 = 6;

pub(crate) const MAX_SOURCES_TOTAL: usize = 1 << MAX_SOURCES;
pub(crate) const MAX_SUBSOURCES_TOTAL: usize = 1 << MAX_SUBSOURCES;
pub(crate) const MAX_SOURCES_MASK: usize = MAX_SOURCES_TOTAL - 1;

/// A token representing a registration in the [`EventLoop`].
///
/// This token is given to you by the [`EventLoop`] when an [`EventSource`] is inserted or
/// a [`Dispatcher`] is registered. You can use it to [disable](LoopHandle#method.disable),
/// [enable](LoopHandle#method.enable), [update`](LoopHandle#method.update),
/// [remove](LoopHandle#method.remove) or [kill](LoopHandle#method.kill) it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegistrationToken {
    key: usize,
}

pub(crate) struct LoopInner<'l, Data> {
    pub(crate) poll: RefCell<Poll>,
    pub(crate) sources: RefCell<Slab<Rc<dyn EventDispatcher<Data> + 'l>>>,
    pub(crate) sources_with_additional_lifetime_events: AdditionalLifetimeEventsSet,
    idles: RefCell<Vec<IdleCallback<'l, Data>>>,
    pending_action: Cell<PostAction>,
}

/// An handle to an event loop
///
/// This handle allows you to insert new sources and idles in this event loop,
/// it can be cloned, and it is possible to insert new sources from within a source
/// callback.
pub struct LoopHandle<'l, Data> {
    inner: Rc<LoopInner<'l, Data>>,
}

impl<'l, Data> std::fmt::Debug for LoopHandle<'l, Data> {
    #[cfg_attr(feature = "nightly_coverage", no_coverage)]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LoopHandle { ... }")
    }
}

impl<'l, Data> Clone for LoopHandle<'l, Data> {
    #[cfg_attr(feature = "nightly_coverage", no_coverage)]
    fn clone(&self) -> Self {
        LoopHandle {
            inner: self.inner.clone(),
        }
    }
}

impl<'l, Data> LoopHandle<'l, Data> {
    /// Inserts a new event source in the loop.
    ///
    /// The provided callback will be called during the dispatching cycles whenever the
    /// associated source generates events, see `EventLoop::dispatch(..)` for details.
    ///
    /// This function takes ownership of the event source. Use `register_dispatcher`
    /// if you need access to the event source after this call.
    pub fn insert_source<S, F>(
        &self,
        source: S,
        callback: F,
    ) -> Result<RegistrationToken, InsertError<S>>
    where
        S: EventSource + 'l,
        F: FnMut(S::Event, &mut S::Metadata, &mut Data) -> S::Ret + 'l,
    {
        let dispatcher = Dispatcher::new(source, callback);
        self.register_dispatcher(dispatcher.clone())
            .map_err(|error| InsertError {
                error,
                inserted: dispatcher.into_source_inner(),
            })
    }

    /// Registers a `Dispatcher` in the loop.
    ///
    /// Use this function if you need access to the event source after its insertion in the loop.
    ///
    /// See also `insert_source`.
    #[cfg_attr(feature = "nightly_coverage", no_coverage)] // Contains a branch we can't hit w/o OOM
    pub fn register_dispatcher<S>(
        &self,
        dispatcher: Dispatcher<'l, S, Data>,
    ) -> crate::Result<RegistrationToken>
    where
        S: EventSource + 'l,
    {
        let mut sources = self.inner.sources.borrow_mut();
        let mut poll = self.inner.poll.borrow_mut();

        // Make sure we won't overflow the token.
        if sources.vacant_key() >= MAX_SOURCES_TOTAL {
            return Err(crate::Error::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Too many sources",
            )));
        }

        let key = sources.insert(dispatcher.clone_as_event_dispatcher());
        let ret = sources.get(key).unwrap().register(
            &mut poll,
            self.inner
                .sources_with_additional_lifetime_events
                .create_register_for_token(RegistrationToken { key }),
            &mut TokenFactory::new(key),
        );

        if let Err(error) = ret {
            sources.try_remove(key).expect("Source was just inserted?!");
            return Err(error);
        }

        Ok(RegistrationToken { key })
    }

    /// Inserts an idle callback.
    ///
    /// This callback will be called during a dispatching cycle when the event loop has
    /// finished processing all pending events from the sources and becomes idle.
    pub fn insert_idle<'i, F: FnOnce(&mut Data) + 'l + 'i>(&self, callback: F) -> Idle<'i> {
        let mut opt_cb = Some(callback);
        let callback = Rc::new(RefCell::new(Some(move |data: &mut Data| {
            if let Some(cb) = opt_cb.take() {
                cb(data);
            }
        })));
        self.inner.idles.borrow_mut().push(callback.clone());
        Idle { callback }
    }

    /// Enables this previously disabled event source.
    ///
    /// This previously disabled source will start generating events again.
    ///
    /// **Note:** this cannot be done from within the source callback.
    pub fn enable(&self, token: &RegistrationToken) -> crate::Result<()> {
        if let Some(source) = self.inner.sources.borrow().get(token.key) {
            source.register(
                &mut self.inner.poll.borrow_mut(),
                self.inner
                    .sources_with_additional_lifetime_events
                    .create_register_for_token(*token),
                &mut TokenFactory::new(token.key),
            )?;
        }
        Ok(())
    }

    /// Makes this source update its registration.
    ///
    /// If after accessing the source you changed its parameters in a way that requires
    /// updating its registration.
    pub fn update(&self, token: &RegistrationToken) -> crate::Result<()> {
        if let Some(source) = self.inner.sources.borrow().get(token.key) {
            if !source.reregister(
                &mut self.inner.poll.borrow_mut(),
                self.inner
                    .sources_with_additional_lifetime_events
                    .create_register_for_token(*token),
                &mut TokenFactory::new(token.key),
            )? {
                // we are in a callback, store for later processing
                self.inner.pending_action.set(PostAction::Reregister);
            }
        }
        Ok(())
    }

    /// Disables this event source.
    ///
    /// The source remains in the event loop, but it'll no longer generate events
    pub fn disable(&self, token: &RegistrationToken) -> crate::Result<()> {
        if let Some(source) = self.inner.sources.borrow().get(token.key) {
            if !source.unregister(
                &mut self.inner.poll.borrow_mut(),
                self.inner
                    .sources_with_additional_lifetime_events
                    .create_register_for_token(*token),
            )? {
                // we are in a callback, store for later processing
                self.inner.pending_action.set(PostAction::Disable);
            }
        }
        Ok(())
    }

    /// Removes this source from the event loop.
    pub fn remove(&self, token: RegistrationToken) {
        if let Some(source) = self.inner.sources.borrow_mut().try_remove(token.key) {
            if let Err(e) = source.unregister(
                &mut self.inner.poll.borrow_mut(),
                self.inner
                    .sources_with_additional_lifetime_events
                    .create_register_for_token(token),
            ) {
                log::warn!(
                    "[calloop] Failed to unregister source from the polling system: {:?}",
                    e
                );
            }
        }
    }

    /// Wrap an IO object into an async adapter
    ///
    /// This adapter turns the IO object into an async-aware one that can be used in futures.
    /// The readiness of these futures will be driven by the event loop.
    ///
    /// The produced futures can be polled in any executor, and notably the one provided by
    /// calloop.
    pub fn adapt_io<F: AsFd>(&self, fd: F) -> crate::Result<crate::io::Async<'l, F>> {
        crate::io::Async::new(self.inner.clone(), fd)
    }
}

/// An event loop
///
/// This loop can host several event sources, that can be dynamically added or removed.
pub struct EventLoop<'l, Data> {
    handle: LoopHandle<'l, Data>,
    signals: Arc<Signals>,
    // A caching vector for synthetic poll events
    synthetic_events: Vec<PollEvent>,
}

impl<'l, Data> std::fmt::Debug for EventLoop<'l, Data> {
    #[cfg_attr(feature = "nightly_coverage", no_coverage)]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EventLoop { ... }")
    }
}

/// Signals related to the event loop.
struct Signals {
    /// Signal to stop the event loop.
    stop: AtomicBool,

    /// Signal that the future is ready.
    #[cfg(feature = "block_on")]
    future_ready: AtomicBool,
}

impl<'l, Data> EventLoop<'l, Data> {
    /// Create a new event loop
    ///
    /// Fails if the initialization of the polling system failed.
    pub fn try_new() -> crate::Result<Self> {
        let poll = Poll::new()?;
        let handle = LoopHandle {
            inner: Rc::new(LoopInner {
                poll: RefCell::new(poll),
                sources: RefCell::new(Slab::new()),
                idles: RefCell::new(Vec::new()),
                pending_action: Cell::new(PostAction::Continue),
                sources_with_additional_lifetime_events: Default::default(),
            }),
        };

        Ok(EventLoop {
            handle,
            signals: Arc::new(Signals {
                stop: AtomicBool::new(false),
                #[cfg(feature = "block_on")]
                future_ready: AtomicBool::new(false),
            }),
            synthetic_events: vec![],
        })
    }

    /// Retrieve a loop handle
    pub fn handle(&self) -> LoopHandle<'l, Data> {
        self.handle.clone()
    }

    fn dispatch_events(
        &mut self,
        mut timeout: Option<Duration>,
        data: &mut Data,
    ) -> crate::Result<()> {
        let now = Instant::now();
        {
            let mut extra_lifecycle_sources = self
                .handle
                .inner
                .sources_with_additional_lifetime_events
                .values
                .borrow_mut();
            for (source, has_event) in &mut *extra_lifecycle_sources {
                *has_event = false;
                if let Some(disp) = self.handle.inner.sources.borrow().get(source.key) {
                    if let Some((readiness, token)) = disp.before_will_sleep()? {
                        // Wake up instantly after polling
                        timeout = Some(Duration::ZERO);
                        self.synthetic_events.push(PollEvent { readiness, token });
                    }
                } else {
                    unreachable!()
                }
            }
        }
        let events = {
            let poll = self.handle.inner.poll.borrow();
            loop {
                let result = poll.poll(timeout);

                match result {
                    Ok(events) => break events,
                    Err(crate::Error::IoError(err)) if err.kind() == io::ErrorKind::Interrupted => {
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
        {
            let mut extra_lifecycle_sources = self
                .handle
                .inner
                .sources_with_additional_lifetime_events
                .values
                .borrow_mut();
            if !extra_lifecycle_sources.is_empty() {
                for (source, has_event) in &mut *extra_lifecycle_sources {
                    for event in &events {
                        *has_event |= (event.token.key & MAX_SOURCES_MASK) == source.key;
                    }
                }
            }
            for (source, has_event) in &*extra_lifecycle_sources {
                if let Some(disp) = self.handle.inner.sources.borrow().get(source.key) {
                    disp.before_handle_events(*has_event);
                } else {
                    unreachable!()
                }
            }
        }

        for event in events {
            // Get the registration token associated with the event.
            let registroken_token = event.token.key & MAX_SOURCES_MASK;

            let opt_disp = self
                .handle
                .inner
                .sources
                .borrow()
                .get(registroken_token)
                .cloned();

            if let Some(disp) = opt_disp {
                let mut ret = disp.process_events(event.readiness, event.token, data)?;

                // if the returned PostAction is Continue, it may be overwritten by an user-specified pending action
                let pending_action = self
                    .handle
                    .inner
                    .pending_action
                    .replace(PostAction::Continue);
                if let PostAction::Continue = ret {
                    ret = pending_action;
                }

                match ret {
                    PostAction::Reregister => {
                        disp.reregister(
                            &mut self.handle.inner.poll.borrow_mut(),
                            self.handle
                                .inner
                                .sources_with_additional_lifetime_events
                                .create_register_for_token(RegistrationToken {
                                    key: registroken_token,
                                }),
                            &mut TokenFactory::new(event.token.key),
                        )?;
                    }
                    PostAction::Disable => {
                        disp.unregister(
                            &mut self.handle.inner.poll.borrow_mut(),
                            self.handle
                                .inner
                                .sources_with_additional_lifetime_events
                                .create_register_for_token(RegistrationToken {
                                    key: registroken_token,
                                }),
                        )?;
                    }
                    PostAction::Remove => {
                        // delete the source from the list, it'll be cleaned up with the if just below
                        self.handle
                            .inner
                            .sources
                            .borrow_mut()
                            .remove(event.token.key);
                    }
                    PostAction::Continue => {}
                }

                if !self
                    .handle
                    .inner
                    .sources
                    .borrow()
                    .contains(registroken_token)
                {
                    // the source has been removed from within its callback, unregister it
                    let mut poll = self.handle.inner.poll.borrow_mut();
                    if let Err(e) = disp.unregister(
                        &mut poll,
                        self.handle
                            .inner
                            .sources_with_additional_lifetime_events
                            .create_register_for_token(RegistrationToken {
                                key: registroken_token,
                            }),
                    ) {
                        log::warn!(
                            "[calloop] Failed to unregister source from the polling system: {:?}",
                            e
                        );
                    }
                }
            } else {
                log::warn!(
                    "[calloop] Received an event for non-existence source: {:?}",
                    event.token.key
                );
            }
        }

        Ok(())
    }

    fn dispatch_idles(&mut self, data: &mut Data) {
        let idles = std::mem::take(&mut *self.handle.inner.idles.borrow_mut());
        for idle in idles {
            idle.borrow_mut().dispatch(data);
        }
    }

    /// Dispatch pending events to their callbacks
    ///
    /// If some sources have events available, their callbacks will be immediatly called.
    /// Otherwise this will wait until an event is receive or the provided `timeout`
    /// is reached. If `timeout` is `None`, it will wait without a duration limit.
    ///
    /// Once pending events have been processed or the timeout is reached, all pending
    /// idle callbacks will be fired before this method returns.
    pub fn dispatch<D: Into<Option<Duration>>>(
        &mut self,
        timeout: D,
        data: &mut Data,
    ) -> crate::Result<()> {
        self.dispatch_events(timeout.into(), data)?;
        self.dispatch_idles(data);

        Ok(())
    }

    /// Get a signal to stop this event loop from running
    ///
    /// To be used in conjunction with the `run()` method.
    pub fn get_signal(&self) -> LoopSignal {
        LoopSignal {
            signal: self.signals.clone(),
            notifier: self.handle.inner.poll.borrow().notifier(),
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
    pub fn run<F, D: Into<Option<Duration>>>(
        &mut self,
        timeout: D,
        data: &mut Data,
        mut cb: F,
    ) -> crate::Result<()>
    where
        F: FnMut(&mut Data),
    {
        let timeout = timeout.into();
        self.signals.stop.store(false, Ordering::Release);
        while !self.signals.stop.load(Ordering::Acquire) {
            self.dispatch(timeout, data)?;
            cb(data);
        }
        Ok(())
    }

    /// Block a future on this event loop.
    ///
    /// This will run the provided future on this event loop, blocking until it is
    /// resolved.
    ///
    /// If [`LoopSignal::stop()`] is called before the future is resolved, this function returns
    /// `None`.
    #[cfg(feature = "block_on")]
    pub fn block_on<R>(
        &mut self,
        future: impl Future<Output = R>,
        data: &mut Data,
        mut cb: impl FnMut(&mut Data),
    ) -> crate::Result<Option<R>> {
        use std::task::{Context, Poll, Wake, Waker};

        /// A waker that will wake up the event loop when it is ready to make progress.
        struct EventLoopWaker(LoopSignal);

        impl Wake for EventLoopWaker {
            fn wake(self: Arc<Self>) {
                // Set the waker.
                self.0.signal.future_ready.store(true, Ordering::Release);
                self.0.notifier.notify().ok();
            }

            fn wake_by_ref(self: &Arc<Self>) {
                // Set the waker.
                self.0.signal.future_ready.store(true, Ordering::Release);
                self.0.notifier.notify().ok();
            }
        }

        // Pin the future to the stack.
        pin_utils::pin_mut!(future);

        // Create a waker that will wake up the event loop when it is ready to make progress.
        let waker = {
            let handle = EventLoopWaker(self.get_signal());

            Waker::from(Arc::new(handle))
        };
        let mut context = Context::from_waker(&waker);

        // Begin running the loop.
        let mut output = None;

        self.signals.stop.store(false, Ordering::Release);
        self.signals.future_ready.store(true, Ordering::Release);

        self.invoke_pre_run(data)?;

        while !self.signals.stop.load(Ordering::Acquire) {
            // If the future is ready to be polled, poll it.
            if self.signals.future_ready.swap(false, Ordering::AcqRel) {
                // Poll the future and break the loop if it's ready.
                if let Poll::Ready(result) = future.as_mut().poll(&mut context) {
                    output = Some(result);
                    break;
                }
            }

            // Otherwise, block on the event loop.
            self.dispatch_events(None, data)?;
            self.dispatch_idles(data);
            cb(data);
        }

        self.invoke_post_run(data)?;
        Ok(output)
    }
}

/// A signal that can be shared between thread to stop or wakeup a running
/// event loop
#[derive(Clone)]
pub struct LoopSignal {
    signal: Arc<Signals>,
    notifier: Notifier,
}

impl std::fmt::Debug for LoopSignal {
    #[cfg_attr(feature = "nightly_coverage", no_coverage)]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LoopSignal { ... }")
    }
}

impl LoopSignal {
    /// Stop the event loop
    ///
    /// Once this method is called, the next time the event loop has finished
    /// waiting for events, it will return rather than starting to wait again.
    ///
    /// This is only useful if you are using the `EventLoop::run()` method.
    pub fn stop(&self) {
        self.signal.stop.store(true, Ordering::Release);
    }

    /// Wake up the event loop
    ///
    /// This sends a dummy event to the event loop to simulate the reception
    /// of an event, making the wait return early. Called after `stop()`, this
    /// ensures the event loop will terminate quickly if you specified a long
    /// timeout (or no timeout at all) to the `dispatch` or `run` method.
    pub fn wakeup(&self) {
        self.notifier.notify().ok();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{
        generic::Generic, ping::*, Dispatcher, Interest, Mode, Poll, PostAction, Readiness,
        RegistrationToken, Token, TokenFactory,
    };

    use super::EventLoop;

    #[test]
    fn dispatch_idle() {
        let mut event_loop = EventLoop::try_new().unwrap();

        let mut dispatched = false;

        event_loop.handle().insert_idle(|d| {
            *d = true;
        });

        event_loop
            .dispatch(Some(Duration::ZERO), &mut dispatched)
            .unwrap();

        assert!(dispatched);
    }

    #[test]
    fn cancel_idle() {
        let mut event_loop = EventLoop::try_new().unwrap();

        let mut dispatched = false;

        let handle = event_loop.handle();
        let idle = handle.insert_idle(move |d| {
            *d = true;
        });

        idle.cancel();

        event_loop
            .dispatch(Duration::ZERO, &mut dispatched)
            .unwrap();

        assert!(!dispatched);
    }

    #[test]
    fn wakeup() {
        let mut event_loop = EventLoop::try_new().unwrap();

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
        let mut event_loop = EventLoop::try_new().unwrap();

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
    fn insert_bad_source() {
        use std::os::unix::io::FromRawFd;

        let event_loop = EventLoop::<()>::try_new().unwrap();
        let fd = unsafe { io_lifetimes::OwnedFd::from_raw_fd(420) };
        let ret = event_loop.handle().insert_source(
            crate::sources::generic::Generic::new(fd, Interest::READ, Mode::Level),
            |_, _, _| Ok(PostAction::Continue),
        );
        assert!(ret.is_err());
    }

    #[test]
    fn insert_source_no_interest() {
        use rustix::pipe::pipe;

        // Create a pipe to get an arbitrary fd.
        let (read, _write) = pipe().unwrap();

        let source = crate::sources::generic::Generic::new(read, Interest::EMPTY, Mode::Level);
        let dispatcher = Dispatcher::new(source, |_, _, _| Ok(PostAction::Continue));

        let event_loop = EventLoop::<()>::try_new().unwrap();
        let handle = event_loop.handle();
        let ret = handle.register_dispatcher(dispatcher.clone());

        if let Ok(token) = ret {
            // Unwrap the dispatcher+source and close the read end.
            handle.remove(token);
        } else {
            // Fail the test.
            panic!();
        }
    }

    #[test]
    fn disarm_rearm() {
        let mut event_loop = EventLoop::<bool>::try_new().unwrap();
        let (ping, ping_source) = make_ping().unwrap();

        let ping_token = event_loop
            .handle()
            .insert_source(ping_source, |(), &mut (), dispatched| {
                *dispatched = true;
            })
            .unwrap();

        ping.ping();
        let mut dispatched = false;
        event_loop
            .dispatch(Duration::ZERO, &mut dispatched)
            .unwrap();
        assert!(dispatched);

        // disable the source
        ping.ping();
        event_loop.handle().disable(&ping_token).unwrap();
        let mut dispatched = false;
        event_loop
            .dispatch(Duration::ZERO, &mut dispatched)
            .unwrap();
        assert!(!dispatched);

        // reenable it, the previous ping now gets dispatched
        event_loop.handle().enable(&ping_token).unwrap();
        let mut dispatched = false;
        event_loop
            .dispatch(Duration::ZERO, &mut dispatched)
            .unwrap();
        assert!(dispatched);
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
            type Error = PingError;

            fn process_events<F>(
                &mut self,
                readiness: Readiness,
                token: Token,
                mut callback: F,
            ) -> Result<PostAction, Self::Error>
            where
                F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
            {
                self.ping1
                    .process_events(readiness, token, |(), &mut ()| callback(1, &mut ()))?;
                self.ping2
                    .process_events(readiness, token, |(), &mut ()| callback(2, &mut ()))?;
                Ok(PostAction::Continue)
            }

            fn register(
                &mut self,
                poll: &mut Poll,
                token_factory: &mut TokenFactory,
            ) -> crate::Result<()> {
                self.ping1.register(poll, token_factory)?;
                self.ping2.register(poll, token_factory)?;
                Ok(())
            }

            fn reregister(
                &mut self,
                poll: &mut Poll,
                token_factory: &mut TokenFactory,
            ) -> crate::Result<()> {
                self.ping1.reregister(poll, token_factory)?;
                self.ping2.reregister(poll, token_factory)?;
                Ok(())
            }

            fn unregister(&mut self, poll: &mut Poll) -> crate::Result<()> {
                self.ping1.unregister(poll)?;
                self.ping2.unregister(poll)?;
                Ok(())
            }
        }

        let mut event_loop = EventLoop::<u32>::try_new().unwrap();

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
            .dispatch(Duration::ZERO, &mut dispatched)
            .unwrap();
        assert_eq!(dispatched, 1);

        dispatched = 0;
        ping2.ping();
        event_loop
            .dispatch(Duration::ZERO, &mut dispatched)
            .unwrap();
        assert_eq!(dispatched, 2);

        dispatched = 0;
        ping1.ping();
        ping2.ping();
        event_loop
            .dispatch(Duration::ZERO, &mut dispatched)
            .unwrap();
        assert_eq!(dispatched, 3);
    }

    #[test]
    fn change_interests() {
        use rustix::io::write;
        use rustix::net::{recv, socketpair, AddressFamily, RecvFlags, SocketFlags, SocketType};
        let mut event_loop = EventLoop::<bool>::try_new().unwrap();

        let (sock1, sock2) = socketpair(
            AddressFamily::UNIX,
            SocketType::STREAM,
            SocketFlags::empty(),
            None, // recv with DONTWAIT will suffice for platforms without SockFlag::SOCK_NONBLOCKING such as macOS
        )
        .unwrap();

        let source = Generic::new(sock1, Interest::READ, Mode::Level);
        let dispatcher = Dispatcher::new(source, |_, fd, dispatched| {
            *dispatched = true;
            // read all contents available to drain the socket
            let mut buf = [0u8; 32];
            loop {
                match recv(&*fd, &mut buf, RecvFlags::DONTWAIT) {
                    Ok(0) => break, // closed pipe, we are now inert
                    Ok(_) => {}
                    Err(e) => {
                        let e: std::io::Error = e.into();
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
            Ok(PostAction::Continue)
        });

        let sock_token_1 = event_loop
            .handle()
            .register_dispatcher(dispatcher.clone())
            .unwrap();

        // first dispatch, nothing is readable
        let mut dispatched = false;
        event_loop
            .dispatch(Duration::ZERO, &mut dispatched)
            .unwrap();
        assert!(!dispatched);

        // write something, the socket becomes readable
        write(&sock2, &[1, 2, 3]).unwrap();
        dispatched = false;
        event_loop
            .dispatch(Duration::ZERO, &mut dispatched)
            .unwrap();
        assert!(dispatched);

        // All has been read, no longer readable
        dispatched = false;
        event_loop
            .dispatch(Duration::ZERO, &mut dispatched)
            .unwrap();
        assert!(!dispatched);

        // change the interests for writability instead
        dispatcher.as_source_mut().interest = Interest::WRITE;
        event_loop.handle().update(&sock_token_1).unwrap();

        // the socket is writable
        dispatched = false;
        event_loop
            .dispatch(Duration::ZERO, &mut dispatched)
            .unwrap();
        assert!(dispatched);

        // change back to readable
        dispatcher.as_source_mut().interest = Interest::READ;
        event_loop.handle().update(&sock_token_1).unwrap();

        // the socket is not readable
        dispatched = false;
        event_loop
            .dispatch(Duration::ZERO, &mut dispatched)
            .unwrap();
        assert!(!dispatched);
    }

    #[test]
    fn kill_source() {
        let mut event_loop = EventLoop::<Option<RegistrationToken>>::try_new().unwrap();

        let handle = event_loop.handle();
        let (ping, ping_source) = make_ping().unwrap();
        let ping_token = event_loop
            .handle()
            .insert_source(ping_source, move |(), &mut (), opt_src| {
                if let Some(src) = opt_src.take() {
                    handle.remove(src);
                }
            })
            .unwrap();

        ping.ping();

        let mut opt_src = Some(ping_token);

        event_loop.dispatch(Duration::ZERO, &mut opt_src).unwrap();

        assert!(opt_src.is_none());
    }

    #[test]
    fn non_static_data() {
        use std::sync::mpsc;

        let (sender, receiver) = mpsc::channel();

        {
            struct RefSender<'a>(&'a mpsc::Sender<()>);
            let mut ref_sender = RefSender(&sender);

            let mut event_loop = EventLoop::<RefSender<'_>>::try_new().unwrap();
            let (ping, ping_source) = make_ping().unwrap();
            let _ping_token = event_loop
                .handle()
                .insert_source(ping_source, |_, _, ref_sender| {
                    ref_sender.0.send(()).unwrap();
                })
                .unwrap();

            ping.ping();

            event_loop
                .dispatch(Duration::ZERO, &mut ref_sender)
                .unwrap();
        }

        receiver.recv().unwrap();
        // sender still usable (e.g. for another EventLoop)
        drop(sender);
    }

    #[cfg(feature = "block_on")]
    #[test]
    fn block_on_test() {
        use crate::sources::timer::TimeoutFuture;
        use std::time::Duration;

        let mut evl = EventLoop::<()>::try_new().unwrap();

        let mut data = 22;
        let timeout = {
            let data = &mut data;
            let evl_handle = evl.handle();

            async move {
                TimeoutFuture::from_duration(&evl_handle, Duration::from_secs(2)).await;
                *data = 32;
                11
            }
        };

        let result = evl.block_on(timeout, &mut (), |&mut ()| {}).unwrap();
        assert_eq!(result, Some(11));
        assert_eq!(data, 32);
    }

    #[cfg(feature = "block_on")]
    #[test]
    fn block_on_early_cancel() {
        use crate::sources::timer;
        use std::time::Duration;

        let mut evl = EventLoop::<()>::try_new().unwrap();

        let mut data = 22;
        let timeout = {
            let data = &mut data;
            let evl_handle = evl.handle();

            async move {
                timer::TimeoutFuture::from_duration(&evl_handle, Duration::from_secs(2)).await;
                *data = 32;
                11
            }
        };

        let timer_source = timer::Timer::from_duration(Duration::from_secs(1));
        let handle = evl.get_signal();
        let _timer_token = evl
            .handle()
            .insert_source(timer_source, move |_, _, _| {
                handle.stop();
                timer::TimeoutAction::Drop
            })
            .unwrap();

        let result = evl.block_on(timeout, &mut (), |&mut ()| {}).unwrap();
        assert_eq!(result, None);
        assert_eq!(data, 22);
    }

    // A dummy EventSource to test insertion and removal of sources
    struct DummySource;

    impl crate::EventSource for DummySource {
        type Event = ();
        type Metadata = ();
        type Ret = ();
        type Error = crate::Error;

        fn process_events<F>(
            &mut self,
            _: Readiness,
            _: Token,
            mut callback: F,
        ) -> Result<PostAction, Self::Error>
        where
            F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
        {
            callback((), &mut ());
            Ok(PostAction::Continue)
        }

        fn register(&mut self, _: &mut Poll, _: &mut TokenFactory) -> crate::Result<()> {
            Ok(())
        }

        fn reregister(&mut self, _: &mut Poll, _: &mut TokenFactory) -> crate::Result<()> {
            Ok(())
        }

        fn unregister(&mut self, _: &mut Poll) -> crate::Result<()> {
            Ok(())
        }
    }
}
