use ahash::AHashMap;
use glob::glob;
use serde_json::Value;
use std::fs;

fn build_type_map() -> AHashMap<&'static str, &'static str> {
    let mut m = AHashMap::new();
    // Scalars
    m.insert("bool", "bool");
    m.insert("i16", "int16");
    m.insert("i32", "int32");
    m.insert("i64", "int64");
    m.insert("f32", "float32");
    m.insert("f64", "float64");
    m.insert("String", "string");
    m.insert("Vec<u8>", "bytes");
    m.insert("uuid::Uuid", "uuid");
    m.insert("rust_decimal::Decimal", "decimal");
    m.insert("chrono::NaiveDate", "date");
    m.insert("chrono::NaiveTime", "time");
    m.insert("sqlx::postgres::types::PgTimeTz", "time_tz");
    m.insert("chrono::NaiveDateTime", "datetime");
    m.insert("chrono::DateTime<chrono::Utc>", "datetime_tz");
    m.insert("sqlx::postgres::types::PgInterval", "interval");
    m.insert("serde_json::Value", "json");
    m.insert("ipnetwork::IpNetwork", "inet");
    // Compound types
    m.insert("Vec<i32>", "array<int32>");
    m.insert("Vec<String>", "array<string>");
    m.insert("UserStatus", "enum::user_status");
    m.insert("Address", "composite::address");
    m.insert("sqlx::postgres::types::PgRange<i32>", "range<int32>");
    m.insert(
        "sqlx::postgres::types::PgRange<chrono::DateTime<chrono::Utc>>",
        "range<datetime_tz>",
    );
    m.insert("sqlx::types::Json<EventData>", "json_typed<EventData>");
    m
}

/// Strip Option<...> wrapper, handling nested angle brackets.
/// Returns (inner_type, was_option).
fn strip_option(rust_type: &str) -> (&str, bool) {
    if let Some(inner) = rust_type.strip_prefix("Option<") {
        // Find the last '>' which closes the Option
        if let Some(pos) = inner.rfind('>') {
            (&inner[..pos], true)
        } else {
            (rust_type, false)
        }
    } else {
        (rust_type, false)
    }
}

fn map_rust_type(rust_type: &str, type_map: &AHashMap<&str, &str>) -> Option<String> {
    // First try direct lookup
    if let Some(neutral) = type_map.get(rust_type) {
        return Some(neutral.to_string());
    }

    // Try stripping Option<>
    let (inner, was_option) = strip_option(rust_type);
    if was_option && let Some(neutral) = type_map.get(inner) {
        return Some(neutral.to_string());
    }

    None
}

fn migrate_params_or_columns(
    arr: &mut [Value],
    type_map: &AHashMap<&str, &str>,
    unmapped: &mut Vec<String>,
) {
    for item in arr.iter_mut() {
        if let Some(obj) = item.as_object_mut()
            && let Some(rust_type_val) = obj.remove("rust_type")
        {
            if let Some(rust_type_str) = rust_type_val.as_str() {
                match map_rust_type(rust_type_str, type_map) {
                    Some(neutral) => {
                        obj.insert("type".to_string(), Value::String(neutral));
                    }
                    None => {
                        unmapped.push(rust_type_str.to_string());
                        // Put it back so we don't lose data
                        obj.insert("rust_type".to_string(), rust_type_val);
                    }
                }
            } else {
                // Not a string, put it back
                obj.insert("rust_type".to_string(), rust_type_val);
            }
        }
    }
}

fn migrate_type_overrides(overrides: &mut [Value]) {
    for item in overrides.iter_mut() {
        if let Some(obj) = item.as_object_mut()
            && let Some(val) = obj.remove("rust_type")
        {
            obj.insert("lang_type".to_string(), val);
        }
    }
}

fn main() {
    let type_map = build_type_map();
    let mut files_modified = 0u32;
    let mut unmapped_types: Vec<String> = Vec::new();

    let pattern = "testing_data/**/*.json";
    let entries: Vec<_> = glob(pattern)
        .expect("Failed to read glob pattern")
        .filter_map(Result::ok)
        .filter(|path| {
            path.file_name()
                .map(|f| f != "00-FIXTURE-SCHEMA.json")
                .unwrap_or(false)
        })
        .collect();

    println!("Found {} fixture files", entries.len());

    for path in &entries {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Error reading {}: {}", path.display(), e);
                continue;
            }
        };

        let mut root: Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Error parsing {}: {}", path.display(), e);
                continue;
            }
        };

        let mut modified = false;

        // Migrate expected.query.params[] and expected.query.columns[]
        if let Some(expected) = root.get_mut("expected") {
            if let Some(query) = expected.get_mut("query") {
                if let Some(params) = query.get_mut("params")
                    && let Some(arr) = params.as_array_mut()
                {
                    let before = arr.iter().any(|v| v.get("rust_type").is_some());
                    migrate_params_or_columns(arr, &type_map, &mut unmapped_types);
                    if before {
                        modified = true;
                    }
                }
                if let Some(columns) = query.get_mut("columns")
                    && let Some(arr) = columns.as_array_mut()
                {
                    let before = arr.iter().any(|v| v.get("rust_type").is_some());
                    migrate_params_or_columns(arr, &type_map, &mut unmapped_types);
                    if before {
                        modified = true;
                    }
                }
            }

            // Migrate expected.generated_rust → expected.generated_code.rust-sqlx
            if let Some(generated_rust) = expected
                .as_object_mut()
                .and_then(|o| o.remove("generated_rust"))
            {
                let Some(expected_obj) = expected.as_object_mut() else {
                    continue;
                };
                let generated_code = expected_obj
                    .entry("generated_code")
                    .or_insert_with(|| Value::Object(serde_json::Map::new()));
                if let Some(gc_obj) = generated_code.as_object_mut() {
                    gc_obj.insert("rust-sqlx".to_string(), generated_rust);
                }
                modified = true;
            }
        }

        // Migrate config.type_overrides[].rust_type → lang_type
        if let Some(config) = root.get_mut("config")
            && let Some(overrides) = config.get_mut("type_overrides")
            && let Some(arr) = overrides.as_array_mut()
        {
            let before = arr.iter().any(|v| v.get("rust_type").is_some());
            migrate_type_overrides(arr);
            if before {
                modified = true;
            }
        }

        if modified {
            let output = serde_json::to_string_pretty(&root).expect("Failed to serialize JSON");
            if let Err(e) = fs::write(path, output + "\n") {
                eprintln!("Error writing {}: {}", path.display(), e);
            } else {
                files_modified += 1;
            }
        }
    }

    // Deduplicate unmapped types
    unmapped_types.sort();
    unmapped_types.dedup();

    println!("\n=== Migration Summary ===");
    println!("Files modified: {}", files_modified);
    if unmapped_types.is_empty() {
        println!("No unmapped types found.");
    } else {
        println!("Unmapped types ({}):", unmapped_types.len());
        for t in &unmapped_types {
            println!("  - {}", t);
        }
    }
}
