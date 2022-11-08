# Working with timers

Timer event sources are used to manipulate time-related actions. Those are provided under the [`calloop::timer`](api/calloop/timer/index.html) module, with the `Timer` type at its core.

A `Timer` source has a simple behavior: it is programmed to wait for some duration, or until a certain point in time. Once that deadline is reached, the source generates an event.

So with `use calloop::timer::Timer` at the top of our `.rs` file, we can create a timer that will wait for 5 seconds:

```rust,noplayground
{{#rustdoc_include timer_example.rs:decl_source}}
```

## Adding sources to the loop
We have an event source, we have our shared data, and we know how to start our loop running. All that is left is to learn how to combine these things:

```rust,noplayground
{{#rustdoc_include timer_example.rs:insert_source}}
```

Breaking this down, the callback we provide receives 3 arguments:

- The first one is an [`Instant`](https://doc.rust-lang.org/stable/std/time/struct.Instant.html) representing the time at which this timer was scheduled to expire. Due to how the event loop works, it might be that your callback is not invoked at the exact time where the timer expired (if an other callback was being processed at the time for example), so the original deadline is given if you need precise time tracking.
- The second argument is just `&mut ()`, as the timers don't use the `EventSource` functionality.
- The third argumument is the shared data passed to your event loop.

In addition your callback is expected to return a [`TimeoutAction`](api/calloop/timer/enum.TimeoutAction.html), that will instruct calloop what to do next. This enum has 3 values:

- `Drop` will disable the timer and destroy it, freeing the callback.
- `ToInstant` will rechedule the callback to fire again at given `Instant`, invoking the same callback again. This is useful if you need to create a timer that fires events at regular intervals, for example to encode key repetition in a graphical app. You would compute the next instant by adding the duration to the previous instant. It is not a problem if that duration is in the past, it'll simply cause the timer to fire again instantly. THis way, even if some other part of your app lags, you'll still have on average the correct amount of events per second.
- `ToDuration` will reschedule the callback to fire again after a given `Duration`. This is useful if you need to schedule some background task to execute again after some time after it was last completed, when there is no point in catching up some previous lag.

## The whole program

Putting it all together, we have:

```rust,noplayground
{{#rustdoc_include timer_example.rs:all}}
```

