//! Timer scheduler which is using timerfd system interface to schedule timers.

use std::os::unix::io::{AsRawFd, RawFd};
use std::time::Instant;

use nix::sys::time::TimeSpec;
use nix::sys::timerfd::{ClockId, Expiration, TimerFd, TimerFlags, TimerSetTimeFlags};

use crate::generic::Generic;
use crate::{EventSource, Poll, PostAction, Readiness, Token, TokenFactory};

#[derive(Debug)]
pub struct TimerScheduler {
    current_deadline: Option<Instant>,
    timerfd: TimerFd,
}

impl TimerScheduler {
    pub fn new() -> crate::Result<(Self, TimerSource)> {
        let timerfd = TimerFd::new(
            ClockId::CLOCK_MONOTONIC,
            TimerFlags::TFD_CLOEXEC | TimerFlags::TFD_NONBLOCK,
        )?;

        let source = TimerSource::new(&timerfd);
        let scheduler = Self {
            timerfd,
            current_deadline: None,
        };

        Ok((scheduler, source))
    }

    pub fn reschedule(&mut self, new_deadline: Instant) {
        let now = Instant::now();
        let time = TimeSpec::from_duration(new_deadline.saturating_duration_since(now));
        let time = match self.current_deadline {
            Some(current_deadline) if new_deadline > current_deadline && current_deadline > now => {
                return;
            }
            _ => time,
        };

        self.current_deadline = Some(new_deadline);

        let expiration = Expiration::OneShot(time);
        let flags = TimerSetTimeFlags::empty();
        self.timerfd
            .set(expiration, flags)
            .expect("setting timerfd failed.");
    }

    pub fn deschedule(&mut self) {
        self.current_deadline = None;
        self.timerfd.unset().expect("failed unsetting timerfd.");
    }
}

#[derive(Debug)]
pub struct TimerSource {
    source: Generic<RawFd>,
}

impl TimerSource {
    fn new(timerfd: &TimerFd) -> Self {
        Self {
            source: Generic::new(
                timerfd.as_raw_fd(),
                crate::Interest::READ,
                crate::Mode::Level,
            ),
        }
    }
}

impl EventSource for TimerSource {
    type Event = ();
    type Metadata = ();
    type Ret = ();
    type Error = std::io::Error;

    fn process_events<C>(
        &mut self,
        readiness: Readiness,
        token: Token,
        mut callback: C,
    ) -> Result<PostAction, Self::Error>
    where
        C: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
    {
        self.source.process_events(readiness, token, |_, &mut _| {
            callback((), &mut ());
            Ok(PostAction::Continue)
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
