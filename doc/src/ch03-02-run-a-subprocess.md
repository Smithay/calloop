# Run a subprocess

This one is pretty easy after reading [how to run async code](ch04-01-run-async-code.md)!

1. Add a dependency on [`async_process`](https://crates.io/crates/async-process).
2. Use it in an `async` block.
3. Add the async block to the loop with [`calloop::futures::executor()`](api/calloop/futures/fn.executor.html).

And you're done!