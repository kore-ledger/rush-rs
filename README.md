# Rush

**Rush** is a set of libraries and tools for building concurrent systems using [actor model](https://en.wikipedia.org/wiki/Actor_model) as described by Carl Hewitt in 1973. It has been developed from a fork of Tiny Tokio Actors, to include some of our specific needs, such as persistent actors. Also it is inspired by [Akka](https://akka.io/) and [Erlang](https://www.erlang.org/).

The actor model is a model of concurrent computation that treats "actors" as the universal primitives of concurrent computation. In response to a message that it receives, an actor can:

- make local decisions
- update it's private state
- create more actors
- send more messages
- determine how to respond to the next message received

Actors may modify their own private state, but can only affect each other indirectly through messaging (no actor can access the state of another actor directly). The actor model provides some guarantees about message delivery, such as at-most-once delivery, and message ordering.

## Features

- **Actor System**: The actor system is the core of the library. It is responsible for creating actors, managing their lifecycle, and dispatching messages to them. It is also responsible for managing the lifecycle of the threads that execute the actors.

- **Actor**: The actor is the basic unit of computation in the actor model. It is responsible for processing messages, updating its state, and sending messages to other actors. It is also responsible for managing its own lifecycle.

- **Message**: The message is the basic unit of communication in the actor model. It is a data structure that contains information that is sent from one actor to another.

- **ActorRef**: The actor reference is a reference to an actor that can be used to send messages to the actor. It is an opaque handle that is used to communicate with the actor.

- **ActorContext**: The actor context is a context object that is passed to the actor when it is created. It contains information about the actor, such as its actor reference and the actor system that created it.

- **Event**: The event is a data structure representing an event that has occurred from processing a message. It is used to notify actors or other components of events in which they are interested.
