# Change Log

## Unreleased

## 0.6.5 -- 2020-10-07

#### Fixes

- Channel now signals readinnes after the event has actually been sent, fixing a race
  condition where the event loop would try to read the message before it has been
  written.

## 0.6.4 -- 2020-08-30

#### Fixes

- Fix double borrow during dispatch when some event source is getting removed

## 0.6.3 -- 2020-08-27

#### Aditions

- Add support for `openbsd`, `netbsd`, and `dragonfly`.
- `InsertError<E>` now implements `std::error::Error`.

#### Changes

- Allow non-`'static` dispatch `Data`. `Data` is passed as an argument to the
  `callback`s while dispatching. This change allows defining `Data` types which
  can hold references to other values.
- `dispatch` now will retry on `EINTR`.

## 0.6.2 -- 2020-04-23

- Update the README and keywords for crates.io

## 0.6.1 -- 2020-04-22

- Introduce `LoopHandle::kill` to allow dropping a source from within its callback

## 0.6.0 -- 2020-04-22

- Drop the `mio` dependency
- **Breaking Change**: Significantly rework the `calloop` API, notably:
  - Event sources are now owned by the `EventLoop`
  - Users can now again set the polling mode (Level/Edge/OneShot)
- Introduce the `Ping` event source

## 0.5.2 -- 2020-04-14

- `channel::Channel` is now `Send`, allowing you to create a channel in one thread and sending
  its receiving end to an other thread for event loop insertion.

## 0.5.1 -- 2020-03-14

- Update `mio` to `0.7`

## 0.5.0 -- 2020-02-07

- Update to 2018 edition
- Update `nix` dependency to `0.17`
- **Breaking** Update `mio` dependency to `0.7.0-alpha.1`. The API of `calloop` for custom
  event sources significantly changed, and the channel and timer event sources are now
  implemented in `calloop` rather than pulled from `mio-extras`.

## 0.4.4 -- 2019-06-13

- Update `nix` dependency to `0.14`

## 0.4.3 -- 2019-02-17

- Update `mio` dependency
- Update `nix` dependency

## 0.4.2 -- 2018-11-15

- Implement `Debug` for `InsertError`.

## 0.4.1 -- 2018-11-14

- Disable the `sources::signal` module on FreeBSD so that the library can be built on this
  platform.

## 0.4.0 -- 2018-11-04

- **Breaking** Use `mio-extras` instead of `mio-more` which is not maintained.
- **Breaking** Reexport `mio` rather than selectively re-exporting some of its types.
- Add methods to `Generic` to retrive the inner `Rc` and construct it from an `Rc`
- **Breaking** `LoopHandle::insert_source` now allows to retrieve the source on error.

## 0.3.2 -- 2018-09-25

- Fix the contents of `EventedRawFd` which was erroneously not public.

## 0.3.1 -- 2018-09-25

- introduce `EventedRawFd` as a special case for the `Generic` event source, for when
  you really have to manipulate raw fds
- Don't panic when the removal of an event source trigger the removal of an other one

## 0.3.0 -- 2018-09-10

- Fixed a bug where inserting an event source from within a callback caused a panic.
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
