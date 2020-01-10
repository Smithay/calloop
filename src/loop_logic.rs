use std::cell::RefCell;
use std::fmt::{self, Debug, Formatter};
use std::io;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use mio::{Events, Poll, Registry, Token, Waker};

use crate::list::SourceList;
use crate::sources::{EventSource, Idle, Source};

const WAKER_TOKEN: Token = Token(std::usize::MAX);

/// An handle to an event loop
///
/// This handle allows you to insert new sources and idles in this event loop,
/// it can be cloned, and it is possible to insert new sources from within a source
/// callback.
pub struct LoopHandle<Data> {
    registry: Rc<Registry>,
    mio_list: Rc<RefCell<SourceList<Data>>>,
    user_list: Rc<RefCell<SourceList<Data>>>,
    idles: Rc<RefCell<Vec<Rc<RefCell<Option<Box<dyn FnMut(&mut Data)>>>>>>>,
    waker: Arc<Waker>,
}

impl<Data> Clone for LoopHandle<Data> {
    fn clone(&self) -> LoopHandle<Data> {
        LoopHandle {
            registry: self.registry.clone(),
            mio_list: self.mio_list.clone(),
            user_list: self.user_list.clone(),
            idles: self.idles.clone(),
            waker: self.waker.clone(),
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

impl<E> Debug for InsertError<E> {
    fn fmt(&self, formatter: &mut Formatter) -> Result<(), fmt::Error> {
        write!(formatter, "{:?}", self.error)
    }
}

impl<E> From<InsertError<E>> for io::Error {
    fn from(e: InsertError<E>) -> io::Error {
        e.error
    }
}

impl<Data: 'static> LoopHandle<Data> {
    /// Insert an new event source in the loop
    ///
    /// The provided callback will be called during the dispatching cycles whenever the
    /// associated source generates events, see `EventLoop::dispatch(..)` for details.
    pub fn insert_source<E: EventSource, F: FnMut(E::Event, &mut Data) + 'static>(
        &self,
        mut source: E,
        callback: F,
    ) -> Result<Source<E>, InsertError<E>> {
        let dispatcher = source.make_dispatcher(callback, &self.waker);

        let interest = source.interest();

        let (list, token) = if let Some(mio_source) = source.as_mio_source() {
            let token = self.mio_list.borrow_mut().add_source(dispatcher);

            if let Err(e) = self.registry.register(mio_source, token, interest) {
                return Err(InsertError { source, error: e });
            }
            (self.mio_list.clone(), token)
        } else {
            let token = self.user_list.borrow_mut().add_source(dispatcher);
            (self.user_list.clone(), token)
        };

        Ok(Source {
            source,
            registry: self.registry.clone(),
            list,
            token,
        })
    }

    /// Insert an idle callback
    ///
    /// This callback will be called during a dispatching cycle when the event loop has
    /// finished processing all pending events from the sources and becomes idle.
    pub fn insert_idle<F: FnOnce(&mut Data) + 'static>(&self, callback: F) -> Idle {
        let mut opt_cb = Some(callback);
        let callback = Rc::new(RefCell::new(Some(Box::new(move |data: &mut Data| {
            if let Some(cb) = opt_cb.take() {
                cb(data);
            }
        }) as Box<dyn FnMut(&mut Data)>)));
        self.idles.borrow_mut().push(callback.clone());
        Idle { callback }
    }
}

/// An event loop
///
/// This loop can host several event sources, that can be dynamically added or removed.
pub struct EventLoop<Data> {
    poll: Poll,
    handle: LoopHandle<Data>,
    events_buffer: Events,
    stop_signal: Arc<AtomicBool>,
}

impl<Data: 'static> EventLoop<Data> {
    /// Create a new event loop
    ///
    /// It is backed by an `mio` provided machinnery, and will fail if the `mio`
    /// initialization fails.
    pub fn new() -> io::Result<EventLoop<Data>> {
        let poll = Poll::new()?;
        let waker = Arc::new(Waker::new(poll.registry(), WAKER_TOKEN)?);
        let handle = LoopHandle {
            registry: Rc::new(poll.registry().try_clone()?),
            mio_list: Rc::new(RefCell::new(SourceList::new())),
            user_list: Rc::new(RefCell::new(SourceList::new())),
            idles: Rc::new(RefCell::new(Vec::new())),
            waker,
        };
        Ok(EventLoop {
            poll,
            handle,
            events_buffer: Events::with_capacity(32),
            stop_signal: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Retrieve a loop handle
    pub fn handle(&self) -> LoopHandle<Data> {
        self.handle.clone()
    }

    fn dispatch_events(&mut self, timeout: Option<Duration>, data: &mut Data) -> io::Result<()> {
        self.events_buffer.clear();
        self.poll.poll(&mut self.events_buffer, timeout)?;

        loop {
            if self.events_buffer.is_empty() {
                break;
            }

            for event in &self.events_buffer {
                let opt_dispatcher = self.handle.mio_list.borrow().get_dispatcher(event.token());
                if let Some(dispatcher) = opt_dispatcher {
                    dispatcher.borrow_mut().ready(Some(event), data);
                }
            }

            // process remaining events if any
            self.events_buffer.clear();
            self.poll
                .poll(&mut self.events_buffer, Some(Duration::from_millis(0)))?;
        }

        for dispatcher in self.handle.user_list.borrow().all() {
            dispatcher.borrow_mut().ready(None, data);
        }

        Ok(())
    }

    fn dispatch_idles(&mut self, data: &mut Data) {
        let idles = ::std::mem::replace(&mut *self.handle.idles.borrow_mut(), Vec::new());
        for idle in idles {
            if let Some(ref mut callback) = *idle.borrow_mut() {
                callback(data);
            }
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
    pub fn dispatch(&mut self, timeout: Option<Duration>, data: &mut Data) -> io::Result<()> {
        self.dispatch_events(timeout, data)?;

        self.dispatch_idles(data);

        Ok(())
    }

    /// Get a signal to stop this event loop from running
    ///
    /// To be used in conjunction with the `run()` method.
    pub fn get_signal(&self) -> LoopSignal {
        LoopSignal {
            signal: self.stop_signal.clone(),
            waker: self.handle.waker.clone(),
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
    waker: Arc<Waker>,
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
    pub fn wakeup(&self) -> io::Result<()> {
        self.waker.wake()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

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
            .dispatch(Some(Duration::from_millis(0)), &mut dispatched)
            .unwrap();

        assert!(!dispatched);
    }

    #[test]
    fn wakeup() {
        let mut event_loop = EventLoop::new().unwrap();

        let signal = event_loop.get_signal();

        ::std::thread::spawn(move || {
            ::std::thread::sleep(Duration::from_millis(500));
            signal.wakeup().unwrap();
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
            signal.wakeup().unwrap();
        });

        // the test should return
        event_loop.run(None, &mut (), |_| {}).unwrap();
    }

    #[test]
    fn insert_remove() {
        use crate::mio::Waker;
        use std::{cell::RefCell, rc::Rc, sync::Arc};

        // A dummy EventSource to test insertion and removal of sources
        struct DummySource;

        struct Dispatcher<F>(F);

        impl<Data, F: FnMut((), &mut Data) + 'static> crate::EventDispatcher<Data> for Dispatcher<F> {
            fn ready(&mut self, _ready: Option<&crate::mio::event::Event>, data: &mut Data) {
                (self.0)((), data)
            }
        }

        impl crate::EventSource for DummySource {
            type Event = ();
            fn make_dispatcher<Data: 'static, F: FnMut(Self::Event, &mut Data) + 'static>(
                &mut self,
                callback: F,
                _waker: &Arc<Waker>,
            ) -> Rc<RefCell<dyn crate::EventDispatcher<Data>>> {
                Rc::new(RefCell::new(Dispatcher(callback)))
            }
        }

        // the actual test
        let event_loop = EventLoop::<()>::new().unwrap();
        let source_1 = event_loop
            .handle()
            .insert_source(DummySource, |_, _| {})
            .unwrap();
        assert_eq!(source_1.token.0, 0);
        let source_2 = event_loop
            .handle()
            .insert_source(DummySource, |_, _| {})
            .unwrap();
        assert_eq!(source_2.token.0, 1);
        // ensure token reuse on source removal
        source_1.remove();
        let source_3 = event_loop
            .handle()
            .insert_source(DummySource, |_, _| {})
            .unwrap();
        assert_eq!(source_3.token.0, 0);
    }
}
