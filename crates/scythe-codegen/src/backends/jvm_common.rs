//! Reader helpers shared by the five JVM backends (`java-jdbc`, `java-r2dbc`,
//! `kotlin-jdbc`, `kotlin-r2dbc`, `kotlin-exposed`).
//!
//! Every one of those backends used to resolve a column's accessor from a
//! hardcoded table of neutral-ish type spellings whose final arm was an
//! *untyped* fallback -- `rs.getObject(col)` on the JDBC family,
//! `row.get(col, Object.class)` / `row.get(col, Any::class.java)` on the R2DBC
//! pair. The declared field type, meanwhile, comes from the manifest. The two
//! were free to disagree, and for every type outside that table they did: the
//! accessor's static type is `Object`/`Any`, which is assignable to nothing,
//! so the emitted file did not compile (`incompatible types: Object cannot be
//! converted to TortureAddress`, `actual type is 'Any!', but 'UUID' was
//! expected`).
//!
//! The fix is to stop maintaining a second, independent table of types. A
//! reference-typed column is read through the accessor overload that *takes
//! the target class* -- `rs.getObject(col, T.class)` (JDBC 4.1) and
//! `row.get(col, T.class)` (R2DBC) -- with `T` derived from the declared type
//! itself. Declaration and reader then cannot drift apart, because there is
//! only one of them. The named accessors below stay for the types JDBC gives
//! a directly typed getter (`getInt`, `getString`, `getBytes`, ...), which is
//! both idiomatic and, for the primitives, the only way to reach `wasNull()`
//! semantics.
//!
//! What this does *not* claim: that every driver can satisfy every class it is
//! now handed. `rs.getObject("home_address", TortureAddress.class)` against
//! pgjdbc with no registered mapping throws `conversion to class
//! TortureAddress ... not supported` at runtime. That is a driver-level
//! failure with a message naming the type, which is strictly better than a
//! file that never compiles -- and it is now the *only* remaining failure
//! mode, rather than one of two.

/// Strip generic arguments from a declared type, leaving something that can be
/// suffixed into a class literal.
///
/// `List<String>` has no class literal in Java (`List<String>.class` is a
/// syntax error); the raw `List.class` is the only spelling, and assigning its
/// `List` result to a `List<String>` field is an unchecked-but-legal
/// conversion. Types with no generic arguments are returned unchanged.
pub(crate) fn erase_generics(lang_type: &str) -> &str {
    match lang_type.find('<') {
        Some(index) => lang_type[..index].trim_end(),
        None => lang_type,
    }
}

/// The Java class literal for a declared type: `TortureAddress` ->
/// `TortureAddress.class`, `java.util.UUID` -> `java.util.UUID.class`,
/// `byte[]` -> `byte[].class`.
pub(crate) fn java_class_literal(java_type: &str) -> String {
    format!("{}.class", erase_generics(java_type))
}

/// The Kotlin class literal for a declared type: `TortureAddress` ->
/// `TortureAddress::class.java`.
///
/// Kotlin's primitive-backed types are handled by
/// [`kotlin_boxed_class_literal`] instead -- `Int::class.java` is
/// `int.class`, the primitive `Class` object, which no JDBC or R2DBC driver
/// returns a value for.
pub(crate) fn kotlin_class_literal(kotlin_type: &str) -> String {
    format!("{}::class.java", erase_generics(kotlin_type))
}

/// Kotlin's types that map to a JVM primitive, and therefore need
/// `::class.javaObjectType` (the boxed `Class`) rather than `::class.java`
/// (the primitive one) when handed to a driver that returns boxed values.
fn is_kotlin_primitive(kotlin_type: &str) -> bool {
    matches!(
        kotlin_type,
        "Boolean" | "Byte" | "Short" | "Int" | "Long" | "Float" | "Double" | "Char"
    )
}

/// The Kotlin class literal to hand a driver accessor that returns a boxed
/// value: the boxed `Class` for primitive-backed types, the plain one
/// otherwise.
pub(crate) fn kotlin_boxed_class_literal(kotlin_type: &str) -> String {
    if is_kotlin_primitive(kotlin_type) {
        format!("{kotlin_type}::class.javaObjectType")
    } else {
        kotlin_class_literal(kotlin_type)
    }
}

/// The `ResultSet` accessor for the Java types JDBC exposes a directly typed
/// getter for.
///
/// `None` means there is none, and the caller must read through
/// `rs.getObject(col, T.class)` -- never bare `rs.getObject(col)`, whose
/// static type is `Object`.
pub(crate) fn java_named_getter(java_type: &str) -> Option<&'static str> {
    match java_type {
        "boolean" | "Boolean" => Some("getBoolean"),
        "byte" | "Byte" => Some("getByte"),
        "short" | "Short" => Some("getShort"),
        "int" | "Integer" => Some("getInt"),
        "long" | "Long" => Some("getLong"),
        "float" | "Float" => Some("getFloat"),
        "double" | "Double" => Some("getDouble"),
        "String" => Some("getString"),
        "byte[]" => Some("getBytes"),
        _ if java_type.contains("BigDecimal") => Some("getBigDecimal"),
        _ => None,
    }
}

/// The `ResultSet` accessor for the Kotlin types JDBC exposes a directly typed
/// getter for. See [`java_named_getter`] for what `None` obliges the caller to
/// do.
pub(crate) fn kotlin_named_getter(kotlin_type: &str) -> Option<&'static str> {
    match kotlin_type {
        "Boolean" => Some("getBoolean"),
        "Byte" => Some("getByte"),
        "Short" => Some("getShort"),
        "Int" => Some("getInt"),
        "Long" => Some("getLong"),
        "Float" => Some("getFloat"),
        "Double" => Some("getDouble"),
        "String" => Some("getString"),
        "ByteArray" => Some("getBytes"),
        _ if kotlin_type.contains("BigDecimal") => Some("getBigDecimal"),
        _ => None,
    }
}

/// The full `ResultSet` read call for a Java column, typed either way:
/// `rs.getInt("n")` where JDBC has a named getter, `rs.getObject("n",
/// T.class)` where it does not.
pub(crate) fn java_jdbc_read_call(column: &str, java_type: &str) -> String {
    match java_named_getter(java_type) {
        Some(getter) => format!("rs.{getter}(\"{column}\")"),
        None => format!("rs.getObject(\"{column}\", {})", java_class_literal(java_type)),
    }
}

/// The full `ResultSet` read call for a Kotlin column. See
/// [`java_jdbc_read_call`].
pub(crate) fn kotlin_jdbc_read_call(column: &str, kotlin_type: &str) -> String {
    match kotlin_named_getter(kotlin_type) {
        Some(getter) => format!("rs.{getter}(\"{column}\")"),
        None => format!("rs.getObject(\"{column}\", {})", kotlin_class_literal(kotlin_type)),
    }
}

/// Whether a neutral type names a generated enum, whose column is read as
/// text and converted, not read through a driver accessor for the enum class.
pub(crate) fn is_enum_column(neutral_type: &str) -> bool {
    neutral_type.starts_with("enum::")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erase_generics_strips_type_arguments_and_leaves_plain_types_alone() {
        assert_eq!(erase_generics("List<String>"), "List");
        assert_eq!(erase_generics("java.util.Map<String, Integer>"), "java.util.Map");
        assert_eq!(erase_generics("java.util.UUID"), "java.util.UUID");
        assert_eq!(erase_generics("byte[]"), "byte[]");
    }

    #[test]
    fn class_literals_are_valid_for_qualified_array_and_generic_types() {
        assert_eq!(java_class_literal("TortureAddress"), "TortureAddress.class");
        assert_eq!(java_class_literal("java.util.UUID"), "java.util.UUID.class");
        assert_eq!(java_class_literal("byte[]"), "byte[].class");
        assert_eq!(java_class_literal("List<String>"), "List.class");
        assert_eq!(kotlin_class_literal("TortureAddress"), "TortureAddress::class.java");
        assert_eq!(kotlin_class_literal("java.util.UUID"), "java.util.UUID::class.java");
    }

    #[test]
    fn kotlin_primitive_backed_types_use_the_boxed_class_object() {
        assert_eq!(kotlin_boxed_class_literal("Int"), "Int::class.javaObjectType");
        assert_eq!(kotlin_boxed_class_literal("Boolean"), "Boolean::class.javaObjectType");
        assert_eq!(kotlin_boxed_class_literal("String"), "String::class.java");
        assert_eq!(kotlin_boxed_class_literal("ByteArray"), "ByteArray::class.java");
    }

    /// The regression itself: every type outside the named-getter table must
    /// resolve to a *class-literal* read, never a bare `getObject`.
    #[test]
    fn types_without_a_named_getter_read_through_a_class_literal() {
        assert_eq!(java_named_getter("java.util.UUID"), None);
        assert_eq!(java_named_getter("TortureAddress"), None);
        assert_eq!(
            java_jdbc_read_call("home_address", "TortureAddress"),
            "rs.getObject(\"home_address\", TortureAddress.class)"
        );
        assert_eq!(
            kotlin_jdbc_read_call("external_id", "java.util.UUID"),
            "rs.getObject(\"external_id\", java.util.UUID::class.java)"
        );
    }

    #[test]
    fn named_getters_are_used_where_jdbc_has_one() {
        assert_eq!(java_jdbc_read_call("id", "int"), "rs.getInt(\"id\")");
        assert_eq!(java_jdbc_read_call("payload", "byte[]"), "rs.getBytes(\"payload\")");
        assert_eq!(kotlin_jdbc_read_call("id", "Int"), "rs.getInt(\"id\")");
        assert_eq!(
            kotlin_jdbc_read_call("amount", "java.math.BigDecimal"),
            "rs.getBigDecimal(\"amount\")"
        );
    }
}
