use std::{io, os::unix::io::RawFd};

use nix::sys::event::{kevent, kevent_ts, kqueue, EventFilter, EventFlag, FilterFlag, KEvent};

use super::{Interest, Mode, PollEvent, Readiness, Token};

pub struct Kqueue {
    kq: RawFd,
}

fn mode_to_flag(mode: Mode) -> EventFlag {
    match mode {
        Mode::Level => EventFlag::empty(),
        Mode::OneShot => EventFlag::EV_DISPATCH,
        Mode::Edge => EventFlag::EV_CLEAR,
    }
}

impl Kqueue {
    pub(crate) fn new() -> crate::Result<Kqueue> {
        let kq = kqueue()?;
        Ok(Kqueue { kq })
    }

    pub(crate) fn poll(
        &mut self,
        timeout: Option<std::time::Duration>,
    ) -> crate::Result<Vec<PollEvent>> {
        let mut buffer = [KEvent::new(
            0,
            EventFilter::EVFILT_READ,
            EventFlag::empty(),
            FilterFlag::empty(),
            0,
            0,
        ); 32];

        let nevents = match timeout {
            None => kevent_ts(self.kq, &[], &mut buffer, None),
            Some(t) => kevent(self.kq, &[], &mut buffer, t.as_millis() as usize),
        }?;

        let ret = buffer
            .iter()
            .take(nevents)
            .map(|event| PollEvent {
                readiness: Readiness {
                    readable: event.filter() == Ok(EventFilter::EVFILT_READ),
                    writable: event.filter() == Ok(EventFilter::EVFILT_WRITE),
                    error: event.flags().contains(EventFlag::EV_ERROR) && event.data() != 0,
                },
                token: unsafe { *(event.udata() as usize as *const Token) },
            })
            .collect();
        Ok(ret)
    }

    pub unsafe fn register(
        &mut self,
        fd: RawFd,
        interest: Interest,
        mode: Mode,
        token: *const Token,
    ) -> crate::Result<()> {
        self.reregister(fd, interest, mode, token)
    }

    pub unsafe fn reregister(
        &mut self,
        fd: RawFd,
        interest: Interest,
        mode: Mode,
        token: *const Token,
    ) -> crate::Result<()> {
        let write_flags = if interest.writable {
            EventFlag::EV_ADD | EventFlag::EV_RECEIPT | mode_to_flag(mode)
        } else {
            EventFlag::EV_DELETE
        };
        let read_flags = if interest.readable {
            EventFlag::EV_ADD | EventFlag::EV_RECEIPT | mode_to_flag(mode)
        } else {
            EventFlag::EV_DELETE
        };

        let changes = [
            KEvent::new(
                fd as usize,
                EventFilter::EVFILT_WRITE,
                write_flags,
                FilterFlag::empty(),
                0,
                token as usize as isize,
            ),
            KEvent::new(
                fd as usize,
                EventFilter::EVFILT_READ,
                read_flags,
                FilterFlag::empty(),
                0,
                token as usize as isize,
            ),
        ];

        let mut out = [
            KEvent::new(
                0,
                EventFilter::EVFILT_WRITE,
                EventFlag::empty(),
                FilterFlag::empty(),
                0,
                0,
            ),
            KEvent::new(
                0,
                EventFilter::EVFILT_READ,
                EventFlag::empty(),
                FilterFlag::empty(),
                0,
                0,
            ),
        ];

        kevent_ts(self.kq, &changes, &mut out, None)?;

        for o in &out {
            if o.flags().contains(EventFlag::EV_ERROR) && o.data() != 0 {
                let e = io::Error::from_raw_os_error(o.data() as i32);
                // ignore NotFound error which is raised if we tried to remove a non-existent filter
                if e.kind() != io::ErrorKind::NotFound {
                    return Err(e.into());
                }
            }
        }
        Ok(())
    }

    pub fn unregister(&mut self, fd: RawFd) -> crate::Result<()> {
        let changes = [
            KEvent::new(
                fd as usize,
                EventFilter::EVFILT_WRITE,
                EventFlag::EV_DELETE,
                FilterFlag::empty(),
                0,
                0,
            ),
            KEvent::new(
                fd as usize,
                EventFilter::EVFILT_READ,
                EventFlag::EV_DELETE,
                FilterFlag::empty(),
                0,
                0,
            ),
        ];

        let mut out = [
            KEvent::new(
                0,
                EventFilter::EVFILT_WRITE,
                EventFlag::empty(),
                FilterFlag::empty(),
                0,
                0,
            ),
            KEvent::new(
                0,
                EventFilter::EVFILT_READ,
                EventFlag::empty(),
                FilterFlag::empty(),
                0,
                0,
            ),
        ];

        kevent_ts(self.kq, &changes, &mut out, None)?;

        // Report an error if *both* fd were missing, meaning we were not registered at all
        let mut notfound = 0;

        for o in &out {
            if o.flags().contains(EventFlag::EV_ERROR) && o.data() != 0 {
                let e = io::Error::from_raw_os_error(o.data() as i32);
                // ignore NotFound error which is raised if we tried to remove a non-existent filter
                if e.kind() != io::ErrorKind::NotFound {
                    return Err(e.into());
                } else {
                    notfound += 1;
                }
            }
        }

        if notfound == 2 {
            return Err(std::io::Error::from(io::ErrorKind::NotFound).into());
        }

        Ok(())
    }
}

impl Drop for Kqueue {
    fn drop(&mut self) {
        let _ = nix::unistd::close(self.kq);
    }
}
