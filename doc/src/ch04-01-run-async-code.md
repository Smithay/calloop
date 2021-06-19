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
use std::time::Duration;
use async_std::task::sleep;

sleep(Duration::from_secs(1)).await;
print!("Hello, ");
sleep(Duration::from_secs(1)).await;
println!("world!");
"Bye!"
```

To run it in Calloop, you use the components in [`calloop::futures`](api/calloop/futures/). First, obtain both an executor and a scheduler with [`calloop::futures::executor()`](api/calloop/futures/fn.executor.html):

```rust
{{#rustdoc_include async_example.rs:decl_executor}}
```

The *executor*, the part that *executes* the future, goes in the event loop:

```rust
{{#rustdoc_include async_example.rs:decl_loop}}
```

Note that we haven't even done anything with any async code yet. We could, but we don't *have* to. It can happen in any order. The executor will sit there in the loop until the scheduler tells it there's something to run. So let's do that:

```rust
{{#rustdoc_include async_example.rs:decl_async}}
```

Now the async code will be run as expected in our loop, printing:

    Hello,
    world!
    Bye!

...with a one second delay after the "`Hello,`".

Other tips:

- Remember that the value of your async block is the last expression ie. it is effectively "returned" from the block, which means it will be provided to your executor's callback as the first argument.
- If you need to send data repeatedly, use a channel. The sending end of an MPSC channel can be cloned or moved into an async block, and the receiving end can be added to the event loop as a source.
- If Calloop is missing an event source or abstraction you need, see if there's an async crate for it!