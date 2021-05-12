# How an event loop works

An event loop is one way to write *concurrent* code. Other ways include threading (sort of), or asynchronous syntax.

When you write concurrent code, you need to know two things:

- where in your program it could potentially *block*, and
- how to let other parts of your program run while waiting for those operations to stop blocking

This chapter covers what the first thing means, and how Calloop accomplishes the second thing.

## Terminology

A *blocking* operation is one that waits for an event to happen, and doesn't use the CPU while it's waiting. For example, if you try to read from a network socket, and there is no data available, the read operation could wait for some indefinite amount of time. Your program will be in a state where it does not need to use any CPU cycles, but it won't proceed until there is data to read.

Examples of blocking operations are:

- waiting for a certain time to elapse
- reading or writing to a file
- reading or writing to a network socket
- waiting for a thread to finish

When any of these operations are ready to go, we call it an *event*. We call the underlying things (files, network sockets, timers, etc.) *sources* for events. So, for example, you can create an event source that corresponds to a file, and it will generate events when it is ready for reading, or writing, or encounters an error.

## Events and callbacks

An event loop like Calloop, as the name suggests, runs in a loop.  At the start of the loop, Calloop checks all the sources you've added to see if any events have happened for those sources. If they have, Calloop will call a function that you provide (known as a *callback*).

This function will (possibly) be given some data for the event itself (eg. the bytes received), some state for the event source (eg. the socket, or a type that wraps it in a neater API), and some state for the whole program.

Calloop will do this one by one for each source that has a new event. If a file is ready for reading, your file-event-source callback will be called. If a timer has elapsed, your timer-event-source callback will be called.

It is up to you to write the code to do things when events happen. For example, your callback might read data from a file "ready for reading" event into a queue. When the queue contains a valid message, the same callback could send that message over an internal channel to another event source. That second event source could have its own callback that processes entire messages and updates the program's state. And so on.

## Concurrency vs parallelism

This "one by one" nature of event loops is important. When you approach concurrency using threads, operations in any thread can be interleaved with operations in any other thread. This is typically made robust by either passing messages or using shared memory with synchronisation.

Callbacks in an event loop do not run in parallel, they run one after the other. Unless you (or your dependencies) have introduced threading, you can (and should) write your callbacks as single-threaded code.

## Event loops vs async code

This single-threaded nature makes event loops much more similar to code that uses `async`/`await` than to multithreaded code. There are benefits and tradeoffs to either approach.

Calloop will take care of a lot of integration and error handling boilerplate for you. It also makes it clearer what parts of your code are the non-blocking actions to perform as a result of events. If you like to think of your program in terms of taking action in reaction to events, this can be a great advantage!

However, this comes at the expense of needing to make your program's state much more explicit. For example, take this async code:

```rust,noplayground
do_thing_one().await;
do_thing_two().await;
do_thing_three().await;
```

The state of the program is simply given by: what line is it up to? You know if it's done "thing one" because execution has proceeded to line two. No other state is required. In Calloop, however, you will need extra variables and code so that when your callback is called, it knows whether to run `do_thing_one()`, `do_thing_two()`, or `do_thing_three()`.

## Never block the loop!

Both of the above principles lead us to the most important rule of event loop code: **never block the loop!** This means: never use blocking calls inside one of your event callbacks. Do not use synchronous file `write()` calls in a callback. Do not `sleep()` in a callback. Do not `join()` a thread in a callback. Don't you do it!

If you do, the event loop will have no way to proceed, and just... wait for your blocking operation to complete. Nothing is going to run in a parallel thread. Nothing is going to stop your callback and move on to the next one. If your callback needs to wait for a blocking operation, your code must allow it to keep track of where it's up to, return from the callback, and wait for the event like any other.

## Calloop and composition

Calloop is designed to work by *composition*. This means that you build up more complex logic in your program by combining simpler event sources. Want a network socket with custom backoff/timeout logic? Create a type containing a network socket from the [async IO adapter](api/calloop/io/), a [timer](api/calloop/timer), and add your backoff state logic. There is a much more detailed example of composition in our [ZeroMQ example](ch03-00-a-full-example-zeromq.md).
