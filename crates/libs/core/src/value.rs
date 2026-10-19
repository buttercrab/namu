//! Values: any Rust value, typed while it stays in this process, MessagePack when it leaves.
//!
//! A [`Value`] is a payload and a pointer to a static vtable for its type. A type that fits in
//! two words is stored in place; anything larger is shared behind an `Arc`, so cloning a value
//! never copies its data. The engine reads values in only three places: a fan-out splits a list,
//! a collect builds one, and a branch reads a bool. A type's vtable answers those from its
//! [`Data`] implementation, and everything else passes through untouched (design §8).
//!
//! A value that arrives from outside the process is [encoded](Value::encoded): MessagePack bytes,
//! split without decoding, and decoded into a type only when something asks for that type. A
//! collect makes a list of values, whatever their types, which becomes a `Vec<T>` only when
//! something asks for one.

use std::any::{TypeId, type_name};
use std::collections::{BTreeMap, HashMap};
use std::hash::{BuildHasher, Hash};
use std::mem::{self, ManuallyDrop, MaybeUninit};
use std::sync::{Arc, OnceLock};
use std::{fmt, ptr};

use anyhow::{Context, bail};
use bytes::Bytes;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A type the engine can hold as a value. Derive it for your own types with
/// `#[derive(namu::Data)]`; it is implemented for the standard ones.
pub trait Data: Clone + Send + Sync + Serialize + DeserializeOwned + 'static {
    /// The elements of a list or tuple, which a fan-out or a task's several outputs split into.
    /// `None` for everything else.
    const ITEMS: Option<fn(&Self) -> Vec<Value>> = None;

    /// Builds a list or tuple from its elements: from a collected list, or from a task's several
    /// arguments. `None` for everything else.
    const FROM_ITEMS: Option<fn(Vec<Value>) -> anyhow::Result<Self>> = None;
}

/// A value of any [`Data`] type, or MessagePack bytes from outside the process.
pub struct Value {
    payload: Payload,
    vtable: &'static VTable,
}

/// The value itself (when it fits) or a pointer to an `Arc` holding it.
union Payload {
    inline: [MaybeUninit<usize>; 2],
    shared: *const (),
}

// SAFETY: a payload holds a `T: Send + Sync` (every constructor requires it), owned or shared
// through an `Arc`.
unsafe impl Send for Value {}
unsafe impl Sync for Value {}

struct VTable {
    type_id: TypeId,
    type_name: fn() -> &'static str,
    clone: unsafe fn(&Payload) -> Payload,
    drop: unsafe fn(&mut Payload),
    encode: unsafe fn(&Payload) -> anyhow::Result<Bytes>,
    items: Option<unsafe fn(&Payload) -> anyhow::Result<Vec<Value>>>,
}

const fn is_inline<T>() -> bool {
    mem::size_of::<T>() <= mem::size_of::<Payload>()
        && mem::align_of::<T>() <= mem::align_of::<Payload>()
}

/// Storage of a `T` in a payload.
struct Stored<T>(std::marker::PhantomData<T>);

impl<T: Clone + Send + Sync + 'static> Stored<T> {
    fn store(value: T) -> Payload {
        if is_inline::<T>() {
            let mut payload = Payload {
                inline: [MaybeUninit::uninit(); 2],
            };
            // SAFETY: `T` fits in the inline words, with their alignment.
            unsafe { ptr::write((&raw mut payload.inline).cast::<T>(), value) };
            payload
        } else {
            Payload {
                shared: Arc::into_raw(Arc::new(value)).cast(),
            }
        }
    }

    /// SAFETY: `payload` holds a `T`.
    unsafe fn get(payload: &Payload) -> &T {
        unsafe {
            match is_inline::<T>() {
                true => &*(&raw const payload.inline).cast::<T>(),
                false => &*payload.shared.cast::<T>(),
            }
        }
    }

    /// Moves the `T` out: the only copy if nothing else shares it, a clone otherwise.
    /// SAFETY: `payload` holds a `T`, and is not used again.
    unsafe fn take(payload: Payload) -> T {
        unsafe {
            match is_inline::<T>() {
                true => ptr::read((&raw const payload.inline).cast::<T>()),
                false => {
                    let shared = Arc::from_raw(payload.shared.cast::<T>());
                    Arc::try_unwrap(shared).unwrap_or_else(|shared| (*shared).clone())
                }
            }
        }
    }

    unsafe fn clone(payload: &Payload) -> Payload {
        unsafe {
            match is_inline::<T>() {
                true => Self::store(Self::get(payload).clone()),
                false => {
                    Arc::increment_strong_count(payload.shared.cast::<T>());
                    Payload {
                        shared: payload.shared,
                    }
                }
            }
        }
    }

    unsafe fn drop(payload: &mut Payload) {
        unsafe {
            match is_inline::<T>() {
                true => ptr::drop_in_place((&raw mut payload.inline).cast::<T>()),
                false => drop(Arc::from_raw(payload.shared.cast::<T>())),
            }
        }
    }
}

/// The vtable of a [`Data`] type.
struct Typed<T>(std::marker::PhantomData<T>);

impl<T: Data> Typed<T> {
    const VTABLE: VTable = VTable {
        type_id: TypeId::of::<T>(),
        type_name: type_name::<T>,
        clone: Stored::<T>::clone,
        drop: Stored::<T>::drop,
        encode: Self::encode,
        items: match T::ITEMS {
            Some(_) => Some(Self::items),
            None => None,
        },
    };

    unsafe fn encode(payload: &Payload) -> anyhow::Result<Bytes> {
        let value = unsafe { Stored::<T>::get(payload) };
        let bytes = rmp_serde::to_vec_named(value)
            .with_context(|| format!("encoding a {}", type_name::<T>()))?;
        Ok(bytes.into())
    }

    unsafe fn items(payload: &Payload) -> anyhow::Result<Vec<Value>> {
        let items = T::ITEMS.expect("a vtable with items is a list's");
        Ok(items(unsafe { Stored::<T>::get(payload) }))
    }
}

/// MessagePack bytes, and what they were first decoded into.
#[derive(Clone)]
struct Encoded {
    bytes: Bytes,
    decoded: OnceLock<Value>,
}

impl Encoded {
    const VTABLE: VTable = VTable {
        type_id: TypeId::of::<Encoded>(),
        type_name: || "encoded MessagePack",
        clone: Stored::<Encoded>::clone,
        drop: Stored::<Encoded>::drop,
        encode: Self::encode,
        items: Some(Self::items),
    };

    unsafe fn encode(payload: &Payload) -> anyhow::Result<Bytes> {
        Ok(unsafe { Stored::<Encoded>::get(payload) }.bytes.clone())
    }

    /// The elements of an encoded array, each a slice of the same bytes.
    unsafe fn items(payload: &Payload) -> anyhow::Result<Vec<Value>> {
        let bytes = &unsafe { Stored::<Encoded>::get(payload) }.bytes;
        msgpack::elements(bytes)
            .map(|elements| elements.into_iter().map(Value::encoded).collect())
    }
}

/// The values a collect gathered.
#[derive(Clone)]
struct List(Vec<Value>);

impl List {
    const VTABLE: VTable = VTable {
        type_id: TypeId::of::<List>(),
        type_name: || "list",
        clone: Stored::<List>::clone,
        drop: Stored::<List>::drop,
        encode: Self::encode,
        items: Some(Self::items),
    };

    /// An array of the items, each encoded.
    unsafe fn encode(payload: &Payload) -> anyhow::Result<Bytes> {
        let items = &unsafe { Stored::<List>::get(payload) }.0;
        Ok(msgpack::array(&items.iter().map(Value::encode).collect::<anyhow::Result<Vec<_>>>()?))
    }

    unsafe fn items(payload: &Payload) -> anyhow::Result<Vec<Value>> {
        Ok(unsafe { Stored::<List>::get(payload) }.0.clone())
    }
}

impl Value {
    pub fn new<T: Data>(value: T) -> Self {
        Self {
            payload: Stored::<T>::store(value),
            vtable: &Typed::<T>::VTABLE,
        }
    }

    /// A value from MessagePack bytes.
    pub fn encoded(bytes: impl Into<Bytes>) -> Self {
        let encoded = Encoded {
            bytes: bytes.into(),
            decoded: OnceLock::new(),
        };
        Self {
            payload: Stored::<Encoded>::store(encoded),
            vtable: &Encoded::VTABLE,
        }
    }

    fn is_encoded(&self) -> bool {
        self.is::<Encoded>()
    }

    /// Whether this is a live `T`.
    pub fn is<T: 'static>(&self) -> bool {
        self.vtable.type_id == TypeId::of::<T>()
    }

    /// The `T` this value holds, if it is a live `T`.
    pub fn downcast_ref<T: Data>(&self) -> Option<&T> {
        // SAFETY: the vtable says the payload holds a `T`.
        self.is::<T>()
            .then(|| unsafe { Stored::<T>::get(&self.payload) })
    }

    /// This value as a `T`: moved out if nothing else shares it, cloned if something does,
    /// built from its items if it is a collected list, decoded if it is encoded.
    pub fn take<T: Data>(self) -> anyhow::Result<T> {
        if self.is::<T>() || self.is::<List>() {
            let this = ManuallyDrop::new(self);
            // SAFETY: the vtable says which type the payload holds; `this` is not dropped, so
            // the payload is moved out exactly once.
            let payload = unsafe { ptr::read(&this.payload) };
            if this.is::<T>() {
                return Ok(unsafe { Stored::<T>::take(payload) });
            }
            return from_list(unsafe { Stored::<List>::take(payload) }.0);
        }
        self.to()
    }

    /// A copy of this value as a `T`, built from its items if it is a collected list, decoded if
    /// it is encoded. An encoded value keeps what it was first decoded into, so asking again is a
    /// clone.
    pub fn to<T: Data>(&self) -> anyhow::Result<T> {
        if let Some(value) = self.downcast_ref::<T>() {
            return Ok(value.clone());
        }
        if self.is::<List>() {
            // SAFETY: the vtable says the payload holds a `List`.
            return from_list(unsafe { Stored::<List>::get(&self.payload) }.0.clone());
        }
        if !self.is_encoded() {
            bail!("expected a {}, got a {}", type_name::<T>(), self.type_name());
        }
        // SAFETY: the vtable says the payload holds an `Encoded`.
        let encoded = unsafe { Stored::<Encoded>::get(&self.payload) };
        if let Some(value) = encoded.decoded.get().and_then(Value::downcast_ref::<T>) {
            return Ok(value.clone());
        }
        let value: T = rmp_serde::from_slice(&encoded.bytes)
            .with_context(|| format!("decoding a {}", type_name::<T>()))?;
        let _ = encoded.decoded.set(Value::new(value.clone()));
        Ok(value)
    }

    /// Whether a branch on this value is taken.
    pub fn as_bool(&self) -> anyhow::Result<bool> {
        self.to::<bool>()
    }

    /// This value as MessagePack.
    pub fn encode(&self) -> anyhow::Result<Bytes> {
        // SAFETY: the vtable matches the payload.
        unsafe { (self.vtable.encode)(&self.payload) }
    }

    /// The elements of a list or tuple.
    pub fn items(&self) -> anyhow::Result<Vec<Value>> {
        match self.vtable.items {
            // SAFETY: the vtable matches the payload.
            Some(items) => unsafe { items(&self.payload) },
            None => bail!("a {} is not a list", self.type_name()),
        }
    }

    /// The list of `items`, whatever their types.
    pub fn list(items: Vec<Value>) -> Value {
        Self {
            payload: Stored::<List>::store(List(items)),
            vtable: &List::VTABLE,
        }
    }

    /// The name of the type this value holds.
    pub fn type_name(&self) -> &'static str {
        (self.vtable.type_name)()
    }
}

impl Clone for Value {
    fn clone(&self) -> Self {
        Self {
            // SAFETY: the vtable matches the payload.
            payload: unsafe { (self.vtable.clone)(&self.payload) },
            vtable: self.vtable,
        }
    }
}

impl Drop for Value {
    fn drop(&mut self) {
        // SAFETY: the vtable matches the payload, which is dropped exactly once.
        unsafe { (self.vtable.drop)(&mut self.payload) }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Value({})", self.type_name())
    }
}

/// A value crosses a process boundary as its MessagePack bytes.
impl Serialize for Value {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let bytes = self.encode().map_err(serde::ser::Error::custom)?;
        serializer.serialize_bytes(&bytes)
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let bytes: serde_bytes::ByteBuf = Deserialize::deserialize(deserializer)?;
        Ok(Value::encoded(bytes.into_vec()))
    }
}

/// Packs a task's arguments into one MessagePack document: nil for none, the argument itself for
/// one, an array for several. This is the task ABI's convention.
pub fn encode_args(args: &[Value]) -> anyhow::Result<Bytes> {
    match args {
        [] => Ok(Bytes::from_static(&[msgpack::NIL])),
        [arg] => arg.encode(),
        args => {
            let parts = args
                .iter()
                .map(Value::encode)
                .collect::<anyhow::Result<Vec<_>>>()?;
            Ok(msgpack::array(&parts))
        }
    }
}

fn from_list<T: Data>(items: Vec<Value>) -> anyhow::Result<T> {
    match T::FROM_ITEMS {
        Some(from_items) => from_items(items),
        None => bail!("expected a {}, got a list", type_name::<T>()),
    }
}

/// Builds a task's input from its arguments, by the same convention as [`encode_args`].
pub fn from_args<T: Data>(mut args: Vec<Value>) -> anyhow::Result<T> {
    match (args.len(), T::FROM_ITEMS) {
        (0, _) => Value::new(()).take(),
        (1, _) => args.pop().expect("one argument").take(),
        (_, Some(from_items)) => from_items(args),
        (n, None) => bail!("{n} arguments do not make a {}", type_name::<T>()),
    }
}

macro_rules! scalars {
    ($($t:ty),* $(,)?) => { $(impl Data for $t {})* };
}

scalars!(
    (),
    bool,
    char,
    i8,
    i16,
    i32,
    i64,
    i128,
    isize,
    u8,
    u16,
    u32,
    u64,
    u128,
    usize,
    f32,
    f64,
    String
);

impl<T: Data> Data for Option<T> {}

impl<T: Data> Data for Box<T> {}

impl<T: Data> Data for Vec<T> {
    const ITEMS: Option<fn(&Self) -> Vec<Value>> =
        Some(|list| list.iter().cloned().map(Value::new).collect());

    const FROM_ITEMS: Option<fn(Vec<Value>) -> anyhow::Result<Self>> =
        Some(|items| items.into_iter().map(Value::take).collect());
}

impl<K: Data + Ord, V: Data> Data for BTreeMap<K, V> {}

impl<K, V, S> Data for HashMap<K, V, S>
where
    K: Data + Eq + Hash,
    V: Data,
    S: BuildHasher + Default + Clone + Send + Sync + 'static,
{
}

macro_rules! tuples {
    ($(($($name:ident $index:tt),+))*) => {$(
        impl<$($name: Data),+> Data for ($($name,)+) {
            const ITEMS: Option<fn(&Self) -> Vec<Value>> =
                Some(|tuple| vec![$(Value::new(tuple.$index.clone())),+]);

            const FROM_ITEMS: Option<fn(Vec<Value>) -> anyhow::Result<Self>> = Some(|items| {
                let count = [$(stringify!($name)),+].len();
                anyhow::ensure!(items.len() == count, "expected {count} values, got {}", items.len());
                let mut items = items.into_iter();
                Ok(($(items.next().expect("counted").take::<$name>()?,)+))
            });
        }
    )*};
}

tuples! {
    (A 0)
    (A 0, B 1)
    (A 0, B 1, C 2)
    (A 0, B 1, C 2, D 3)
    (A 0, B 1, C 2, D 3, E 4)
    (A 0, B 1, C 2, D 3, E 4, F 5)
    (A 0, B 1, C 2, D 3, E 4, F 5, G 6)
    (A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7)
    (A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8)
    (A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9)
    (A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10)
    (A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10, L 11)
}

/// The few parts of MessagePack the engine reads directly: array headers, and where each
/// element ends.
mod msgpack {
    use anyhow::{Context, bail};
    use bytes::{BufMut, Bytes, BytesMut};

    pub const NIL: u8 = 0xc0;

    /// An array of already encoded elements.
    pub fn array(parts: &[Bytes]) -> Bytes {
        let len = parts.iter().map(Bytes::len).sum::<usize>();
        let mut out = BytesMut::with_capacity(len + 5);
        let n = parts.len();
        if n < 16 {
            out.put_u8(0x90 | n as u8);
        } else if n <= u16::MAX as usize {
            out.put_u8(0xdc);
            out.put_u16(n as u16);
        } else {
            out.put_u8(0xdd);
            out.put_u32(n as u32);
        }
        for part in parts {
            out.extend_from_slice(part);
        }
        out.freeze()
    }

    /// The elements of an encoded array, as slices of `bytes`.
    pub fn elements(bytes: &Bytes) -> anyhow::Result<Vec<Bytes>> {
        let mut reader = Reader { bytes, at: 0 };
        let n = match reader.u8()? {
            tag @ 0x90..=0x9f => (tag & 0x0f) as usize,
            0xdc => reader.uint(2)?,
            0xdd => reader.uint(4)?,
            tag => bail!("not a MessagePack array (tag {tag:#04x})"),
        };
        let mut elements = Vec::with_capacity(n);
        for _ in 0..n {
            let start = reader.at;
            reader.skip()?;
            elements.push(bytes.slice(start..reader.at));
        }
        Ok(elements)
    }

    struct Reader<'a> {
        bytes: &'a [u8],
        at: usize,
    }

    impl Reader<'_> {
        fn u8(&mut self) -> anyhow::Result<u8> {
            let byte = *self.bytes.get(self.at).context("truncated MessagePack")?;
            self.at += 1;
            Ok(byte)
        }

        fn uint(&mut self, width: usize) -> anyhow::Result<usize> {
            let end = self.at + width;
            let raw = self.bytes.get(self.at..end).context("truncated MessagePack")?;
            self.at = end;
            Ok(raw.iter().fold(0, |n, &byte| (n << 8) | byte as usize))
        }

        fn advance(&mut self, n: usize) -> anyhow::Result<()> {
            anyhow::ensure!(self.at + n <= self.bytes.len(), "truncated MessagePack");
            self.at += n;
            Ok(())
        }

        /// Moves past one whole value.
        fn skip(&mut self) -> anyhow::Result<()> {
            // Values still to skip: nested arrays and maps add their elements.
            let mut pending = 1usize;
            while pending > 0 {
                pending -= 1;
                match self.u8()? {
                    0x00..=0x7f | 0xe0..=0xff | 0xc0 | 0xc2 | 0xc3 => {}
                    tag @ 0x80..=0x8f => pending += 2 * (tag & 0x0f) as usize,
                    tag @ 0x90..=0x9f => pending += (tag & 0x0f) as usize,
                    tag @ 0xa0..=0xbf => self.advance((tag & 0x1f) as usize)?,
                    0xcc | 0xd0 => self.advance(1)?,
                    0xcd | 0xd1 => self.advance(2)?,
                    0xca | 0xce | 0xd2 => self.advance(4)?,
                    0xcb | 0xcf | 0xd3 => self.advance(8)?,
                    0xd4 => self.advance(2)?,
                    0xd5 => self.advance(3)?,
                    0xd6 => self.advance(5)?,
                    0xd7 => self.advance(9)?,
                    0xd8 => self.advance(17)?,
                    0xc4 | 0xd9 => {
                        let n = self.uint(1)?;
                        self.advance(n)?
                    }
                    0xc5 | 0xda => {
                        let n = self.uint(2)?;
                        self.advance(n)?
                    }
                    0xc6 | 0xdb => {
                        let n = self.uint(4)?;
                        self.advance(n)?
                    }
                    0xc7 => {
                        let n = self.uint(1)?;
                        self.advance(n + 1)?
                    }
                    0xc8 => {
                        let n = self.uint(2)?;
                        self.advance(n + 1)?
                    }
                    0xc9 => {
                        let n = self.uint(4)?;
                        self.advance(n + 1)?
                    }
                    0xdc => pending += self.uint(2)?,
                    0xdd => pending += self.uint(4)?,
                    0xde => pending += 2 * self.uint(2)?,
                    0xdf => pending += 2 * self.uint(4)?,
                    tag => bail!("invalid MessagePack tag {tag:#04x}"),
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct Doc {
        title: String,
        words: Vec<String>,
    }

    impl Data for Doc {}

    fn doc(title: &str) -> Doc {
        Doc {
            title: title.into(),
            words: vec!["a".into(), "b".into()],
        }
    }

    #[test]
    fn small_values_live_inline_and_large_ones_are_shared() {
        assert!(is_inline::<i64>() && is_inline::<(u32, u32)>());
        assert!(!is_inline::<String>() && !is_inline::<Doc>());
        let value = Value::new(doc("x"));
        let copy = value.clone();
        let (a, b) = (value.downcast_ref::<Doc>(), copy.downcast_ref::<Doc>());
        assert!(ptr::eq(a.unwrap(), b.unwrap()), "a clone shares the data");
        assert_eq!(Value::new(7i64).to::<i64>().unwrap(), 7);
    }

    #[test]
    fn take_moves_the_only_copy_and_clones_a_shared_one() {
        static CLONES: AtomicUsize = AtomicUsize::new(0);
        #[derive(Debug, Serialize, Deserialize)]
        struct Counted(String);
        impl Clone for Counted {
            fn clone(&self) -> Self {
                CLONES.fetch_add(1, Ordering::Relaxed);
                Counted(self.0.clone())
            }
        }
        impl Data for Counted {}

        let value = Value::new(Counted("x".into()));
        let shared = value.clone();
        assert_eq!(value.take::<Counted>().unwrap().0, "x");
        assert_eq!(CLONES.load(Ordering::Relaxed), 1);
        assert_eq!(shared.take::<Counted>().unwrap().0, "x");
        assert_eq!(CLONES.load(Ordering::Relaxed), 1, "the last copy moves");
    }

    #[test]
    fn values_cross_as_messagepack_and_decode_once_per_type() {
        let encoded = Value::encoded(Value::new(doc("x")).encode().unwrap());
        assert!(encoded.downcast_ref::<Doc>().is_none());
        assert_eq!(encoded.to::<Doc>().unwrap(), doc("x"));
        assert_eq!(encoded.clone().take::<Doc>().unwrap(), doc("x"));
        assert!(encoded.to::<i32>().is_err());
        assert!(Value::new(1i32).to::<String>().is_err());
    }

    #[test]
    fn lists_split_and_collect_live_or_encoded() {
        let list = Value::new(vec![doc("a"), doc("b")]);
        let items = list.items().unwrap();
        assert_eq!(items[1].to::<Doc>().unwrap(), doc("b"));
        let back = Value::list(items);
        assert_eq!(back.to::<Vec<Doc>>().unwrap(), vec![doc("a"), doc("b")]);
        assert_eq!(back.items().unwrap().len(), 2);

        let encoded = Value::encoded(list.encode().unwrap());
        let items = encoded.items().unwrap();
        assert_eq!(items[0].to::<Doc>().unwrap(), doc("a"));
        // One live item and one encoded, taken as a list, or encoded and decoded.
        let mixed = Value::list(vec![Value::new(doc("c")), items[1].clone()]);
        assert_eq!(mixed.to::<Vec<Doc>>().unwrap(), vec![doc("c"), doc("b")]);
        let sent = Value::encoded(mixed.encode().unwrap());
        assert_eq!(sent.take::<Vec<Doc>>().unwrap(), vec![doc("c"), doc("b")]);
        assert_eq!(Value::list(vec![]).to::<Vec<Doc>>().unwrap(), vec![]);
        assert!(Value::new(doc("x")).items().is_err());
    }

    #[test]
    fn encoded_arrays_split_past_nested_values() {
        let nested: Vec<(String, Vec<i64>, BTreeMap<String, f64>)> = (0..40)
            .map(|i| {
                let map = BTreeMap::from([("k".repeat(i), i as f64)]);
                ("s".repeat(i * 9), vec![i as i64 * 1_000_000; i], map)
            })
            .collect();
        let encoded = Value::encoded(Value::new(nested.clone()).encode().unwrap());
        let items = encoded.items().unwrap();
        assert_eq!(items.len(), 40);
        for (item, expected) in items.iter().zip(&nested) {
            assert_eq!(&item.to::<(String, Vec<i64>, BTreeMap<String, f64>)>().unwrap(), expected);
        }
    }

    #[test]
    fn arguments_follow_the_packing_convention() {
        let args = vec![Value::new(2i32), Value::new(String::from("x"))];
        assert_eq!(from_args::<(i32, String)>(args.clone()).unwrap(), (2, "x".into()));
        assert_eq!(from_args::<i32>(vec![Value::new(5i32)]).unwrap(), 5);
        from_args::<()>(vec![]).unwrap();
        let packed = Value::encoded(encode_args(&args).unwrap());
        assert_eq!(packed.to::<(i32, String)>().unwrap(), (2, "x".into()));
        assert!(Value::encoded(encode_args(&[]).unwrap()).to::<()>().is_ok());
    }

    #[test]
    fn tuples_split_into_their_fields() {
        let fields = Value::new((1i32, doc("x"))).items().unwrap();
        assert_eq!(fields[0].to::<i32>().unwrap(), 1);
        assert_eq!(fields[1].to::<Doc>().unwrap(), doc("x"));
    }
}
