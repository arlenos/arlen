//! Whether a proposed settings write may proceed.
//!
//! Every rule here is a refusal the caller cannot talk its way out of: the
//! broker consults the app's DECLARED schema, never the request's claims about
//! itself. A request carries a key and a value; it does not carry the key's
//! type, bounds or scope, because those come from the schema and a request that
//! could supply them could lie about them.

use arlen_forage_recipe::settings::{SettingScope, SettingType, SettingsItem, SettingsSchema};
use toml::Value;

/// A proposed write to one key of one app.
#[derive(Debug, Clone, PartialEq)]
pub struct WriteRequest {
    /// The app whose settings are being written.
    pub app_id: String,
    /// The dotted key.
    pub key: String,
    /// The proposed value.
    pub value: Value,
}

/// Why a write was refused. Each variant names a rule the schema states, so the
/// caller can be told what it violated rather than just "no".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteRejection {
    /// The schema does not declare this key. An app's settings surface is
    /// exactly what it declared; writing anything else would let the broker
    /// author keys the app never agreed to answer for.
    UndeclaredKey,
    /// The key is declared as shipped-defaults-only and is not user-writable.
    NotUserWritable,
    /// The value is not of the declared type.
    WrongType {
        /// What the schema declares.
        expected: SettingType,
    },
    /// A numeric value outside the declared inclusive bounds.
    OutOfRange,
    /// An enum value that is not one of the declared options.
    NotAnOption,
}

impl std::fmt::Display for WriteRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteRejection::UndeclaredKey => write!(f, "the app does not declare this key"),
            WriteRejection::NotUserWritable => {
                write!(f, "this key ships as a default and is not user-writable")
            }
            WriteRejection::WrongType { expected } => {
                write!(f, "the value does not match the declared type {expected:?}")
            }
            WriteRejection::OutOfRange => write!(f, "the value is outside the declared range"),
            WriteRejection::NotAnOption => {
                write!(f, "the value is not one of the declared options")
            }
        }
    }
}

impl std::error::Error for WriteRejection {}

/// Decide whether `request` may be applied to the app declaring `schema`.
///
/// Returns the declared item on success, so the caller applying the write has
/// the same view of the key the decision was made from and cannot re-resolve it
/// differently.
pub fn decide_write<'a>(
    schema: &'a SettingsSchema,
    request: &WriteRequest,
) -> Result<&'a SettingsItem, WriteRejection> {
    let item = schema
        .sections
        .iter()
        .flat_map(|s| s.items.iter())
        .find(|i| i.key == request.key)
        .ok_or(WriteRejection::UndeclaredKey)?;

    // Scope is read from the schema, never from the request. `Machine` stays
    // writable here: it describes where the value lives (not carried between
    // machines by sync), not whether the user may set it.
    if item.scope == SettingScope::DefaultsOnly {
        return Err(WriteRejection::NotUserWritable);
    }

    check_type(item, &request.value)?;
    check_range(item, &request.value)?;
    check_option(item, &request.value)?;

    Ok(item)
}

/// The value must match the declared type. A `secret_ref` carries a HANDLE, so
/// it is a string here; the secret itself never reaches this file.
fn check_type(item: &SettingsItem, value: &Value) -> Result<(), WriteRejection> {
    let ok = match item.value_type {
        // A handoff row opens the app's own window and stores nothing, so
        // there is no value a write could carry. Refused rather than given a
        // permissive arm, which is why the type is matched exhaustively.
        SettingType::Handoff => false,
        SettingType::Bool => value.is_bool(),
        // An integer is accepted for a float field (TOML writes `1`, not `1.0`,
        // for a whole number), but not the reverse: silently truncating 1.5 to 1
        // would store something the user did not ask for.
        SettingType::Int | SettingType::Duration => value.is_integer(),
        SettingType::Float => value.is_float() || value.is_integer(),
        SettingType::String
        | SettingType::Enum
        | SettingType::Path
        | SettingType::Color
        | SettingType::Keybind
        | SettingType::SecretRef => value.is_str(),
        // Raw is the escape from the type vocabulary, so there is no type to
        // check. Every other rule still stands: the key must be declared and its
        // scope must permit the write, which is what keeps a raw item from
        // becoming a way to write anything anywhere.
        SettingType::Raw => true,
        SettingType::StringList => value
            .as_array()
            .is_some_and(|a| a.iter().all(Value::is_str)),
    };
    if ok {
        Ok(())
    } else {
        Err(WriteRejection::WrongType {
            expected: item.value_type,
        })
    }
}

/// Declared bounds are inclusive, and only meaningful for numerics.
fn check_range(item: &SettingsItem, value: &Value) -> Result<(), WriteRejection> {
    if !item.value_type.is_numeric() {
        return Ok(());
    }
    let Some(n) = numeric(value) else {
        return Ok(());
    };
    if item.min.is_some_and(|min| n < min) || item.max.is_some_and(|max| n > max) {
        return Err(WriteRejection::OutOfRange);
    }
    Ok(())
}

/// An enum may only hold a declared option.
fn check_option(item: &SettingsItem, value: &Value) -> Result<(), WriteRejection> {
    if item.value_type != SettingType::Enum {
        return Ok(());
    }
    // An enum sourced from live system state has no declared list to check
    // against - its valid values are the machine's audio devices, installed
    // themes and so on, which the broker cannot see. Holding it to the empty
    // `options` would refuse every write and ship a setting nobody can set.
    //
    // This costs nothing that was ever a boundary: the option list is a
    // correctness aid, and the value lands in the app's own config for the app
    // itself to interpret. The type check above still applies.
    if item.options_from.is_some() {
        return Ok(());
    }
    let Some(s) = value.as_str() else {
        return Ok(()); // Already refused by the type check.
    };
    if item.options.iter().any(|o| o.value == s) {
        Ok(())
    } else {
        Err(WriteRejection::NotAnOption)
    }
}

/// A TOML number as `f64`, for bound comparison.
fn numeric(value: &Value) -> Option<f64> {
    match value {
        Value::Integer(i) => Some(*i as f64),
        Value::Float(f) => Some(*f),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_forage_recipe::settings::{SettingOption, SettingsSection};

    fn item(key: &str, value_type: SettingType) -> SettingsItem {
        SettingsItem::new(key, value_type, "L")
    }

    fn schema_of(items: Vec<SettingsItem>) -> SettingsSchema {
        SettingsSchema {
            version: 1,
            sections: vec![SettingsSection {
                label: "S".into(),
                description: None,
                order: None,
                items,
            }],
        }
    }

    fn req(key: &str, value: Value) -> WriteRequest {
        WriteRequest {
            app_id: "org.example.App".into(),
            key: key.into(),
            value,
        }
    }

    #[test]
    fn a_declared_key_of_the_right_type_is_accepted() {
        let schema = schema_of(vec![item("enabled", SettingType::Bool)]);
        assert!(decide_write(&schema, &req("enabled", Value::Boolean(true))).is_ok());
    }

    /// The broker must not author keys the app never declared: its settings
    /// surface is exactly its schema.
    #[test]
    fn an_undeclared_key_is_refused() {
        let schema = schema_of(vec![item("known", SettingType::Bool)]);
        assert_eq!(
            decide_write(&schema, &req("unknown", Value::Boolean(true))),
            Err(WriteRejection::UndeclaredKey)
        );
    }

    /// The editor enforces scope. A caller cannot write a defaults-only key by
    /// asking nicely, because the scope comes from the schema, not the request.
    #[test]
    fn a_defaults_only_key_is_not_user_writable() {
        let mut locked = item("build_channel", SettingType::String);
        locked.scope = SettingScope::DefaultsOnly;
        let schema = schema_of(vec![locked]);
        assert_eq!(
            decide_write(&schema, &req("build_channel", Value::String("beta".into()))),
            Err(WriteRejection::NotUserWritable)
        );
    }

    /// Machine scope says where the value lives, not that it is read-only, so it
    /// must remain writable.
    #[test]
    fn a_machine_scoped_key_is_still_writable() {
        let mut machine = item("device_name", SettingType::String);
        machine.scope = SettingScope::Machine;
        let schema = schema_of(vec![machine]);
        assert!(decide_write(&schema, &req("device_name", Value::String("desk".into()))).is_ok());
    }

    #[test]
    fn a_value_of_the_wrong_type_is_refused() {
        let schema = schema_of(vec![item("enabled", SettingType::Bool)]);
        assert_eq!(
            decide_write(&schema, &req("enabled", Value::String("yes".into()))),
            Err(WriteRejection::WrongType {
                expected: SettingType::Bool
            })
        );
    }

    /// TOML writes a whole number as `1`, so a float field must accept an
    /// integer; the reverse would silently truncate the user's value.
    #[test]
    fn a_float_accepts_an_integer_but_an_int_does_not_accept_a_float() {
        let float_schema = schema_of(vec![item("scale", SettingType::Float)]);
        assert!(decide_write(&float_schema, &req("scale", Value::Integer(2))).is_ok());
        assert!(decide_write(&float_schema, &req("scale", Value::Float(1.5))).is_ok());

        let int_schema = schema_of(vec![item("count", SettingType::Int)]);
        assert_eq!(
            decide_write(&int_schema, &req("count", Value::Float(1.5))),
            Err(WriteRejection::WrongType {
                expected: SettingType::Int
            })
        );
    }

    #[test]
    fn declared_bounds_are_inclusive_and_enforced() {
        let mut bounded = item("width", SettingType::Int);
        bounded.min = Some(10.0);
        bounded.max = Some(20.0);
        let schema = schema_of(vec![bounded]);

        assert!(decide_write(&schema, &req("width", Value::Integer(10))).is_ok());
        assert!(decide_write(&schema, &req("width", Value::Integer(20))).is_ok());
        assert_eq!(
            decide_write(&schema, &req("width", Value::Integer(9))),
            Err(WriteRejection::OutOfRange)
        );
        assert_eq!(
            decide_write(&schema, &req("width", Value::Integer(21))),
            Err(WriteRejection::OutOfRange)
        );
    }

    /// The same bounds, written as a float to a float setting. The test above
    /// only ever sends `Value::Integer`, so the float branch of `numeric` was
    /// never exercised - mutation testing found it could be DELETED, and then
    /// `numeric` returns `None`, `check_range` returns `Ok`, and **every declared
    /// bound on every float setting stops being enforced**. A `0.0..=1.0` volume
    /// would accept -5.0.
    #[test]
    fn declared_bounds_are_enforced_for_floats_too() {
        let mut bounded = item("volume", SettingType::Float);
        bounded.min = Some(0.0);
        bounded.max = Some(1.0);
        let schema = schema_of(vec![bounded]);

        assert!(decide_write(&schema, &req("volume", Value::Float(0.0))).is_ok());
        assert!(decide_write(&schema, &req("volume", Value::Float(1.0))).is_ok());
        assert!(decide_write(&schema, &req("volume", Value::Float(0.5))).is_ok());
        assert_eq!(
            decide_write(&schema, &req("volume", Value::Float(-0.1))),
            Err(WriteRejection::OutOfRange)
        );
        assert_eq!(
            decide_write(&schema, &req("volume", Value::Float(1.1))),
            Err(WriteRejection::OutOfRange)
        );
        // An integer written to a float field is accepted by the type check, so
        // it must meet the same bounds rather than slipping past them.
        assert_eq!(
            decide_write(&schema, &req("volume", Value::Integer(9))),
            Err(WriteRejection::OutOfRange)
        );
    }

    #[test]
    fn an_enum_only_accepts_a_declared_option() {
        let mut e = item("theme", SettingType::Enum);
        let opt = |v: &str| SettingOption {
            value: v.into(),
            label: "L".into(),
            description: "d".into(),
        };
        e.options = vec![opt("dark"), opt("light")];
        let schema = schema_of(vec![e]);

        assert!(decide_write(&schema, &req("theme", Value::String("dark".into()))).is_ok());
        assert_eq!(
            decide_write(&schema, &req("theme", Value::String("neon".into()))),
            Err(WriteRejection::NotAnOption)
        );
    }

    /// The values of a system-sourced enum are the machine's, not the package's.
    /// Holding one to its (empty) declared list would refuse every write and
    /// leave a setting nobody can set.
    #[test]
    fn a_system_sourced_enum_accepts_a_value_the_package_never_declared() {
        let mut e = item("output", SettingType::Enum);
        e.options_from = Some(arlen_forage_recipe::settings::ValueSource::AudioOutputs);
        let schema = schema_of(vec![e]);

        assert!(decide_write(
            &schema,
            &req("output", Value::String("alsa_output.pci-0000_00_1f.3".into()))
        )
        .is_ok());

        // The type check still applies: it is an enum of strings.
        assert_eq!(
            decide_write(&schema, &req("output", Value::Integer(3))),
            Err(WriteRejection::WrongType {
                expected: SettingType::Enum
            })
        );
    }

    /// Raw escapes the type vocabulary and nothing else. A table, an array or a
    /// scalar all go through, because "raw" means the schema gave up on typing
    /// this one value.
    #[test]
    fn a_raw_key_takes_a_value_of_any_shape() {
        let schema = schema_of(vec![item("tuning", SettingType::Raw)]);
        let table = Value::Table(
            [("retries".to_string(), Value::Integer(3))]
                .into_iter()
                .collect(),
        );
        assert!(decide_write(&schema, &req("tuning", table)).is_ok());
        assert!(decide_write(&schema, &req("tuning", Value::Integer(1))).is_ok());
        assert!(decide_write(
            &schema,
            &req("tuning", Value::Array(vec![Value::Integer(1)]))
        )
        .is_ok());
    }

    /// The important half: a raw item is not a way to write anything anywhere.
    /// The key must still be declared, and its scope must still permit the write.
    #[test]
    fn a_raw_item_does_not_lift_the_other_rules() {
        let mut locked = item("tuning", SettingType::Raw);
        locked.scope = SettingScope::DefaultsOnly;
        let schema = schema_of(vec![locked]);

        assert_eq!(
            decide_write(&schema, &req("tuning", Value::Integer(1))),
            Err(WriteRejection::NotUserWritable)
        );
        assert_eq!(
            decide_write(&schema, &req("undeclared", Value::Integer(1))),
            Err(WriteRejection::UndeclaredKey)
        );
    }

    #[test]
    fn a_string_list_must_be_all_strings() {
        let schema = schema_of(vec![item("hosts", SettingType::StringList)]);
        let good = Value::Array(vec![Value::String("a".into()), Value::String("b".into())]);
        assert!(decide_write(&schema, &req("hosts", good)).is_ok());

        let mixed = Value::Array(vec![Value::String("a".into()), Value::Integer(2)]);
        assert_eq!(
            decide_write(&schema, &req("hosts", mixed)),
            Err(WriteRejection::WrongType {
                expected: SettingType::StringList
            })
        );
    }

    /// The decision hands back the declared item, so whoever applies the write
    /// works from the same view the decision used rather than resolving the key
    /// again and possibly differently.
    #[test]
    fn the_decision_returns_the_declared_item() {
        let schema = schema_of(vec![item("enabled", SettingType::Bool)]);
        let decided = decide_write(&schema, &req("enabled", Value::Boolean(false))).unwrap();
        assert_eq!(decided.key, "enabled");
        assert_eq!(decided.value_type, SettingType::Bool);
    }
}
