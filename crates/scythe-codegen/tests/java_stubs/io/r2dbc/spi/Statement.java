// R2DBC SPI stub for the `java-r2dbc` checker. See `ConnectionFactory.java`.
package io.r2dbc.spi;

import org.reactivestreams.Publisher;

public interface Statement {
    Statement add();

    Statement bind(int index, Object value);

    Statement bind(String name, Object value);

    Statement bindNull(int index, Class<?> type);

    Publisher<? extends Result> execute();
}
