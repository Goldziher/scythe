// Reactor's `Flux`, stubbed for the `java-r2dbc` checker. See `Mono.java` in
// this directory for why these stubs exist and why the generic bounds are
// copied verbatim from Reactor 3 rather than loosened.
//
// Note `usingWhen`'s closure parameter differs from `Mono`'s on purpose: the
// real `Flux.usingWhen` takes `? extends Publisher<? extends T>` where
// `Mono.usingWhen` takes `? extends Mono<? extends T>`. `java-r2dbc` relies on
// exactly that difference -- its `:many` path hands the closure a `Flux`.
package reactor.core.publisher;

import java.util.function.Consumer;
import java.util.function.Function;
import org.reactivestreams.Publisher;

public abstract class Flux<T> implements Publisher<T> {

    public static <T> Flux<T> from(Publisher<? extends T> source) {
        throw new UnsupportedOperationException("stub");
    }

    public static <T> Flux<T> error(Throwable error) {
        throw new UnsupportedOperationException("stub");
    }

    public static <T, D> Flux<T> usingWhen(
            Publisher<D> resourceSupplier,
            Function<? super D, ? extends Publisher<? extends T>> resourceClosure,
            Function<? super D, ? extends Publisher<?>> asyncCleanup) {
        throw new UnsupportedOperationException("stub");
    }

    public abstract <R> Flux<R> flatMap(Function<? super T, ? extends Publisher<? extends R>> mapper);

    public abstract <R> Flux<R> map(Function<? super T, ? extends R> mapper);

    public abstract Mono<Void> then();

    public abstract Mono<T> next();

    public abstract Mono<java.util.List<T>> collectList();

    public abstract Flux<T> onErrorResume(Function<? super Throwable, ? extends Publisher<? extends T>> fallback);

    public abstract Flux<T> doFinally(Consumer<Object> onFinally);
}
