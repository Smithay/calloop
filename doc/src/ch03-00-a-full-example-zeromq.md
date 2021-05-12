# A full example: ZeroMQ
The previous chapter showed how to use callbacks, event data and shared data to control our program. However, more complex programs will require more complex shared data, and more complex interactions between events. Eventually this will run up against ownership issues and just the basic mental load of the poor programmer.

In this chapter we're going to create a more complex program: one based on [ZeroMQ sockets](https://zeromq.org/).

ZeroMQ is (very) basically a highly abstracted socket library. You can create ZeroMQ sockets over TCP, PGM, IPC, in-process and more, and generally not worry about the transport mechanics. It guarantees atomic message transfer, and handles queuing, retries, reconnection and balancing under the hood. It **also** lets you integrate it with event loops and reactors by exposing a file descriptor that you can wait on.

But we can't just wrap this file descriptor in Calloop's `generic::Generic` source and be done — there are a few subtleties we need to take care of for it to work right and be useful.

> ## Disclaimer
> It might be tempting, at the end of this chapter, to think that the code we've written is *the* definitive ZeroMQ wrapper, able to address any use case or higher level pattern you like. Certainly it will be a lot more suited to Calloop than using ZeroMQ sockets by themselves, but it is not the only way to use them with Calloop. Here are some things I have not addressed, for the sake of simplicity:
>
> - We will not handle fairness — our code will totally monopolise the event loop if we receive many messages at once.
> - We do not consider [back pressure](https://lucumr.pocoo.org/2020/1/1/async-pressure/) beyond whatever built-in zsocket settings the caller might use.
> - We just drop pending messages in the zsocket's internal queue (in and out) on shutdown. In a real application, you might want to make more specific decisions about the timeout and linger periods before dropping the zsocket, depending on your application's requirements.
  > - We don't deal with zsocket errors much. In fact, the overall error handling of event sources is usually highly specific to your application, so what we end up writing here is almost certainly not going to survive contact with your own code base. Here we just use `?` everywhere, which will eventually cause the event loop to exit with an error.
>
> So by all means, take the code we write here and use and adapt it, but please *please* note the caveats above and think carefully about your own program.