//! One key of a typed settings schema as a form field (M2 W4c; the Options dialog, D-128): the GUI has no `toml`
//! dependency (D-133), and its pages are data over [`super::key_infos`], so a key is read and written here by its
//! dotted path. The path grammar is D-73's, without the file.

use serde::Serialize;
use serde::de::DeserializeOwned;
use toml::Value;

use super::file::full_table;
use super::keys::{ValueKind, parse_path};
use super::tree;

/// The value of one form field.
#[derive(Debug, Clone, PartialEq)]
pub enum FormValue {
    /// A check box.
    Bool(bool),
    /// An integer field.
    Int(i64),
    /// A float field.
    Float(f64),
    /// A text field or a combo (enums are their spelling).
    Text(String),
}

impl FormValue {
    fn of(value: &Value) -> Option<FormValue> {
        match value {
            Value::Boolean(b) => Some(FormValue::Bool(*b)),
            Value::Integer(n) => Some(FormValue::Int(*n)),
            Value::Float(x) => Some(FormValue::Float(*x)),
            Value::String(s) => Some(FormValue::Text(s.clone())),
            _ => None,
        }
    }

    fn to_toml(&self) -> Value {
        match self {
            FormValue::Bool(b) => Value::Boolean(*b),
            FormValue::Int(n) => Value::Integer(*n),
            FormValue::Float(x) => Value::Float(*x),
            FormValue::Text(s) => Value::String(s.clone()),
        }
    }
}

/// The key `path` of `value`; `None` for an unknown path or a table.
pub fn form_value<T: Serialize>(value: &T, path: &str) -> Option<FormValue> {
    let segments = parse_path(path).ok()?;
    let table = full_table(value);
    tree::get_in(&table, &segments).and_then(FormValue::of)
}

/// Writes the key `path` of `value`. An unknown path or a value of the wrong type is an error, and `value` stays.
pub fn set_form_value<T: Serialize + DeserializeOwned>(
    value: &mut T,
    path: &str,
    new: &FormValue,
) -> Result<(), String> {
    let segments = parse_path(path)?;
    let mut root = Value::Table(full_table(value));
    if tree::get(&root, &segments).is_none() {
        return Err(format!("no key `{path}`"));
    }
    tree::set(&mut root, &segments, new.to_toml())?;
    *value = root
        .try_into()
        .map_err(|e: toml::de::Error| format!("{path}: {e}"))?;
    Ok(())
}

/// The value a widget of `kind` may hold: a number clamped to the range (D-128); anything else as it is.
pub fn clamp_value(kind: ValueKind, value: FormValue) -> FormValue {
    match (kind, value) {
        (ValueKind::Int { min, max }, FormValue::Int(n)) => FormValue::Int(n.clamp(min, max)),
        (ValueKind::Float { min, .. }, FormValue::Float(x)) if x.is_nan() => FormValue::Float(min),
        (ValueKind::Float { min, max }, FormValue::Float(x)) => FormValue::Float(x.clamp(min, max)),
        (_, value) => value,
    }
}

/// The caption of an enum value of `options.toml` in a form (`None`: a value without a caption). The spellings are
/// the schema's vocabulary, so the table lives beside it (T11); a GUI test checks every value of its pages has one.
pub fn enum_caption(value: &str) -> Option<&'static str> {
    ENUM_CAPTIONS
        .iter()
        .find(|(v, _)| *v == value)
        .map(|(_, c)| *c)
}

/// The captions by spelling.
const ENUM_CAPTIONS: &[(&str, &str)] = &[
    ("binary", "Binary quick compare"),
    ("rules_based", "Rules-based quick compare"),
    ("tab", "A new tab"),
    ("window", "A new window"),
    ("follow_os", "Follow OS"),
    ("light", "Light"),
    ("dark", "Dark"),
    ("name_tilde", "abc.txt~"),
    ("stem_tilde", "abc~.txt"),
    ("name_bak", "abc.txt.bak"),
    ("tilde_name", "~abc.txt"),
    ("stem_bak", "abc.bak"),
    ("stem_bak_ext", "abc.bak.txt"),
    ("folder", "abc.txt in the backup folder"),
    ("prompt", "Prompt"),
    ("yes_to_all", "Yes to All"),
    ("no_to_all", "No to All"),
    ("normal", "Normal"),
    ("below_normal", "Below normal"),
    ("idle", "Idle"),
    ("compress", "Compress to fit"),
    ("scroll", "Allow scrolling"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{FileId, Options, key_infos};

    #[test]
    fn every_option_key_reads_and_writes_back() {
        let mut o = Options::default();
        for info in key_infos(&FileId::Options) {
            let path = info.key.strip_prefix("options.").expect("the prefix");
            let value = form_value(&o, path).unwrap_or_else(|| panic!("{path}"));
            set_form_value(&mut o, path, &value).unwrap_or_else(|e| panic!("{e}"));
        }
        assert_eq!(o, Options::default());
    }

    #[test]
    fn a_key_is_set_by_its_path() {
        let mut o = Options::default();
        set_form_value(
            &mut o,
            "tabs.open_sessions_in",
            &FormValue::Text("window".into()),
        )
        .unwrap();
        assert_eq!(
            form_value(&o, "tabs.open_sessions_in"),
            Some(FormValue::Text("window".into()))
        );
        set_form_value(&mut o, "tweaks.metrics.max_mru_items", &FormValue::Int(7)).unwrap();
        assert_eq!(o.tweaks.metrics.max_mru_items, 7);
        let before = o.clone();
        assert!(set_form_value(&mut o, "tabs.nope", &FormValue::Bool(true)).is_err());
        assert!(
            set_form_value(
                &mut o,
                "tabs.open_sessions_in",
                &FormValue::Text("x".into())
            )
            .is_err()
        );
        assert!(form_value(&o, "tabs").is_none());
        assert_eq!(o, before);
    }

    #[test]
    fn numbers_clamp_at_both_bounds() {
        let int = ValueKind::Int { min: 10, max: 20 };
        assert_eq!(clamp_value(int, FormValue::Int(9)), FormValue::Int(10));
        assert_eq!(clamp_value(int, FormValue::Int(21)), FormValue::Int(20));
        assert_eq!(clamp_value(int, FormValue::Int(15)), FormValue::Int(15));
        let float = ValueKind::Float {
            min: 6.0,
            max: 72.0,
        };
        assert_eq!(
            clamp_value(float, FormValue::Float(1.0)),
            FormValue::Float(6.0)
        );
        assert_eq!(
            clamp_value(float, FormValue::Float(99.0)),
            FormValue::Float(72.0)
        );
        assert_eq!(
            clamp_value(float, FormValue::Float(f64::NAN)),
            FormValue::Float(6.0)
        );
        assert_eq!(
            clamp_value(ValueKind::Bool, FormValue::Bool(true)),
            FormValue::Bool(true)
        );
    }
}
