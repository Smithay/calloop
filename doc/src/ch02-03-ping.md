# "Pinging" the event loop

The [`Ping`](api/calloop/ping/struct.Ping.html) event source has one very simple job — wake up the event loop. Use this when you know there are events for your event source to process, but those events aren't going to wake the event loop up themselves.

For example, calloop's own [`message channel`](api/calloop/channel/struct.Channel.html) uses Rust's native MPSC channel internally. Because there's no way for the internal message queue to wake up the event loop, it's coupled with a `Ping` source that wakes the loop up when there are new messages.

## How to use the Ping source

The `Ping` has two ends — the event source part ([`PingSource`](api/calloop/ping/struct.PingSource.html)), that goes in the event loop, and the sending end (`Ping`) you use to "send" the ping. To wake the event loop up, call [`ping()`](api/calloop/ping/struct.Ping.html#method.ping) on the sending end.

> Do not forget to process the events of the `PingSource` if you are using it as part of a larger event source! Even though the events carry no information (they are just `()` values), the `process_events()` method must be called in order to "reset" the `PingSource`. Otherwise the event loop will be continually woken up until you do, effectively becoming a busy-loop.