# Creating our source, part III: processing events (almost)

Finally, the real functionality we care about! Processing events! This is also a method in the `calloop::EventSource` trait:

```rust,noplayground
fn process_events<F>(
    &mut self,
    readiness: calloop::Readiness,
    token: calloop::Token,
    mut callback: F,
) -> io::Result<calloop::PostAction>
where
    F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
```

What a mouthful! But when you break it down, it's not so complicated:

- We take our own state, of course, as `&mut self`.

- We take a `Readiness` value - this is mainly useful for "real" file descriptors, and tells you whether the event source was woken up for a read or write event. We ignore it though, because our internal sources are always only readable (remember that even if the zsocket is writeable, the FD it exposes is only ever readable).

- We take a token, just like our register/re-register methods. The ID will always correspond to our own source, but we can check the sub-ID to see which of our internal sources caused us to run.

- We take a callback. We call this callback with any "real" events that our caller will care about; in our case, that means messages we receive on the zsocket. It is closely related to [the `EventSource` trait's associated types](ch03-02-creating-our-source-part-1-our-types.md#associated-types). Note that the callback our caller supplies when adding our source to the loop actually takes an extra argument, which is some data that we won't know about in our source. Calloop's internals take care of combining our arguments here with this extra data.

- Finally we return a `PostAction`, which tells the loop whether it needs to change the state of our event source, perhaps as a result of actions we took. For example, you might require that your source be removed from the loop (with `PostAction::Remove`) if it only has a certain thing to do. Ordinarily though, you'd return `PostAction::Continue` for your source to keep waiting for events.

Note that these `PostAction` values correspond to various methods on the `LoopHandle` type (eg. `PostAction::Disable` does the same as `LoopHandle::disable()`). Whether you control your event source by returning a `PostAction` or using the `LoopHandle` methods depends on whether it makes more sense for these actions to be taken from within your event source or by something else in your code.

Implementing `process_events()` for a type that contains various Calloop sources composed together, like we have, is done recursively by calling our internal sources' `process_events()` method. The `token` that Calloop gives us is how each event source determines whether it was responsible for the wakeup and has events to process.

If we were woken up because of the ping source, then the ping source's `process_events()` will see that the token matches its own, and call the callback (possibly multiple times). If we were woken up because a message was sent through the MPSC channel, then the channel's `process_events()` will match on the token instead and call the callback for every message waiting. The zsocket is a little different, and we'll go over that in detail.

So a first draft of our code might look like:

```rust,noplayground
fn process_events<F>(
    &mut self,
    readiness: calloop::Readiness,
    token: calloop::Token,
    mut callback: F,
) -> io::Result<calloop::PostAction>
where
    F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
{
    // Runs if we were woken up on startup/registration.
    self.wake_ping_receiver
        .process_events(readiness, token, |_, _| {})?;

    // Runs if we received a message over the MPSC channel.
    self.mpsc_receiver
        .process_events(readiness, token, |evt, _| {
            // 'evt' could be a message or a "sending end closed"
            // notification. We don't care about the latter.
            if let calloop::channel::Event::Msg(msg) = evt {
                self.socket.send_multipart(msg, 0)?;
            }
        })?;

	// Runs if the zsocket became read/write-able.
    self.socket
        .process_events(readiness, token, |_, _| {
            let events = self.socket.get_events()?;
        
            if events.contains(zmq::POLLOUT) {
                // Wait, what do we do here?
            }

            if events.contains(zmq::POLLIN) {
                let messages = self.socket.recv_multipart(0)?;
                callback(messages, &mut ())?;
            }
        })?;

    Ok(calloop::PostAction::Continue)
}
```

We process the events from whichever source woke up our composed source, and if we woke up because the zsocket became readable, we call the callback with the message we received. Finally we return `PostAction::Continue` to remain in the event loop.

Don't worry about getting this to compile, it is a good start but it's wrong in a few ways.

Firstly, we've gone to all the trouble of using a ping to wake up the source, and then we just... drain its internal events and return. Which achieves nothing.

Secondly, we don't seem to know what to do when our zsocket becomes writeable (the actual zsocket, not the "interface" file descriptor).

Thirdly, we commit one of the worst sins you can commit in an event-loop-based system. Can you see it? It's this part:

```rust,noplayground
self.mpsc_receiver
    .process_events(readiness, token, |evt, _| {
        if let calloop::channel::Event::Msg(msg) = evt {
            self.socket.send_multipart(msg, 0)?;
        }
    })?;
```

We block the event loop! In the middle of processing events from the MPSC channel, we call `zmq::Socket::send_multipart()` which *could*, under certain circumstances, block! [**We shouldn't do that.**](ch01-00-how-an-event-loop-works.md#never-block-the-loop)

Let's deal with this badness first then. We want to decouple "receiving messages over the MPSC channel" from "sending messages on the zsocket". There are different ways to do this, but they boil down to: buffer messages or drop messages (or maybe a combination of both). We'll use the first approach, with an internal FIFO queue. When we receive messages, we push them onto the back of the queue. When the zsocket is writeable, we pop messages from the front of the queue.

The standard library has `collections::VecDeque<T>` which provides efficient double-ended queuing, so let's use that. This is some extra internal state, so we need to add it to our type, which becomes:

```rust,noplayground
pub struct ZeroMQSource<T>
where
    T: IntoIterator,
    T::Item: Into<zmq::Message>,
{
    // Calloop components.
    socket_source: calloop::generic::Generic<calloop::generic::Fd>,
    mpsc_receiver: calloop::channel::Channel<T>,
    wake_ping_receiver: calloop::ping::PingSource,

    /// Sending end of the ping source.
    wake_ping_sender: calloop::ping::Ping,

    /// The underlying ZeroMQ socket.
    socket: zmq::Socket,

    /// FIFO queue for the messages to be published.
    outbox: std::collections::VecDeque<T>,
}
```

Our MPSC receiving code becomes:

```rust,noplayground
let outbox = &mut self.outbox;

self.mpsc_receiver
    .process_events(readiness, token, |evt, _| {
        if let calloop::channel::Event::Msg(msg) = evt {
            outbox.push_back(msg);
        }
    })?;
```

And our "zsocket is writeable" code becomes:

```rust,noplayground
self.socket
    .process_events(readiness, token, |_, _| {
        let events = self.socket.get_events()?;
    
        if events.contains(zmq::POLLOUT) {
            if let Some(parts) = self.outbox.pop_front() {
                self.socket
                    .send_multipart(parts, 0)?;
            }
       }

        if events.contains(zmq::POLLIN) {
            let messages = self.socket.recv_multipart(0)?;
            callback(messages, &mut ())?;
        }
    })?;

```

So we've not only solved problem #3, we've also figured out #2, which suggests we're on the right track. But we still have (at least) that first issue to sort out.