//! OpenRTB JSON runtime used by the code `openrtb-codegen` generates for
//! `openrtb-model` and `adcom`.
//!
//! Decoding is lenient the way production traffic requires: numbers may come
//! as strings, booleans as `0`/`1`/`true`/`false`, a scalar where an array is
//! expected, and `null` anywhere means "absent". Encoding writes compact JSON
//! straight into a `Vec<u8>`, omits absent fields (absent ≠ 0) and writes
//! booleans as `0`/`1`.

use std::borrow::Cow;
use std::fmt;
use std::marker::PhantomData;
use std::ops::Deref;

use buffa::{UnknownField, UnknownFieldData, UnknownFields};
use serde::de::{self, Deserialize, DeserializeOwned, Deserializer, MapAccess, SeqAccess, Visitor};

/// Field number under which a JSON object's unknown keys are stored, as one raw
/// JSON object, in the unknown fields of the message. They survive a
/// JSON → proto → JSON round trip. 536 870 911 is the highest valid field
/// number, far from every extension range in use.
pub const RAW_JSON_FIELD: u32 = 536_870_911;

/// Marks a native request/response that arrived wrapped as `{"native": {...}}`
/// (Native 1.0 style), so that it is written back the same way.
pub const NATIVE_WRAPPER_FIELD: u32 = 536_870_910;

/// OpenRTB JSON encoding/decoding, implemented by every generated message.
pub trait OpenRtbJson: Sized {
    /// Appends the compact JSON encoding of `self` to `out`.
    fn write_json(&self, out: &mut Vec<u8>);

    fn to_json_vec(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1024);
        self.write_json(&mut out);
        out
    }

    fn to_json_string(&self) -> String {
        // The writer only emits valid UTF-8.
        String::from_utf8(self.to_json_vec()).expect("generated JSON is UTF-8")
    }

    fn from_json_slice(json: &[u8]) -> Result<Self, serde_json::Error>
    where
        Self: DeserializeOwned,
    {
        serde_json::from_slice(json)
    }

    fn from_json_str(json: &str) -> Result<Self, serde_json::Error>
    where
        Self: DeserializeOwned,
    {
        serde_json::from_str(json)
    }
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// Object key, borrowed from the input when it contains no escapes.
pub struct Key<'de>(Cow<'de, str>);

impl Deref for Key<'_> {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Key<'de> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Key<'de>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object key")
            }
            fn visit_borrowed_str<E: de::Error>(self, v: &'de str) -> Result<Self::Value, E> {
                Ok(Key(Cow::Borrowed(v)))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(Key(Cow::Owned(v.to_owned())))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(Key(Cow::Owned(v)))
            }
        }
        d.deserialize_str(V)
    }
}

/// A scalar accepted leniently from any JSON scalar that converts losslessly.
pub trait Scalar: Sized {
    const EXPECTING: &'static str;
    fn from_i64(v: i64) -> Option<Self>;
    fn from_u64(v: u64) -> Option<Self>;
    fn from_f64(v: f64) -> Option<Self>;
    fn from_bool(v: bool) -> Option<Self>;
    fn from_str(v: &str) -> Option<Self>;
    fn from_string(v: String) -> Option<Self> {
        Self::from_str(&v)
    }
}

impl Scalar for String {
    const EXPECTING: &'static str = "a string";
    fn from_i64(v: i64) -> Option<Self> {
        Some(v.to_string())
    }
    fn from_u64(v: u64) -> Option<Self> {
        Some(v.to_string())
    }
    fn from_f64(v: f64) -> Option<Self> {
        Some(v.to_string())
    }
    fn from_bool(_: bool) -> Option<Self> {
        None
    }
    fn from_str(v: &str) -> Option<Self> {
        Some(v.to_owned())
    }
    fn from_string(v: String) -> Option<Self> {
        Some(v)
    }
}

impl Scalar for bool {
    const EXPECTING: &'static str = "a boolean (0, 1, true or false)";
    // Out-of-spec integers (`"dnt": 2` is seen in the wild) read as `true`;
    // validation reports them.
    fn from_i64(v: i64) -> Option<Self> {
        Some(v != 0)
    }
    fn from_u64(v: u64) -> Option<Self> {
        Some(v != 0)
    }
    fn from_f64(v: f64) -> Option<Self> {
        Some(v != 0.0)
    }
    fn from_bool(v: bool) -> Option<Self> {
        Some(v)
    }
    fn from_str(v: &str) -> Option<Self> {
        match v {
            "0" | "false" => Some(false),
            "1" | "true" => Some(true),
            _ => None,
        }
    }
}

macro_rules! int_scalar {
    ($($t:ty),*) => {$(
        impl Scalar for $t {
            const EXPECTING: &'static str = concat!("an integer (", stringify!($t), ")");
            fn from_i64(v: i64) -> Option<Self> { <$t>::try_from(v).ok() }
            fn from_u64(v: u64) -> Option<Self> { <$t>::try_from(v).ok() }
            fn from_f64(v: f64) -> Option<Self> {
                // Accept 300.0 but not 300.5.
                (v.fract() == 0.0 && v >= <$t>::MIN as f64 && v <= <$t>::MAX as f64).then(|| v as $t)
            }
            fn from_bool(v: bool) -> Option<Self> { Some(v as $t) }
            fn from_str(v: &str) -> Option<Self> {
                let v = v.trim();
                v.parse().ok().or_else(|| v.parse::<f64>().ok().and_then(Self::from_f64))
            }
        }
    )*};
}
int_scalar!(i32, i64, u32, u64);

macro_rules! float_scalar {
    ($($t:ty),*) => {$(
        impl Scalar for $t {
            const EXPECTING: &'static str = "a number";
            fn from_i64(v: i64) -> Option<Self> { Some(v as $t) }
            fn from_u64(v: u64) -> Option<Self> { Some(v as $t) }
            fn from_f64(v: f64) -> Option<Self> { Some(v as $t) }
            fn from_bool(_: bool) -> Option<Self> { None }
            fn from_str(v: &str) -> Option<Self> { v.trim().parse().ok() }
        }
    )*};
}
float_scalar!(f32, f64);

/// Visits one JSON scalar into `T`; `null` gives `None`.
struct ScalarVisitor<T>(PhantomData<T>);

impl<'de, T: Scalar> Visitor<'de> for ScalarVisitor<T> {
    type Value = Option<T>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(T::EXPECTING)
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
        T::from_i64(v)
            .map(Some)
            .ok_or_else(|| E::invalid_type(de::Unexpected::Signed(v), &self))
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
        T::from_u64(v)
            .map(Some)
            .ok_or_else(|| E::invalid_type(de::Unexpected::Unsigned(v), &self))
    }
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
        T::from_f64(v)
            .map(Some)
            .ok_or_else(|| E::invalid_type(de::Unexpected::Float(v), &self))
    }
    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
        T::from_bool(v)
            .map(Some)
            .ok_or_else(|| E::invalid_type(de::Unexpected::Bool(v), &self))
    }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
        T::from_str(v)
            .map(Some)
            .ok_or_else(|| E::invalid_value(de::Unexpected::Str(v), &self))
    }
    fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
        // Error message needs the value, so convert by reference first.
        match T::from_str(&v) {
            Some(_) => Ok(T::from_string(v)),
            None => Err(E::invalid_value(de::Unexpected::Str(&v), &self)),
        }
    }
    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_any(self)
    }
}

/// An optional scalar, decoded leniently.
pub struct Lenient<T>(pub Option<T>);

impl<'de, T: Scalar> Deserialize<'de> for Lenient<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(ScalarVisitor(PhantomData)).map(Lenient)
    }
}

/// A repeated scalar: an array (null elements skipped), a lone scalar (one
/// element), or `null` (empty).
pub struct LenientVec<T>(pub Vec<T>);

impl<'de, T: Scalar> Deserialize<'de> for LenientVec<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T>(PhantomData<T>);
        impl<'de, T: Scalar> Visitor<'de> for V<T> {
            type Value = Vec<T>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "an array of {}", T::EXPECTING)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(4));
                while let Some(Lenient(v)) = seq.next_element::<Lenient<T>>()? {
                    out.extend(v);
                }
                Ok(out)
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(Vec::new())
            }
            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(Vec::new())
            }
            fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
                d.deserialize_any(self)
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(ScalarVisitor::<T>(PhantomData)
                    .visit_i64(v)?
                    .into_iter()
                    .collect())
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(ScalarVisitor::<T>(PhantomData)
                    .visit_u64(v)?
                    .into_iter()
                    .collect())
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
                Ok(ScalarVisitor::<T>(PhantomData)
                    .visit_f64(v)?
                    .into_iter()
                    .collect())
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(ScalarVisitor::<T>(PhantomData)
                    .visit_bool(v)?
                    .into_iter()
                    .collect())
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(ScalarVisitor::<T>(PhantomData)
                    .visit_str(v)?
                    .into_iter()
                    .collect())
            }
        }
        d.deserialize_any(V(PhantomData)).map(LenientVec)
    }
}

/// In-place JSON decoding, generated for every message.
///
/// Messages are large (sub-messages are stored inline), so building them by
/// value and moving them up the tree would copy kilobytes per level. Instead
/// every object is filled where it finally lives.
pub trait JsonFill: Default {
    /// Reads the members of a JSON object into `self`.
    fn fill<'de, A: MapAccess<'de>>(&mut self, map: A) -> Result<(), A::Error>;
}

/// `Deserialize` for a [`JsonFill`] message.
pub fn deserialize_filled<'de, T: JsonFill, D: Deserializer<'de>>(d: D) -> Result<T, D::Error> {
    let mut m = T::default();
    de::DeserializeSeed::deserialize(Fill(&mut m), d)?;
    Ok(m)
}

/// Seed filling an existing message from a JSON object. Yields `false` for
/// `null` (the message is then left untouched).
pub struct Fill<'a, T>(pub &'a mut T);

impl<'de, T: JsonFill> de::DeserializeSeed<'de> for Fill<'_, T> {
    type Value = bool;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<bool, D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de, T: JsonFill> Visitor<'de> for Fill<'_, T> {
    type Value = bool;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an object")
    }
    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<bool, A::Error> {
        self.0.fill(map)?;
        Ok(true)
    }
    fn visit_unit<E: de::Error>(self) -> Result<bool, E> {
        Ok(false)
    }
    fn visit_none<E: de::Error>(self) -> Result<bool, E> {
        Ok(false)
    }
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<bool, D::Error> {
        d.deserialize_any(self)
    }
}

/// Seed filling a repeated message field: elements are decoded in place at the
/// end of the vector; `null` elements are skipped.
pub struct FillVec<'a, T>(pub &'a mut Vec<T>);

impl<'de, T: JsonFill> de::DeserializeSeed<'de> for FillVec<'_, T> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de, T: JsonFill> Visitor<'de> for FillVec<'_, T> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an array of objects")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        loop {
            self.0.push(T::default());
            let slot = self.0.last_mut().expect("just pushed");
            match seq.next_element_seed(Fill(slot))? {
                Some(true) => {}
                Some(false) => {
                    self.0.pop();
                }
                None => {
                    self.0.pop();
                    return Ok(());
                }
            }
        }
    }
    /// A lone object where an array is expected.
    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<(), A::Error> {
        let mut m = T::default();
        m.fill(map)?;
        self.0.push(m);
        Ok(())
    }
    fn visit_unit<E: de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_none<E: de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_any(self)
    }
}

/// A value sent either as a JSON-encoded string or as the object itself, like
/// `imp.native.request` and `bid.adm` for native ads.
pub enum StrOrMsg<M> {
    Str(String),
    Msg(M),
    Null,
}

impl<'de, M: Deserialize<'de>> Deserialize<'de> for StrOrMsg<M> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<M>(PhantomData<M>);
        impl<'de, M: Deserialize<'de>> Visitor<'de> for V<M> {
            type Value = StrOrMsg<M>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a string or an object")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(StrOrMsg::Str(v.to_owned()))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(StrOrMsg::Str(v))
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                M::deserialize(de::value::MapAccessDeserializer::new(map)).map(StrOrMsg::Msg)
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrOrMsg::Null)
            }
            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrOrMsg::Null)
            }
            fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
                d.deserialize_any(self)
            }
        }
        d.deserialize_any(V(PhantomData))
    }
}

/// Collects the unknown keys of a JSON object into one raw JSON object.
///
/// Values are transcoded token by token while being parsed, so this works with
/// any serde deserializer (serde_json, sonic-rs, simd-json…), with no
/// intermediate `Value`.
#[derive(Default)]
pub struct UnknownKeys {
    buf: Vec<u8>,
}

impl UnknownKeys {
    /// Reads the value of unknown key `key` from `map` and records it.
    pub fn capture<'de, A: MapAccess<'de>>(
        &mut self,
        key: &str,
        map: &mut A,
    ) -> Result<(), A::Error> {
        self.buf.push(if self.buf.is_empty() { b'{' } else { b',' });
        write_str(&mut self.buf, key);
        self.buf.push(b':');
        map.next_value_seed(Transcode(&mut self.buf))
    }

    pub fn finish(mut self, fields: &mut UnknownFields) {
        if self.buf.is_empty() {
            return;
        }
        self.buf.push(b'}');
        fields.retain(|f| f.number != RAW_JSON_FIELD);
        fields.push(UnknownField {
            number: RAW_JSON_FIELD,
            data: UnknownFieldData::LengthDelimited(self.buf),
        });
    }
}

/// Writes whatever JSON value is being deserialized, compactly, into the buffer.
struct Transcode<'b>(&'b mut Vec<u8>);

impl<'de> de::DeserializeSeed<'de> for Transcode<'_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Transcode<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }
    fn visit_bool<E: de::Error>(self, v: bool) -> Result<(), E> {
        self.0.extend_from_slice(if v { b"true" } else { b"false" });
        Ok(())
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<(), E> {
        write_int(self.0, v);
        Ok(())
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<(), E> {
        write_int(self.0, v);
        Ok(())
    }
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<(), E> {
        write_f64(self.0, v);
        Ok(())
    }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<(), E> {
        write_str(self.0, v);
        Ok(())
    }
    fn visit_unit<E: de::Error>(self) -> Result<(), E> {
        self.0.extend_from_slice(b"null");
        Ok(())
    }
    fn visit_none<E: de::Error>(self) -> Result<(), E> {
        self.visit_unit()
    }
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_any(self)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        self.0.push(b'[');
        let mut first = true;
        loop {
            let mark = self.0.len();
            if !first {
                self.0.push(b',');
            }
            if seq.next_element_seed(Transcode(self.0))?.is_none() {
                self.0.truncate(mark);
                break;
            }
            first = false;
        }
        self.0.push(b']');
        Ok(())
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        self.0.push(b'{');
        let mut first = true;
        while let Some(key) = map.next_key::<Key<'de>>()? {
            if !first {
                self.0.push(b',');
            }
            first = false;
            write_str(self.0, &key);
            self.0.push(b':');
            map.next_value_seed(Transcode(self.0))?;
        }
        self.0.push(b'}');
        Ok(())
    }
}

/// The raw JSON object of a message's unknown keys, if any.
pub fn unknown_keys(fields: &UnknownFields) -> Option<&str> {
    fields
        .iter()
        .find(|f| f.number == RAW_JSON_FIELD)
        .and_then(|f| match &f.data {
            UnknownFieldData::LengthDelimited(b) => std::str::from_utf8(b).ok(),
            _ => None,
        })
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

/// Writes one JSON object, inserting commas between members.
pub struct ObjWriter<'a> {
    pub out: &'a mut Vec<u8>,
    first: bool,
}

impl<'a> ObjWriter<'a> {
    #[inline]
    pub fn new(out: &'a mut Vec<u8>) -> Self {
        out.push(b'{');
        Self { out, first: true }
    }

    /// `key` is a pre-escaped `"name":` literal.
    #[inline]
    pub fn key(&mut self, key: &[u8]) {
        if !self.first {
            self.out.push(b',');
        }
        self.first = false;
        self.out.extend_from_slice(key);
    }

    /// Appends the members of a raw JSON object (`{...}`).
    fn merge_raw_object(&mut self, raw: &[u8]) {
        let inner = raw.trim_ascii();
        let inner = inner
            .strip_prefix(b"{")
            .and_then(|r| r.strip_suffix(b"}"))
            .unwrap_or(inner)
            .trim_ascii();
        if inner.is_empty() {
            return;
        }
        if !self.first {
            self.out.push(b',');
        }
        self.first = false;
        self.out.extend_from_slice(inner);
    }

    #[inline]
    pub fn end(self) {
        self.out.push(b'}');
    }
}

const ESCAPE: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 0x20 {
        t[i] = b'u';
        i += 1;
    }
    t[b'"' as usize] = b'"';
    t[b'\\' as usize] = b'\\';
    t[0x08] = b'b';
    t[0x0c] = b'f';
    t[b'\n' as usize] = b'n';
    t[b'\r' as usize] = b'r';
    t[b'\t' as usize] = b't';
    t
};

#[inline]
pub fn write_str(out: &mut Vec<u8>, s: &str) {
    out.push(b'"');
    let bytes = s.as_bytes();
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        let esc = ESCAPE[b as usize];
        if esc == 0 {
            continue;
        }
        out.extend_from_slice(&bytes[start..i]);
        if esc == b'u' {
            const HEX: &[u8; 16] = b"0123456789abcdef";
            out.extend_from_slice(&[
                b'\\',
                b'u',
                b'0',
                b'0',
                HEX[(b >> 4) as usize],
                HEX[(b & 0xf) as usize],
            ]);
        } else {
            out.extend_from_slice(&[b'\\', esc]);
        }
        start = i + 1;
    }
    out.extend_from_slice(&bytes[start..]);
    out.push(b'"');
}

#[inline]
pub fn write_int<I: itoa::Integer>(out: &mut Vec<u8>, v: I) {
    out.extend_from_slice(itoa::Buffer::new().format(v).as_bytes());
}

/// Map keys must be strings in JSON.
#[inline]
pub fn write_key_int<I: itoa::Integer>(out: &mut Vec<u8>, v: I) {
    out.push(b'"');
    write_int(out, v);
    out.push(b'"');
}

/// Non-finite floats have no JSON form; they are written as `null`.
#[inline]
pub fn write_f64(out: &mut Vec<u8>, v: f64) {
    if v.is_finite() {
        out.extend_from_slice(ryu::Buffer::new().format_finite(v).as_bytes());
    } else {
        out.extend_from_slice(b"null");
    }
}

#[inline]
pub fn write_f32(out: &mut Vec<u8>, v: f32) {
    if v.is_finite() {
        out.extend_from_slice(ryu::Buffer::new().format_finite(v).as_bytes());
    } else {
        out.extend_from_slice(b"null");
    }
}

#[inline]
pub fn write_bool(out: &mut Vec<u8>, v: bool) {
    out.push(if v { b'1' } else { b'0' });
}

#[inline]
pub fn write_array<T>(out: &mut Vec<u8>, items: &[T], mut write: impl FnMut(&mut Vec<u8>, &T)) {
    out.push(b'[');
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(b',');
        }
        write(out, item);
    }
    out.push(b']');
}

pub fn write_map<'m, K: 'm, V: 'm>(
    out: &mut Vec<u8>,
    entries: impl Iterator<Item = (&'m K, &'m V)>,
    mut write_key: impl FnMut(&mut Vec<u8>, &K),
    mut write_value: impl FnMut(&mut Vec<u8>, &V),
) {
    out.push(b'{');
    for (i, (k, v)) in entries.enumerate() {
        if i > 0 {
            out.push(b',');
        }
        write_key(out, k);
        out.push(b':');
        write_value(out, v);
    }
    out.push(b'}');
}

/// For well-known types (google.protobuf.Value…) whose serde form is already plain JSON.
pub fn write_serde<T: serde::Serialize>(out: &mut Vec<u8>, v: &T) {
    if serde_json::to_writer(&mut *out, v).is_err() {
        out.extend_from_slice(b"null");
    }
}

pub fn set_native_wrapper(fields: &mut UnknownFields) {
    fields.retain(|f| f.number != NATIVE_WRAPPER_FIELD);
    fields.push(UnknownField {
        number: NATIVE_WRAPPER_FIELD,
        data: UnknownFieldData::Varint(1),
    });
}

pub fn has_native_wrapper(fields: &UnknownFields) -> bool {
    fields.iter().any(|f| f.number == NATIVE_WRAPPER_FIELD)
}

/// Re-emits the unknown keys captured at decode time.
pub fn write_unknown_keys(w: &mut ObjWriter<'_>, fields: &UnknownFields) {
    for f in fields.iter().filter(|f| f.number == RAW_JSON_FIELD) {
        if let UnknownFieldData::LengthDelimited(raw) = &f.data {
            w.merge_raw_object(raw);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_strings() {
        let mut out = Vec::new();
        write_str(&mut out, "a\"b\\c\nd\u{1}é");
        assert_eq!(std::str::from_utf8(&out).unwrap(), r#""a\"b\\c\nd\u0001é""#);
    }

    #[test]
    fn lenient_scalars() {
        let v: Lenient<i32> = serde_json::from_str(r#""300""#).unwrap();
        assert_eq!(v.0, Some(300));
        let v: Lenient<i32> = serde_json::from_str("300.0").unwrap();
        assert_eq!(v.0, Some(300));
        assert!(serde_json::from_str::<Lenient<i32>>("300.5").is_err());
        let v: Lenient<bool> = serde_json::from_str("1").unwrap();
        assert_eq!(v.0, Some(true));
        let v: Lenient<String> = serde_json::from_str("123").unwrap();
        assert_eq!(v.0.as_deref(), Some("123"));
        let v: Lenient<f64> = serde_json::from_str("null").unwrap();
        assert_eq!(v.0, None);
        let v: LenientVec<String> = serde_json::from_str(r#""USD""#).unwrap();
        assert_eq!(v.0, ["USD"]);
    }

    #[test]
    fn transcodes_any_value() {
        let mut buf = Vec::new();
        let mut de = serde_json::Deserializer::from_str(
            r#"{"a" : [1, -2, 3.5, "x\"y", null, true, {}, []] }"#,
        );
        de::DeserializeSeed::deserialize(Transcode(&mut buf), &mut de).unwrap();
        assert_eq!(
            std::str::from_utf8(&buf).unwrap(),
            r#"{"a":[1,-2,3.5,"x\"y",null,true,{},[]]}"#
        );
    }
}
