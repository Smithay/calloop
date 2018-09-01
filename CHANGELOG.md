# Change Log

## Unreleased

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
