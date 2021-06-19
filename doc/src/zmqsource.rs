//! A Calloop event source implementation for ZeroMQ sockets.

use std::{collections, io};

/// A Calloop event source that contains a ZeroMQ socket (of any kind) and a
/// Calloop MPSC channel for sending over it.
///
/// The basic interface is:
/// - create a zmq::Socket for your ZeroMQ socket
/// - use `ZeroMQSource::from_socket()` to turn it into a Calloop event source
///   (plus the sending end of the channel)
/// - queue messages to be sent by sending them on the sending end of the MPSC
///   channel
/// - add the event source to the Calloop event loop with a callback to handle
///   reading
/// - the sending end of the MPSC channel can be cloned and sent across threads
///   if necessary
///
/// This type is parameterised by `T`:
///
///     T where T: IntoIterator, T::Item: Into<zmq::Message>
//
/// This means that `T` is anything that can be converted to an iterator, and
/// the items in the iterator are anything that can be converted to a
/// `zmq::Message`. So eg. a `Vec<String>` would work.
///
/// The callback is called whenever the underlying socket becomes readable. It
/// is called with a vec of byte sequences (`Vec<Vec<u8>>`) and the event loop
/// data set by the user.
///
/// Note about why the read data is a vec of multipart message parts: we don't
/// know what kind of socket this is, or what will be sent, so the most general
/// thing we can do is receive the entirety of a multipart message and call the
/// user callback with the whole set. Usually the number of parts in a multipart
/// message will be one, but the code will work just the same when it's not.
///
/// This event source also allows you to use different event sources to publish
/// messages over the same writeable ZeroMQ socket (usually PUB or PUSH).
/// Messages should be sent over the Calloop MPSC channel sending end. This end
/// can be cloned and used by multiple senders.

pub struct ZeroMQSource<T>
where
    T: IntoIterator,
    T::Item: Into<zmq::Message>,
{
    // Calloop components.
    /// Event source for ZeroMQ socket.
    socket_source: calloop::generic::Generic<calloop::generic::Fd>,

    /// Event source for channel.
    mpsc_receiver: calloop::channel::Channel<T>,

    /// Because the ZeroMQ socket is edge triggered, we need a way to "wake" the
    /// event source upon (re-)registration. We do this with a separate
    /// `calloop::ping::Ping` source.
    wake_ping_receiver: calloop::ping::PingSource,

    /// Sending end of the ping source.
    wake_ping_sender: calloop::ping::Ping,

    // ZeroMQ socket.
    /// The underlying ZeroMQ socket that we're proxying things to.
    socket: zmq::Socket,

    /// FIFO queue for the messages to be published.
    outbox: collections::VecDeque<T>,
}

impl<T> ZeroMQSource<T>
where
    T: IntoIterator,
    T::Item: Into<zmq::Message>,
{
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
                outbox: collections::VecDeque::new(),
            },
            mpsc_sender,
        ))
    }
}

/// This event source runs for three events:
///
/// 1. The event source was registered. It is forced to run so that any pending
///    events on the socket are processed.
///
/// 2. A message was sent over the MPSC channel. In this case we put it in the
///    internal queue.
///
/// 3. The ZeroMQ socket is readable. For this, we read off a complete multipart
///    message and call the user callback with it.
///
/// The callback provided to `process_events()` may be called multiple times
/// within a single call to `process_events()`.
impl<T> calloop::EventSource for ZeroMQSource<T>
where
    T: IntoIterator,
    T::Item: Into<zmq::Message>,
{
    type Event = Vec<Vec<u8>>;
    type Metadata = ();
    type Ret = io::Result<()>;

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

        // Runs if we were woken up because a message was sent on the channel.
        let outbox = &mut self.outbox;

        self.mpsc_receiver
            .process_events(readiness, token, |evt, _| {
                if let calloop::channel::Event::Msg(msg) = evt {
                    outbox.push_back(msg);
                }
            })?;

        // The ZeroMQ file descriptor is edge triggered. This means that if (a)
        // messages are added to the queue before registration, or (b) the
        // socket became writeable before messages were enqueued, we will need
        // to run the loop below. Hence, it always runs if this event source
        // fires. The process_events() method doesn't do anything though, so we
        // ignore it.

        loop {
            // According to the docs, the edge-triggered FD will not change
            // state if a socket goes directly from being readable to being
            // writeable (or vice-versa) without there being an in-between point
            // where there are no events. This can happen as a result of sending
            // or receiving on the socket while processing such an event. The
            // "used_socket" flag below tracks whether we perform an operation
            // on the socket that warrants reading the events again.
            let events = self.socket.get_events()?;
            let mut used_socket = false;

            if events.contains(zmq::POLLOUT) {
                if let Some(parts) = self.outbox.pop_front() {
                    self.socket.send_multipart(parts, 0)?;
                    used_socket = true;
                }
            }

            if events.contains(zmq::POLLIN) {
                // Batch up multipart messages. ZeroMQ guarantees atomic message
                // sending, which includes all parts of a multipart message.
                let messages = self.socket.recv_multipart(0)?;
                used_socket = true;

                // Capture and report errors from the callback, but don't propagate
                // them up.
                callback(messages, &mut ())?;
            }

            if !used_socket {
                break;
            }
        }

        Ok(calloop::PostAction::Continue)
    }

    fn register(
        &mut self,
        poll: &mut calloop::Poll,
        token_factory: &mut calloop::TokenFactory,
    ) -> io::Result<()> {
        self.socket_source.register(poll, token_factory)?;
        self.mpsc_receiver.register(poll, token_factory)?;
        self.wake_ping_receiver.register(poll, token_factory)?;

        self.wake_ping_sender.ping();

        Ok(())
    }

    fn reregister(
        &mut self,
        poll: &mut calloop::Poll,
        token_factory: &mut calloop::TokenFactory,
    ) -> io::Result<()> {
        self.socket_source.reregister(poll, token_factory)?;
        self.mpsc_receiver.reregister(poll, token_factory)?;
        self.wake_ping_receiver.reregister(poll, token_factory)?;

        self.wake_ping_sender.ping();

        Ok(())
    }

    fn unregister(&mut self, poll: &mut calloop::Poll) -> io::Result<()> {
        self.socket_source.unregister(poll)?;
        self.mpsc_receiver.unregister(poll)?;
        self.wake_ping_receiver.unregister(poll)?;
        Ok(())
    }
}

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
