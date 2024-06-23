// ANCHOR: all
use std::time::Duration;

use calloop::{
    timer::{TimeoutAction, Timer},
    EventLoop,
};

fn main() {
    let mut event_loop = EventLoop::try_new().expect("Failed to initialize the event loop!");

    // ANCHOR: decl_source
    let timer = Timer::from_duration(Duration::from_secs(5));
    // ANCHOR_END: decl_source

    // ANCHOR: insert_source
    event_loop
        .handle()
        .insert_source(timer, |deadline, _: &mut (), _shared_data| {
            println!("Event fired for: {:?}", deadline);
            TimeoutAction::Drop
        })
        .expect("Failed to insert event source!");
    // ANCHOR_END: insert_source

    event_loop
        .dispatch(None, &mut ())
        .expect("Error during event loop!");
}
// ANCHOR_END: all
