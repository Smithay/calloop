# Use a network socket

This will almost certainly be a specialised case of [adapting a blocking IO type](ch04-03-use-blocking-io-types.md) — as long as your socket type implements the [`AsRawFd`](https://doc.rust-lang.org/stable/std/os/unix/io/trait.AsRawFd.html) trait, you can use the approach there.

If it is something more sophisticated, it may expose some other way to integrate with event loops eg. exposing a file descriptor for notification like [ZeroMQ does](ch03-00-a-full-example-zeromq.md). In that case you can create a new type that combines components from Calloop to utilise such an interface.