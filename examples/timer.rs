use calloop::{timer::Timer, EventLoop, LoopSignal};

fn main() {
    // Create the event loop. The loop is parameterised by the kind of shared
    // data you want the callbacks to use. In this case, we want to be able to
    // stop the loop when the timer fires, so we provide the loop with a
    // LoopSignal, which has the ability to stop the loop from within events. We
    // just annotate the type here; the actual data is provided later in the
    // run() call.
    let mut event_loop: EventLoop<LoopSignal> =
        EventLoop::try_new().expect("Failed to initialize the event loop!");

    // Retrieve a handle. It is used to insert new sources into the event loop
    // It can be cloned, allowing you to insert sources from within source
    // callbacks.
    let handle = event_loop.handle();

    // Create our event source, a timer. Note that this is also parameterised by
    // the data for the events it generates. We've let Rust infer that here.
    let source = Timer::new().expect("Failed to create timer event source!");

    // Most event source APIs provide two things: an event source to go into the
    // event loop, and some way of triggering that source from elsewhere. In
    // this case, we use a handle to the timer to set timeouts.
    //
    // Note that this can go before or after the call to insert_source(), and
    // even inside another event callback.
    let timer_handle = source.handle();
    timer_handle.add_timeout(std::time::Duration::from_secs(1), "Timeout reached!");

    // Inserting an event source takes this general form. It can also be done
    // from within the callback of another event source.
    handle
        .insert_source(
            // a type which implements the EventSource trait
            source,
            // a callback that is invoked whenever this source generates an event
            |event, _metadata, shared_data| {
                // This callback is given 3 values:
                // - the event generated by the source (in our case, a string slice)
                // - &mut access to some metadata, specific to the event source (in our case, a
                //   timer handle)
                // - &mut access to the global shared data that was passed to EventLoop::run or
                //   EventLoop::dispatch (in our case, a LoopSignal object to stop the loop)
                //
                // The return type is just () because nothing uses it. Some
                // sources will expect a Result of some kind instead.
                println!("Event fired: {}", event);
                shared_data.stop();
            },
        )
        .expect("Failed to insert event source!");

    // Create the shared data for our loop.
    let mut shared_data = event_loop.get_signal();

    // Actually run the event loop. This will dispatch received events to their
    // callbacks, waiting at most 20ms for new events between each invocation of
    // the provided callback (pass None for the timeout argument if you want to
    // wait indefinitely between events).
    //
    // This is where we pass the *value* of the shared data, as a mutable
    // reference that will be forwarded to all your callbacks, allowing them to
    // share some state
    event_loop
        .run(
            std::time::Duration::from_millis(20),
            &mut shared_data,
            |_shared_data| {
                // Finally, this is where you can insert the processing you need
                // to do do between each waiting event eg. drawing logic if
                // you're doing a GUI app.
            },
        )
        .expect("Error during event loop!");
}