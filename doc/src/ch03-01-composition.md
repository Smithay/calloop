# Composing event sources
Calloop is designed to work by *composition*. It provides you with some single-responsibility sources (timers, message channels, file descriptors), and you can combine these together bit by bit to make more complex sources. You can greatly simplify even a highly complex program if you identify and expose the "real" events you care about and use composition to tidy the other events away in internal details of event sources.

So what do we need to compose?

## The generic source

Most obviously, ZeroMQ exposes a file descriptor for us to use. (This is a common thing for event-related libraries to do, so if you're wondering how to integrate, say, I²C or GPIO on Linux with Calloop, that's your answer.)

Calloop can use file descriptors via the `calloop::generic::Generic` source. So that's one.

## The MPSC channel source

Secondly, we might want to send messages on the socket. This means our event source needs to react when we send it a message. Calloop has a message channel for precisely this purpose: `calloop::channel::Channel`. That's another one.

## The wakeup call

The third event source we need is a bit subtle, but since this isn't a mystery novel I can save you hours of debugging and spoil the ending now: we need a "ping" event source because ZeroMQ's FD is edge triggered.

[ZeroMQ's file descriptor](http://api.zeromq.org/master:zmq-getsockopt#toc11) is not the FD of an actual file or socket — you do not actually read data from it. It exists as an interface, with three important details:

- It is only ever readable. Even if the underlying socket can be written to, the FD that ZeroMQ gives you signals this by becoming readable. In fact, this FD will become readable under three circumstances: the ZeroMQ socket (henceforth called a "zsocket") is readable, writeable, or has an error. There is a separate function call, `zmq::Socket::get_events()` that will tell you which.

- **It is edge triggered.** It will only ever change from not-readable to readable when the socket's state changes. So if a zsocket receives two messages, and you only read one, **the file descriptor will not wake up the event loop again**. Why not? Because it hasn't changed state! After you read one message, the zsocket still has events waiting. If it receives yet another message... it still has events waiting. No change in internal state = no external event.

- This edge triggering **also covers user actions.** If a zsocket becomes writeable, and then you write to the zsocket, it might immediately (and atomically) change from writeable to readable. In this case **you will not get another event on the FD**.

(The docs make this quite explicit, but there's a lot of docs to read so I'm spelling it out here.)

What this adds up to is this: when we create our zsocket, it might already be readable or writeable. So when we add it to our event loop, it won't fire any events. Our entire source will just sit there until we wake it up by sending a message (which we might never do if it's eg. a pull socket).

So the last event source we need is something that doesn't really convey any kind of message except "please wake up the event loop on the next iteration", and that is exactly what a `calloop::ping::PingSource` does. And that's three.