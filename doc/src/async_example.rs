// ANCHOR: all
use calloop::EventLoop;

// futures = "0.3"
use futures::sink::SinkExt;
use futures::stream::StreamExt;

fn main() -> std::io::Result<()> {
    // ANCHOR: decl_executor
    let (exec, sched) = calloop::futures::executor()?;
    // ANCHOR_END: decl_executor

    // ANCHOR: decl_loop
    let mut event_loop = EventLoop::new()?;
    let handle = event_loop.handle();

    handle
        .insert_source(exec, |evt, _metadata, _shared| {
            // Print the value of the async block ie. the return value.
            println!("Async block ended with: {}", evt);
        })
        .map_err(|e| e.error)?;
    // ANCHOR_END: decl_loop

    // ANCHOR: decl_async
    // Let's create two channels for our async blocks below. The blocks will
    // exchange messages via these channels.
    let (mut sender_friendly, mut receiver_friendly) = futures::channel::mpsc::unbounded();
    let (mut sender_aloof, mut receiver_aloof) = futures::channel::mpsc::unbounded();

    // Our toy async code.
    let async_friendly_task = async move {
        sender_friendly.send("Hello,").await.ok();
        if let Some(msg) = receiver_aloof.next().await {
            println!("Aloof said: {}", msg);
        }
        sender_friendly.send("world!").await.ok();
        if let Some(msg) = receiver_aloof.next().await {
            println!("Aloof said: {}", msg);
        }
        "Bye!"
    };

    let async_aloof_task = async move {
        if let Some(msg) = receiver_friendly.next().await {
            println!("Friendly said: {}", msg);
        }
        sender_aloof.send("Oh,").await.ok();
        if let Some(msg) = receiver_friendly.next().await {
            println!("Friendly said: {}", msg);
        }
        sender_aloof.send("it's you.").await.ok();
        "Regards."
    };
    // ANCHOR_END: decl_async

    // ANCHOR: run_loop
    // Schedule the async block to be run in the event loop.
    sched.schedule(async_friendly_task).unwrap();
    sched.schedule(async_aloof_task).unwrap();

    // Run the event loop.
    println!("Starting event loop. Use Ctrl-C to exit.");
    event_loop.run(None, &mut (), |_| {})?;
    println!("Event loop ended.");
    // ANCHOR_END: run_loop

    Ok(())
}
// ANCHOR_END: all
