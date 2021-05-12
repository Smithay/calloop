// ANCHOR: all
use calloop::{timer::Timer, EventLoop, LoopSignal};

fn main() {
    // ANCHOR: decl_loop
    let mut event_loop: EventLoop<LoopSignal> =
        EventLoop::try_new()
            .expect("Failed to initialize the event loop!");
    // ANCHOR_END: decl_loop

    // ANCHOR: decl_source
    let source = Timer::new()
        .expect("Failed to create timer event source!");

    let timer_handle = source.handle();
    timer_handle
        .add_timeout(Duration::from_secs(5), "Timeout reached!");
    // ANCHOR_END: decl_source

    // ANCHOR: insert_source
    let handle = event_loop.handle();

    handle
        .insert_source(
            source,
            |event, _metadata, shared_data| {
                println!("Event fired: {}", event);
                shared_data.stop();
            },
        )
        .expect("Failed to insert event source!");
    // ANCHOR_END: insert_source

    // ANCHOR: run_loop
    let mut shared_data = event_loop.get_signal();

    event_loop
        .run(None, &mut shared_data, |_shared_data| {})
        .expect("Error during event loop!");
    // ANCHOR_END: run_loop
}
// ANCHOR_END: all
