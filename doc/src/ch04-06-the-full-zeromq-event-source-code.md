# The full ZeroMQ event source code

This is the full source code for a Calloop event source based on a ZeroMQ socket. You might find it useful as a kind of reference. Please read the [disclaimer at the start of this chapter](ch03-00-a-full-example-zeromq.md#disclaimer) if you skipped straight here!

```rust
{{#rustdoc_include zmqsource.rs}}
```

Dependencies are only `calloop` and `zmq`:

```toml
[dependencies]
calloop = "0.8"
zmq = "0.9"
```
