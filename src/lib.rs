//! Calloop, a Callback-based Event Loop
//!
//! This crate provides an [`EventLoop`] type, which is a small abstraction
//! over a polling system. The main difference between this crate
//! and other traditional rust event loops is that it is based on callbacks:
//! you can register several event sources, each being associated with a callback
//! closure that will be invoked whenever the associated event source generates
//! events.
//!
//! The main target use of this event loop is thus for apps that expect to spend
//! most of their time waiting for events and wishes to do so in a cheap and convenient
//! way. It is not meant for large scale high performance IO.
//!
//! ## How to use it
//!
//! Below is a quick usage example of calloop. For a more in-depth tutorial, see
//! the [calloop book](https://smithay.github.io/calloop).
//!
//! For simple uses, you can just add event sources with callbacks to the event
//! loop. For example, here's a runnable program that exits after five seconds:
//!
//! ```no_run
//! use calloop::{timer::{Timer, TimeoutAction}, EventLoop, LoopSignal};
//!
//! fn main() {
//!     // Create the event loop. The loop is parameterised by the kind of shared
//!     // data you want the callbacks to use. In this case, we want to be able to
//!     // stop the loop when the timer fires, so we provide the loop with a
//!     // LoopSignal, which has the ability to stop the loop from within events. We
//!     // just annotate the type here; the actual data is provided later in the
//!     // run() call.
//!     let mut event_loop: EventLoop<LoopSignal> =
//!         EventLoop::new().expect("Failed to initialize the event loop!");
//!
//!     // Retrieve a handle. It is used to insert new sources into the event loop
//!     // It can be cloned, allowing you to insert sources from within source
//!     // callbacks.
//!     let handle = event_loop.handle();
//!
//!     // Create our event source, a timer, that will expire in 2 seconds
//!     let source = Timer::from_duration(std::time::Duration::from_secs(2));
//!
//!     // Inserting an event source takes this general form. It can also be done
//!     // from within the callback of another event source.
//!     handle
//!         .insert_source(
//!             // a type which implements the EventSource trait
//!             source,
//!             // a callback that is invoked whenever this source generates an event
//!             |event, _metadata, shared_data| {
//!                 // This callback is given 3 values:
//!                 // - the event generated by the source (in our case, timer events are the Instant
//!                 //   representing the deadline for which it has fired)
//!                 // - &mut access to some metadata, specific to the event source (in our case, a
//!                 //   timer handle)
//!                 // - &mut access to the global shared data that was passed to EventLoop::run or
//!                 //   EventLoop::dispatch (in our case, a LoopSignal object to stop the loop)
//!                 //
//!                 // The return type is just () because nothing uses it. Some
//!                 // sources will expect a Result of some kind instead.
//!                 println!("Timeout for {:?} expired!", event);
//!                 // notify the event loop to stop running using the signal in the shared data
//!                 // (see below)
//!                 shared_data.stop();
//!                 // The timer event source requires us to return a TimeoutAction to
//!                 // specify if the timer should be rescheduled. In our case we just drop it.
//!                 TimeoutAction::Drop
//!             },
//!         )
//!         .expect("Failed to insert event source!");
//!
//!     // Create the shared data for our loop.
//!     let mut shared_data = event_loop.get_signal();
//!
//!     // Actually run the event loop. This will dispatch received events to their
//!     // callbacks, waiting at most 20ms for new events between each invocation of
//!     // the provided callback (pass None for the timeout argument if you want to
//!     // wait indefinitely between events).
//!     //
//!     // This is where we pass the *value* of the shared data, as a mutable
//!     // reference that will be forwarded to all your callbacks, allowing them to
//!     // share some state
//!     event_loop
//!         .run(
//!             std::time::Duration::from_millis(20),
//!             &mut shared_data,
//!             |_shared_data| {
//!                 // Finally, this is where you can insert the processing you need
//!                 // to do do between each waiting event eg. drawing logic if
//!                 // you're doing a GUI app.
//!             },
//!         )
//!         .expect("Error during event loop!");
//! }
//! ```
//!
//! ## Event source types
//!
//! The event loop is backed by an OS provided polling selector (epoll on Linux).
//!
//! This crate also provide some adapters for common event sources such as:
//!
//! - [MPSC channels](channel)
//! - [Timers](timer)
//! - [unix signals](signals) on Linux
//!
//! As well as generic objects backed by file descriptors.
//!
//! It is also possible to insert "idle" callbacks. These callbacks represent computations that
//! need to be done at some point, but are not as urgent as processing the events. These callbacks
//! are stored and then executed during [`EventLoop::dispatch`](EventLoop#method.dispatch), once all
//! events from the sources have been processed.
//!
//! ## Async/Await compatibility
//!
//! `calloop` can be used with futures, both as an executor and for monitoring Async IO.
//!
//! Activating the `executor` cargo feature will add the [`futures`] module, which provides
//! a future executor that can be inserted into an [`EventLoop`] as yet another [`EventSource`].
//!
//! IO objects can be made Async-aware via the [`LoopHandle::adapt_io`](LoopHandle#method.adapt_io)
//! method. Waking up the futures using these objects is handled by the associated [`EventLoop`]
//! directly.
//!
//! ## Custom event sources
//!
//! You can create custom event sources can will be inserted in the event loop by
//! implementing the [`EventSource`] trait. This can be done either directly from the file
//! descriptors of your source of interest, or by wrapping an other event source and further
//! processing its events. An [`EventSource`] can register more than one file descriptor and
//! aggregate them.
//!
//! ## Platforms support
//!
//! Currently, calloop is tested on Linux, FreeBSD and macOS.
//!
//! The following platforms are also enabled at compile time but not tested: Android, NetBSD,
//! OpenBSD, DragonFlyBSD.
//!
//! Those platforms *should* work based on the fact that they have the same polling mechanism as
//! tested platforms, but some subtle bugs might still occur.

#![warn(missing_docs, missing_debug_implementations)]
#![allow(clippy::needless_doctest_main)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(feature = "nightly_coverage", feature(coverage_attribute))]

mod sys;

pub use sys::{Interest, Mode, Poll, Readiness, Token, TokenFactory};

pub use self::loop_logic::{EventLoop, LoopHandle, LoopSignal, RegistrationToken};
pub use self::sources::*;

pub mod error;
pub use error::{Error, InsertError, Result};

pub mod io;
mod list;
mod loop_logic;
mod macros;
mod sources;
mod token;
