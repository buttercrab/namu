//! Reading into MessagePack without decoding it: where a value starts and ends, a struct's field
//! and a list's element as ranges of its bytes. Values cross every boundary as MessagePack (a
//! struct as a map of its fields, a tuple or a list as an array), so built-ins that take them apart
//! need only find the bytes of a part.

use std::fmt::Write;
use std::ops::Range;

use anyhow::{Context, bail};
use bytes::Bytes;

/// What a value's header says it is.
enum Head {
    /// A value of `len` bytes after the header: a number, a string, binary, an extension.
    Bytes(usize),
    Array(usize),
    Map(usize),
}

/// The header of the value at `at`: what it is, and where its contents start.
fn head(bytes: &[u8], at: usize) -> anyhow::Result<(Head, usize)> {
    let marker = *bytes.get(at).context("MessagePack ends early")?;
    let length = |size: usize| -> anyhow::Result<(usize, usize)> {
        let raw = bytes
            .get(at + 1..at + 1 + size)
            .context("MessagePack ends early")?;
        let len = raw
            .iter()
            .fold(0usize, |len, &byte| len << 8 | byte as usize);
        Ok((len, at + 1 + size))
    };
    let fixed = |len: usize| Ok((Head::Bytes(len), at + 1));
    match marker {
        0x00..=0x7f | 0xe0..=0xff | 0xc0 | 0xc2 | 0xc3 => fixed(0),
        0x80..=0x8f => Ok((Head::Map((marker & 0x0f) as usize), at + 1)),
        0x90..=0x9f => Ok((Head::Array((marker & 0x0f) as usize), at + 1)),
        0xa0..=0xbf => fixed((marker & 0x1f) as usize),
        0xcc | 0xd0 => fixed(1),
        0xcd | 0xd1 => fixed(2),
        0xca | 0xce | 0xd2 => fixed(4),
        0xcb | 0xcf | 0xd3 => fixed(8),
        // Fixed extensions: a type byte, then 1, 2, 4, 8 or 16 bytes.
        0xd4..=0xd8 => fixed(1 + (1 << (marker - 0xd4))),
        // Strings and binary, then extensions (with their type byte), by the size of their length.
        0xd9 | 0xc4 => length(1).map(|(len, start)| (Head::Bytes(len), start)),
        0xda | 0xc5 => length(2).map(|(len, start)| (Head::Bytes(len), start)),
        0xdb | 0xc6 => length(4).map(|(len, start)| (Head::Bytes(len), start)),
        0xc7 => length(1).map(|(len, start)| (Head::Bytes(len + 1), start)),
        0xc8 => length(2).map(|(len, start)| (Head::Bytes(len + 1), start)),
        0xc9 => length(4).map(|(len, start)| (Head::Bytes(len + 1), start)),
        0xdc => length(2).map(|(len, start)| (Head::Array(len), start)),
        0xdd => length(4).map(|(len, start)| (Head::Array(len), start)),
        0xde => length(2).map(|(len, start)| (Head::Map(len), start)),
        0xdf => length(4).map(|(len, start)| (Head::Map(len), start)),
        0xc1 => bail!("not MessagePack (0xc1)"),
    }
}

/// Where the value at `at` ends.
fn end(bytes: &[u8], at: usize) -> anyhow::Result<usize> {
    // Values still to step over: nested arrays and maps add theirs.
    let (mut at, mut pending) = (at, 1usize);
    while pending > 0 {
        pending -= 1;
        let (head, start) = head(bytes, at)?;
        at = match head {
            Head::Bytes(len) => start + len,
            Head::Array(len) => {
                pending += len;
                start
            }
            Head::Map(len) => {
                pending += 2 * len;
                start
            }
        };
    }
    anyhow::ensure!(at <= bytes.len(), "MessagePack ends early");
    Ok(at)
}

/// Where the field `name` of the struct in `bytes` is: a map's entry, by key, or a tuple's
/// element, by position. `None` if the struct has no such field.
pub fn field(bytes: &[u8], name: &str) -> anyhow::Result<Option<Range<usize>>> {
    match head(bytes, 0)? {
        (Head::Map(len), mut at) => {
            for _ in 0..len {
                let value = end(bytes, at)?;
                if key(bytes, at)? == Some(name) {
                    return Ok(Some(value..end(bytes, value)?));
                }
                at = end(bytes, value)?;
            }
            Ok(None)
        }
        (Head::Array(_), _) => match name.parse() {
            Ok(index) => element(bytes, index).map(Some),
            Err(_) => Ok(None),
        },
        _ => bail!("a field of something that is not a struct or a tuple"),
    }
}

/// The string at `at`, if it is one.
fn key(bytes: &[u8], at: usize) -> anyhow::Result<Option<&str>> {
    let marker = bytes[at];
    if !matches!(marker, 0xa0..=0xbf | 0xd9..=0xdb) {
        return Ok(None);
    }
    let (Head::Bytes(len), start) = head(bytes, at)? else {
        unreachable!("a string's header")
    };
    let raw = bytes
        .get(start..start + len)
        .context("MessagePack ends early")?;
    Ok(std::str::from_utf8(raw).ok())
}

/// Where element `index` of the list or tuple in `bytes` is.
pub fn element(bytes: &[u8], index: usize) -> anyhow::Result<Range<usize>> {
    let (Head::Array(len), mut at) = head(bytes, 0)? else {
        bail!("an element of something that is not a list or a tuple")
    };
    anyhow::ensure!(
        index < len,
        "index {index} is out of bounds: the length is {len}"
    );
    for _ in 0..index {
        at = end(bytes, at)?;
    }
    Ok(at..end(bytes, at)?)
}

/// An array of `parts`, each already MessagePack.
pub fn array(parts: &[Bytes]) -> Bytes {
    let len = parts.len();
    let mut out = Vec::with_capacity(5 + parts.iter().map(Bytes::len).sum::<usize>());
    match len {
        0..=15 => out.push(0x90 | len as u8),
        16..=0xffff => {
            out.push(0xdc);
            out.extend_from_slice(&(len as u16).to_be_bytes());
        }
        _ => {
            out.push(0xdd);
            out.extend_from_slice(&(len as u32).to_be_bytes());
        }
    }
    for part in parts {
        out.extend_from_slice(part);
    }
    out.into()
}

/// Writes the scalar in `bytes` (a number, a string or a bool) to `out` as Rust's `Display`
/// writes it.
pub fn display(bytes: &[u8], out: &mut String) -> anyhow::Result<()> {
    let marker = *bytes.first().context("an empty value")?;
    let be = |n: usize| -> anyhow::Result<[u8; 8]> {
        let raw = bytes.get(1..1 + n).context("MessagePack ends early")?;
        let mut word = [0; 8];
        word[8 - n..].copy_from_slice(raw);
        Ok(word)
    };
    let signed = |word: [u8; 8], n: usize| i64::from_be_bytes(word) << (64 - 8 * n) >> (64 - 8 * n);
    let _ = match marker {
        0x00..=0x7f => write!(out, "{marker}"),
        0xe0..=0xff => write!(out, "{}", marker as i8),
        0xc2 => write!(out, "false"),
        0xc3 => write!(out, "true"),
        0xcc..=0xcf => {
            let n = 1 << (marker - 0xcc);
            write!(out, "{}", u64::from_be_bytes(be(n)?))
        }
        0xd0..=0xd3 => {
            let n = 1 << (marker - 0xd0);
            write!(out, "{}", signed(be(n)?, n))
        }
        0xca => write!(out, "{}", f32::from_bits(u64::from_be_bytes(be(4)?) as u32)),
        0xcb => write!(out, "{}", f64::from_bits(u64::from_be_bytes(be(8)?))),
        _ => match key(bytes, 0)? {
            Some(text) => write!(out, "{text}"),
            None => bail!("only numbers, strings and bools are formatted"),
        },
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde::Serialize;

    use super::*;

    #[derive(Serialize)]
    struct Doc {
        id: u64,
        tags: Vec<String>,
        nested: Inner,
        title: String,
    }

    #[derive(Serialize)]
    struct Inner {
        score: f64,
        pair: (i32, String),
    }

    fn pack(value: &impl Serialize) -> Vec<u8> {
        rmp_serde::to_vec_named(value).unwrap()
    }

    fn part<T: serde::de::DeserializeOwned>(bytes: &[u8], at: Range<usize>) -> T {
        rmp_serde::from_slice(&bytes[at]).unwrap()
    }

    #[test]
    fn fields_and_elements_are_found_without_decoding() {
        let doc = Doc {
            id: 70_000,
            tags: (0..20).map(|i| format!("t{i}")).collect(),
            nested: Inner {
                score: 0.5,
                pair: (-3, "x".repeat(300)),
            },
            title: "naïve".into(),
        };
        let bytes = pack(&doc);
        let at = |name| field(&bytes, name).unwrap().unwrap();
        assert_eq!(part::<u64>(&bytes, at("id")), 70_000);
        assert_eq!(part::<String>(&bytes, at("title")), "naïve");
        let tags = &bytes[at("tags")];
        assert_eq!(part::<String>(tags, element(tags, 17).unwrap()), "t17");
        let nested = &bytes[at("nested")];
        let pair = &nested[field(nested, "pair").unwrap().unwrap()];
        assert_eq!(part::<i32>(pair, field(pair, "0").unwrap().unwrap()), -3);
        assert_eq!(part::<String>(pair, element(pair, 1).unwrap()).len(), 300);

        assert!(field(&bytes, "missing").unwrap().is_none());
        assert!(
            element(tags, 20)
                .unwrap_err()
                .to_string()
                .contains("out of bounds")
        );
        assert!(field(&pack(&5), "x").is_err());
    }

    #[test]
    fn arrays_are_put_together_from_their_parts() {
        let parts: Vec<Bytes> = (0..17).map(|i| pack(&i).into()).collect();
        let list: Vec<i32> = rmp_serde::from_slice(&array(&parts)).unwrap();
        assert_eq!(list, (0..17).collect::<Vec<_>>());
        let pair: (bool, String) =
            rmp_serde::from_slice(&array(&[pack(&true).into(), pack(&"a").into()])).unwrap();
        assert_eq!(pair, (true, "a".to_string()));
    }

    #[test]
    fn scalars_are_written_as_display_writes_them() {
        let shown = |bytes: Vec<u8>| {
            let mut out = String::new();
            display(&bytes, &mut out).unwrap();
            out
        };
        for value in [
            0i64,
            5,
            127,
            128,
            255,
            70_000,
            -1,
            -32,
            -33,
            -129,
            -70_000,
            i64::MIN,
        ] {
            assert_eq!(shown(pack(&value)), value.to_string());
        }
        assert_eq!(shown(pack(&u64::MAX)), u64::MAX.to_string());
        for value in [1.0f64, 0.1, -2.5e300] {
            assert_eq!(shown(pack(&value)), value.to_string());
        }
        assert_eq!(shown(pack(&1.5f32)), "1.5");
        assert_eq!(shown(pack(&true)), "true");
        assert_eq!(shown(pack(&"naïve")), "naïve");
        let mut out = String::new();
        assert!(display(&pack(&vec![1]), &mut out).is_err());
    }
}
