//! A futures executor as an event source
//!
//! Only available with the `executor` cargo feature of `calloop`.
//!
//! This executor is intended for light futures, which will be polled as part of your
//! event loop. Such futures may be waiting for IO, or for some external computation on an
//! other thread for example.
//!
//! You can create a new executor using the `executor` function, which creates a pair
//! `(Executor<T>, Scheduler<T>)` to handle futures that all evaluate to type `T`. The
//! executor should be inserted into your event loop, and will yield the return values of
//! the futures as they finish into your callback. The scheduler can be cloned and used
//! to send futures to be executed into the executor. A generic executor can be obtained
//! by choosing `T = ()` and letting futures handle the forwarding of their return values
//! (if any) by their own means.
//!
//! **Note:** The futures must have their own means of being woken up, as this executor is,
//! by itself, not I/O aware. See [`LoopHandle::adapt_io`](crate::LoopHandle#method.adapt_io)
//! for that, or you can use some other mechanism if you prefer.

use async_task::{Builder, Runnable};
use slab::Slab;
use std::{cell::RefCell, collections::VecDeque, future::Future, rc::Rc, task::Waker};

use crate::{
    sources::{
        channel::ChannelError,
        ping::{make_ping, Ping, PingError, PingSource},
        EventSource,
    },
    Poll, PostAction, Readiness, Token, TokenFactory,
};

/// A future executor as an event source
#[derive(Debug)]
pub struct Executor<T> {
    /// Shared state between the executor and the scheduler.
    state: Rc<State<T>>,

    /// The ping source to register into the poller.
    ping_source: PingSource,
}

/// A scheduler to send futures to an executor
#[derive(Clone, Debug)]
pub struct Scheduler<T> {
    state: Rc<State<T>>,
}

/// The inner state of the executor.
#[derive(Debug)]
struct State<T> {
    /// The incoming queue of futures to be executed.
    incoming: RefCell<VecDeque<Runnable<usize>>>,

    /// The list of currently active tasks.
    ///
    /// This is set to `None` when the executor is destroyed.
    active: RefCell<Option<Slab<Active<T>>>>,

    /// The ping to wake up the executor.
    ping: Ping,
}

/// An active future or its result.
#[derive(Debug)]
enum Active<T> {
    /// The future is currently being polled.
    ///
    /// Waking this waker will insert the runnable into `incoming`.
    Future(Waker),

    /// The future has finished polling, and its result is stored here.
    Finished(T),
}

impl<T> Active<T> {
    fn is_finished(&self) -> bool {
        match self {
            Active::Finished(_) => true,
            _ => false,
        }
    }
}

impl<T> Scheduler<T> {
    /// Sends the given future to the executor associated to this scheduler
    ///
    /// Returns an error if the the executor not longer exists.
    pub fn schedule<Fut: 'static>(&self, future: Fut) -> Result<(), ExecutorDestroyed>
    where
        Fut: Future<Output = T>,
    {
        /// Store this future's result in the executor.
        struct StoreOnDrop<'a, T> {
            index: usize,
            value: Option<T>,
            state: &'a State<T>,
        }

        impl<T> Drop for StoreOnDrop<'_, T> {
            fn drop(&mut self) {
                let mut active = self.state.active.borrow_mut();
                if let Some(active) = active.as_mut() {
                    if let Some(value) = self.value.take() {
                        active[self.index] = Active::Finished(value);
                    } else {
                        // The future was dropped before it finished.
                        // Remove it from the active list.
                        active.remove(self.index);
                    }
                }
            }
        }

        let mut active_guard = self.state.active.borrow_mut();
        let active = active_guard.as_mut().ok_or(ExecutorDestroyed)?;

        // Wrap the future in another future that polls it and stores the result.
        let index = active.vacant_key();
        let future = {
            let state = self.state.clone();
            async move {
                let mut guard = StoreOnDrop {
                    index,
                    value: None,
                    state: &state,
                };

                // Get the value of the future.
                let value = future.await;

                // Store it in the executor.
                guard.value = Some(value);
            }
        };

        // A schedule function that inserts the runnable into the incoming queue.
        let schedule = {
            let state = self.state.clone();
            move |runnable| {
                let mut incoming = state.incoming.borrow_mut();
                incoming.push_back(runnable);

                // Wake up the executor.
                state.ping.ping();
            }
        };

        // Spawn the future.
        let (runnable, task) = {
            let builder = Builder::new().metadata(index).propagate_panic(true);

            // SAFETY: todo
            unsafe { builder.spawn_unchecked(move |_| future, schedule) }
        };

        // Insert the runnable into the set of active tasks.
        active.insert(Active::Future(runnable.waker()));
        drop(active_guard);

        // Schedule the runnable and detach the task so it isn't cancellable.
        runnable.schedule();
        task.detach();

        Ok(())
    }
}

impl<T> Drop for Executor<T> {
    fn drop(&mut self) {
        let active = self.state.active.borrow_mut().take().unwrap();

        // Wake all of the active tasks in order to destroy their runnables.
        for (_, task) in active {
            // Don't let a panicking waker blow everything up.
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if let Active::Future(waker) = task {
                    waker.wake();
                }
            }))
            .ok();
        }

        // Drain the queue in order to drop all of the runnables.
        self.state.incoming.borrow_mut().clear();
    }
}

/// Error generated when trying to schedule a future after the
/// executor was destroyed.
#[derive(thiserror::Error, Debug)]
#[error("the executor was destroyed")]
pub struct ExecutorDestroyed;

/// Create a new executor, and its associated scheduler
///
/// May fail due to OS errors preventing calloop to setup its internal pipes (if your
/// process has reatched its file descriptor limit for example).
pub fn executor<T>() -> crate::Result<(Executor<T>, Scheduler<T>)> {
    let (ping, ping_source) = make_ping()?;
    let state = Rc::new(State {
        incoming: RefCell::new(VecDeque::new()),
        active: RefCell::new(Some(Slab::new())),
        ping,
    });

    Ok((
        Executor {
            state: state.clone(),
            ping_source,
        },
        Scheduler { state },
    ))
}

impl<T> EventSource for Executor<T> {
    type Event = T;
    type Metadata = ();
    type Ret = ();
    type Error = ExecutorError;

    fn process_events<F>(
        &mut self,
        readiness: Readiness,
        token: Token,
        mut callback: F,
    ) -> Result<PostAction, Self::Error>
    where
        F: FnMut(T, &mut ()),
    {
        // Process all of the newly inserted futures.
        let clear_readiness = {
            let mut incoming = self.state.incoming.borrow_mut();
            let mut clear_readiness = false;

            // Only process a limited number of tasks at a time; better to move onto the next event soon.
            for _ in 0..1024 {
                if let Some(runnable) = incoming.pop_front() {
                    let index = *runnable.metadata();
                    runnable.run();

                    // If the runnable finished with a result, call the callback.
                    let mut active_guard = self.state.active.borrow_mut();
                    let active = active_guard.as_mut().unwrap();

                    if let Some(state) = active.get(index) {
                        if state.is_finished() {
                            // Take out the state and provide it.
                            let result = match active.remove(index) {
                                Active::Finished(result) => result,
                                _ => unreachable!(),
                            };

                            callback(result, &mut ());
                        }
                    }
                } else {
                    clear_readiness = true;
                    break;
                }
            }

            clear_readiness
        };

        // Clear the readiness of the ping event if we processed all of the incoming tasks.
        if clear_readiness {
            self.ping_source
                .process_events(readiness, token, |(), &mut ()| {})
                .map_err(ExecutorError::WakeError)?;
        }

        Ok(PostAction::Continue)
    }

    fn register(&mut self, poll: &mut Poll, token_factory: &mut TokenFactory) -> crate::Result<()> {
        self.ping_source.register(poll, token_factory)?;
        Ok(())
    }

    fn reregister(
        &mut self,
        poll: &mut Poll,
        token_factory: &mut TokenFactory,
    ) -> crate::Result<()> {
        self.ping_source.reregister(poll, token_factory)?;
        Ok(())
    }

    fn unregister(&mut self, poll: &mut Poll) -> crate::Result<()> {
        self.ping_source.unregister(poll)?;
        Ok(())
    }
}

/// An error arising from processing events in an async executor event source.
#[derive(thiserror::Error, Debug)]
pub enum ExecutorError {
    /// Error while reading new futures added via [`Scheduler::schedule()`].
    #[error("error adding new futures")]
    NewFutureError(ChannelError),

    /// Error while processing wake events from existing futures.
    #[error("error processing wake events")]
    WakeError(PingError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready() {
        let mut event_loop = crate::EventLoop::<u32>::try_new().unwrap();

        let handle = event_loop.handle();

        let (exec, sched) = executor::<u32>().unwrap();

        handle
            .insert_source(exec, move |ret, &mut (), got| {
                *got = ret;
            })
            .unwrap();

        let mut got = 0;

        let fut = async { 42 };

        event_loop
            .dispatch(Some(::std::time::Duration::ZERO), &mut got)
            .unwrap();

        // the future is not yet inserted, and thus has not yet run
        assert_eq!(got, 0);

        sched.schedule(fut).unwrap();

        event_loop
            .dispatch(Some(::std::time::Duration::ZERO), &mut got)
            .unwrap();

        // the future has run
        assert_eq!(got, 42);
    }
}
