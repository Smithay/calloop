//! Timer-based event sources
//!
//! A [`Timer<T>`](Timer) is a general time-tracking object. It is used by setting timeouts,
//! and generates events whenever a timeout expires.
//!
//! The [`Timer<T>`](Timer) event source provides an handle [`TimerHandle<T>`](TimerHandle), which
//! is used to set or cancel timeouts. This handle is cloneable and can be sent accross threads
//! if `T: Send`, allowing you to setup timeouts from any point of your program.

use std::cell::RefCell;
use std::collections::BinaryHeap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::{EventSource, Poll, PostAction, Readiness, Token, TokenFactory};

#[cfg(target_os = "linux")]
mod timerfd;
#[cfg(target_os = "linux")]
use timerfd::{TimerScheduler, TimerSource};

#[cfg(any(
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "macos"
))]
mod threaded;

#[cfg(any(
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "macos"
))]
use threaded::{TimerScheduler, TimerSource};

/// An error arising from processing events for a timer.
#[derive(thiserror::Error, Debug)]
#[error(transparent)]
pub struct TimerError(Box<dyn std::error::Error + Sync + Send>);

/// A Timer event source
///
/// It generates events of type `(T, TimerHandle<T>)`, providing you
/// an handle inside the event callback, allowing you to set new timeouts
/// as a response to a timeout being reached (for reccuring ticks for example).
#[derive(Debug)]
pub struct Timer<T> {
    inner: Arc<Mutex<TimerInner<T>>>,
    source: TimerSource,
}

impl<T> Timer<T> {
    /// Create a new timer
    pub fn new() -> crate::Result<Timer<T>> {
        let (scheduler, source) = TimerScheduler::new()?;
        let inner = TimerInner::new(scheduler);
        Ok(Timer {
            inner: Arc::new(Mutex::new(inner)),
            source,
        })
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
#[derive(Debug)]
pub struct TimerHandle<T> {
    inner: Arc<Mutex<TimerInner<T>>>,
}

// Manual impl of `Clone` as #[derive(Clone)] adds a `T: Clone` bound
#[cfg(not(tarpaulin_include))]
impl<T> Clone for TimerHandle<T> {
    fn clone(&self) -> TimerHandle<T> {
        TimerHandle {
            inner: self.inner.clone(),
        }
    }
}

/// An itentifier to cancel a timeout if necessary
#[derive(Debug)]
pub struct Timeout {
    counter: u32,
}

impl<T> TimerHandle<T> {
    /// Set a new timeout
    ///
    /// The associated `data` will be given as argument to the callback.
    ///
    /// The returned `Timeout` can be used to cancel it. You can drop it if you don't
    /// plan to cancel this timeout.
    pub fn add_timeout(&self, delay_from_now: Duration, data: T) -> Timeout {
        self.inner
            .lock()
            .unwrap()
            .insert(Instant::now() + delay_from_now, data)
    }

    /// Cancel a previsouly set timeout and retrieve the associated data
    ///
    /// This method returns `None` if the timeout does not exist (it has already fired
    /// or has already been cancelled).
    pub fn cancel_timeout(&self, timeout: &Timeout) -> Option<T> {
        self.inner.lock().unwrap().cancel(timeout)
    }

    /// Cancel all planned timeouts for this timer
    ///
    /// All associated data will be dropped.
    pub fn cancel_all_timeouts(&self) {
        self.inner.lock().unwrap().cancel_all();
    }
}

impl<T> EventSource for Timer<T> {
    type Event = T;
    type Metadata = TimerHandle<T>;
    type Ret = ();
    type Error = TimerError;

    fn process_events<C>(
        &mut self,
        readiness: Readiness,
        token: Token,
        mut callback: C,
    ) -> Result<PostAction, Self::Error>
    where
        C: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
    {
        let mut handle = TimerHandle {
            inner: self.inner.clone(),
        };
        let inner = &self.inner;
        self.source.process_events(readiness, token, |(), &mut ()| {
            loop {
                let next_expired: Option<T> = {
                    let mut guard = inner.lock().unwrap();
                    guard.next_expired()
                };
                if let Some(val) = next_expired {
                    callback(val, &mut handle);
                } else {
                    break;
                }
            }
            // now compute the next timeout and signal if necessary
            inner.lock().unwrap().reschedule();
        })
    }

    fn register(&mut self, poll: &mut Poll, token_factory: &mut TokenFactory) -> crate::Result<()> {
        self.source.register(poll, token_factory)
    }

    fn reregister(
        &mut self,
        poll: &mut Poll,
        token_factory: &mut TokenFactory,
    ) -> crate::Result<()> {
        self.source.reregister(poll, token_factory)
    }

    fn unregister(&mut self, poll: &mut Poll) -> crate::Result<()> {
        self.source.unregister(poll)
    }
}

/*
 * Timer logic
 */

#[derive(Debug)]
struct TimeoutData<T> {
    deadline: Instant,
    data: RefCell<Option<T>>,
    counter: u32,
}

#[derive(Debug)]
struct TimerInner<T> {
    heap: BinaryHeap<TimeoutData<T>>,
    scheduler: TimerScheduler,
    counter: u32,
}

impl<T> TimerInner<T> {
    fn new(scheduler: TimerScheduler) -> TimerInner<T> {
        TimerInner {
            heap: BinaryHeap::new(),
            scheduler,
            counter: 0,
        }
    }

    fn insert(&mut self, deadline: Instant, value: T) -> Timeout {
        self.heap.push(TimeoutData {
            deadline,
            data: RefCell::new(Some(value)),
            counter: self.counter,
        });
        let ret = Timeout {
            counter: self.counter,
        };
        self.counter += 1;
        self.reschedule();
        ret
    }

    fn cancel(&mut self, timeout: &Timeout) -> Option<T> {
        for data in self.heap.iter() {
            if data.counter == timeout.counter {
                let udata = data.data.borrow_mut().take();
                self.reschedule();
                return udata;
            }
        }
        None
    }

    fn cancel_all(&mut self) {
        self.heap.clear();
        self.reschedule();
    }

    fn next_expired(&mut self) -> Option<T> {
        let now = Instant::now();
        loop {
            // check if there is an expired item
            if let Some(data) = self.heap.peek() {
                if data.deadline > now {
                    return None;
                }
            // there is an expired timeout, continue the
            // loop body
            } else {
                return None;
            }

            // There is an item in the heap, this unwrap cannot blow
            let data = self.heap.pop().unwrap();
            if let Some(val) = data.data.into_inner() {
                return Some(val);
            }
            // otherwise this timeout was cancelled, continue looping
        }
    }

    fn reschedule(&mut self) {
        if let Some(next_deadline) = self.heap.peek().map(|data| data.deadline) {
            self.scheduler.reschedule(next_deadline);
        } else {
            self.scheduler.deschedule();
        }
    }
}

// trait implementations for TimeoutData

impl<T> std::cmp::Ord for TimeoutData<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // earlier values have priority
        self.deadline.cmp(&other.deadline).reverse()
    }
}

impl<T> std::cmp::PartialOrd for TimeoutData<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        // earlier values have priority
        Some(self.deadline.cmp(&other.deadline).reverse())
    }
}

impl<T> std::cmp::PartialEq for TimeoutData<T> {
    fn eq(&self, other: &Self) -> bool {
        // earlier values have priority
        self.deadline == other.deadline
    }
}

impl<T> std::cmp::Eq for TimeoutData<T> {}

/*
 * Tests
 */

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn single_timer() {
        let mut event_loop = crate::EventLoop::try_new().unwrap();

        let evl_handle = event_loop.handle();

        let mut fired = false;

        let timer = Timer::<()>::new().unwrap();
        let timer_handle = timer.handle();
        evl_handle
            .insert_source(timer, move |(), _, f| {
                *f = true;
            })
            .unwrap();

        timer_handle.add_timeout(Duration::from_millis(300), ());

        event_loop
            .dispatch(Some(::std::time::Duration::from_millis(100)), &mut fired)
            .unwrap();

        // it should not have fired yet
        assert!(!fired);

        event_loop.dispatch(None, &mut fired).unwrap();

        // it should have fired now
        assert!(fired);
    }

    #[test]
    fn multi_timout_order() {
        let mut event_loop = crate::EventLoop::try_new().unwrap();

        let evl_handle = event_loop.handle();

        let mut fired = Vec::new();

        let timer = Timer::<u32>::new().unwrap();
        let timer_handle = timer.handle();

        evl_handle
            .insert_source(timer, |val, _, fired: &mut Vec<u32>| {
                fired.push(val);
            })
            .unwrap();

        timer_handle.add_timeout(Duration::from_millis(300), 1);
        timer_handle.add_timeout(Duration::from_millis(100), 2);
        timer_handle.add_timeout(Duration::from_millis(600), 3);

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
        let mut event_loop = crate::EventLoop::try_new().unwrap();

        let evl_handle = event_loop.handle();

        let mut fired = Vec::new();

        let timer = Timer::<u32>::new().unwrap();
        let timer_handle = timer.handle();

        evl_handle
            .insert_source(timer, |val, _, fired: &mut Vec<u32>| fired.push(val))
            .unwrap();

        let timeout1 = timer_handle.add_timeout(Duration::from_millis(300), 1);
        let timeout2 = timer_handle.add_timeout(Duration::from_millis(100), 2);
        let timeout3 = timer_handle.add_timeout(Duration::from_millis(600), 3);

        // 3 dispatches as each returns once at least one event occured
        //
        // The timeouts 1 and 3 and not cancelled right away, but still before they
        // fire

        event_loop
            .dispatch(Some(::std::time::Duration::from_millis(200)), &mut fired)
            .unwrap();

        assert_eq!(&fired, &[2]);

        // timeout2 has already fired, we cancel timeout1
        assert_eq!(timer_handle.cancel_timeout(&timeout2), None);
        assert_eq!(timer_handle.cancel_timeout(&timeout1), Some(1));

        event_loop
            .dispatch(Some(::std::time::Duration::from_millis(300)), &mut fired)
            .unwrap();

        assert_eq!(&fired, &[2]);

        // cancel timeout3
        assert_eq!(timer_handle.cancel_timeout(&timeout3), Some(3));

        event_loop
            .dispatch(Some(::std::time::Duration::from_millis(600)), &mut fired)
            .unwrap();

        assert_eq!(&fired, &[2]);
    }

    #[test]
    fn timeout_cancel_early() {
        // Cancelling an earlier timeout should not prevent later ones from running
        let mut event_loop = crate::EventLoop::try_new().unwrap();
        let handle = event_loop.handle();

        let timer_source = Timer::new().unwrap();
        let timers = timer_source.handle();

        handle
            .insert_source(timer_source, |_, _, count| {
                *count += 1;
            })
            .unwrap();
        timers.add_timeout(Duration::from_secs(1), ());

        let sooner = timers.add_timeout(Duration::from_millis(500), ());
        timers.cancel_timeout(&sooner);

        let mut timeout_count = 0;
        event_loop
            .dispatch(
                Some(::std::time::Duration::from_secs(2)),
                &mut timeout_count,
            )
            .unwrap();
        // first timeout was cancelled, but the event loop still wakes up for nothing
        assert_eq!(timeout_count, 0);

        event_loop
            .dispatch(
                Some(::std::time::Duration::from_secs(2)),
                &mut timeout_count,
            )
            .unwrap();
        // second dispatch gets the second timeout
        assert_eq!(timeout_count, 1);
    }
}
