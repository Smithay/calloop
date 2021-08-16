use std::os::unix::io::RawFd;

use super::{Interest, Mode, PollEvent, Readiness, Token};

use nix::sys::epoll;

pub struct Epoll {
    epoll_fd: RawFd,
}

fn make_flags(interest: Interest, mode: Mode) -> epoll::EpollFlags {
    let mut flags = epoll::EpollFlags::empty();
    if interest.readable {
        flags |= epoll::EpollFlags::EPOLLIN;
    }
    if interest.writable {
        flags |= epoll::EpollFlags::EPOLLOUT;
    }
    match mode {
        Mode::Level => { /* This is the default */ }
        Mode::Edge => flags |= epoll::EpollFlags::EPOLLET,
        Mode::OneShot => flags |= epoll::EpollFlags::EPOLLONESHOT,
    }
    flags
}

fn flags_to_readiness(flags: epoll::EpollFlags) -> Readiness {
    Readiness {
        readable: flags.contains(epoll::EpollFlags::EPOLLIN),
        writable: flags.contains(epoll::EpollFlags::EPOLLOUT),
        error: flags.contains(epoll::EpollFlags::EPOLLERR),
    }
}

impl Epoll {
    pub(crate) fn new() -> crate::Result<Epoll> {
        let epoll_fd = epoll::epoll_create1(epoll::EpollCreateFlags::EPOLL_CLOEXEC)?;
        Ok(Epoll { epoll_fd })
    }

    pub(crate) fn poll(
        &mut self,
        timeout: Option<std::time::Duration>,
    ) -> crate::Result<Vec<PollEvent>> {
        let mut buffer = [epoll::EpollEvent::empty(); 32];
        let timeout = timeout.map(|d| d.as_millis() as isize).unwrap_or(-1);
        let n_ready = epoll::epoll_wait(self.epoll_fd, &mut buffer, timeout)?;
        let events = buffer
            .iter()
            .take(n_ready)
            .map(|event| PollEvent {
                readiness: flags_to_readiness(event.events()),
                token: unsafe { *(event.data() as usize as *const Token) },
            })
            .collect();
        Ok(events)
    }

    pub unsafe fn register(
        &mut self,
        fd: RawFd,
        interest: Interest,
        mode: Mode,
        token: *const Token,
    ) -> crate::Result<()> {
        let mut event = epoll::EpollEvent::new(make_flags(interest, mode), token as usize as u64);
        epoll::epoll_ctl(self.epoll_fd, epoll::EpollOp::EpollCtlAdd, fd, &mut event)
            .map_err(Into::into)
    }

    pub unsafe fn reregister(
        &mut self,
        fd: RawFd,
        interest: Interest,
        mode: Mode,
        token: *const Token,
    ) -> crate::Result<()> {
        let mut event = epoll::EpollEvent::new(make_flags(interest, mode), token as usize as u64);
        epoll::epoll_ctl(self.epoll_fd, epoll::EpollOp::EpollCtlMod, fd, &mut event)
            .map_err(Into::into)
    }

    pub fn unregister(&mut self, fd: RawFd) -> crate::Result<()> {
        epoll::epoll_ctl(self.epoll_fd, epoll::EpollOp::EpollCtlDel, fd, None).map_err(Into::into)
    }
}

impl Drop for Epoll {
    fn drop(&mut self) {
        let _ = nix::unistd::close(self.epoll_fd);
    }
}
