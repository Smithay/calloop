// These tests cannot run as a regular test because cargo would spawn a thread to run it,
// failing the signal masking. So we make our own, non-threaded harnessing

#[cfg(unix)]
fn main() {
    for test in self::test::TESTS {
        test();
        // reset the signal mask between tests
        self::test::reset_mask();
    }
}

#[cfg(not(unix))]
fn main() {}

#[cfg(unix)]
mod test {
    extern crate calloop;
    extern crate nix;

    use std::io;
    use std::time::Duration;

    use self::calloop::signals::{Signal, Signals};
    use self::calloop::EventLoop;

    use self::nix::sys::signal::{kill, SigSet};
    use self::nix::unistd::Pid;

    pub const TESTS: &'static [fn()] = &[single_usr1, usr2_added_afterwards, usr2_signal_removed];

    pub fn reset_mask() {
        SigSet::empty().thread_set_mask().unwrap();
    }

    fn single_usr1() {
        let mut event_loop = EventLoop::new().unwrap();

        let mut signal_received = false;

        let _signal_source = event_loop
            .handle()
            .insert_source(
                Signals::new(&[Signal::SIGUSR1]).unwrap(),
                move |evt, rcv| {
                    assert!(evt.signal() == Signal::SIGUSR1);
                    *rcv = true;
                },
            )
            .map_err(Into::<io::Error>::into)
            .unwrap();

        // send ourselves a SIGUSR1
        kill(Pid::this(), Signal::SIGUSR1).unwrap();

        event_loop
            .dispatch(Some(Duration::from_millis(10)), &mut signal_received)
            .unwrap();

        assert!(signal_received);
    }

    fn usr2_added_afterwards() {
        let mut event_loop = EventLoop::new().unwrap();

        let mut signal_received = None;

        let mut signal_source = event_loop
            .handle()
            .insert_source(
                Signals::new(&[Signal::SIGUSR1]).unwrap(),
                move |evt, rcv| {
                    *rcv = Some(evt.signal());
                },
            )
            .map_err(Into::<io::Error>::into)
            .unwrap();

        signal_source.add_signals(&[Signal::SIGUSR2]).unwrap();

        // send ourselves a SIGUSR2
        kill(Pid::this(), Signal::SIGUSR2).unwrap();

        event_loop
            .dispatch(Some(Duration::from_millis(10)), &mut signal_received)
            .unwrap();

        assert_eq!(signal_received, Some(Signal::SIGUSR2));
    }

    fn usr2_signal_removed() {
        let mut event_loop = EventLoop::new().unwrap();

        let mut signal_received = None;

        let mut signal_source = event_loop
            .handle()
            .insert_source(
                Signals::new(&[Signal::SIGUSR1, Signal::SIGUSR2]).unwrap(),
                move |evt, rcv| {
                    *rcv = Some(evt.signal());
                },
            )
            .map_err(Into::<io::Error>::into)
            .unwrap();

        signal_source.remove_signals(&[Signal::SIGUSR2]).unwrap();

        // block sigusr2 anyway, to not be killed by it
        let mut set = SigSet::empty();
        set.add(Signal::SIGUSR2);
        set.thread_block().unwrap();

        // send ourselves a SIGUSR2
        kill(Pid::this(), Signal::SIGUSR2).unwrap();

        event_loop
            .dispatch(Some(Duration::from_millis(10)), &mut signal_received)
            .unwrap();

        // we should not have received anything, as we don't listen to SIGUSR2 any more
        assert!(signal_received.is_none());

        // swap the signals from [SIGUSR1] to [SIGUSR2]
        signal_source.set_signals(&[Signal::SIGUSR2]).unwrap();

        event_loop
            .dispatch(Some(Duration::from_millis(10)), &mut signal_received)
            .unwrap();

        // we should get back the pending SIGUSR2 now
        assert_eq!(signal_received, Some(Signal::SIGUSR2));
    }
}
