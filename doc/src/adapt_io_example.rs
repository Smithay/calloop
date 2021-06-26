// ANCHOR: all
use calloop::EventLoop;

// ANCHOR: use_futures_io_traits
// futures = "0.3"
use futures::io::{AsyncReadExt, AsyncWriteExt};
// ANCHOR_END: use_futures_io_traits

fn main() -> std::io::Result<()> {
    // ANCHOR: decl_executor
    let (exec, sched) = calloop::futures::executor()?;
    // ANCHOR_END: decl_executor

    // ANCHOR: decl_loop
    let mut event_loop = EventLoop::try_new()?;
    let handle = event_loop.handle();

    handle.insert_source(exec, |evt, _metadata, _shared| {
        // Print the value of the async block ie. the return value.
        println!("Async block ended with: {}", evt);
    })?;
    // ANCHOR_END: decl_loop

    // ANCHOR: decl_io
    let (sender, receiver) = std::os::unix::net::UnixStream::pair().unwrap();
    let mut sender = handle.adapt_io(sender).unwrap();
    let mut receiver = handle.adapt_io(receiver).unwrap();
    // ANCHOR_END: decl_io

    // ANCHOR: decl_async_receive
    let async_receive = async move {
        let mut buf = [0u8; 12];
        // Here's our async-ified Unix domain socket.
        receiver.read_exact(&mut buf).await.unwrap();
        std::str::from_utf8(&buf).unwrap().to_owned()
    };

    // Schedule the async block to be run in the event loop.
    sched.schedule(async_receive).unwrap();
    // ANCHOR_END: decl_async_receive

    // ANCHOR: decl_async_send
    let async_send = async move {
        // Here's our async-ified Unix domain socket.
        sender.write_all(b"Hello, world!").await.unwrap();
        "Sent data...".to_owned()
    };

    // Schedule the async block to be run in the event loop.
    sched.schedule(async_send).unwrap();
    // ANCHOR_END: decl_async_send

    // ANCHOR: run_loop
    // Run the event loop.
    println!("Starting event loop. Use Ctrl-C to exit.");
    event_loop.run(None, &mut event_loop.get_signal(), |_| {})?;
    println!("Event loop ended.");
    // ANCHOR_END: run_loop

    Ok(())
}
// ANCHOR_END: all
