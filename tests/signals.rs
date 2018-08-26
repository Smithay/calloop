// These tests cannot run as a regular test because cargo would spawn a thread to run it,
// failing the signal masking. So we make our own, non-threaded harnessing

#[cfg(unix)]
fn main() {
    self::test::single_usr1();
}

#[cfg(not(unix))]
fn main() {}

#[cfg(unix)]
mod test {
    extern crate calloop;
    extern crate nix;

    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    use self::calloop::signals::{Signal, Signals};
    use self::calloop::EventLoop;

    use self::nix::sys::signal::kill;
    use self::nix::unistd::Pid;

    pub fn single_usr1() {
        let mut event_loop = EventLoop::new().unwrap();

        let signal_received = Rc::new(Cell::new(false));
        let signal_received2 = signal_received.clone();

        let _signal_source = event_loop
            .handle()
            .insert_source(Signals::new(&[Signal::SIGUSR1]).unwrap(), move |evt| {
                assert!(evt.signal() == Signal::SIGUSR1);
                signal_received2.set(true);
            })
            .unwrap();

        // send ourselves a SIGUSR1
        kill(Pid::this(), Signal::SIGUSR1).unwrap();

        event_loop
            .dispatch(Some(Duration::from_millis(10)))
            .unwrap();

        assert!(signal_received.get());
    }
}
