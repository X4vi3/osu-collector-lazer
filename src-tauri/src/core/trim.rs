//! Removing unwanted difficulties from an `.osz` without touching the remaining bytes.
//!
//! Kept entries are copied raw (no recompression), so every remaining `.osu`
//! keeps exactly the MD5 lazer uses to match collections.

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read};
use std::path::Path;

use md5::{Digest, Md5};
use zip::write::SimpleFileOptions;

#[derive(Debug, thiserror::Error)]
pub enum TrimError {
    #[error("{p}: {0}", p = crate::core::i18n::tr("архив повреждён", "archive is corrupt"))]
    BadArchive(String),
    #[error(
        "{}",
        crate::core::i18n::tr(
            "в архиве нет ни одной сложности (.osu)",
            "the archive has no difficulties (.osu)"
        )
    )]
    NoOsu,
    #[error(
        "{}",
        crate::core::i18n::tr(
            "версия карты на зеркале отличается: ни одна нужная сложность не найдена",
            "the mirror has a different version of this map: none of the wanted difficulties are in it"
        )
    )]
    VersionMismatch,
    #[error("{p}: {0}", p = crate::core::i18n::tr("ошибка записи", "write error"))]
    Io(#[from] std::io::Error),
}

impl From<zip::result::ZipError> for TrimError {
    fn from(e: zip::result::ZipError) -> Self {
        match e {
            zip::result::ZipError::Io(io) => TrimError::Io(io),
            other => TrimError::BadArchive(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrimOutcome {
    /// MD5 of every `.osu` written to the output
    pub kept: Vec<String>,
    /// Number of `.osu` entries removed
    pub dropped: usize,
    /// No wanted hash was present, so the whole set was written
    pub mismatch: bool,
}

const VIDEO_EXT: &[&str] = &[
    "mp4", "avi", "flv", "mkv", "webm", "wmv", "m4v", "mpg", "mpeg", "mov",
];

fn ext_lower(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

pub fn md5_hex(bytes: &[u8]) -> String {
    let digest = Md5::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Lists `(entry index, md5)` of every `.osu` in the archive. Errors if it is not a zip or has no `.osu`.
pub fn osu_hashes(path: &Path) -> Result<Vec<(usize, String)>, TrimError> {
    let mut archive = zip::ZipArchive::new(BufReader::new(File::open(path)?))?;
    let mut out = Vec::new();
    for i in 0..archive.len() {
        let mut f = archive.by_index(i)?;
        if f.is_dir() || ext_lower(&f.name()?) != "osu" {
            continue;
        }
        let mut buf = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut buf)?;
        out.push((i, md5_hex(&buf)));
    }
    if out.is_empty() {
        return Err(TrimError::NoOsu);
    }
    Ok(out)
}

/// Writes `output` containing every non-video entry of `input`, but only the `.osu` files whose
/// MD5 is in `wanted`. `wanted = None` keeps all difficulties.
/// If none of `wanted` is present: writes the full set when `full_on_mismatch`, else errors.
pub fn trim_osz(
    input: &Path,
    output: &Path,
    wanted: Option<&HashSet<String>>,
    full_on_mismatch: bool,
) -> Result<TrimOutcome, TrimError> {
    let hashes = osu_hashes(input)?;
    let mut mismatch = false;
    let keep_idx: HashSet<usize> = match wanted {
        None => hashes.iter().map(|(i, _)| *i).collect(),
        Some(w) => {
            let matched: HashSet<usize> = hashes
                .iter()
                .filter(|(_, h)| w.contains(h))
                .map(|(i, _)| *i)
                .collect();
            if matched.is_empty() {
                if !full_on_mismatch {
                    return Err(TrimError::VersionMismatch);
                }
                mismatch = true;
                hashes.iter().map(|(i, _)| *i).collect()
            } else {
                matched
            }
        }
    };
    let osu_idx: HashSet<usize> = hashes.iter().map(|(i, _)| *i).collect();

    let mut archive = zip::ZipArchive::new(BufReader::new(File::open(input)?))?;
    let tmp = output.with_extension("osz.part");
    {
        let mut writer = zip::ZipWriter::new(BufWriter::new(File::create(&tmp)?));
        for i in 0..archive.len() {
            if osu_idx.contains(&i) && !keep_idx.contains(&i) {
                continue;
            }
            let entry = archive.by_index_raw(i)?;
            let name = entry.name()?.to_string();
            if VIDEO_EXT.contains(&ext_lower(&name).as_str()) {
                continue;
            }
            if writer.raw_copy_file(entry).is_err() {
                // unusual headers: fall back to recompressing; content bytes (and so MD5) stay identical
                let _ = writer.abort_file();
                let mut f = archive.by_index(i)?;
                if f.is_dir() {
                    writer.add_directory(name, SimpleFileOptions::default())?;
                    continue;
                }
                writer.start_file(
                    name,
                    SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Deflated),
                )?;
                std::io::copy(&mut f, &mut writer)?;
            }
        }
        writer.finish()?;
    }
    std::fs::rename(&tmp, output)?;

    let kept = hashes
        .iter()
        .filter(|(i, _)| keep_idx.contains(i))
        .map(|(_, h)| h.clone())
        .collect();
    Ok(TrimOutcome {
        kept,
        dropped: osu_idx.len() - keep_idx.len(),
        mismatch,
    })
}

/// `{setId} {artist} - {title}.osz`, safe for Windows and kept reasonably short.
pub fn osz_file_name(set_id: u64, artist: &str, title: &str) -> String {
    let raw = format!("{set_id} {artist} - {title}");
    let mut s: String = raw
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    if s.chars().count() > 120 {
        s = s.chars().take(120).collect();
    }
    let s = s.trim_end_matches(['.', ' ']).to_string();
    format!("{s}.osz")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_osz(path: &Path, files: &[(&str, &[u8])]) {
        let mut w = zip::ZipWriter::new(File::create(path).unwrap());
        for (name, data) in files {
            w.start_file(*name, SimpleFileOptions::default()).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }

    fn entries(path: &Path) -> Vec<(String, Vec<u8>)> {
        let mut a = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
        (0..a.len())
            .map(|i| {
                let mut f = a.by_index(i).unwrap();
                let mut b = Vec::new();
                f.read_to_end(&mut b).unwrap();
                (f.name().unwrap().to_string(), b)
            })
            .collect()
    }

    const EASY: &[u8] = b"osu file format v14\r\n[Metadata]\r\nVersion:Easy\r\n";
    const HARD: &[u8] = b"osu file format v14\r\n[Metadata]\r\nVersion:Hard\r\n";
    const INSANE: &[u8] = b"osu file format v14\r\n[Metadata]\r\nVersion:Insane\r\n";

    fn sample(dir: &Path) -> std::path::PathBuf {
        let p = dir.join("in.osz");
        make_osz(
            &p,
            &[
                ("a - b (m) [Easy].osu", EASY),
                ("a - b (m) [Hard].osu", HARD),
                ("a - b (m) [Insane].OSU", INSANE),
                ("audio.mp3", b"ID3fakeaudio"),
                ("bg.jpg", b"\xff\xd8fakejpg"),
                ("sb.osb", b"[Events]"),
                ("video.mp4", b"fakevideo"),
            ],
        );
        p
    }

    #[test]
    fn keeps_only_wanted_and_preserves_md5() {
        let dir = tempfile::tempdir().unwrap();
        let input = sample(dir.path());
        let out = dir.path().join("out.osz");
        let wanted: HashSet<String> = [
            md5_hex(HARD),
            md5_hex(INSANE),
            "ffffffffffffffffffffffffffffffff".into(),
        ]
        .into();

        let r = trim_osz(&input, &out, Some(&wanted), true).unwrap();
        assert_eq!(r.dropped, 1);
        assert!(!r.mismatch);
        assert_eq!(r.kept.len(), 2);

        let e = entries(&out);
        let names: Vec<&str> = e.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            [
                "a - b (m) [Hard].osu",
                "a - b (m) [Insane].OSU",
                "audio.mp3",
                "bg.jpg",
                "sb.osb"
            ]
        );
        // remaining difficulties are byte-identical
        let out_hashes: HashSet<String> = osu_hashes(&out)
            .unwrap()
            .into_iter()
            .map(|(_, h)| h)
            .collect();
        assert_eq!(out_hashes, [md5_hex(HARD), md5_hex(INSANE)].into());
        assert_eq!(e[2].1, b"ID3fakeaudio");
    }

    #[test]
    fn mismatch_writes_full_set_or_errors() {
        let dir = tempfile::tempdir().unwrap();
        let input = sample(dir.path());
        let out = dir.path().join("out.osz");
        let wanted: HashSet<String> = ["00000000000000000000000000000000".to_string()].into();

        let r = trim_osz(&input, &out, Some(&wanted), true).unwrap();
        assert!(r.mismatch);
        assert_eq!(r.kept.len(), 3);
        assert_eq!(osu_hashes(&out).unwrap().len(), 3);

        let out2 = dir.path().join("out2.osz");
        assert!(matches!(
            trim_osz(&input, &out2, Some(&wanted), false),
            Err(TrimError::VersionMismatch)
        ));
        assert!(!out2.exists());
    }

    #[test]
    fn no_filter_keeps_all_difficulties_but_drops_video() {
        let dir = tempfile::tempdir().unwrap();
        let input = sample(dir.path());
        let out = dir.path().join("out.osz");
        let r = trim_osz(&input, &out, None, true).unwrap();
        assert_eq!((r.kept.len(), r.dropped), (3, 0));
        assert!(entries(&out).iter().all(|(n, _)| n != "video.mp4"));
    }

    #[test]
    fn rejects_non_zip_and_empty_sets() {
        let dir = tempfile::tempdir().unwrap();
        let junk = dir.path().join("junk.osz");
        std::fs::write(&junk, b"<html>rate limited</html>").unwrap();
        assert!(matches!(osu_hashes(&junk), Err(TrimError::BadArchive(_))));

        let empty = dir.path().join("empty.osz");
        make_osz(&empty, &[("audio.mp3", b"x")]);
        assert!(matches!(osu_hashes(&empty), Err(TrimError::NoOsu)));
    }

    #[test]
    fn file_names_are_windows_safe() {
        assert_eq!(
            osz_file_name(1, "A/B", "What?: <x>"),
            "1 A_B - What__ _x_.osz"
        );
        assert_eq!(osz_file_name(2, "a", "dots..."), "2 a - dots.osz");
        let long = osz_file_name(3, &"x".repeat(300), "t");
        assert!(long.chars().count() <= 124);
        assert_eq!(
            osz_file_name(4, "ゆよゆっぺ", "-ELIS-"),
            "4 ゆよゆっぺ - -ELIS-.osz"
        );
    }

    #[test]
    #[allow(deprecated)]
    fn copies_entries_with_unix_extra_fields() {
        // Info-ZIP unix extra fields (0x7875 / 0x5455) are common in old .osz files
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("unix.osz");
        {
            let mut w = zip::ZipWriter::new(File::create(&input).unwrap());
            let mut opts = zip::write::FullFileOptions::default();
            opts.add_extra_data(
                0x7875,
                vec![1, 4, 0xe8, 3, 0, 0, 4, 0xe8, 3, 0, 0].into_boxed_slice(),
                false,
            )
            .unwrap();
            for (name, data) in [
                ("a [Hard].osu", HARD),
                ("a [Easy].osu", EASY),
                ("audio.mp3", b"ID3x".as_slice()),
            ] {
                w.start_file(name, opts.clone()).unwrap();
                w.write_all(data).unwrap();
            }
            w.finish().unwrap();
        }
        let out = dir.path().join("out.osz");
        let wanted: HashSet<String> = [md5_hex(HARD)].into();
        let r = trim_osz(&input, &out, Some(&wanted), true).unwrap();
        assert_eq!(r.kept, vec![md5_hex(HARD)]);
        let names: Vec<String> = entries(&out).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["a [Hard].osu", "audio.mp3"]);
    }
}
