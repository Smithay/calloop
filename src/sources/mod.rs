use std::{
    cell::{Ref, RefCell, RefMut},
    ops::{BitOr, BitOrAssign},
    rc::Rc,
};

use crate::{sys::TokenFactory, Poll, Readiness, Token};

pub mod channel;
#[cfg(feature = "executor")]
#[cfg_attr(docsrs, doc(cfg(feature = "executor")))]
pub mod futures;
pub mod generic;
pub mod ping;
#[cfg(target_os = "linux")]
#[cfg_attr(docsrs, doc(cfg(target_os = "linux")))]
pub mod signals;
#[cfg(target_os = "linux")]
#[cfg_attr(docsrs, doc(cfg(target_os = "linux")))]
pub mod socket;
pub mod timer;
pub mod transient;

/// Possible actions that can be requested to the event loop by an
/// event source once its events have been processed.
///
/// `PostAction` values can be combined with the `|` (bit-or) operator (or with
/// `|=`) with the result that:
/// - if both values are identical, the result is that value
/// - if they are different, the result is [`Reregister`](PostAction::Reregister)
///
/// Bit-or-ing these results is useful for composed sources to combine the
/// results of their child sources, but note that it only applies to the child
/// sources. For example, if every child source returns `Continue`, the result
/// will be `Continue`, but the parent source might still need to return
/// `Reregister` or something else depending on any additional logic it uses.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PostAction {
    /// Continue listening for events on this source as before
    Continue,
    /// Trigger a re-registration of this source
    Reregister,
    /// Disable this source
    ///
    /// Has the same effect as [`LoopHandle::disable`](crate::LoopHandle#method.disable)
    Disable,
    /// Remove this source from the eventloop
    ///
    /// Has the same effect as [`LoopHandle::kill`](crate::LoopHandle#method.kill)
    Remove,
}

/// Combines `PostAction` values returned from nested event sources.
impl BitOr for PostAction {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        if matches!(self, x if x == rhs) {
            self
        } else {
            Self::Reregister
        }
    }
}

/// Combines `PostAction` values returned from nested event sources.
impl BitOrAssign for PostAction {
    fn bitor_assign(&mut self, rhs: Self) {
        if *self != rhs {
            *self = Self::Reregister;
        }
    }
}

/// Trait representing an event source
///
/// This is the trait you need to implement if you wish to create your own
/// calloop-compatible event sources.
///
/// The 3 associated types define the type of closure the user will need to
/// provide to process events for your event source.
///
/// The `process_events` method will be called when one of the FD you registered
/// is ready, with the associated readiness and token.
///
/// The `register`, `reregister` and `unregister` methods are plumbing to let your
/// source register itself with the polling system. See their documentation for details.
///
/// In case your event source needs to do some special processing before or after a
/// polling session occurs (to prepare the underlying source for polling, and cleanup
/// after that), you can override the `pre_run` and `post_run`, that do nothing by
/// default. Depending on the underlying events, `process_events` may be invoked once,
/// several times, or none at all between `pre_run` and `post_run` are called, but when
/// it is invoked, it'll always be between those two.
pub trait EventSource {
    /// The type of events generated by your source.
    type Event;
    /// Some metadata of your event source
    ///
    /// This is typically useful if your source contains some internal state that
    /// the user may need to interact with when processing events. The user callback
    /// will receive a `&mut Metadata` reference.
    ///
    /// Set to `()` if not needed.
    type Metadata;
    /// The return type of the user callback
    ///
    /// If the user needs to return some value back to your event source once its
    /// processing is finshed (to indicate success or failure for example), you can
    /// specify it using this type.
    ///
    /// Set to `()` if not needed.
    type Ret;
    /// The error type returned from
    /// [`process_events()`](Self::process_events()) (not the user callback!).
    type Error: Into<Box<dyn std::error::Error + Sync + Send>>;

    /// Process any relevant events
    ///
    /// This method will be called every time one of the FD you registered becomes
    /// ready, including the readiness details and the associated token.
    ///
    /// Your event source will then do some processing of the file descriptor(s) to generate
    /// events, and call the provided `callback` for each one of them.
    ///
    /// You should ensure you drained the file descriptors of their events, especially if using
    /// edge-triggered mode.
    fn process_events<F>(
        &mut self,
        readiness: Readiness,
        token: Token,
        callback: F,
    ) -> Result<PostAction, Self::Error>
    where
        F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret;

    /// Register yourself to this poll instance
    ///
    /// You should register all your relevant file descriptors to the provided [`Poll`](crate::Poll)
    /// using its [`Poll::register`](crate::Poll#method.register) method.
    ///
    /// If you need to register more than one file descriptor, you can change the
    /// `sub_id` field of the [`Token`](crate::Token) to differentiate between them.
    fn register(&mut self, poll: &mut Poll, token_factory: &mut TokenFactory) -> crate::Result<()>;

    /// Re-register your file descriptors
    ///
    /// Your should update the registration of all your relevant file descriptor to
    /// the provided [`Poll`](crate::Poll) using its [`Poll::reregister`](crate::Poll#method.reregister),
    /// if necessary.
    fn reregister(
        &mut self,
        poll: &mut Poll,
        token_factory: &mut TokenFactory,
    ) -> crate::Result<()>;

    /// Unregister your file descriptors
    ///
    /// You should unregister all your file descriptors from this [`Poll`](crate::Poll) using its
    /// [`Poll::unregister`](crate::Poll#method.unregister) method.
    fn unregister(&mut self, poll: &mut Poll) -> crate::Result<()>;

    /// Notification that a polling session is going to start
    ///
    /// You can generate events from this method as you would from `process_events`.
    fn pre_run<F>(&mut self, _callback: F) -> crate::Result<()>
    where
        F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
    {
        Ok(())
    }

    /// Notification that the current polling session ended
    ///
    /// You can generate events from this method as you would from `process_events`.
    fn post_run<F>(&mut self, _callback: F) -> crate::Result<()>
    where
        F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
    {
        Ok(())
    }
}

/// Blanket implementation for boxed event sources. [`EventSource`] is not an
/// object safe trait, so this does not include trait objects.
impl<T: EventSource> EventSource for Box<T> {
    type Event = T::Event;
    type Metadata = T::Metadata;
    type Ret = T::Ret;
    type Error = T::Error;

    fn process_events<F>(
        &mut self,
        readiness: Readiness,
        token: Token,
        callback: F,
    ) -> Result<PostAction, Self::Error>
    where
        F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
    {
        T::process_events(&mut **self, readiness, token, callback)
    }

    fn register(&mut self, poll: &mut Poll, token_factory: &mut TokenFactory) -> crate::Result<()> {
        T::register(&mut **self, poll, token_factory)
    }

    fn reregister(
        &mut self,
        poll: &mut Poll,
        token_factory: &mut TokenFactory,
    ) -> crate::Result<()> {
        T::reregister(&mut **self, poll, token_factory)
    }

    fn unregister(&mut self, poll: &mut Poll) -> crate::Result<()> {
        T::unregister(&mut **self, poll)
    }

    fn pre_run<F>(&mut self, callback: F) -> crate::Result<()>
    where
        F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
    {
        T::pre_run(&mut **self, callback)
    }

    fn post_run<F>(&mut self, callback: F) -> crate::Result<()>
    where
        F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
    {
        T::post_run(&mut **self, callback)
    }
}

/// Blanket implementation for exclusive references to event sources.
/// [`EventSource`] is not an object safe trait, so this does not include trait
/// objects.
impl<T: EventSource> EventSource for &mut T {
    type Event = T::Event;
    type Metadata = T::Metadata;
    type Ret = T::Ret;
    type Error = T::Error;

    fn process_events<F>(
        &mut self,
        readiness: Readiness,
        token: Token,
        callback: F,
    ) -> Result<PostAction, Self::Error>
    where
        F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
    {
        T::process_events(&mut **self, readiness, token, callback)
    }

    fn register(&mut self, poll: &mut Poll, token_factory: &mut TokenFactory) -> crate::Result<()> {
        T::register(&mut **self, poll, token_factory)
    }

    fn reregister(
        &mut self,
        poll: &mut Poll,
        token_factory: &mut TokenFactory,
    ) -> crate::Result<()> {
        T::reregister(&mut **self, poll, token_factory)
    }

    fn unregister(&mut self, poll: &mut Poll) -> crate::Result<()> {
        T::unregister(&mut **self, poll)
    }

    fn pre_run<F>(&mut self, callback: F) -> crate::Result<()>
    where
        F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
    {
        T::pre_run(&mut **self, callback)
    }

    fn post_run<F>(&mut self, callback: F) -> crate::Result<()>
    where
        F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
    {
        T::post_run(&mut **self, callback)
    }
}

pub(crate) struct DispatcherInner<S, F> {
    source: S,
    callback: F,
}

impl<Data, S, F> EventDispatcher<Data> for RefCell<DispatcherInner<S, F>>
where
    S: EventSource,
    F: FnMut(S::Event, &mut S::Metadata, &mut Data) -> S::Ret,
{
    fn process_events(
        &self,
        readiness: Readiness,
        token: Token,
        data: &mut Data,
    ) -> crate::Result<PostAction> {
        let mut disp = self.borrow_mut();
        let DispatcherInner {
            ref mut source,
            ref mut callback,
        } = *disp;
        source
            .process_events(readiness, token, |event, meta| callback(event, meta, data))
            .map_err(|e| crate::Error::OtherError(e.into()))
    }

    fn register(&self, poll: &mut Poll, token_factory: &mut TokenFactory) -> crate::Result<()> {
        self.borrow_mut().source.register(poll, token_factory)
    }

    fn reregister(&self, poll: &mut Poll, token_factory: &mut TokenFactory) -> crate::Result<bool> {
        if let Ok(mut me) = self.try_borrow_mut() {
            me.source.reregister(poll, token_factory)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn unregister(&self, poll: &mut Poll) -> crate::Result<bool> {
        if let Ok(mut me) = self.try_borrow_mut() {
            me.source.unregister(poll)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn pre_run(&self, data: &mut Data) -> crate::Result<()> {
        let mut disp = self.borrow_mut();
        let DispatcherInner {
            ref mut source,
            ref mut callback,
        } = *disp;
        source.pre_run(|event, meta| callback(event, meta, data))
    }

    fn post_run(&self, data: &mut Data) -> crate::Result<()> {
        let mut disp = self.borrow_mut();
        let DispatcherInner {
            ref mut source,
            ref mut callback,
        } = *disp;
        source.post_run(|event, meta| callback(event, meta, data))
    }
}

pub(crate) trait EventDispatcher<Data> {
    fn process_events(
        &self,
        readiness: Readiness,
        token: Token,
        data: &mut Data,
    ) -> crate::Result<PostAction>;

    fn register(&self, poll: &mut Poll, token_factory: &mut TokenFactory) -> crate::Result<()>;

    fn reregister(&self, poll: &mut Poll, token_factory: &mut TokenFactory) -> crate::Result<bool>;

    fn unregister(&self, poll: &mut Poll) -> crate::Result<bool>;

    fn pre_run(&self, data: &mut Data) -> crate::Result<()>;

    fn post_run(&self, data: &mut Data) -> crate::Result<()>;
}

// An internal trait to erase the `F` type parameter of `DispatcherInner`
trait ErasedDispatcher<'a, S, Data> {
    fn as_source_ref(&self) -> Ref<S>;
    fn as_source_mut(&self) -> RefMut<S>;
    fn into_source_inner(self: Rc<Self>) -> S;
    fn into_event_dispatcher(self: Rc<Self>) -> Rc<dyn EventDispatcher<Data> + 'a>;
}

impl<'a, S, Data, F> ErasedDispatcher<'a, S, Data> for RefCell<DispatcherInner<S, F>>
where
    S: EventSource + 'a,
    F: FnMut(S::Event, &mut S::Metadata, &mut Data) -> S::Ret + 'a,
{
    fn as_source_ref(&self) -> Ref<S> {
        Ref::map(self.borrow(), |inner| &inner.source)
    }

    fn as_source_mut(&self) -> RefMut<S> {
        RefMut::map(self.borrow_mut(), |inner| &mut inner.source)
    }

    fn into_source_inner(self: Rc<Self>) -> S {
        if let Ok(ref_cell) = Rc::try_unwrap(self) {
            ref_cell.into_inner().source
        } else {
            panic!("Dispatcher is still registered");
        }
    }

    fn into_event_dispatcher(self: Rc<Self>) -> Rc<dyn EventDispatcher<Data> + 'a>
    where
        S: 'a,
    {
        self as Rc<dyn EventDispatcher<Data> + 'a>
    }
}

/// An event source with its callback.
///
/// The `Dispatcher` can be registered in an event loop.
/// Use the `as_source_{ref,mut}` functions to interact with the event source.
/// Use `into_source_inner` to get the event source back.
pub struct Dispatcher<'a, S, Data>(Rc<dyn ErasedDispatcher<'a, S, Data> + 'a>);

impl<'a, S, Data> std::fmt::Debug for Dispatcher<'a, S, Data> {
    #[cfg_attr(coverage, no_coverage)]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Dispatcher { ... }")
    }
}

impl<'a, S, Data> Dispatcher<'a, S, Data>
where
    S: EventSource + 'a,
{
    /// Builds a dispatcher.
    ///
    /// The resulting `Dispatcher`
    pub fn new<F>(source: S, callback: F) -> Self
    where
        F: FnMut(S::Event, &mut S::Metadata, &mut Data) -> S::Ret + 'a,
    {
        Dispatcher(Rc::new(RefCell::new(DispatcherInner { source, callback })))
    }

    /// Returns an immutable reference to the event source.
    ///
    /// # Panics
    ///
    /// Has the same semantics as `RefCell::borrow()`.
    ///
    /// The dispatcher being mutably borrowed while its events are dispatched,
    /// this method will panic if invoked from within the associated dispatching closure.
    pub fn as_source_ref(&self) -> Ref<S> {
        self.0.as_source_ref()
    }

    /// Returns a mutable reference to the event source.
    ///
    /// # Panics
    ///
    /// Has the same semantics as `RefCell::borrow_mut()`.
    ///
    /// The dispatcher being mutably borrowed while its events are dispatched,
    /// this method will panic if invoked from within the associated dispatching closure.
    pub fn as_source_mut(&self) -> RefMut<S> {
        self.0.as_source_mut()
    }

    /// Consumes the Dispatcher and returns the inner event source.
    ///
    /// # Panics
    ///
    /// Panics if the `Dispatcher` is still registered.
    pub fn into_source_inner(self) -> S {
        self.0.into_source_inner()
    }

    pub(crate) fn clone_as_event_dispatcher(&self) -> Rc<dyn EventDispatcher<Data> + 'a> {
        Rc::clone(&self.0).into_event_dispatcher()
    }
}

impl<'a, S, Data> Clone for Dispatcher<'a, S, Data> {
    fn clone(&self) -> Dispatcher<'a, S, Data> {
        Dispatcher(Rc::clone(&self.0))
    }
}

/// An idle callback that was inserted in this loop
///
/// This handle allows you to cancel the callback. Dropping
/// it will *not* cancel it.
pub struct Idle<'i> {
    pub(crate) callback: Rc<RefCell<dyn CancellableIdle + 'i>>,
}

impl<'i> std::fmt::Debug for Idle<'i> {
    #[cfg_attr(coverage, no_coverage)]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Idle { ... }")
    }
}

impl<'i> Idle<'i> {
    /// Cancel the idle callback if it was not already run
    pub fn cancel(self) {
        self.callback.borrow_mut().cancel();
    }
}

pub(crate) trait CancellableIdle {
    fn cancel(&mut self);
}

impl<F> CancellableIdle for Option<F> {
    fn cancel(&mut self) {
        self.take();
    }
}

pub(crate) trait IdleDispatcher<Data> {
    fn dispatch(&mut self, data: &mut Data);
}

impl<Data, F> IdleDispatcher<Data> for Option<F>
where
    F: FnMut(&mut Data),
{
    fn dispatch(&mut self, data: &mut Data) {
        if let Some(callabck) = self.as_mut() {
            callabck(data);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{ping::make_ping, EventLoop};

    // Test event source boxing.
    #[test]
    fn test_boxed_source() {
        let mut fired = false;

        let (pinger, source) = make_ping().unwrap();
        let boxed = Box::new(source);

        let mut event_loop = EventLoop::try_new().unwrap();
        let handle = event_loop.handle();

        let token = handle
            .insert_source(boxed, |_, _, fired| *fired = true)
            .unwrap();

        pinger.ping();

        event_loop
            .dispatch(Duration::new(0, 0), &mut fired)
            .unwrap();

        assert!(fired);
        fired = false;

        handle.update(&token).unwrap();

        pinger.ping();

        event_loop
            .dispatch(Duration::new(0, 0), &mut fired)
            .unwrap();

        assert!(fired);
        fired = false;

        handle.remove(token);

        event_loop
            .dispatch(Duration::new(0, 0), &mut fired)
            .unwrap();

        assert!(!fired);
    }

    // Test event source trait methods via mut ref.
    #[test]
    fn test_mut_ref_source() {
        let mut fired = false;

        let (pinger, mut source) = make_ping().unwrap();
        let source_ref = &mut source;

        let mut event_loop = EventLoop::try_new().unwrap();
        let handle = event_loop.handle();

        let token = handle
            .insert_source(source_ref, |_, _, fired| *fired = true)
            .unwrap();

        pinger.ping();

        event_loop
            .dispatch(Duration::new(0, 0), &mut fired)
            .unwrap();

        assert!(fired);
        fired = false;

        handle.update(&token).unwrap();

        pinger.ping();

        event_loop
            .dispatch(Duration::new(0, 0), &mut fired)
            .unwrap();

        assert!(fired);
        fired = false;

        handle.remove(token);

        event_loop
            .dispatch(Duration::new(0, 0), &mut fired)
            .unwrap();

        assert!(!fired);
    }

    // Test PostAction combinations.
    #[test]
    fn post_action_combine() {
        use super::PostAction::*;
        assert_eq!(Continue | Continue, Continue);
        assert_eq!(Continue | Reregister, Reregister);
        assert_eq!(Continue | Disable, Reregister);
        assert_eq!(Continue | Remove, Reregister);

        assert_eq!(Reregister | Continue, Reregister);
        assert_eq!(Reregister | Reregister, Reregister);
        assert_eq!(Reregister | Disable, Reregister);
        assert_eq!(Reregister | Remove, Reregister);

        assert_eq!(Disable | Continue, Reregister);
        assert_eq!(Disable | Reregister, Reregister);
        assert_eq!(Disable | Disable, Disable);
        assert_eq!(Disable | Remove, Reregister);

        assert_eq!(Remove | Continue, Reregister);
        assert_eq!(Remove | Reregister, Reregister);
        assert_eq!(Remove | Disable, Reregister);
        assert_eq!(Remove | Remove, Remove);
    }

    // Test PostAction self-assignment.
    #[test]
    fn post_action_combine_assign() {
        use super::PostAction::*;

        let mut action = Continue;
        action |= Continue;
        assert_eq!(action, Continue);

        let mut action = Continue;
        action |= Reregister;
        assert_eq!(action, Reregister);

        let mut action = Continue;
        action |= Disable;
        assert_eq!(action, Reregister);

        let mut action = Continue;
        action |= Remove;
        assert_eq!(action, Reregister);

        let mut action = Reregister;
        action |= Continue;
        assert_eq!(action, Reregister);

        let mut action = Reregister;
        action |= Reregister;
        assert_eq!(action, Reregister);

        let mut action = Reregister;
        action |= Disable;
        assert_eq!(action, Reregister);

        let mut action = Reregister;
        action |= Remove;
        assert_eq!(action, Reregister);

        let mut action = Disable;
        action |= Continue;
        assert_eq!(action, Reregister);

        let mut action = Disable;
        action |= Reregister;
        assert_eq!(action, Reregister);

        let mut action = Disable;
        action |= Disable;
        assert_eq!(action, Disable);

        let mut action = Disable;
        action |= Remove;
        assert_eq!(action, Reregister);

        let mut action = Remove;
        action |= Continue;
        assert_eq!(action, Reregister);

        let mut action = Remove;
        action |= Reregister;
        assert_eq!(action, Reregister);

        let mut action = Remove;
        action |= Disable;
        assert_eq!(action, Reregister);

        let mut action = Remove;
        action |= Remove;
        assert_eq!(action, Remove);
    }
}
