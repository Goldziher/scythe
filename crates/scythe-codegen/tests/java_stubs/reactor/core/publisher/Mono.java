// Reactor's `Mono`, stubbed for the `java-r2dbc` checker in
// `src/validation.rs`.
//
// Why a stub at all: `java-r2dbc` generates `Mono.usingWhen(...)` chains whose
// element type is only ever pinned by inference -- nothing in the generated
// source writes `Mono<GetUserRow>` twice. A checker that cannot resolve
// `Mono`/`Flux` therefore cannot check the part of the file most likely to be
// wrong (the row-mapping lambda), which is exactly what the reader defects
// behind #191/#213/#214 lived in.
//
// Why these signatures: every generic parameter and wildcard below is copied
// from Reactor 3's real declarations, because that is the whole point. A
// looser stub -- `usingWhen(Publisher<D>, Function<D, Mono<T>>, ...)` without
// the `? super`/`? extends` bounds, or a raw `flatMap` -- would accept
// generated code that real Reactor rejects, and the checker would pass
// vacuously. Verified by mutation: replacing a row-mapping class literal with
// `Object.class` makes `javac` fail against these stubs with `inference
// variable T has incompatible bounds`, the same shape the real library gives.
//
// The method bodies are unreachable: `javac` only ever type-checks against
// them, and the class is never loaded, let alone run.
package reactor.core.publisher;

import java.util.function.Consumer;
import java.util.function.Function;
import java.util.function.Supplier;
import org.reactivestreams.Publisher;

public abstract class Mono<T> implements Publisher<T> {

    public static <T> Mono<T> from(Publisher<? extends T> source) {
        throw new UnsupportedOperationException("stub");
    }

    public static <T> Mono<T> just(T data) {
        throw new UnsupportedOperationException("stub");
    }

    public static <T> Mono<T> empty() {
        throw new UnsupportedOperationException("stub");
    }

    public static <T> Mono<T> error(Throwable error) {
        throw new UnsupportedOperationException("stub");
    }

    public static <T> Mono<T> defer(Supplier<? extends Mono<? extends T>> supplier) {
        throw new UnsupportedOperationException("stub");
    }

    public static <T, D> Mono<T> usingWhen(
            Publisher<D> resourceSupplier,
            Function<? super D, ? extends Mono<? extends T>> resourceClosure,
            Function<? super D, ? extends Publisher<?>> asyncCleanup) {
        throw new UnsupportedOperationException("stub");
    }

    public abstract <R> Mono<R> flatMap(Function<? super T, ? extends Mono<? extends R>> transformer);

    public abstract <R> Flux<R> flatMapMany(Function<? super T, ? extends Publisher<? extends R>> mapper);

    public abstract <R> Mono<R> map(Function<? super T, ? extends R> mapper);

    public abstract Mono<Void> then();

    public abstract <V> Mono<V> then(Mono<V> other);

    public abstract Mono<T> onErrorResume(Function<? super Throwable, ? extends Mono<? extends T>> fallback);

    public abstract Mono<T> doFinally(Consumer<Object> onFinally);
}
