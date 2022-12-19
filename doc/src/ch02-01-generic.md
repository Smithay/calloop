# Monitoring a file descriptor
The `Generic` event source wraps a file descriptor ("fd") and fires its callback any time there are events on it ie. becoming readable or writable, or encountering an error. It's pretty simple, but it's what every other event source is based around. And since the platforms that calloop runs on expose many different kinds of events via fds, it's usually the key to using those events in calloop.

For example on Linux, fd-based interfaces are available for GPIO, I2C, USB, UART, network interfaces, timers and many other systems. Integrating these into calloop starts with obtaining the appropriate fd, creating a `Generic` event source from it, and building up a more useful, abstracted event source around that. A detailed example of this is [given for ZeroMQ](ch04-00-a-full-example-zeromq.md).

You do not have to use a low-level fd either: any type that implements [`AsFd`](https://doc.rust-lang.org/stable/std/os/fd/trait.AsFd.html) can be provided. This means that you can use a wrapper type that handles allocation and disposal itself, and implement `AsRawFd` on it so that `Generic` can manage it in the event loop.

## Creating a Generic source

Creating a `Generic` event source requires three things:
- An `OwnedFd` or a wrapper type that implements `AsFd`
- The ["interest"](api/calloop/struct.Interest.html) you have in this descriptor — this means, whether you want to generate events when the fd becomes readable, writeable, or both
- The ["mode"](api/calloop/enum.Mode.html) of event generation - level triggered or edge triggered

The easiest constructor to use is the [`new()`](api/calloop/generic/struct.Generic.html#method.new) method, but if you need control over the [associated error type](ch02-06-errors.md) there is also [`new_with_error()`](api/calloop/generic/struct.Generic.html#method.new_with_error).

## Ownership and AsFd wrappers

Rust 1.63 introduced a concept of file descriptor ownership and borrowing through the `OwnedFd` and `BorrowedFd` types, which are also available on older Rust versions through the `io-lifetimes` crate. The `AsFd` trait provides a way to get a `BorrowedFd` corresponding to a file, socket, etc. while guaranteeing the fd will be valid for the lifetime of the `BorrowedFd`.

Not all third party crates use `AsFd` yet, and may instead provide types implementing `AsRawFd`. ['AsFdWrapper'](api/calloop/generic/struct.AsFdWrapper.html) provides a way to adapt these types. To use this safely, ensure the `AsRawFd` implementation of the type it wraps returns a valid fd as long as the type exists. And to avoid an fd leak, it should ultimately be `close`d properly.

Safe types like `OwnedFd` and `BorrowedFd` should be preferred over `RawFd`s, and the use of `RawFd`s outside of implementing FFI shouldn't be necessary as libraries move to using the IO safe types and traits.
