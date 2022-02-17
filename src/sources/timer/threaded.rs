//! Timer scheduler which is using thread to schedule timers.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::ping::{make_ping, Ping, PingSource};
use crate::{EventSource, Poll, PostAction, Readiness, Token, TokenFactory};

use super::TimerError;

#[derive(Debug)]
pub struct TimerSource {
    source: PingSource,
}

impl TimerSource {
    fn new() -> std::io::Result<(Ping, Self)> {
        let (ping, source) = make_ping()?;
        Ok((ping, Self { source }))
    }
}

impl EventSource for TimerSource {
    type Event = ();
    type Metadata = ();
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
        self.source
            .process_events(readiness, token, |_, &mut _| {
                callback((), &mut ());
            })
            .map_err(|err| TimerError(err.into()))
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

#[derive(Debug)]
pub struct TimerScheduler {
    current_deadline: Arc<Mutex<Option<Instant>>>,
    kill_switch: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl TimerScheduler {
    pub fn new() -> crate::Result<(TimerScheduler, TimerSource)> {
        let current_deadline = Arc::new(Mutex::new(None::<Instant>));
        let thread_deadline = current_deadline.clone();

        let kill_switch = Arc::new(AtomicBool::new(false));
        let thread_kill = kill_switch.clone();

        let (ping, source) = TimerSource::new()?;

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
                    *guard
                };
                if let Some(deadline) = opt_deadline {
                    if let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
                        // it is not yet expired, go to sleep until it
                        std::thread::park_timeout(remaining);
                    } else {
                        // it is expired, wake the event loop and go to sleep
                        ping.ping();
                        std::thread::park();
                    }
                } else {
                    // there is none, got to sleep
                    std::thread::park();
                }
            })?;

        let scheduler = TimerScheduler {
            current_deadline,
            kill_switch,
            thread: Some(thread),
        };
        Ok((scheduler, source))
    }

    pub fn reschedule(&mut self, new_deadline: Instant) {
        let mut deadline_guard = self.current_deadline.lock().unwrap();
        if let Some(current_deadline) = *deadline_guard {
            if new_deadline < current_deadline || current_deadline <= Instant::now() {
                *deadline_guard = Some(new_deadline);
                self.thread.as_ref().unwrap().thread().unpark();
            }
        } else {
            *deadline_guard = Some(new_deadline);
            self.thread.as_ref().unwrap().thread().unpark();
        }
    }

    pub fn deschedule(&mut self) {
        *(self.current_deadline.lock().unwrap()) = None;
    }
}

impl Drop for TimerScheduler {
    fn drop(&mut self) {
        self.kill_switch.store(true, Ordering::Release);
        let thread = self.thread.take().unwrap();
        thread.thread().unpark();
        let _ = thread.join();
    }
}
