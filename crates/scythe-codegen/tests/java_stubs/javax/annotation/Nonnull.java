// Hand-written stand-in for `javax.annotation.Nonnull`, which the
// `java-jdbc` backend's non-nullable column annotations reference (see
// `file_header` in `src/backends/java_jdbc.rs`).
//
// `javax.annotation.*` was dropped from the JDK itself in Java 11 (JSR-305
// was never bundled -- it shipped as a Maven Central jar,
// `com.google.code.findbugs:jsr305`, that generated code's real consumers
// add to their own classpath). Fetching that jar here would need network
// access this harness must not depend on, and the annotation `javac` needs
// to resolve is a one-line `@interface`, so a hand-written stub -- same
// hermetic-stub precedent as `tests/js_mode_stubs/driver-stubs.d.ts` -- costs
// far less than vendoring the real dependency.
package javax.annotation;

import java.lang.annotation.Documented;
import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;

@Documented
@Retention(RetentionPolicy.RUNTIME)
public @interface Nonnull {}
