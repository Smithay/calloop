# Calloop's Documentation

## API

If you're looking for calloop's API documentation, they are available [on `docs.rs`](https://docs.rs/calloop/) for the released versions. There are also [the docs of the current developpment version](api).

## Tutorial

This book presents a step-by-step tutorial to get yourself familiar with calloop and how it is used:

- [Chapter 1](ch01-00-how-an-event-loop-works.md) presents the general principles of an event loop that are important to have in mind when working with calloop.
- [Chapter 2](ch02-00-event-sources.md) goes through the different kind of event sources that are provided in calloop, and provides examples of how to use them
- [Chapter 3](ch03-00-async-await.md) presents the integration with Rust's Async/Await ecosystem provided by calloop
- [Chapter 4](ch04-00-a-full-example-zeromq.md) gives a detailed example of building a custom event source in calloop, by combining other sources
