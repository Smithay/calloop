# Using event sources

Calloop's structure is entirely built around the `EventSource` trait, which represents something that is capable of generating events. To receive those events, you need to give ownership of the event source to calloop, along with a closure that will be invoked whenever said source generated an event. This is thus a push-based model, that is most suited for contexts where your program needs to react to (unpredictable) outside events, rather than wait efficiently for the completion of some operation it initiated.
