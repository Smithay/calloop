// ANCHOR: all
use std::time::Duration;

// async-std = "1.9"
use async_std::task::sleep;
use calloop::{EventLoop, LoopSignal};

fn main() -> std::io::Result<()> {
    // ANCHOR: decl_executor
    let (exec, sched) = calloop::futures::executor()?;
    // ANCHOR_END: decl_executor

    // ANCHOR: decl_loop

    // Just like in the timer example, we want to use a LoopSignal as the loop
    // data so we can exit the program later.
    let mut event_loop: EventLoop<LoopSignal> = EventLoop::try_new()?;
    let handle = event_loop.handle();

    handle.insert_source(exec, |event, _metadata, loop_signaller| {
        // Print the value of the async block ie. the return value.
        println!("{}", event);
        // Stop the loop.
        loop_signaller.stop();
    })?;
    // ANCHOR_END: decl_loop

    // ANCHOR: decl_async

    // Our toy async code.
    let async_task = async {
        sleep(Duration::from_secs(1)).await;
        println!("Hello,");
        sleep(Duration::from_secs(1)).await;
        println!("world!");
        "Bye!"
    };

    // Schedule the async block to be run in the event loop.
    sched.schedule(async_task).unwrap();

    // Run the event loop.
    event_loop.run(None, &mut event_loop.get_signal(), |_| {})?;
    // ANCHOR_END: decl_async

    Ok(())
}
// ANCHOR_END: all
