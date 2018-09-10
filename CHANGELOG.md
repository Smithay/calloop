# Change Log

## Unreleased

- **[breaking]** Erase the `Data` type parameter from `Source` and `Idle`, for
  improved ergonomics.

## 0.2.2 -- 2018-09-10

- Introduce an `EventLoop::run` method, as well as the `LoopSignal` handle allowing to
  wakeup or stop the event loop from anywhere.

## 0.2.1 -- 2018-09-01

- Use `FnOnce` for insertion in idle callbacks.

## 0.2.0 -- 2018-08-30

- **[breaking]** Add a `&mut shared_data` argument to `EventLoop::dispatch(..)` to share data
  between callbacks.

## 0.1.1 -- 2018-08-29

- `Generic` event source for wrapping arbitrary `Evented` types
- timer event sources
- UNIX signal event sources
- channel event sources

## 0.1.0 -- 2018-08-24

Initial release
