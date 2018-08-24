use std::cell::RefCell;
use std::io;
use std::rc::Rc;
use std::time::Duration;

use mio::{Events, Poll};

use list::SourceList;
use sources::{EventSource, Idle, Source};

/// An handle to an event loop
/// 
/// This handle allows you to insert new sources and idles in this event loop,
/// it can be cloned, and it is possible to insert new sources from within a source
/// callback.
#[derive(Clone)]
pub struct LoopHandle {
    poll: Rc<Poll>,
    list: Rc<RefCell<SourceList>>,
    idles: Rc<RefCell<Vec<Rc<RefCell<Option<Box<FnMut()>>>>>>>,
}

impl LoopHandle {
    /// Insert an new event source in the loop
    /// 
    /// The provided callback will be called during the dispatching cycles whenever the
    /// associated source generates events, see `EventLoop::dispatch(..)` for details.
    pub fn insert_source<E: EventSource, F: FnMut(E::Event) + 'static>(
        &self,
        source: E,
        callback: F,
    ) -> io::Result<Source<E>> {
        let dispatcher = source.make_dispatcher(callback);

        let token = self.list.borrow_mut().add_source(dispatcher);

        let interest = source.interest();
        let opt = source.pollopts();

        self.poll.register(&source, token, interest, opt)?;

        Ok(Source {
            source,
            poll: self.poll.clone(),
            list: self.list.clone(),
            token,
        })
    }

    /// Insert an idle callback
    /// 
    /// This callback will be called during a dispatching cycle when the event loop has
    /// finished processing all pending events from the sources and becomes idle.
    pub fn insert_idle<F: FnMut() + 'static>(&self, callback: F) -> Idle {
        let callback = Rc::new(RefCell::new(Some(Box::new(callback) as Box<FnMut()>)));
        self.idles.borrow_mut().push(callback.clone());
        Idle { callback }
    }
}

/// An event loop
/// 
/// This loop can host several event sources, that can be dynamically added or removed.
pub struct EventLoop {
    handle: LoopHandle,
    events_buffer: Events,
}

impl EventLoop {
    /// Create a new event loop
    /// 
    /// It is backed by an `mio` provided machinnery, and will fail if the `mio`
    /// initialization fails.
    pub fn new() -> io::Result<EventLoop> {
        Ok(EventLoop {
            handle: LoopHandle {
                poll: Rc::new(Poll::new()?),
                list: Rc::new(RefCell::new(SourceList::new())),
                idles: Rc::new(RefCell::new(Vec::new())),
            },
            events_buffer: Events::with_capacity(32),
        })
    }

    /// Retrieve a loop handle
    pub fn handle(&self) -> LoopHandle {
        self.handle.clone()
    }

    fn dispatch_events(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.events_buffer.clear();
        self.handle.poll.poll(&mut self.events_buffer, timeout)?;

        loop {
            if self.events_buffer.is_empty() {
                break;
            }

            for event in &self.events_buffer {
                if let Some(dispatcher) = self.handle.list.borrow().get_dispatcher(event.token()) {
                    dispatcher.borrow_mut().ready(event.readiness());
                }
            }

            // process remaining events if any
            self.events_buffer.clear();
            self.handle
                .poll
                .poll(&mut self.events_buffer, Some(Duration::from_millis(0)))?;
        }

        Ok(())
    }

    fn dispatch_idles(&mut self) {
        let idles = ::std::mem::replace(&mut *self.handle.idles.borrow_mut(), Vec::new());
        for idle in idles {
            if let Some(ref mut callback) = *idle.borrow_mut() {
                callback();
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
    pub fn dispatch(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.dispatch_events(timeout)?;

        self.dispatch_idles();

        Ok(())
    }
}
