// R2DBC SPI stub for the `java-r2dbc` checker. See
// `reactor/core/publisher/Mono.java` for why this stub set exists; the
// signatures here are copied from R2DBC SPI 1.0.
package io.r2dbc.spi;

import org.reactivestreams.Publisher;

public interface ConnectionFactory {
    Publisher<? extends Connection> create();
}
