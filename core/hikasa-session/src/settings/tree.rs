//! Value trees of the settings engine (D-70): key paths over `toml::Value`, leaves, unknown-key detection, sparse
//! pruning, and [`Ordered`], the tree that writes a file with its sections in the declaration order of the schema.
//!
//! `toml` is built without `preserve_order` (its tables are sorted maps), so the declaration order is taken from
//! the schema's serde serialization: the value is written as JSON (fields in declaration order) and read back
//! into [`Ordered`], whose tables are vectors.

use std::fmt;

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};
use toml::{Table, Value};

use super::keys::KeySegment;

/// The serde field name of a TOML datetime (`toml_datetime`'s private struct).
const DATETIME_FIELD: &str = "$__toml_private_datetime";

/// A TOML value whose tables keep their key order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Ordered {
    /// A string, integer, float, boolean or datetime.
    Scalar(Value),
    /// An array.
    Array(Vec<Ordered>),
    /// A table, in key order.
    Table(Vec<(String, Ordered)>),
    /// A JSON `null` (an absent optional value); dropped from tables and arrays.
    Null,
}

impl Ordered {
    /// The tree of a serializable value, tables in declaration order.
    pub(crate) fn of<T: Serialize>(value: &T) -> Result<Ordered, String> {
        let json = serde_json::to_string(value).map_err(|e| e.to_string())?;
        serde_json::from_str(&json).map_err(|e| e.to_string())
    }

    /// The plain TOML value.
    pub(crate) fn to_value(&self) -> Value {
        match self {
            Ordered::Scalar(v) => v.clone(),
            Ordered::Array(items) => Value::Array(items.iter().map(Ordered::to_value).collect()),
            Ordered::Table(entries) => Value::Table(
                entries
                    .iter()
                    .map(|(k, v)| (k.clone(), v.to_value()))
                    .collect(),
            ),
            Ordered::Null => Value::Table(Table::new()),
        }
    }

    /// Removes every leaf (or whole array) equal to the same path of `default`, then every table left empty.
    /// `None` when nothing is left. Below a key that `default` lacks (an entry of a map or of an array of tables),
    /// `fallback(path)` gives the default of the entry (its template), so entries are written sparsely too; an
    /// array of tables that differs from its default keeps every element (an element left empty stays `{}`).
    pub(crate) fn prune(
        self,
        default: Option<&Value>,
        path: &mut Vec<KeySegment>,
        fallback: &dyn Fn(&[KeySegment]) -> Option<Value>,
    ) -> Option<Ordered> {
        match self {
            Ordered::Table(entries) => {
                let defaults = default.and_then(Value::as_table);
                let mut kept = Vec::new();
                for (k, v) in entries {
                    path.push(KeySegment::Name(k.clone()));
                    let d = defaults
                        .and_then(|t| t.get(&k))
                        .cloned()
                        .or_else(|| fallback(path));
                    if let Some(v) = v.prune(d.as_ref(), path, fallback) {
                        kept.push((k, v));
                    }
                    path.pop();
                }
                (!kept.is_empty()).then_some(Ordered::Table(kept))
            }
            Ordered::Null => None,
            Ordered::Array(items) => {
                let whole = Ordered::Array(items);
                if default.is_some_and(|d| *d == whole.to_value()) {
                    return None;
                }
                let Ordered::Array(items) = whole else {
                    return None;
                };
                let tables =
                    !items.is_empty() && items.iter().all(|i| matches!(i, Ordered::Table(_)));
                if !tables {
                    return Some(Ordered::Array(items));
                }
                let mut out = Vec::with_capacity(items.len());
                for (i, item) in items.into_iter().enumerate() {
                    path.push(KeySegment::Index(i));
                    let d = fallback(path);
                    out.push(
                        item.prune(d.as_ref(), path, fallback)
                            .unwrap_or(Ordered::Table(Vec::new())),
                    );
                    path.pop();
                }
                Some(Ordered::Array(out))
            }
            other => match default {
                Some(d) if *d == other.to_value() => None,
                _ => Some(other),
            },
        }
    }

    /// Adds the kept unknown keys of `raw`: inside a table of the same name, after its known keys; elsewhere
    /// at the end, in `raw`'s (sorted) order.
    pub(crate) fn merge(&mut self, raw: &Table) {
        let Ordered::Table(entries) = self else {
            return;
        };
        for (key, value) in raw {
            match entries.iter_mut().find(|(k, _)| k == key) {
                Some((_, existing)) => {
                    if let (Ordered::Table(_), Value::Table(inner)) = (&*existing, value) {
                        existing.merge(inner);
                    }
                }
                None => entries.push((key.clone(), Ordered::from(value))),
            }
        }
    }
}

impl From<&Value> for Ordered {
    fn from(value: &Value) -> Ordered {
        match value {
            Value::Array(items) => Ordered::Array(items.iter().map(Ordered::from).collect()),
            Value::Table(table) => Ordered::Table(
                table
                    .iter()
                    .map(|(k, v)| (k.clone(), Ordered::from(v)))
                    .collect(),
            ),
            scalar => Ordered::Scalar(scalar.clone()),
        }
    }
}

impl Serialize for Ordered {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Ordered::Scalar(v) => v.serialize(serializer),
            Ordered::Array(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items.iter().filter(|i| **i != Ordered::Null) {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Ordered::Table(entries) => {
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                for (k, v) in entries.iter().filter(|(_, v)| *v != Ordered::Null) {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
            Ordered::Null => serializer.serialize_unit(),
        }
    }
}

impl<'de> Deserialize<'de> for Ordered {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Ordered, D::Error> {
        deserializer.deserialize_any(OrderedVisitor)
    }
}

/// Reads JSON (or any self-describing format) into [`Ordered`], keeping the key order of maps.
struct OrderedVisitor;

impl<'de> Visitor<'de> for OrderedVisitor {
    type Value = Ordered;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a TOML value")
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Ordered, E> {
        Ok(Ordered::Scalar(Value::Boolean(v)))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Ordered, E> {
        Ok(Ordered::Scalar(Value::Integer(v)))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Ordered, E> {
        i64::try_from(v)
            .map(|v| Ordered::Scalar(Value::Integer(v)))
            .map_err(|_| E::custom(format!("integer {v} does not fit TOML")))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Ordered, E> {
        Ok(Ordered::Scalar(Value::Float(v)))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Ordered, E> {
        Ok(Ordered::Scalar(Value::String(v.to_owned())))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Ordered, E> {
        Ok(Ordered::Null)
    }

    fn visit_none<E: de::Error>(self) -> Result<Ordered, E> {
        Ok(Ordered::Null)
    }

    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Ordered, D::Error> {
        d.deserialize_any(OrderedVisitor)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Ordered, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element::<Ordered>()? {
            if item != Ordered::Null {
                items.push(item);
            }
        }
        Ok(Ordered::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Ordered, A::Error> {
        let mut entries: Vec<(String, Ordered)> = Vec::new();
        while let Some((key, value)) = map.next_entry::<String, Ordered>()? {
            if value != Ordered::Null {
                entries.push((key, value));
            }
        }
        // A TOML datetime serializes as a one-field struct with a private name.
        if let [(key, Ordered::Scalar(Value::String(text)))] = entries.as_slice()
            && key == DATETIME_FIELD
        {
            let datetime = text
                .parse::<toml::value::Datetime>()
                .map_err(de::Error::custom)?;
            return Ok(Ordered::Scalar(Value::Datetime(datetime)));
        }
        Ok(Ordered::Table(entries))
    }
}

/// The value at `path` (an index segment also names the table key with its digits).
pub(crate) fn get<'a>(root: &'a Value, path: &[KeySegment]) -> Option<&'a Value> {
    let mut node = root;
    for segment in path {
        node = match (node, segment) {
            (Value::Array(items), KeySegment::Index(i)) => items.get(*i)?,
            (Value::Table(table), segment) => table.get(&segment.as_key())?,
            _ => return None,
        };
    }
    Some(node)
}

/// The value at `path` of a table (an empty path has none).
pub(crate) fn get_in<'a>(table: &'a Table, path: &[KeySegment]) -> Option<&'a Value> {
    let (first, rest) = path.split_first()?;
    get(table.get(&first.as_key())?, rest)
}

/// Sets the value at `path`, creating missing tables (and appending an array element at index `len`).
pub(crate) fn set(root: &mut Value, path: &[KeySegment], new: Value) -> Result<(), String> {
    let Some((last, parents)) = path.split_last() else {
        *root = new;
        return Ok(());
    };
    let mut node = root;
    for (k, segment) in parents.iter().enumerate() {
        // A missing parent is an array when the segment after it is an index, else a table.
        let create = Some(
            path.get(k + 1)
                .map_or(Value::Table(Table::new()), |next| match next {
                    KeySegment::Index(_) => Value::Array(Vec::new()),
                    KeySegment::Name(_) => Value::Table(Table::new()),
                }),
        );
        node = child_mut(node, segment, create)
            .ok_or_else(|| format!("{} is not a table or array", display_path(&path[..=k])))?;
    }
    match (node, last) {
        (Value::Array(items), KeySegment::Index(i)) if *i < items.len() => {
            items[*i] = new;
            Ok(())
        }
        (Value::Array(items), KeySegment::Index(i)) if *i == items.len() => {
            items.push(new);
            Ok(())
        }
        (Value::Table(table), segment) => {
            table.insert(segment.as_key(), new);
            Ok(())
        }
        _ => Err(format!("{} cannot be set", display_path(path))),
    }
}

/// The child of `node` at `segment`, created as `create` when missing (an array grows by one element at index
/// `len`).
fn child_mut<'a>(
    node: &'a mut Value,
    segment: &KeySegment,
    create: Option<Value>,
) -> Option<&'a mut Value> {
    match (node, segment) {
        (Value::Array(items), KeySegment::Index(i)) => {
            if let Some(empty) = create
                && *i == items.len()
            {
                items.push(empty);
            }
            items.get_mut(*i)
        }
        (Value::Table(table), segment) => {
            let key = segment.as_key();
            if let Some(empty) = create
                && !table.contains_key(&key)
            {
                table.insert(key.clone(), empty);
            }
            table.get_mut(&key)
        }
        _ => None,
    }
}

/// The value at `path`, to change (nothing is created).
pub(crate) fn get_mut<'a>(root: &'a mut Value, path: &[KeySegment]) -> Option<&'a mut Value> {
    let mut node = root;
    for segment in path {
        node = child_mut(node, segment, None)?;
    }
    Some(node)
}

/// Removes the value at `path` (an array element shifts the later ones). `None` when absent.
pub(crate) fn remove(root: &mut Value, path: &[KeySegment]) -> Option<Value> {
    let (last, parents) = path.split_last()?;
    let mut node = root;
    for segment in parents {
        node = child_mut(node, segment, None)?;
    }
    match (node, last) {
        (Value::Array(items), KeySegment::Index(i)) if *i < items.len() => Some(items.remove(*i)),
        (Value::Table(table), segment) => table.remove(&segment.as_key()),
        _ => None,
    }
}

/// Every leaf below `table` with its path, in key order: tables are descended; arrays are leaves, except that
/// with `into_arrays` an array whose elements are all tables is descended by index.
pub(crate) fn leaves(table: &Table, into_arrays: bool) -> Vec<(Vec<KeySegment>, Value)> {
    fn walk(
        path: &mut Vec<KeySegment>,
        value: &Value,
        into_arrays: bool,
        out: &mut Vec<(Vec<KeySegment>, Value)>,
    ) {
        match value {
            Value::Table(table) => {
                for (k, v) in table {
                    path.push(KeySegment::Name(k.clone()));
                    walk(path, v, into_arrays, out);
                    path.pop();
                }
            }
            Value::Array(items)
                if into_arrays && !items.is_empty() && items.iter().all(Value::is_table) =>
            {
                for (i, v) in items.iter().enumerate() {
                    path.push(KeySegment::Index(i));
                    walk(path, v, into_arrays, out);
                    path.pop();
                }
            }
            leaf => out.push((path.clone(), leaf.clone())),
        }
    }
    let mut out = Vec::new();
    for (k, v) in table {
        let mut path = vec![KeySegment::Name(k.clone())];
        walk(&mut path, v, into_arrays, &mut out);
    }
    out
}

/// The paths of `raw` that `known` lacks (tables are compared key by key; arrays and scalars are whole).
pub(crate) fn unknown_paths(raw: &Table, known: &Table) -> Vec<Vec<KeySegment>> {
    fn walk(
        path: &mut Vec<KeySegment>,
        raw: &Table,
        known: &Table,
        out: &mut Vec<Vec<KeySegment>>,
    ) {
        for (k, v) in raw {
            path.push(KeySegment::Name(k.clone()));
            match (v, known.get(k)) {
                (_, None) => out.push(path.clone()),
                (Value::Table(inner), Some(Value::Table(known_inner))) => {
                    walk(path, inner, known_inner, out);
                }
                _ => {}
            }
            path.pop();
        }
    }
    let mut out = Vec::new();
    walk(&mut Vec::new(), raw, known, &mut out);
    out
}

/// The part of `effective` that differs from `defaults`: leaves and whole arrays equal to the same path of
/// `defaults` are removed, then every table left empty.
pub(crate) fn sparse(effective: &Value, defaults: &Value) -> Value {
    match Ordered::from(effective).prune(Some(defaults), &mut Vec::new(), &|_| None) {
        Some(ordered) => ordered.to_value(),
        None => Value::Table(Table::new()),
    }
}

/// `overlay` written over `base`: tables merge key by key, anything else replaces.
pub(crate) fn overlay(base: &mut Value, overlay: &Value) {
    match (base, overlay) {
        (Value::Table(base), Value::Table(top)) => {
            for (k, v) in top {
                match base.get_mut(k) {
                    Some(existing) => self::overlay(existing, v),
                    None => {
                        base.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (base, top) => *base = top.clone(),
    }
}

/// A key path for messages and keys: segments joined with `.` ([`KeySegment`]'s spelling).
pub(crate) fn display_path(path: &[KeySegment]) -> String {
    path.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

/// A TOML value as JSON (datetimes as strings; D-73 "TOML values in JSON").
pub(crate) fn to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::Integer(i) => serde_json::Value::from(*i),
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map_or(serde_json::Value::Null, serde_json::Value::Number),
        Value::Boolean(b) => serde_json::Value::Bool(*b),
        Value::Datetime(d) => serde_json::Value::String(d.to_string()),
        Value::Array(items) => serde_json::Value::Array(items.iter().map(to_json).collect()),
        Value::Table(table) => {
            serde_json::Value::Object(table.iter().map(|(k, v)| (k.clone(), to_json(v))).collect())
        }
    }
}

/// A TOML value as it is written on the right of `key = …` (a table as an inline table).
pub(crate) fn inline(value: &Value) -> String {
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> KeySegment {
        KeySegment::Name(s.into())
    }

    #[test]
    fn ordered_keeps_declaration_order_and_writes_toml() {
        #[derive(serde::Serialize)]
        struct Inner {
            zeta: u32,
            alpha: bool,
        }
        #[derive(serde::Serialize)]
        struct Doc {
            schema_version: u32,
            name: String,
            inner: Inner,
            list: Vec<Inner>,
        }
        let doc = Doc {
            schema_version: 1,
            name: "x".into(),
            inner: Inner {
                zeta: 2,
                alpha: true,
            },
            list: vec![Inner {
                zeta: 1,
                alpha: false,
            }],
        };
        let mut ordered = Ordered::of(&doc).unwrap();
        let mut raw = Table::new();
        raw.insert("aaa".into(), Value::Integer(5));
        ordered.merge(&raw);
        let text = toml::to_string(&ordered).unwrap();
        assert_eq!(
            text,
            "schema_version = 1\nname = \"x\"\naaa = 5\n\n[inner]\nzeta = 2\nalpha = true\n\n[[list]]\nzeta = 1\nalpha = false\n"
        );
    }

    #[test]
    fn paths_get_set_remove() {
        let mut root: Value = toml::from_str::<Table>("[a]\nb = [ { c = 1 } ]\n")
            .unwrap()
            .into();
        let path = [n("a"), n("b"), KeySegment::Index(0), n("c")];
        assert_eq!(get(&root, &path), Some(&Value::Integer(1)));
        set(&mut root, &path, Value::Integer(2)).unwrap();
        assert_eq!(get(&root, &path), Some(&Value::Integer(2)));
        set(&mut root, &[n("x"), n("y")], Value::Boolean(true)).unwrap();
        assert_eq!(get(&root, &[n("x"), n("y")]), Some(&Value::Boolean(true)));
        set(
            &mut root,
            &[n("a"), n("b"), KeySegment::Index(1), n("c")],
            Value::Integer(3),
        )
        .unwrap();
        assert!(set(&mut root, &[n("x"), n("y"), n("z")], Value::Integer(1)).is_err());
        assert_eq!(
            remove(&mut root, &[n("a"), n("b"), KeySegment::Index(0)]),
            Some(toml::from_str::<Table>("c = 2").unwrap().into())
        );
        assert_eq!(
            get(&root, &[n("a"), n("b"), KeySegment::Index(0), n("c")]),
            Some(&Value::Integer(3))
        );
        assert_eq!(
            display_path(&[n("a"), KeySegment::Index(0), n("a.b")]),
            "a.0.\"a.b\""
        );
    }

    #[test]
    fn sparse_and_unknown() {
        let defaults: Value = toml::from_str::<Table>("[a]\nx = 1\ny = [1]\n[b]\nz = true\n")
            .unwrap()
            .into();
        let effective: Value = toml::from_str::<Table>("[a]\nx = 2\ny = [1]\n[b]\nz = true\n")
            .unwrap()
            .into();
        assert_eq!(
            sparse(&effective, &defaults),
            toml::from_str::<Table>("[a]\nx = 2\n").unwrap().into()
        );
        let raw = toml::from_str::<Table>("[a]\nx = 1\nq = 1\n[c]\nr = 1\n").unwrap();
        let known = defaults.as_table().unwrap();
        assert_eq!(
            unknown_paths(&raw, known),
            vec![vec![n("a"), n("q")], vec![n("c")]]
        );
        assert_eq!(inline(&Value::String("a\"b".into())), "'a\"b'");
        assert_eq!(inline(&Value::Array(vec![Value::Integer(1)])), "[1]");
    }
}
