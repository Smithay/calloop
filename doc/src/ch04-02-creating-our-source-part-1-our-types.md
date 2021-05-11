# Creating our source, part I: our types
In the last chapter we worked out a list of the event sources we need to compose into a new type:

1. `calloop::generic::Generic`
2. `calloop::channel::Channel`
3. `calloop::ping::Ping`

So at a minimum, our type needs to contain these:

```rust,noplayground
pub struct ZeroMQSource
{
    // Calloop components.
    socket_source: calloop::generic::Generic<calloop::generic::Fd>,
    mpsc_receiver: calloop::channel::Channel<?>,
    wake_ping_receiver: calloop::ping::PingSource,
}
```

Note that I've left the type for the channel as `?` — we'll get to that a bit later.

What else do we need? If the `PingSource` is there to wake up the loop manually, we need to keep the other end of it. The ping is an internal detail — users of our type don't need to know it's there. We also need the zsocket itself, so we can actually detect and process events on it. That gives us:

```rust,noplayground
pub struct ZeroMQSource
{
    // Calloop components.
    socket_source: calloop::generic::Generic<calloop::generic::Fd>,
    mpsc_receiver: calloop::channel::Channel<?>,
    wake_ping_receiver: calloop::ping::PingSource,

    /// Sending end of the ping source.
    wake_ping_sender: calloop::ping::Ping,

    /// The underlying ZeroMQ socket.
    socket: zmq::Socket,
}
```

## The message type

The most obvious candidate for the type of the message queue would be `zmq::Message`. But ZeroMQ sockets are capable of sending multipart messages, and this is even mandatory for eg. the `PUB` zsocket type, where the first part of the message is the topic.

Therefore it makes more sense to accept a sequence of messages to cover the most general case, and that sequence can have a length of one for single-part messages. But with one more tweak: we can accept a sequence of things that *can be transformed* into `zmq::Message` values. The exact type we'll use will be a generic type like so:

```rust,noplayground
pub struct ZeroMQSource<T>
where
    T: IntoIterator,
    T::Item: Into<zmq::Message>,
{
    mpsc_receiver: calloop::channel::Channel<T>,
	// ...
}
```

> ### Enforcing single messages
> Remember that it's not just `Vec<T>` and other sequence types that implement `IntoIterator` — `Option<T>` implements it too! There is also `std::iter::Once<T>`. So if a user of our API wants to enforce that at most (or exactly) one message is sent, they can use this API with `T` being, say, `Option<zmq::Message>`.

## Associated types
The `EventSource` trait has three associated types:

- `Event` - when an event is generated that our caller cares about (ie. not some internal thing), this is the data we provide to their callback. This will be another sequence of messages, but because we're constructing it we can be more opinionated about the type and use the return type of `zmq::Socket::recv_multipart()` which is `Vec<Vec<u8>>`.

- `Metadata` - this is a more persistent kind of data, perhaps the underlying file descriptor or socket, or maybe some stateful object that the callback can manipulate. It is passed by exclusive reference to the `Metadata` type. In our case we don't use this, so it's `()`.

- `Ret` - this is the return type of the callback that's called on an event. Usually this will be a `Result` of some sort; in our case it's `std::io::Result<()>` just to signal whether some underlying operation failed or not.

So together these are:

```rust,noplayground
impl<T> calloop::EventSource for ZeroMQSource<T>
where
    T: IntoIterator,
    T::Item: Into<zmq::Message>,
{
    type Event = Vec<Vec<u8>>;
    type Metadata = ();
    type Ret = io::Result<()>;
    // ...
}
```

----

I have saved one surprise for later to emphasise some important principles, but for now, let's move on to defining some methods!