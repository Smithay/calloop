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
use std::collections::BinaryHeap;
use std::io;
use std::rc::Rc;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

use mio::{
    event::{Event as MioEvent, Source as MioSource},
    Interest, Waker,
};

use crate::{EventDispatcher, EventSource};

/// A Timer event source
///
/// It generates events of type `(T, TimerHandle<T>)`, providing you
/// an handle inside the event callback, allowing you to set new timeouts
/// as a response to a timeout being reached (for reccuring ticks for example).
pub struct Timer<T> {
    inner: Arc<Mutex<TimerInner<T>>>,
}

impl<T> Timer<T> {
    /// Create a new timer with default parameters
    ///
    /// Default time resolution is 100ms
    pub fn new() -> Timer<T> {
        Timer {
            inner: Arc::new(Mutex::new(TimerInner::new())),
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
    inner: Arc<Mutex<TimerInner<T>>>,
}

// Manual impl of `Clone` as #[derive(Clone)] adds a `T: Clone` bound
impl<T> Clone for TimerHandle<T> {
    fn clone(&self) -> TimerHandle<T> {
        TimerHandle {
            inner: self.inner.clone(),
        }
    }
}

/// An itentifier to cancel a timeout if necessary
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

impl<T: 'static> EventSource for Timer<T> {
    type Event = (T, TimerHandle<T>);

    fn interest(&self) -> Interest {
        Interest::READABLE
    }

    fn as_mio_source(&mut self) -> Option<&mut dyn MioSource> {
        None
    }

    fn make_dispatcher<Data: 'static, F: FnMut((T, TimerHandle<T>), &mut Data) + 'static>(
        &mut self,
        callback: F,
        waker: &Arc<Waker>,
    ) -> Rc<RefCell<dyn EventDispatcher<Data>>> {
        {
            let mut inner = self.inner.lock().unwrap();
            inner.scheduler = Some(
                TimerScheduler::new(waker.clone())
                    .expect("[calloop] Failed to initialize the timer scheduler"),
            );
            inner.reschedule();
        }
        Rc::new(RefCell::new(Dispatcher {
            _data: ::std::marker::PhantomData,
            inner: self.inner.clone(),
            callback,
        }))
    }
}

struct Dispatcher<Data, T, F: FnMut((T, TimerHandle<T>), &mut Data)> {
    _data: ::std::marker::PhantomData<fn(&mut Data)>,
    inner: Arc<Mutex<TimerInner<T>>>,
    callback: F,
}

impl<Data, T, F: FnMut((T, TimerHandle<T>), &mut Data)> EventDispatcher<Data>
    for Dispatcher<Data, T, F>
{
    fn ready(&mut self, _event: Option<&MioEvent>, data: &mut Data) {
        // unlock the inner between each user callback, so that they can
        // insert new timers from inside the callback
        let mut some_expired = false;
        loop {
            let next_expired: Option<T> = {
                let mut guard = self.inner.lock().unwrap();
                guard.next_expired()
            };
            if let Some(val) = next_expired {
                some_expired = true;
                (self.callback)(
                    (
                        val,
                        TimerHandle {
                            inner: self.inner.clone(),
                        },
                    ),
                    data,
                );
            } else {
                break;
            }
        }
        // now compute the next timeout and signal if necessary
        if some_expired {
            self.inner.lock().unwrap().reschedule();
        }
    }
}

/*
 * Timer logic
 */

struct TimeoutData<T> {
    deadline: Instant,
    data: RefCell<Option<T>>,
    counter: u32,
}

struct TimerInner<T> {
    heap: BinaryHeap<TimeoutData<T>>,
    scheduler: Option<TimerScheduler>,
    counter: u32,
}

impl<T> TimerInner<T> {
    fn new() -> TimerInner<T> {
        TimerInner {
            heap: BinaryHeap::new(),
            scheduler: None,
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
                return data.data.borrow_mut().take();
            }
        }
        self.reschedule();
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
            if let Some(ref data) = self.heap.peek() {
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
        if let Some(ref mut scheduler) = self.scheduler {
            if let Some(next_deadline) = self.heap.peek().map(|data| data.deadline) {
                scheduler.reschedule(next_deadline);
            } else {
                scheduler.deschedule();
            }
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
 * Scheduling
 */

struct TimerScheduler {
    current_deadline: Arc<Mutex<Option<Instant>>>,
    kill_switch: Arc<AtomicBool>,
    thread: std::thread::JoinHandle<()>,
}

impl TimerScheduler {
    fn new(waker: Arc<Waker>) -> io::Result<TimerScheduler> {
        let current_deadline = Arc::new(Mutex::new(None::<Instant>));
        let thread_deadline = current_deadline.clone();

        let kill_switch = Arc::new(AtomicBool::new(false));
        let thread_kill = kill_switch.clone();

        let thread = std::thread::Builder::new()
            .name("calloop timer".into())
            .spawn(move || loop {
                // stop if requested
                if thread_kill.load(Ordering::Acquire) {
                    return;
                }
                // otherwise check the timeout
                let opt_deadline: Option<Instant> = {
                    // subscope to ensure the mutex does not remain locked while the thread is parked
                    let guard = thread_deadline.lock().unwrap();
                    (*guard).clone()
                };
                if let Some(deadline) = opt_deadline {
                    if let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
                        // it is not yet expired, go to sleep until it
                        std::thread::park_timeout(remaining);
                    } else {
                        // it is expired, wake the event loop and go to sleep
                        let _ = waker.wake();
                        std::thread::park();
                    }
                } else {
                    // there is none, got to sleep
                    std::thread::park();
                }
            })?;

        Ok(TimerScheduler {
            current_deadline,
            kill_switch,
            thread,
        })
    }

    fn reschedule(&mut self, new_deadline: Instant) {
        let mut deadline_guard = self.current_deadline.lock().unwrap();
        if let Some(current_deadline) = deadline_guard.clone() {
            if new_deadline < current_deadline || current_deadline <= Instant::now() {
                *deadline_guard = Some(new_deadline);
                self.thread.thread().unpark();
            }
        } else {
            *deadline_guard = Some(new_deadline);
            self.thread.thread().unpark();
        }
    }

    fn deschedule(&mut self) {
        *(self.current_deadline.lock().unwrap()) = None;
    }
}

impl Drop for TimerScheduler {
    fn drop(&mut self) {
        self.kill_switch.store(true, Ordering::Release);
    }
}

/*
 * Tests
 */

#[cfg(test)]
mod tests {
    use std::io;
    use std::time::Duration;

    use super::*;

    #[test]
    fn single_timer() {
        let mut event_loop = crate::EventLoop::new().unwrap();

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

        event_loop.dispatch(None, &mut fired).unwrap();

        // it should have fired now
        assert!(fired);
    }

    #[test]
    fn multi_timout_order() {
        let mut event_loop = crate::EventLoop::new().unwrap();

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
        let mut event_loop = crate::EventLoop::new().unwrap();

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
