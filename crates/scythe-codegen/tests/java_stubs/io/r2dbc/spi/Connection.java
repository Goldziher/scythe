// R2DBC SPI stub for the `java-r2dbc` checker. See `ConnectionFactory.java`.
package io.r2dbc.spi;

import org.reactivestreams.Publisher;

public interface Connection {
    Statement createStatement(String sql);

    Publisher<Void> beginTransaction();

    Publisher<Void> commitTransaction();

    Publisher<Void> rollbackTransaction();

    Publisher<Void> close();
}
