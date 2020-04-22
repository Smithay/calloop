use std::{io, os::unix::io::RawFd};

use crate::no_nix_err;
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
    pub(crate) fn new() -> io::Result<Kqueue> {
        let kq = kqueue().map_err(no_nix_err)?;
        Ok(Kqueue { kq })
    }

    pub(crate) fn poll(
        &mut self,
        timeout: Option<std::time::Duration>,
    ) -> io::Result<Vec<PollEvent>> {
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
        }
        .map_err(no_nix_err)?;

        let ret = buffer
            .iter()
            .take(nevents)
            .map(|event| PollEvent {
                readiness: Readiness {
                    readable: event.filter() == EventFilter::EVFILT_READ,
                    writable: event.filter() == EventFilter::EVFILT_WRITE,
                    error: event.flags().contains(EventFlag::EV_ERROR) && event.data() != 0,
                },
                token: Token::from_usize(event.udata() as usize),
            })
            .collect();
        Ok(ret)
    }

    pub fn register(
        &mut self,
        fd: RawFd,
        interest: Interest,
        mode: Mode,
        token: Token,
    ) -> io::Result<()> {
        let flags = EventFlag::EV_ADD | EventFlag::EV_RECEIPT | mode_to_flag(mode);

        let changes = [
            KEvent::new(
                fd as usize,
                EventFilter::EVFILT_WRITE,
                flags,
                FilterFlag::empty(),
                0,
                token.to_usize() as isize,
            ),
            KEvent::new(
                fd as usize,
                EventFilter::EVFILT_READ,
                flags,
                FilterFlag::empty(),
                0,
                token.to_usize() as isize,
            ),
        ];

        let changes = match interest {
            Interest::Readable => &changes[1..],
            Interest::Writable => &changes[..1],
            Interest::Both => &changes[..],
        };

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

        kevent_ts(self.kq, changes, &mut out, None).map_err(no_nix_err)?;
        for o in &out {
            if o.flags().contains(EventFlag::EV_ERROR) && o.data() != 0 {
                let e = io::Error::from_raw_os_error(o.data() as i32);
                return Err(e);
            }
        }
        Ok(())
    }

    pub fn reregister(
        &mut self,
        fd: RawFd,
        interest: Interest,
        mode: Mode,
        token: Token,
    ) -> io::Result<()> {
        let write_flags = if interest.contains_write() {
            EventFlag::EV_ADD | EventFlag::EV_RECEIPT | mode_to_flag(mode)
        } else {
            EventFlag::EV_DELETE
        };
        let read_flags = if interest.contains_read() {
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
                token.to_usize() as isize,
            ),
            KEvent::new(
                fd as usize,
                EventFilter::EVFILT_READ,
                read_flags,
                FilterFlag::empty(),
                0,
                token.to_usize() as isize,
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

        kevent_ts(self.kq, &changes, &mut out, None).map_err(no_nix_err)?;

        for o in &out {
            if o.flags().contains(EventFlag::EV_ERROR) && o.data() != 0 {
                let e = io::Error::from_raw_os_error(o.data() as i32);
                // ignore NotFound error which is raised if we tried to remove a non-existent filter
                if e.kind() != io::ErrorKind::NotFound {
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    pub fn unregister(&mut self, fd: RawFd) -> io::Result<()> {
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

        kevent_ts(self.kq, &changes, &mut out, None).map_err(no_nix_err)?;

        // Report an error if *both* fd were missing, meaning we were not registered at all
        let mut notfound = 0;

        for o in &out {
            if o.flags().contains(EventFlag::EV_ERROR) && o.data() != 0 {
                let e = io::Error::from_raw_os_error(o.data() as i32);
                // ignore NotFound error which is raised if we tried to remove a non-existent filter
                if e.kind() != io::ErrorKind::NotFound {
                    return Err(e);
                } else {
                    notfound += 1;
                }
            }
        }

        if notfound == 2 {
            return Err(io::ErrorKind::NotFound.into());
        }

        Ok(())
    }
}

impl Drop for Kqueue {
    fn drop(&mut self) {
        let _ = nix::unistd::close(self.kq);
    }
}
