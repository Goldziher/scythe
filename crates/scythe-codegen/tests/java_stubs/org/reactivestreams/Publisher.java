// Reactive Streams' root interface, stubbed so `javac` can resolve the
// `io.r2dbc.spi` and `reactor.core.publisher` signatures in this directory.
// See `reactor/core/publisher/Mono.java` for why these stubs exist at all and
// what "faithful" means for them.
//
// The real interface declares `subscribe(Subscriber<? super T>)`. Nothing
// generated ever calls it, and leaving it out avoids dragging `Subscriber`,
// `Subscription`, and `Processor` in behind it -- none of which participate in
// the type inference this stub set exists to reproduce.
package org.reactivestreams;

public interface Publisher<T> {}
