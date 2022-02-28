//! Wrapper for a transient Calloop event source.
//!
//! If you have high level event source that you expect to remain in the event
//! loop indefinitely, and another event source nested inside that one that you
//! expect to require removal or disabling from time to time, this module can
//! handle it for you.

/// A [`TransientSource`] wraps a Calloop event source and manages its
/// registration. A user of this type only needs to perform the usual Calloop
/// calls (`process_events()` and `*register()`) and the return value of
/// [`process_events()`](crate::EventSource::process_events).
///
/// Rather than needing to check for the full set of
/// [`PostAction`](crate::PostAction) values returned from `process_events()`,
/// you can just check for `Continue` or `Reregister` and pass that back out
/// through your own `process_events()` implementation. In your registration
/// functions, you then only need to call the same function on this type ie.
/// `register()` inside `register()` etc.
///
/// For example, say you have a source that contains a channel along with some
/// other logic. If the channel's sending end has been dropped, it needs to be
/// removed from the loop. So to manage this, you use this in your struct:
///
/// ```none,actually-rust-but-see-https://github.com/rust-lang/rust/issues/63193
/// struct CompositeSource {
///    // Event source for channel.
///    mpsc_receiver: TransientSource<calloop::channel::Channel<T>>,
///
///    // Any other fields go here...
/// }
/// ```
///
/// To create the transient source, you can simply use the `Into`
/// implementation:
///
/// ```none,actually-rust-but-see-https://github.com/rust-lang/rust/issues/63193
/// let (sender, source) = channel();
/// let mpsc_receiver: TransientSource<Channel> = source.into();
/// ```
///
/// `TransientSource` implements [`EventSource`](crate::EventSource) and passes
/// through `process_events()` calls, so in the parent's `process_events()`
/// implementation you can just do this:
///
/// ```none,actually-rust-but-see-https://github.com/rust-lang/rust/issues/63193
/// fn process_events<F>(
///     &mut self,
///     readiness: calloop::Readiness,
///     token: calloop::Token,
///     callback: F,
/// ) -> Result<calloop::PostAction, Self::Error>
/// where
///     F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
/// {
///     let channel_return = self.mpsc_receiver.process_events(readiness, token, callback)?;
///
///     // Perform other logic here...
///
///     Ok(channel_return)
/// }
/// ```
///
/// Note that:
///
///   - You can call `process_events()` on the `TransientSource<Channel>` even
///     if the channel has been unregistered and dropped. All that will happen
///     is that you won't get any events from it.
///
///   - The [`PostAction`](crate::PostAction) returned from `process_events()`
///     will only ever be `PostAction::Continue` or `PostAction::Reregister`.
///     You will still need to combine this with the result of any other sources
///     (transient or not).
///
/// Once you return `channel_return` from your `process_events()` method (and
/// assuming it propagates all the way up to the event loop itself through any
/// other event sources), the event loop might call `reregister()` on your
/// source. All your source has to do is:
///
/// ```none,actually-rust-but-see-https://github.com/rust-lang/rust/issues/63193
/// fn reregister(
///     &mut self,
///     poll: &mut calloop::Poll,
///     token_factory: &mut calloop::TokenFactory,
/// ) -> crate::Result<()> {
///     self.mpsc_receiver.reregister(poll, token_factory)?;
///
///     // Other registration actions...
///
///     Ok(())
/// }
/// ```
///
/// The `TransientSource` will take care of updating the registration of the
/// inner source, even if it actually needs to be unregistered or initially
/// registered.
#[derive(Debug)]
pub enum TransientSource<T> {
    /// The source should be kept in the loop.
    Keep(T),
    /// The source needs to be registered with the loop.
    Register(T),
    /// The source needs to be disabled but kept.
    Disable(T),
    /// The source needs to be removed from the loop.
    Remove(T),
    /// The source has been removed from the loop and dropped (this might also
    /// be observed if there is a panic while changing states).
    None,
}

impl<T> TransientSource<T> {
    /// Apply a function to the enclosed source, if it exists. It will be
    /// appplied even if the source is ready to be removed or is disabled.
    pub fn map<F, U>(&mut self, f: F) -> Option<U>
    where
        F: FnOnce(&mut T) -> U,
    {
        match self {
            TransientSource::Keep(source)
            | TransientSource::Register(source)
            | TransientSource::Disable(source)
            | TransientSource::Remove(source) => Some(f(source)),
            TransientSource::None => None,
        }
    }

    /// If a caller needs to flag the contained source for removal or
    /// registration, we need to replace the enum variant safely. This requires
    /// having a `None` value in there temporarily while we do the swap.
    ///
    /// If the variant is `None` the value will not change and `replacer` will
    /// not be called.
    ///
    /// The `replacer` function here is expected to be one of the enum variant
    /// constructors eg. `replace(TransientSource::Remove)`.
    fn replace<F>(&mut self, replacer: F)
    where
        F: FnOnce(T) -> Self,
    {
        *self = match std::mem::replace(self, TransientSource::None) {
            TransientSource::Keep(source)
            | TransientSource::Register(source)
            | TransientSource::Remove(source)
            | TransientSource::Disable(source) => replacer(source),
            TransientSource::None => return,
        };
    }
}

impl<T: crate::EventSource> From<T> for TransientSource<T> {
    fn from(source: T) -> Self {
        Self::Register(source)
    }
}

impl<T: crate::EventSource> crate::EventSource for TransientSource<T> {
    type Event = T::Event;
    type Metadata = T::Metadata;
    type Ret = T::Ret;
    type Error = T::Error;

    fn process_events<F>(
        &mut self,
        readiness: crate::Readiness,
        token: crate::Token,
        callback: F,
    ) -> Result<crate::PostAction, Self::Error>
    where
        F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
    {
        let reregister = if let TransientSource::Keep(ref mut source) = self {
            let child_post_action = source.process_events(readiness, token, callback)?;

            match child_post_action {
                // Nothing needs to change.
                crate::PostAction::Continue => false,

                // Our child source needs re-registration, therefore this
                // wrapper needs re-registration.
                crate::PostAction::Reregister => true,

                // If our nested source needs to be removed or disabled, we need
                // to swap it out for the "Remove" or "Disable" variant.
                crate::PostAction::Disable => {
                    self.replace(TransientSource::Disable);
                    true
                }

                crate::PostAction::Remove => {
                    self.replace(TransientSource::Remove);
                    true
                }
            }
        } else {
            false
        };

        let post_action = if reregister {
            crate::PostAction::Reregister
        } else {
            crate::PostAction::Continue
        };

        Ok(post_action)
    }

    fn register(
        &mut self,
        poll: &mut crate::Poll,
        token_factory: &mut crate::TokenFactory,
    ) -> crate::Result<()> {
        match self {
            TransientSource::Keep(source) => {
                source.register(poll, token_factory)?;
            }
            TransientSource::Register(source) | TransientSource::Disable(source) => {
                source.register(poll, token_factory)?;
                self.replace(TransientSource::Keep);
            }
            TransientSource::Remove(_source) => {
                *self = TransientSource::None;
            }
            TransientSource::None => (),
        }
        Ok(())
    }

    fn reregister(
        &mut self,
        poll: &mut crate::Poll,
        token_factory: &mut crate::TokenFactory,
    ) -> crate::Result<()> {
        match self {
            TransientSource::Keep(source) => source.reregister(poll, token_factory)?,
            TransientSource::Register(source) => {
                source.register(poll, token_factory)?;
                self.replace(TransientSource::Keep);
            }
            TransientSource::Disable(source) => {
                source.unregister(poll)?;
            }
            TransientSource::Remove(source) => {
                source.unregister(poll)?;
                *self = TransientSource::None;
            }
            TransientSource::None => (),
        }
        Ok(())
    }

    fn unregister(&mut self, poll: &mut crate::Poll) -> crate::Result<()> {
        match self {
            TransientSource::Keep(source)
            | TransientSource::Register(source)
            | TransientSource::Disable(source) => source.unregister(poll)?,
            TransientSource::Remove(source) => {
                source.unregister(poll)?;
                *self = TransientSource::None;
            }
            TransientSource::None => (),
        }
        Ok(())
    }
}
