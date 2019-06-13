//! Timer-based event sources
//!
//! A `Timer<T>` is a general time-tracking object. It is used by setting timeouts,
//! and generates events whenever a timeout expires.
//!
//! The `Timer<T>` event source provides an handle `TimerHandle<T>`, which is used
//! to set or cancel timeouts. This handle is cloneable and can be send accross threads
//! if `T: Send`, allowing you to setup timeouts from any point of your program.
//!
//! This implementation is based on
//! [`mio_more::timer`](https://docs.rs/mio-more/*/mio_more/timer/index.html).

use std::cell::RefCell;
use std::io;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mio::{Evented, Poll, PollOpt, Ready, Token};

use mio_extras::timer as mio_timer;

pub use self::mio_timer::Timeout;

use {EventDispatcher, EventSource};

/// A Timer event source
///
/// It generates events of type `(T, TimerHandle<T>)`, providing you
/// an handle inside the event callback, allowing you to set new timeouts
/// as a response to a timeout being reached (for reccuring ticks for example).
pub struct Timer<T> {
    inner: Arc<Mutex<mio_timer::Timer<T>>>,
}

impl<T> Timer<T> {
    /// Create a new timer with default parameters
    ///
    /// Default time resolution is 100ms
    pub fn new() -> Timer<T> {
        Timer {
            inner: Arc::new(Mutex::new(mio_timer::Builder::default().build())),
        }
    }

    /// Create a new timer with a specific time resolution
    pub fn with_resolution(resolution: Duration) -> Timer<T> {
        Timer {
            inner: Arc::new(Mutex::new(
                mio_timer::Builder::default()
                    .tick_duration(resolution)
                    .build(),
            )),
        }
    }

    /// Get an handle for this timer
    pub fn handle(&self) -> TimerHandle<T> {
        TimerHandle {
            inner: self.inner.clone(),
        }
    }
}

/// An handle to a timer, used to set or cancel timeouts
///
/// This handle can be cloned, and can be sent accross thread as long
/// as `T: Send`.
pub struct TimerHandle<T> {
    inner: Arc<Mutex<mio_timer::Timer<T>>>,
}

// Manual impl of `Clone` as #[derive(Clone)] adds a `T: Clone` bound
impl<T> Clone for TimerHandle<T> {
    fn clone(&self) -> TimerHandle<T> {
        TimerHandle {
            inner: self.inner.clone(),
        }
    }
}

impl<T> TimerHandle<T> {
    /// Set a new timeout
    ///
    /// The associated `data` will be given as argument to the callback.
    ///
    /// The returned `Timeout` can be used to cancel it. You can drop it if you don't
    /// plan to cancel this timeout.
    ///
    /// This method can fail if the timer already has too many pending timeouts, currently
    /// capacity is `2^16`.
    pub fn add_timeout(&self, delay_from_now: Duration, data: T) -> Timeout {
        self.inner.lock().unwrap().set_timeout(delay_from_now, data)
    }

    /// Cancel a previsouly set timeout and retrieve the associated data
    ///
    /// This method returns `None` if the timeout does not exist (it has already fired
    /// or has already been cancelled).
    pub fn cancel_timeout(&self, timeout: &Timeout) -> Option<T> {
        self.inner.lock().unwrap().cancel_timeout(timeout)
    }
}

impl<T> Evented for Timer<T> {
    fn register(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        self.inner
            .lock()
            .unwrap()
            .register(poll, token, interest, opts)
    }

    fn reregister(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        self.inner
            .lock()
            .unwrap()
            .reregister(poll, token, interest, opts)
    }

    fn deregister(&self, poll: &Poll) -> io::Result<()> {
        self.inner.lock().unwrap().deregister(poll)
    }
}

impl<T: 'static> EventSource for Timer<T> {
    type Event = (T, TimerHandle<T>);

    fn interest(&self) -> Ready {
        Ready::readable()
    }

    fn pollopts(&self) -> PollOpt {
        PollOpt::edge()
    }

    fn make_dispatcher<Data: 'static, F: FnMut((T, TimerHandle<T>), &mut Data) + 'static>(
        &self,
        callback: F,
    ) -> Rc<RefCell<EventDispatcher<Data>>> {
        Rc::new(RefCell::new(Dispatcher {
            _data: ::std::marker::PhantomData,
            timer: self.inner.clone(),
            callback,
        }))
    }
}

struct Dispatcher<Data, T, F: FnMut((T, TimerHandle<T>), &mut Data)> {
    _data: ::std::marker::PhantomData<fn(&mut Data)>,
    timer: Arc<Mutex<mio_timer::Timer<T>>>,
    callback: F,
}

impl<Data, T, F: FnMut((T, TimerHandle<T>), &mut Data)> EventDispatcher<Data>
    for Dispatcher<Data, T, F>
{
    fn ready(&mut self, _: Ready, data: &mut Data) {
        let handle = TimerHandle {
            inner: self.timer.clone(),
        };
        loop {
            let opt_evt = self.timer.lock().unwrap().poll();
            match opt_evt {
                Some(val) => (self.callback)((val, handle.clone()), data),
                None => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::time::Duration;

    use super::*;

    #[test]
    fn single_timer() {
        let mut event_loop = ::EventLoop::new().unwrap();

        let evl_handle = event_loop.handle();

        let mut fired = false;

        let timer = evl_handle
            .insert_source(Timer::<()>::new(), move |((), _), f| {
                *f = true;
            })
            .map_err(Into::<io::Error>::into)
            .unwrap();

        timer.handle().add_timeout(Duration::from_millis(300), ());

        event_loop
            .dispatch(Some(::std::time::Duration::from_millis(100)), &mut fired)
            .unwrap();

        // it should not have fired yet
        assert!(!fired);

        event_loop
            .dispatch(Some(::std::time::Duration::from_millis(300)), &mut fired)
            .unwrap();

        // it should have fired now
        assert!(fired);
    }

    #[test]
    fn multi_timout_order() {
        let mut event_loop = ::EventLoop::new().unwrap();

        let evl_handle = event_loop.handle();

        let mut fired = Vec::new();

        let timer = evl_handle
            .insert_source(Timer::new(), |(val, _), fired: &mut Vec<u32>| {
                fired.push(val);
            })
            .map_err(Into::<io::Error>::into)
            .unwrap();

        timer.handle().add_timeout(Duration::from_millis(300), 1);
        timer.handle().add_timeout(Duration::from_millis(100), 2);
        timer.handle().add_timeout(Duration::from_millis(600), 3);

        // 3 dispatches as each returns once at least one event occured

        event_loop
            .dispatch(Some(::std::time::Duration::from_millis(200)), &mut fired)
            .unwrap();

        assert_eq!(&fired, &[2]);

        event_loop
            .dispatch(Some(::std::time::Duration::from_millis(300)), &mut fired)
            .unwrap();

        assert_eq!(&fired, &[2, 1]);

        event_loop
            .dispatch(Some(::std::time::Duration::from_millis(400)), &mut fired)
            .unwrap();

        assert_eq!(&fired, &[2, 1, 3]);
    }

    #[test]
    fn timer_cancel() {
        let mut event_loop = ::EventLoop::new().unwrap();

        let evl_handle = event_loop.handle();

        let mut fired = Vec::new();

        let timer = evl_handle
            .insert_source(Timer::new(), |(val, _), fired: &mut Vec<u32>| {
                fired.push(val)
            })
            .map_err(Into::<io::Error>::into)
            .unwrap();

        let timeout1 = timer.handle().add_timeout(Duration::from_millis(300), 1);
        let timeout2 = timer.handle().add_timeout(Duration::from_millis(100), 2);
        let timeout3 = timer.handle().add_timeout(Duration::from_millis(600), 3);

        // 3 dispatches as each returns once at least one event occured
        //
        // The timeouts 1 and 3 and not cancelled right away, but still before they
        // fire

        event_loop
            .dispatch(Some(::std::time::Duration::from_millis(200)), &mut fired)
            .unwrap();

        assert_eq!(&fired, &[2]);

        // timeout2 has already fired, we cancel timeout1
        assert_eq!(timer.handle().cancel_timeout(&timeout2), None);
        assert_eq!(timer.handle().cancel_timeout(&timeout1), Some(1));

        event_loop
            .dispatch(Some(::std::time::Duration::from_millis(300)), &mut fired)
            .unwrap();

        assert_eq!(&fired, &[2]);

        // cancel timeout3
        assert_eq!(timer.handle().cancel_timeout(&timeout3), Some(3));

        event_loop
            .dispatch(Some(::std::time::Duration::from_millis(600)), &mut fired)
            .unwrap();

        assert_eq!(&fired, &[2]);
    }
}
