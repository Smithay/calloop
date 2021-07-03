# Creating our source, part II: setup methods

Now that we've figured out the types we need, we can get to work writing some methods. We'll need to implement the methods defined in the `calloop::EventSource` trait, and a constructor function to create the source.

## Our constructor

Creating our source is fairly straightforward. We can let the caller set up the zsocket the way they need, and take ownership of it when it's initialised. Our caller needs not only the source itself, but the sending end of the MPSC channel so they can send messages, so we need to return that too.

A common pattern in Calloop's own constructor functions is to return a tuple containing (a) the source and (b) a type to use the source. So that's what we'll do:

```rust,noplayground
// Converts a `zmq::Socket` into a `ZeroMQSource` plus the sending end of an
// MPSC channel to enqueue outgoing messages.
pub fn from_socket(socket: zmq::Socket) -> io::Result<(Self, calloop::channel::Sender<T>)> {
    let (mpsc_sender, mpsc_receiver) = calloop::channel::channel();
    let (wake_ping_sender, wake_ping_receiver) = calloop::ping::make_ping()?;

    let fd = socket.get_fd()?;

    let socket_source =
        calloop::generic::Generic::from_fd(fd, calloop::Interest::READ, calloop::Mode::Edge);

    Ok((
        Self {
            socket,
            socket_source,
            mpsc_receiver,
            wake_ping_receiver,
            wake_ping_sender,
        },
        mpsc_sender,
    ))
}
```


## Trait methods: registering sources

Calloop's event sources have a kind of life cycle, starting with *registration*. When you add an event source to the event loop, under the hood the source will *register* itself with the loop. Under certain circumstances a source will need to re-register itself. And finally there is the *unregister* action when an event source is removed from the loop. These are expressed via the `calloop::EventSource` methods:

- `fn register(&mut self, poll: &mut calloop::Poll, token_factory: &mut calloop::TokenFactory) -> std::io::Result<()>`
- `fn reregister(&mut self, poll: &mut calloop::Poll, token_factory: &mut calloop::TokenFactory) -> std::io::Result<()>`
- `fn unregister(&mut self, poll: &mut calloop::Poll) -> std::io::Result<()>`

The first two methods take a *token factory*, which is a way for Calloop to keep track of why your source was woken up. When we get to actually processing events, you'll see how this works. But for now, all you need to do is recursively pass the token factory into whatever sources your own event source is composed of. This includes other composed sources, which will pass the token factory into *their* sources, and so on.

In practise this looks like:

```rust,noplayground
fn register(
    &mut self,
    poll: &mut calloop::Poll,
    token_factory: &mut calloop::TokenFactory
) -> io::Result<()>
{
    self.socket_source.register(poll, token_factory)?;
    self.mpsc_receiver.register(poll, token_factory)?;
    self.wake_ping_receiver.register(poll, token_factory)?;
    self.wake_ping_sender.ping();

    Ok(())
}

fn reregister(
    &mut self,
    poll: &mut calloop::Poll,
    token_factory: &mut calloop::TokenFactory
) -> io::Result<()>
{
    self.socket_source.reregister(poll, token_factory)?;
    self.mpsc_receiver.reregister(poll, token_factory)?;
    self.wake_ping_receiver.reregister(poll, token_factory)?;

    self.wake_ping_sender.ping();

    Ok(())
}


fn unregister(&mut self, poll: &mut calloop::Poll)-> std::io::Result<()> {
    self.socket_source.unregister(poll)?;
    self.mpsc_receiver.unregister(poll)?;
    self.wake_ping_receiver.unregister(poll)?;
    Ok(())
}
```

> **Note the `self.wake_ping_sender.ping()` call in the first two functions!** This is how we manually prompt the event loop to wake up and run our source on the next iteration, to properly account for the [zsocket's edge-triggering](ch03-01-composition.md#the-wakeup-call).

## Our drop handler

ZeroMQ sockets have their own internal queues and state, and therefore need a bit of care when shutting down. Depending on zsocket type and settings, when the ZeroMQ context is dropped, it could block waiting for certain operations to complete. We can write a drop handler to avoid this, but again *note that it's only one of many ways* to handle zsocket shutdown.

```rust,noplayground
impl<T> Drop for ZeroMQSource<T>
where
    T: IntoIterator,
    T::Item: Into<zmq::Message>,
{
    fn drop(&mut self) {
        // This is one way to stop socket code (especially PUSH sockets) hanging
        // at the end of any blocking functions.
        //
        // - https://stackoverflow.com/a/38338578/188535
        // - http://api.zeromq.org/4-0:zmq-ctx-term
        self.socket.set_linger(0).ok();
        self.socket.set_rcvtimeo(0).ok();
        self.socket.set_sndtimeo(0).ok();

        // Double result because (a) possible failure on call and (b) possible
        // failure decoding.
        if let Ok(Ok(last_endpoint)) = self.socket.get_last_endpoint() {
            self.socket.disconnect(&last_endpoint).ok();
        }
    }
}
```
