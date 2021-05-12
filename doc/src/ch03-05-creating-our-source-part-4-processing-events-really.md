# Creating our source, part IV: processing events (really)

We have three events that could wake up our event source: the ping, the channel and the zsocket itself becoming ready to use. *All three of these reasons* potentially mean doing something on the zsocket: if the ping fired, we need to check for any pending events. If the channel received a message, we want to check if the zsocket is already readable and send it. If the zsocket becomes readable or writeable, we want to read from or write to it.

When you think about it this way... why do we even need to check for the zsocket token sub ID? We want to run it every time! So our first job is to get rid of the `else if token.sub_id == Self::ID_SOCKET` block:

```rust,noplayground
self.socket
    .process_events(readiness, token, |_, _| {
        let events = self.socket.events()?;
    
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

We're not done yet. There's some more cruft to remove. Notice that in the zsocket `process_events()` call, we don't use any of the arguments. This is also true for our ping source, but there's an important difference: the ping source is "level triggered", which means that if we don't process its internal events, it will just keep waking up our source forever. Our zsocket source is edge triggered, so we can dispense with `process_events()` call altogether:

```rust,noplayground
let events = self.socket.events()?;

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
```

So the second draft of our `process_events()` function is now:

```rust,noplayground
fn process_events<F>(
    &mut self,
    readiness: calloop::Readiness,
    token: calloop::Token,
    mut callback: F,
) -> io::Result<()>
where
    F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
{
    // We were woken up on startup/registration.
    if token.sub_id == Self::ID_WAKER {
        self.wake_ping_receiver
            .process_events(readiness, token, |_, _| {})?;
    }
    // We received a message over the MPSC channel.
    else if token.sub_id == Self::ID_CHANNEL {
        let outbox = &mut self.outbox;

        self.mpsc_receiver
            .process_events(readiness, token, |evt, _| {
                if let calloop::channel::Event::Msg(msg) = evt {
                    outbox.push_back(msg);
                }
            })?;
    }

	// Always process any pending zsocket events.

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

    Ok(())
}
```

There is one more issue to take care of, and it's got nothing to do with Calloop. We still haven't fully dealt with ZeroMQ's edge-triggered nature.

Consider this situation:

- We create a REQ zsocket. These are intended to be used in strict send/receive/send/receive/etc. sequence.
- We wrap it in our `ZeroMQSource` and add that to our loop.
- We send a message.

If we do this, it's possible we'll never actually *receive* any replies that are sent to our zsocket! Why? Because:

- we read the events on the socket into `events`
- then we send a message on the socket
- another process sends a reply so quickly, it arrives more or less immediately
- then we use the same `events` to check if the socket is readable
- then we exit

The zsocket will change from writeable to readable before we leave `process_events()`. So the "interface" file descriptor will become readable again. But because it is edge triggered, it will not wake up our event source after we leave `process_events()`. So our source will not wake up again (at least, not due to the `self.socket` event source).

For *this specific example*, it will suffice to re-read the zsocket events in between the `if` statements. Then when we get to the second `events` check, it will indeed contain `zmq::POLLIN` and receive the pending message. But this is not good enough for the general case! If we replace REQ with REP above, we'll get the opposite problem: our first check (for `POLLOUT`) will be false. Our second check (`POLLIN`) will be true. We'll receive a message, leave `process_events()`, and never wake up again.

The full solution is to recognise that any user action on a ZeroMQ socket can cause the pending events to change, or just to remain active, without re-triggering the "interface" file descriptor. So we need to (a) do this repeatedly and (b) keep track of when we have or haven't performed an action on the zsocket. Here's one way to do it:

```rust,noplayground
loop {
    let events = self.socket.get_events()?;
    let mut used_socket = false;

    if events.contains(zmq::POLLOUT) {
        if let Some(parts) = self.outbox.pop_front() {
            self.socket
                .as_ref()
                .send_multipart(parts, 0)?;
            used_socket = true;
        }
    }

    if events.contains(zmq::POLLIN) {
        let messages = self.socket.recv_multipart(0)?;
        used_socket = true;

        callback(messages, &mut ())?;
    }

    if !used_socket {
        break;
    }
}
```

Now we have a flag that we set if, and only if, we call a send or receive method on the zsocket. If that flag is set at the end of the loop, we go around again.

> ## Greediness
> Remember my disclaimer at the start of the chapter, about this code being "greedy"? This is what I mean. This loop will run until the entire message queue is empty, so if it has a lot of messages in it, any other sources in our event loop will not be run until this loop is finished.
>
> An alternative approach is to use more state to determine whether we want to run again on the next loop iteration (perhaps using the ping source), so that Calloop can run any other sources in between individual messages being received.