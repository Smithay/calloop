# Error handling in Calloop

## Overview

Most error handling crates/guides/documentation for Rust focus on one of two situations:

- Creating errors that an API can propagate out to a user of the API, or
- Making your library deal nicely with the `Result`s from closure or trait methods that it might call

Calloop has to do both of these things. It needs to provide a library user with errors that work well with `?` and common error-handling idioms in their own code, and it needs to handle errors from the callbacks you give to `process_events()` or `insert_source()`. It *also* needs to provide some flexibility in the `EventSource` trait, which is used both for internal event sources and by users of the library.

Because of this, error handling in Calloop leans more towards having separate error types for different concerns. This may mean that there is some extra conversion code in places like returning results from `process_events()`, or in callbacks that use other libraries. However, we try to make it smoother to do these conversions, and to make sure information isn't lost in doing so.

If your crate already has some form of structured error handling, Calloop's error types should pose no problem to integrate into this. All of Calloop's errors implement `std::error::Error` and can be manipulated the same as any other error types.

The place where this becomes the most complex is in the `process_events()` method on the `EventSource` trait.

## The Error type on the EventSource trait

The `EventSource` trait contains an associated type named `Error`, which forms part of the return type from `process_events()`. This type must be convertible into `Box<dyn std::error::Error + Sync + Send>`, which means you can use:

- Your own error type that implements `std::error::Error`
- A structured error type created with [*Thiserror*](https://crates.io/crates/thiserror)
- `Box<dyn std::error::Error + Sync + Send>`
- A flexible string-based error type such as [*Anyhow's*](https://crates.io/crates/anyhow) `anyhow::Error`

As a rule, if you implement `EventSource` you should try to split your errors into two different categories:

- Errors that make sense as a kind of event. These should be a part of the `Event` associated type eg. as an enum or `Result`.
- Errors that mean your event source simply cannot process more events. These should form the `Error` associated type.

For an example, take Calloop's channel type, [`calloop::channel::Channel`](api/calloop/channel/struct.Channel.html). When the sending end is dropped, no more messages can be received after that point. But this is not returned as an error when calling `process_events()`, because you still want to (and can!) receive messages sent before that point that might still be in the queue. Hence the events received by the callback for this source can be `Msg(e)` or `Closed`.

However, if the internal ping source produces an error, there is no way for the sending end of the channel to notify the receiver. It is impossible to process more events on this event source, and the caller needs to decide how to recover from this situation. Hence this is returned as a `ChannelError` from `process_events()`.

Another example might be an event source that represents a running subprocess. If the subprocess exits with a non-zero status code, or the executable can't be found, those don't mean that events can no longer be processed. They can be provided to the caller through the callback. But if the lower level sources being used to run (eg. an asynchronous executor or subprocess file descriptor) fail to work as expected, `process_events()` should return an error.
