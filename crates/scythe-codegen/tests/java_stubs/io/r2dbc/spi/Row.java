// R2DBC SPI stub for the `java-r2dbc` checker. See `ConnectionFactory.java`.
//
// Both `get` overloads are declared, including the untyped
// `Object get(String)`. Keeping the untyped one is deliberate: it is the
// accessor the backend used to reach for, and a checker that could not resolve
// it would reject the old output for the wrong reason ("cannot find symbol")
// instead of the real one ("incompatible types").
package io.r2dbc.spi;

public interface Row {
    Object get(int index);

    Object get(String name);

    <T> T get(int index, Class<T> type);

    <T> T get(String name, Class<T> type);
}
