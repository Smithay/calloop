# Monitoring a file descriptor
The `Generic` event source wraps a file descriptor ("fd") and fires its callback any time there are events on it ie. becoming readable or writable, or encountering an error. It's pretty simple, but it's what every other event source is based around. And since the platforms that calloop runs on expose many different kinds of events via fds, it's usually the key to using those events in calloop.

For example on Linux, fd-based interfaces are available for GPIO, I2C, USB, UART, network interfaces, timers and many other systems. Integrating these into calloop starts with obtaining the appropriate fd, creating a `Generic` event source from it, and building up a more useful, abstracted event source around that. A detailed example of this is [given for ZeroMQ](ch04-00-a-full-example-zeromq.md).

You do not have to use a low-level fd either: any type that implements [`AsRawFd`](https://doc.rust-lang.org/beta/std/os/unix/io/trait.AsRawFd.html) can be provided. This means that you can use a wrapper type that handles allocation and disposal itself, and implement `AsRawFd` on it so that `Generic` can manage it in the event loop.

## Creating a Generic source

Creating a `Generic` event source requires three things:
- The fd itself, of course, possibly in a wrapper type that implements `AsRawFd`
- The ["interest"](api/calloop/struct.Interest.html) you have in this descriptor — this means, whether you want to generate events when the fd becomes readable, writeable, or both
- The ["mode"](api/calloop/enum.Mode.html) of event generation - level triggered or edge triggered

The easiest constructor to use is the [`new()`](api/calloop/generic/struct.Generic.html#method.new) method, but if you need control over the [associated error type](ch02-06-errors.md) there is also [`new_with_error()`](api/calloop/generic/struct.Generic.html#method.new_with_error).

## Ownership and AsRawFd wrappers

It's important to remember that file descriptors by themselves have no concept of ownership attached to them in Rust — they are simply bare integers. Dropping them does close the resource they refer to, and copying them does not carry information about how many there are.

Typically (eg. in the standard library) they would be an underlying implementation detail of another type that *did* encode ownership somehow. This how you can manage them in any of your own integration code - use drop handlers, reference counting (if necessary) and general RAII principles in a wrapper type, and then implement `AsRawFd` to allow `Generic` to use it. The `Generic` source will take ownership of it, so it will be dropped when the `Generic` is.

This means you need to do at least two things:
- follow the rules of the API you obtain the fd from
- wrap them in a type that manages ownership appropriately

For example, on Unix-like systems the [`UdpSocket`](https://doc.rust-lang.org/beta/std/net/struct.UdpSocket.html) contains a fd, and upon being dropped, `libc::close(fd)` is called on it. If you create a `Generic<UdpSocket>` then, it will be closed upon dropping it.