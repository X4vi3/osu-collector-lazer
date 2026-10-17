//! osu!stable `collection.db` reader/writer.
//!
//! Layout (little-endian): `i32 version`, `i32 count`, then per collection
//! `osu-string name`, `i32 n`, `n × osu-string md5`.
//! An osu-string is `0x00` (null) or `0x0b` + ULEB128 length + UTF-8 bytes.

use std::io::{self, Read, Write};

pub const DB_VERSION: i32 = 20250101;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Collection {
    pub name: String,
    pub hashes: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("{p}: {0}", p = crate::core::i18n::tr("ошибка чтения", "read error"))]
    Io(#[from] io::Error),
    #[error("{p}: {0}", p = crate::core::i18n::tr("повреждённый collection.db", "corrupt collection.db"))]
    Corrupt(&'static str),
}

fn write_uleb128<W: Write>(w: &mut W, mut n: u64) -> io::Result<()> {
    loop {
        let mut byte = (n & 0x7f) as u8;
        n >>= 7;
        if n != 0 {
            byte |= 0x80;
        }
        w.write_all(&[byte])?;
        if n == 0 {
            return Ok(());
        }
    }
}

fn read_uleb128<R: Read>(r: &mut R) -> Result<u64, DbError> {
    let mut result = 0u64;
    let mut shift = 0;
    loop {
        let mut b = [0u8];
        r.read_exact(&mut b)?;
        if shift >= 64 {
            return Err(DbError::Corrupt("ULEB128 too long"));
        }
        result |= ((b[0] & 0x7f) as u64) << shift;
        if b[0] & 0x80 == 0 {
            return Ok(result);
        }
        shift += 7;
    }
}

fn write_string<W: Write>(w: &mut W, s: &str) -> io::Result<()> {
    w.write_all(&[0x0b])?;
    write_uleb128(w, s.len() as u64)?;
    w.write_all(s.as_bytes())
}

fn read_string<R: Read>(r: &mut R) -> Result<Option<String>, DbError> {
    let mut tag = [0u8];
    r.read_exact(&mut tag)?;
    match tag[0] {
        0x00 => Ok(None),
        0x0b => {
            let len = read_uleb128(r)? as usize;
            if len > 1 << 20 {
                return Err(DbError::Corrupt("string too long"));
            }
            let mut buf = vec![0u8; len];
            r.read_exact(&mut buf)?;
            String::from_utf8(buf)
                .map(Some)
                .map_err(|_| DbError::Corrupt("string is not UTF-8"))
        }
        _ => Err(DbError::Corrupt("unknown string tag")),
    }
}

fn read_i32<R: Read>(r: &mut R) -> Result<i32, DbError> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(i32::from_le_bytes(b))
}

pub fn write<W: Write>(w: &mut W, collections: &[Collection]) -> io::Result<()> {
    w.write_all(&DB_VERSION.to_le_bytes())?;
    w.write_all(&(collections.len() as i32).to_le_bytes())?;
    for c in collections {
        write_string(w, &c.name)?;
        w.write_all(&(c.hashes.len() as i32).to_le_bytes())?;
        for h in &c.hashes {
            write_string(w, h)?;
        }
    }
    Ok(())
}

pub fn to_bytes(collections: &[Collection]) -> Vec<u8> {
    let mut buf = Vec::new();
    write(&mut buf, collections).expect("writing to Vec never fails");
    buf
}

pub fn read<R: Read>(r: &mut R) -> Result<Vec<Collection>, DbError> {
    let _version = read_i32(r)?;
    let count = read_i32(r)?;
    if count < 0 {
        return Err(DbError::Corrupt("negative collection count"));
    }
    let mut out = Vec::with_capacity(count.min(1024) as usize);
    for _ in 0..count {
        let name = read_string(r)?.unwrap_or_default();
        let n = read_i32(r)?;
        if n < 0 {
            return Err(DbError::Corrupt("negative beatmap count"));
        }
        let mut hashes = Vec::with_capacity(n.min(1 << 16) as usize);
        for _ in 0..n {
            if let Some(h) = read_string(r)? {
                hashes.push(h);
            }
        }
        out.push(Collection { name, hashes });
    }
    Ok(out)
}

pub fn from_bytes(bytes: &[u8]) -> Result<Vec<Collection>, DbError> {
    read(&mut io::Cursor::new(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let cols = vec![
            Collection {
                name: "Тест ★ коллекция".into(),
                hashes: vec![
                    "c3f3d6ba82ce48dfe2bfb049a184582c".into(),
                    "8d2c3b7dfe5192649ac84a2febc7b8b0".into(),
                ],
            },
            Collection {
                name: "empty".into(),
                hashes: vec![],
            },
        ];
        let bytes = to_bytes(&cols);
        assert_eq!(from_bytes(&bytes).unwrap(), cols);
    }

    #[test]
    fn long_name_uses_multibyte_uleb() {
        let name = "x".repeat(300);
        let bytes = to_bytes(&[Collection {
            name: name.clone(),
            hashes: vec![],
        }]);
        // 8 header bytes, then 0x0b, then 300 = 0xAC 0x02
        assert_eq!(&bytes[8..11], &[0x0b, 0xac, 0x02]);
        assert_eq!(from_bytes(&bytes).unwrap()[0].name, name);
    }

    #[test]
    fn exact_bytes_match_reference_layout() {
        let bytes = to_bytes(&[Collection {
            name: "ab".into(),
            hashes: vec!["h".into()],
        }]);
        let mut expected = Vec::new();
        expected.extend_from_slice(&DB_VERSION.to_le_bytes());
        expected.extend_from_slice(&1i32.to_le_bytes());
        expected.extend_from_slice(&[0x0b, 2, b'a', b'b']);
        expected.extend_from_slice(&1i32.to_le_bytes());
        expected.extend_from_slice(&[0x0b, 1, b'h']);
        assert_eq!(bytes, expected);
    }

    #[test]
    fn reads_null_strings() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&20140609i32.to_le_bytes());
        bytes.extend_from_slice(&1i32.to_le_bytes());
        bytes.push(0x00); // null name
        bytes.extend_from_slice(&2i32.to_le_bytes());
        bytes.push(0x00); // null hash, skipped
        bytes.extend_from_slice(&[0x0b, 1, b'z']);
        let cols = from_bytes(&bytes).unwrap();
        assert_eq!(
            cols,
            vec![Collection {
                name: String::new(),
                hashes: vec!["z".into()]
            }]
        );
    }

    #[test]
    fn truncated_file_is_error() {
        let bytes = to_bytes(&[Collection {
            name: "abc".into(),
            hashes: vec!["x".into()],
        }]);
        assert!(from_bytes(&bytes[..bytes.len() - 1]).is_err());
    }

    /// Real collection.db files exported by osu!collector / osu!stable, if present in tests/fixtures.
    #[test]
    fn reads_real_fixture_files() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "db") {
                let bytes = std::fs::read(&p).unwrap();
                let cols =
                    from_bytes(&bytes).unwrap_or_else(|err| panic!("{}: {err}", p.display()));
                assert!(!cols.is_empty(), "{}", p.display());
                for c in &cols {
                    for h in &c.hashes {
                        assert_eq!(h.len(), 32, "{} in {}", h, p.display());
                    }
                }
                // re-serialising must reproduce the same collections
                assert_eq!(from_bytes(&to_bytes(&cols)).unwrap(), cols);
            }
        }
    }
}
