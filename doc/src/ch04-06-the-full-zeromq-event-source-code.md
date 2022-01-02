# The full ZeroMQ event source code

This is the full source code for a Calloop event source based on a ZeroMQ socket. You might find it useful as a kind of reference. Please read the [disclaimer at the start of this chapter](ch03-00-a-full-example-zeromq.md#disclaimer) if you skipped straight here!

```rust
{{#rustdoc_include zmqsource.rs}}
```

Dependencies are:
- calloop (whatever version this document was built from)
- zmq 0.9
- anyhow 1.0

```toml
[dependencies]
calloop = { path = '../..' }
zmq = "0.9"
anyhow = "1.0"
```
