// R2DBC SPI stub for the `java-r2dbc` checker. See `ConnectionFactory.java`.
//
// `map`'s `<T>` is the type variable the whole row-mapping lambda infers
// through, so its bound (`? extends T` on the `BiFunction` result) is copied
// exactly -- loosening it to a raw `BiFunction` would make the checker accept
// a row map real R2DBC rejects.
package io.r2dbc.spi;

import java.util.function.BiFunction;
import org.reactivestreams.Publisher;

public interface Result {
    Publisher<Long> getRowsUpdated();

    <T> Publisher<T> map(BiFunction<Row, RowMetadata, ? extends T> mappingFunction);
}
