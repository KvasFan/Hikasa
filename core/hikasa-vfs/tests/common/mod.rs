//! Shared helpers of the hikasa-vfs integration tests: the golden-file helper (§6.1), the proptest
//! configuration, host-style paths for `MemoryVfs`, a call-counting `Vfs` wrapper, and a minimal JSON
//! serializer/deserializer for the serde goldens (`serde_json` is not a dev-dependency of this crate, §0.5).
#![allow(dead_code)]

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use hikasa_core::{CancelToken, LocalPath, Spec};
use hikasa_vfs::{
    AtomicWrite, Capabilities, Entry, FileName, FollowLinks, ListOptions, Listing, ReadSeek, Vfs,
    VfsError, WriteOptions,
};
use proptest::test_runner::{Config, FileFailurePersistence};

/// Compares `actual` with the golden file `expected`; with `HIKASA_BLESS=1` the golden file is (re)written
/// instead. Text files are shown as a `pretty_assertions` diff.
pub fn assert_golden(expected: &Path, actual: &[u8]) {
    let bless = std::env::var_os("HIKASA_BLESS").is_some_and(|v| v == "1");
    if bless {
        if let Some(dir) = expected.parent() {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(expected, actual).unwrap();
        return;
    }
    let golden = std::fs::read(expected).unwrap_or_else(|e| {
        panic!(
            "missing golden file {} ({e}); run with HIKASA_BLESS=1",
            expected.display()
        )
    });
    if golden != actual {
        match (std::str::from_utf8(&golden), std::str::from_utf8(actual)) {
            (Ok(want), Ok(got)) => {
                pretty_assertions::assert_eq!(want, got, "golden {}", expected.display())
            }
            _ => panic!("golden {} differs (binary)", expected.display()),
        }
    }
}

/// `tests/fixtures/<suite>/<case>`.
pub fn fixture(suite: &str, case: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(suite)
        .join(case)
}

/// Proptest configuration: 256 cases unless `PROPTEST_CASES` says otherwise (§6.1); failing seeds are
/// persisted in `regressions` (`tests/proptest-regressions/<suite>.txt`, relative to the package).
pub fn config(regressions: &'static str) -> Config {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(256);
    Config {
        cases,
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(regressions))),
        ..Config::default()
    }
}

/// A host-style absolute path for `MemoryVfs` from a `/`-separated one: `/a/b` on Unix, `C:\a\b` on Windows.
pub fn host(path: &str) -> String {
    if cfg!(windows) {
        format!("C:{}", path.replace('/', "\\"))
    } else {
        path.to_owned()
    }
}

/// `Spec::Local` of [`host`]`(path)`.
pub fn mspec(path: &str) -> Spec {
    Spec::Local(LocalPath::new(PathBuf::from(host(path))).unwrap())
}

/// `Spec::Local` of a real path (canonicalised, so temp folders behind symlinks are normalised).
pub fn lspec(path: &Path) -> Spec {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    Spec::Local(LocalPath::new(canonical).unwrap())
}

/// A `Spec::Local` for a path that may not exist yet: the canonical parent joined with the name.
pub fn lspec_new(path: &Path) -> Spec {
    let parent = path.parent().unwrap().canonicalize().unwrap();
    Spec::Local(LocalPath::new(parent.join(path.file_name().unwrap())).unwrap())
}

/// A `Vfs` wrapper that counts calls (the prefilter test proves that no `stat` happens through it).
pub struct CountingVfs<'a> {
    pub inner: &'a dyn Vfs,
    pub stats: AtomicU64,
    pub lists: AtomicU64,
    pub opens: AtomicU64,
    pub local_paths: AtomicU64,
}

impl<'a> CountingVfs<'a> {
    pub fn new(inner: &'a dyn Vfs) -> CountingVfs<'a> {
        CountingVfs {
            inner,
            stats: AtomicU64::new(0),
            lists: AtomicU64::new(0),
            opens: AtomicU64::new(0),
            local_paths: AtomicU64::new(0),
        }
    }

    pub fn local_paths(&self) -> u64 {
        self.local_paths.load(Ordering::Relaxed)
    }

    pub fn stats(&self) -> u64 {
        self.stats.load(Ordering::Relaxed)
    }
}

impl Vfs for CountingVfs<'_> {
    fn capabilities(&self, at: &Spec) -> Result<Capabilities, VfsError> {
        self.inner.capabilities(at)
    }
    fn stat(&self, spec: &Spec, follow: FollowLinks) -> Result<Entry, VfsError> {
        self.stats.fetch_add(1, Ordering::Relaxed);
        self.inner.stat(spec, follow)
    }
    fn list(
        &self,
        dir: &Spec,
        opts: &ListOptions<'_>,
        cancel: &CancelToken,
    ) -> Result<Listing, VfsError> {
        self.lists.fetch_add(1, Ordering::Relaxed);
        self.inner.list(dir, opts, cancel)
    }
    fn open_read(&self, spec: &Spec) -> Result<Box<dyn ReadSeek>, VfsError> {
        self.opens.fetch_add(1, Ordering::Relaxed);
        self.inner.open_read(spec)
    }
    fn open_write(
        &self,
        spec: &Spec,
        opts: &WriteOptions,
    ) -> Result<Box<dyn AtomicWrite>, VfsError> {
        self.inner.open_write(spec, opts)
    }
    fn child(&self, dir: &Spec, name: &FileName) -> Result<Spec, VfsError> {
        self.inner.child(dir, name)
    }
    fn local_path(&self, spec: &Spec) -> Option<PathBuf> {
        self.local_paths.fetch_add(1, Ordering::Relaxed);
        self.inner.local_path(spec)
    }
}

/// Reads a whole reader.
pub fn read_to_vec(mut r: impl io::Read) -> Vec<u8> {
    let mut out = Vec::new();
    r.read_to_end(&mut out).unwrap();
    out
}

/// Seeded xorshift64* (no `rand` crate, §6.1).
pub struct XorShift(pub u64);

impl XorShift {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.next() as u8).collect()
    }
}

/// A minimal JSON value model, serializer and deserializer with `serde_json`'s pretty layout (2-space
/// indentation, `[]`/`{}` for empty containers). Integers only (§4.1 rule 4).
pub mod json {
    use std::fmt::Write as _;

    use serde::de::value::{Error, MapDeserializer, SeqDeserializer};
    use serde::de::{DeserializeOwned, IntoDeserializer, Visitor};
    use serde::ser::{self, Impossible, Serialize};
    use serde::{Deserializer, forward_to_deserialize_any};

    /// A JSON value.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum J {
        Null,
        Bool(bool),
        U(u64),
        I(i64),
        Str(String),
        Arr(Vec<J>),
        Obj(Vec<(String, J)>),
    }

    /// Serialises `value` as pretty JSON followed by a newline.
    pub fn to_pretty<T: Serialize + ?Sized>(value: &T) -> String {
        let j = value.serialize(Ser).unwrap();
        let mut out = String::new();
        render(&j, 0, &mut out);
        out.push('\n');
        out
    }

    /// The value model of `value`.
    pub fn to_value<T: Serialize + ?Sized>(value: &T) -> J {
        value.serialize(Ser).unwrap()
    }

    /// Deserialises `T` from a value.
    pub fn from_value<T: DeserializeOwned>(j: J) -> Result<T, Error> {
        T::deserialize(j)
    }

    fn escape(s: &str, out: &mut String) {
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => {
                    let _ = write!(out, "\\u{:04x}", c as u32);
                }
                c => out.push(c),
            }
        }
        out.push('"');
    }

    fn indent(level: usize, out: &mut String) {
        for _ in 0..level {
            out.push_str("  ");
        }
    }

    fn render(j: &J, level: usize, out: &mut String) {
        match j {
            J::Null => out.push_str("null"),
            J::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            J::U(n) => {
                let _ = write!(out, "{n}");
            }
            J::I(n) => {
                let _ = write!(out, "{n}");
            }
            J::Str(s) => escape(s, out),
            J::Arr(items) if items.is_empty() => out.push_str("[]"),
            J::Arr(items) => {
                out.push_str("[\n");
                for (k, item) in items.iter().enumerate() {
                    indent(level + 1, out);
                    render(item, level + 1, out);
                    if k + 1 < items.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                indent(level, out);
                out.push(']');
            }
            J::Obj(fields) if fields.is_empty() => out.push_str("{}"),
            J::Obj(fields) => {
                out.push_str("{\n");
                for (k, (key, value)) in fields.iter().enumerate() {
                    indent(level + 1, out);
                    escape(key, out);
                    out.push_str(": ");
                    render(value, level + 1, out);
                    if k + 1 < fields.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                indent(level, out);
                out.push('}');
            }
        }
    }

    fn unsupported(what: &str) -> Error {
        <Error as ser::Error>::custom(format!("{what} is not supported by the test serializer"))
    }

    /// Serializer into [`J`].
    pub struct Ser;

    /// Sequence serializer.
    pub struct SeqSer(Vec<J>);

    /// Map and struct serializer.
    pub struct MapSer {
        fields: Vec<(String, J)>,
        key: Option<String>,
    }

    impl ser::Serializer for Ser {
        type Ok = J;
        type Error = Error;
        type SerializeSeq = SeqSer;
        type SerializeTuple = SeqSer;
        type SerializeTupleStruct = SeqSer;
        type SerializeTupleVariant = Impossible<J, Error>;
        type SerializeMap = MapSer;
        type SerializeStruct = MapSer;
        type SerializeStructVariant = Impossible<J, Error>;

        fn serialize_bool(self, v: bool) -> Result<J, Error> {
            Ok(J::Bool(v))
        }
        fn serialize_i8(self, v: i8) -> Result<J, Error> {
            Ok(J::I(v.into()))
        }
        fn serialize_i16(self, v: i16) -> Result<J, Error> {
            Ok(J::I(v.into()))
        }
        fn serialize_i32(self, v: i32) -> Result<J, Error> {
            Ok(J::I(v.into()))
        }
        fn serialize_i64(self, v: i64) -> Result<J, Error> {
            Ok(J::I(v))
        }
        fn serialize_u8(self, v: u8) -> Result<J, Error> {
            Ok(J::U(v.into()))
        }
        fn serialize_u16(self, v: u16) -> Result<J, Error> {
            Ok(J::U(v.into()))
        }
        fn serialize_u32(self, v: u32) -> Result<J, Error> {
            Ok(J::U(v.into()))
        }
        fn serialize_u64(self, v: u64) -> Result<J, Error> {
            Ok(J::U(v))
        }
        fn serialize_f32(self, _v: f32) -> Result<J, Error> {
            Err(unsupported("f32"))
        }
        fn serialize_f64(self, _v: f64) -> Result<J, Error> {
            Err(unsupported("f64"))
        }
        fn serialize_char(self, v: char) -> Result<J, Error> {
            Ok(J::Str(v.to_string()))
        }
        fn serialize_str(self, v: &str) -> Result<J, Error> {
            Ok(J::Str(v.to_owned()))
        }
        fn serialize_bytes(self, _v: &[u8]) -> Result<J, Error> {
            Err(unsupported("bytes"))
        }
        fn serialize_none(self) -> Result<J, Error> {
            Ok(J::Null)
        }
        fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<J, Error> {
            value.serialize(Ser)
        }
        fn serialize_unit(self) -> Result<J, Error> {
            Ok(J::Null)
        }
        fn serialize_unit_struct(self, _name: &'static str) -> Result<J, Error> {
            Ok(J::Null)
        }
        fn serialize_unit_variant(
            self,
            _name: &'static str,
            _index: u32,
            variant: &'static str,
        ) -> Result<J, Error> {
            Ok(J::Str(variant.to_owned()))
        }
        fn serialize_newtype_struct<T: Serialize + ?Sized>(
            self,
            _name: &'static str,
            value: &T,
        ) -> Result<J, Error> {
            value.serialize(Ser)
        }
        fn serialize_newtype_variant<T: Serialize + ?Sized>(
            self,
            _name: &'static str,
            _index: u32,
            variant: &'static str,
            value: &T,
        ) -> Result<J, Error> {
            Ok(J::Obj(vec![(variant.to_owned(), value.serialize(Ser)?)]))
        }
        fn serialize_seq(self, len: Option<usize>) -> Result<SeqSer, Error> {
            Ok(SeqSer(Vec::with_capacity(len.unwrap_or(0))))
        }
        fn serialize_tuple(self, len: usize) -> Result<SeqSer, Error> {
            Ok(SeqSer(Vec::with_capacity(len)))
        }
        fn serialize_tuple_struct(self, _name: &'static str, len: usize) -> Result<SeqSer, Error> {
            Ok(SeqSer(Vec::with_capacity(len)))
        }
        fn serialize_tuple_variant(
            self,
            _name: &'static str,
            _index: u32,
            _variant: &'static str,
            _len: usize,
        ) -> Result<Impossible<J, Error>, Error> {
            Err(unsupported("tuple variants"))
        }
        fn serialize_map(self, _len: Option<usize>) -> Result<MapSer, Error> {
            Ok(MapSer {
                fields: Vec::new(),
                key: None,
            })
        }
        fn serialize_struct(self, _name: &'static str, _len: usize) -> Result<MapSer, Error> {
            Ok(MapSer {
                fields: Vec::new(),
                key: None,
            })
        }
        fn serialize_struct_variant(
            self,
            _name: &'static str,
            _index: u32,
            _variant: &'static str,
            _len: usize,
        ) -> Result<Impossible<J, Error>, Error> {
            Err(unsupported("externally tagged struct variants"))
        }
    }

    impl ser::SerializeSeq for SeqSer {
        type Ok = J;
        type Error = Error;
        fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
            self.0.push(value.serialize(Ser)?);
            Ok(())
        }
        fn end(self) -> Result<J, Error> {
            Ok(J::Arr(self.0))
        }
    }

    impl ser::SerializeTuple for SeqSer {
        type Ok = J;
        type Error = Error;
        fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
            ser::SerializeSeq::serialize_element(self, value)
        }
        fn end(self) -> Result<J, Error> {
            ser::SerializeSeq::end(self)
        }
    }

    impl ser::SerializeTupleStruct for SeqSer {
        type Ok = J;
        type Error = Error;
        fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
            ser::SerializeSeq::serialize_element(self, value)
        }
        fn end(self) -> Result<J, Error> {
            ser::SerializeSeq::end(self)
        }
    }

    impl ser::SerializeMap for MapSer {
        type Ok = J;
        type Error = Error;
        fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), Error> {
            match key.serialize(Ser)? {
                J::Str(s) => {
                    self.key = Some(s);
                    Ok(())
                }
                _ => Err(unsupported("non-string keys")),
            }
        }
        fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
            let key = self
                .key
                .take()
                .ok_or_else(|| unsupported("a value without key"))?;
            self.fields.push((key, value.serialize(Ser)?));
            Ok(())
        }
        fn end(self) -> Result<J, Error> {
            Ok(J::Obj(self.fields))
        }
    }

    impl ser::SerializeStruct for MapSer {
        type Ok = J;
        type Error = Error;
        fn serialize_field<T: Serialize + ?Sized>(
            &mut self,
            key: &'static str,
            value: &T,
        ) -> Result<(), Error> {
            self.fields.push((key.to_owned(), value.serialize(Ser)?));
            Ok(())
        }
        fn end(self) -> Result<J, Error> {
            Ok(J::Obj(self.fields))
        }
    }

    impl<'de> IntoDeserializer<'de, Error> for J {
        type Deserializer = J;
        fn into_deserializer(self) -> J {
            self
        }
    }

    impl<'de> Deserializer<'de> for J {
        type Error = Error;

        fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
            match self {
                J::Null => visitor.visit_unit(),
                J::Bool(b) => visitor.visit_bool(b),
                J::U(n) => visitor.visit_u64(n),
                J::I(n) => visitor.visit_i64(n),
                J::Str(s) => visitor.visit_string(s),
                J::Arr(items) => visitor.visit_seq(SeqDeserializer::new(items.into_iter())),
                J::Obj(fields) => visitor.visit_map(MapDeserializer::new(fields.into_iter())),
            }
        }

        fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
            match self {
                J::Null => visitor.visit_none(),
                other => visitor.visit_some(other),
            }
        }

        fn deserialize_enum<V: Visitor<'de>>(
            self,
            _name: &'static str,
            _variants: &'static [&'static str],
            visitor: V,
        ) -> Result<V::Value, Error> {
            match self {
                J::Str(s) => visitor.visit_enum(s.into_deserializer()),
                _ => Err(<Error as serde::de::Error>::custom(
                    "only unit variants are supported by the test deserializer",
                )),
            }
        }

        forward_to_deserialize_any! {
            bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf unit
            unit_struct newtype_struct seq tuple tuple_struct map struct identifier ignored_any
        }
    }
}
