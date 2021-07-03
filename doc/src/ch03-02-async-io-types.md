# Async IO types

> This section is about adapting blocking IO types for use with `async` Rust code, and powering that `async` code with Calloop. If you just want to add blocking IO types to your event loop and use Calloop's callback/composition-based design, you only need to wrap your blocking IO type in a [generic event source](api/calloop/generic/struct.Generic.html).

You may find that you need to write ordinary Rust `async` code around blocking IO types. Calloop provides the ability to wrap blocking types — anything that implements the [`AsRawFd`](https://doc.rust-lang.org/stable/std/os/unix/io/trait.AsRawFd.html) trait — in its own async type. This can be polled in any executor you may have chosen for your async code, but if you're using Calloop you'll probably be using [Calloop's executor](api/calloop/futures/fn.executor.html).

> ## Enable the `futures-io` feature!
> 
> To use `calloop::io` you need to enable the `futures-io` feature in your `Cargo.toml` like so:
> 
> ```toml
> [dependencies.calloop]
> features = [ "futures-io" ]
> version = ...
> ```
>
> Realistically you will probably also want to use this with async code, so you should also enable the `executor` feature too.

Just like in the async example, we will use the components in [`calloop::futures`](api/calloop/futures/). First, obtain both an executor and a scheduler with [`calloop::futures::executor()`](api/calloop/futures/fn.executor.html):

```rust
{{#rustdoc_include adapt_io_example.rs:decl_executor}}
```

The *executor* goes in the event loop:

```rust
{{#rustdoc_include adapt_io_example.rs:decl_loop}}
```

For our blocking IO types, let's use an unnamed pair of [Unix domain stream sockets](https://doc.rust-lang.org/stable/std/os/unix/net/struct.UnixStream.html). To convert them to async types, we simply call [`calloop::LoopHandle::adapt_io()`](api/calloop/struct.LoopHandle.html):

```rust
{{#rustdoc_include adapt_io_example.rs:decl_io}}
```

Note that most of the useful async functionality for the returned type is expressed through various traits in [`futures::io`](https://docs.rs/futures/0.3/futures/io/). So we need to explicitly `use` these:

```rust
{{#rustdoc_include adapt_io_example.rs:use_futures_io_traits}}
```

We can now write async code around these types. Here's the receiving code:

```rust
{{#rustdoc_include adapt_io_example.rs:decl_async_receive}}
```

And here's the sending code. The receiving and sending code can be created and added to the executor in either order.

```rust
{{#rustdoc_include adapt_io_example.rs:decl_async_send}}
```

All that's left is to run the loop:

```rust
{{#rustdoc_include adapt_io_example.rs:run_loop}}
```

And the output we get is:

```text
Starting event loop. Use Ctrl-C to exit.
Async block ended with: Sent data...
Async block ended with: Hello, world
^C
```