# Run async code

> ## Enable the executor feature!
> 
> To use `calloop::futures` you need to enable the `executor` feature in your `Cargo.toml` like so:
> 
> ```toml
> [dependencies.calloop]
> features = [ "executor" ]
> version = ...

Let's say you have some async code that looks like this:

```rust,noplayground
sender.send("Hello,").await.ok();
receiver.next().await.map(|m| println!("Received: {}", m));
sender.send("world!").await.ok();
receiver.next().await.map(|m| println!("Received: {}", m));
"So long!"
```

...and a corresponding block that receives and sends to this one. I will call one of these blocks "friendly" and the other one "aloof".

To run async code in Calloop, you use the components in [`calloop::futures`](api/calloop/futures/). First, obtain both an executor and a scheduler with [`calloop::futures::executor()`](api/calloop/futures/fn.executor.html):

```rust
{{#rustdoc_include async_example.rs:decl_executor}}
```

The *executor*, the part that *executes* the future, goes in the event loop:

```rust
{{#rustdoc_include async_example.rs:decl_loop}}
```

Now let's write our async code in full:

```rust
{{#rustdoc_include async_example.rs:decl_async}}
```

Like any block in Rust, the value of your async block is the last expression ie. it is effectively "returned" from the block, which means it will be provided to your executor's callback as the first argument (the "event"). You'll see this in the output with the `Async block ended with: ...` lines.

Finally, we run the loop:

```rust
{{#rustdoc_include async_example.rs:run_loop}}
```

And our output looks like:

```text
Starting event loop.
Friendly said: Hello,
Aloof said: Oh,
Friendly said: world!
Async block ended with: Regards.
Aloof said: it's you.
Async block ended with: Bye!
Event loop ended.
```

Note that for the sake of keeping this example short, I've written the async code before running the loop. But async code can be scheduled from callbacks, or other sources within the loop too.

> ## Note about threads
> 
> One of Calloop's strengths is that it is completely single threaded as written. However, many async crates are implemented using threads eg. `async-std` and `async-process`. This is not an inherent problem! Calloop will work perfectly well with such implementations in general. However, if you have selected Calloop because of your own constraints around threading, be aware of this.