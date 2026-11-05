//! The subset of the Komikku/Tachiyomi backup schema Dewey understands.
//!
//! Field numbers come from the upstream `@ProtoNumber` annotations in
//! `app/src/main/java/eu/kanade/tachiyomi/data/backup/models/*.kt`
//! (komikku-app/komikku). Every field we do not model is skipped, so a newer
//! Komikku can still export a backup this importer reads.

use anyhow::{bail, Context, Result};
use std::io::Read;

use super::proto::Reader;

/// A decoded `Backup` message.
#[derive(Debug, Default, Clone)]
pub struct Backup {
    pub categories: Vec<BackupCategory>,
    pub sources: Vec<BackupSource>,
    pub mangas: Vec<BackupManga>,
}

/// `BackupSource`: 1 name, 2 source id.
#[derive(Debug, Default, Clone)]
pub struct BackupSource {
    pub id: i64,
    pub name: String,
}

/// `BackupCategory`: 1 name, 2 order, 3 id, 100 flags, 900 hidden.
#[derive(Debug, Default, Clone)]
pub struct BackupCategory {
    pub id: i64,
    pub name: String,
    pub hidden: bool,
}

/// `BackupManga`: 1 source, 2 url, 3 title, 4 artist, 5 author, 6 description,
/// 7 genre, 8 status, 9 thumbnailUrl, 13 dateAdded, 14 viewer, 16 chapters,
/// 17 categories, 18 tracking, 100 favorite, 104 history, 110 notes.
#[derive(Debug, Clone)]
pub struct BackupManga {
    pub source: i64,
    pub url: String,
    pub title: String,
    pub artist: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub genre: Vec<String>,
    pub status: i32,
    pub thumbnail_url: Option<String>,
    pub viewer: i32,
    pub category_ids: Vec<i64>,
    /// Proto default is `true`; only un-favorited entries encode the field.
    pub favorite: bool,
    pub chapters: Vec<BackupChapter>,
    pub history: Vec<BackupHistory>,
}

/// `BackupChapter`: 1 url, 2 name, 3 scanlator, 4 read, 5 bookmark,
/// 6 lastPageRead, 7 dateFetch, 8 dateUpload, 9 chapterNumber, 10 sourceOrder.
#[derive(Debug, Default, Clone)]
pub struct BackupChapter {
    pub url: String,
    pub name: String,
    pub scanlator: Option<String>,
    pub read: bool,
    pub bookmark: bool,
    pub last_page_read: i64,
    pub chapter_number: f32,
    pub source_order: i64,
}

/// `BackupHistory`: 1 url, 2 lastRead (epoch millis), 3 readDuration.
#[derive(Debug, Default, Clone)]
pub struct BackupHistory {
    pub url: String,
    pub last_read: i64,
}

impl Backup {
    /// Reads a `.tachibk` file: zlib-compressed protobuf, with gzip and raw
    /// protobuf accepted as fallbacks so a hand-unpacked file still imports.
    pub fn from_file(path: &std::path::Path) -> Result<Self> {
        let raw = std::fs::read(path)
            .with_context(|| format!("reading backup file {}", path.display()))?;
        if raw.is_empty() {
            bail!("backup file {} is empty", path.display());
        }

        for (label, decompressed) in [
            ("zlib", decompress_zlib(&raw)),
            ("gzip", decompress_gzip(&raw)),
            ("raw", Ok(raw.clone())),
        ] {
            if let Ok(bytes) = decompressed {
                if let Ok(backup) = Self::decode(&bytes) {
                    return Ok(backup);
                }
                tracing::debug!("backup did not decode as protobuf via {label}");
            }
        }

        bail!(
            "{} is not a readable Komikku backup (tried zlib, gzip and raw protobuf)",
            path.display()
        )
    }

    pub fn decode(buf: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(buf);
        let mut backup = Backup::default();

        while !reader.is_empty() {
            let (field, wire) = reader.take_tag()?;
            match (field, wire) {
                (1, 2) => backup
                    .mangas
                    .push(BackupManga::decode(reader.take_bytes()?)?),
                (2, 2) => backup
                    .categories
                    .push(BackupCategory::decode(reader.take_bytes()?)?),
                (101, 2) => backup
                    .sources
                    .push(BackupSource::decode(reader.take_bytes()?)?),
                _ => reader.skip(wire, field)?,
            }
        }

        Ok(backup)
    }
}

impl BackupCategory {
    fn decode(buf: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(buf);
        let mut out = BackupCategory::default();

        while !reader.is_empty() {
            let (field, wire) = reader.take_tag()?;
            match (field, wire) {
                (1, 2) => out.name = reader.take_string()?,
                (3, 0) => out.id = reader.take_varint()? as i64,
                (900, 0) => out.hidden = reader.take_bool()?,
                _ => reader.skip(wire, field)?,
            }
        }

        Ok(out)
    }
}

impl BackupSource {
    fn decode(buf: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(buf);
        let mut out = BackupSource::default();

        while !reader.is_empty() {
            let (field, wire) = reader.take_tag()?;
            match (field, wire) {
                (1, 2) => out.name = reader.take_string()?,
                (2, 0) => out.id = reader.take_varint()? as i64,
                _ => reader.skip(wire, field)?,
            }
        }

        Ok(out)
    }
}

impl BackupManga {
    fn decode(buf: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(buf);
        let mut out = BackupManga {
            favorite: true,
            source: 0,
            url: String::new(),
            title: String::new(),
            artist: None,
            author: None,
            description: None,
            genre: Vec::new(),
            status: 0,
            thumbnail_url: None,
            viewer: 0,
            category_ids: Vec::new(),
            chapters: Vec::new(),
            history: Vec::new(),
        };

        while !reader.is_empty() {
            let (field, wire) = reader.take_tag()?;
            match (field, wire) {
                (1, 0) => out.source = reader.take_varint()? as i64,
                (2, 2) => out.url = reader.take_string()?,
                (3, 2) => out.title = reader.take_string()?,
                (4, 2) => out.artist = Some(reader.take_string()?),
                (5, 2) => out.author = Some(reader.take_string()?),
                (6, 2) => out.description = Some(reader.take_string()?),
                (7, 2) => out.genre.push(reader.take_string()?),
                (8, 0) => out.status = reader.take_varint()? as i32,
                (9, 2) => out.thumbnail_url = Some(reader.take_string()?),
                (14, 0) => out.viewer = reader.take_varint()? as i32,
                (16, 2) => out
                    .chapters
                    .push(BackupChapter::decode(reader.take_bytes()?)?),
                (17, 2) => out.category_ids.extend(reader.take_packed_i64()?),
                (17, 0) => out.category_ids.push(reader.take_varint()? as i64),
                (100, 0) => out.favorite = reader.take_bool()?,
                (104, 2) => out
                    .history
                    .push(BackupHistory::decode(reader.take_bytes()?)?),
                _ => reader.skip(wire, field)?,
            }
        }

        Ok(out)
    }
}

impl BackupChapter {
    fn decode(buf: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(buf);
        let mut out = BackupChapter::default();

        while !reader.is_empty() {
            let (field, wire) = reader.take_tag()?;
            match (field, wire) {
                (1, 2) => out.url = reader.take_string()?,
                (2, 2) => out.name = reader.take_string()?,
                (3, 2) => out.scanlator = Some(reader.take_string()?),
                (4, 0) => out.read = reader.take_bool()?,
                (5, 0) => out.bookmark = reader.take_bool()?,
                (6, 0) => out.last_page_read = reader.take_varint()? as i64,
                (9, 5) => out.chapter_number = reader.take_f32()?,
                (10, 0) => out.source_order = reader.take_varint()? as i64,
                _ => reader.skip(wire, field)?,
            }
        }

        Ok(out)
    }
}

impl BackupHistory {
    fn decode(buf: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(buf);
        let mut out = BackupHistory::default();

        while !reader.is_empty() {
            let (field, wire) = reader.take_tag()?;
            match (field, wire) {
                (1, 2) => out.url = reader.take_string()?,
                (2, 0) => out.last_read = reader.take_varint()? as i64,
                _ => reader.skip(wire, field)?,
            }
        }

        Ok(out)
    }
}

fn decompress_zlib(raw: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(raw)
        .read_to_end(&mut out)
        .context("zlib")?;
    Ok(out)
}

fn decompress_gzip(raw: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(raw)
        .read_to_end(&mut out)
        .context("gzip")?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal backup: one category, one favorite manga with two
    /// chapters (one read+bookmarked) and one history entry.
    fn fixture() -> Vec<u8> {
        fn varint(mut v: u64) -> Vec<u8> {
            let mut out = Vec::new();
            loop {
                let mut byte = (v & 0x7f) as u8;
                v >>= 7;
                if v != 0 {
                    byte |= 0x80;
                }
                out.push(byte);
                if v == 0 {
                    return out;
                }
            }
        }
        fn tag(field: u32, wire: u8) -> Vec<u8> {
            varint(u64::from(field) << 3 | u64::from(wire))
        }
        fn len_field(field: u32, body: &[u8]) -> Vec<u8> {
            let mut out = tag(field, 2);
            out.extend(varint(body.len() as u64));
            out.extend_from_slice(body);
            out
        }
        fn str_field(field: u32, s: &str) -> Vec<u8> {
            len_field(field, s.as_bytes())
        }

        let mut category = str_field(1, "Manhwa");
        category.extend(tag(3, 0));
        category.extend(varint(3));

        let mut chapter_read = str_field(1, "https://example.test/ch/1");
        chapter_read.extend(str_field(2, "Chapter 1"));
        chapter_read.extend(tag(4, 0));
        chapter_read.extend(varint(1));
        chapter_read.extend(tag(5, 0));
        chapter_read.extend(varint(1));
        chapter_read.extend(tag(6, 0));
        chapter_read.extend(varint(17));
        chapter_read.extend(tag(9, 5));
        chapter_read.extend_from_slice(&1.0f32.to_le_bytes());

        let mut chapter_unread = str_field(1, "/ch/2");
        chapter_unread.extend(str_field(2, "Chapter 2"));
        chapter_unread.extend(tag(9, 5));
        chapter_unread.extend_from_slice(&2.0f32.to_le_bytes());

        let mut history = str_field(1, "https://example.test/ch/1");
        history.extend(tag(2, 0));
        history.extend(varint(1_760_000_000_000));

        let mut manga = str_field(2, "/series/abc/Test-Series");
        manga.extend(str_field(3, "Test Series"));
        manga.extend(str_field(5, "An Author"));
        manga.extend(tag(1, 0));
        manga.extend(varint(42));
        manga.extend(len_field(16, &chapter_read));
        manga.extend(len_field(16, &chapter_unread));
        manga.extend(len_field(17, &[3]));
        manga.extend(len_field(104, &history));

        let mut buf = len_field(1, &manga);
        buf.extend(len_field(2, &category));
        buf
    }

    #[test]
    fn decodes_backup_fields() {
        let backup = Backup::decode(&fixture()).unwrap();

        assert_eq!(backup.categories.len(), 1);
        assert_eq!(backup.categories[0].name, "Manhwa");
        assert_eq!(backup.categories[0].id, 3);
        assert!(!backup.categories[0].hidden);

        assert_eq!(backup.mangas.len(), 1);
        let m = &backup.mangas[0];
        assert_eq!(m.title, "Test Series");
        assert_eq!(m.author.as_deref(), Some("An Author"));
        assert_eq!(m.url, "/series/abc/Test-Series");
        assert_eq!(m.source, 42);
        assert!(m.favorite, "favorite defaults to true");
        assert_eq!(m.category_ids, vec![3]);

        assert_eq!(m.chapters.len(), 2);
        assert!(m.chapters[0].read);
        assert!(m.chapters[0].bookmark);
        assert_eq!(m.chapters[0].last_page_read, 17);
        assert_eq!(m.chapters[0].chapter_number, 1.0);
        assert!(!m.chapters[1].read);
        assert_eq!(m.chapters[1].url, "/ch/2");

        assert_eq!(m.history.len(), 1);
        assert_eq!(m.history[0].last_read, 1_760_000_000_000);
    }

    #[test]
    fn favorite_false_is_explicit() {
        // A manga that carries favorite=false (field 100 varint 0).
        let mut manga = {
            let mut v = vec![];
            v.extend([0x1a, 0x01, b'X']); // field 3 (title) = "X"
            v
        };
        manga.extend([0xa0, 0x06, 0x00]); // field 100, varint 0
        let mut buf = Vec::new();
        buf.extend([0x0a, manga.len() as u8]);
        buf.extend(manga);

        let backup = Backup::decode(&buf).unwrap();
        assert!(!backup.mangas[0].favorite);
    }

    #[test]
    fn unknown_fields_are_skipped() {
        let mut manga = vec![];
        manga.extend([0x1a, 0x01, b'X']); // title
        manga.extend([0x40, 0x05]); // field 8 varint
        manga.extend([0x62, 0x02, b'h', b'i']); // field 12 string
        manga.extend([0xb1, 0x06, 1, 2, 3, 4, 5, 6, 7, 8]); // field 102 fixed64
        manga.extend([0xc5, 0x06, 1, 2, 3, 4]); // field 104 fixed32

        let mut buf = Vec::new();
        buf.extend([0x0a, manga.len() as u8]);
        buf.extend(manga);

        let backup = Backup::decode(&buf).unwrap();
        assert_eq!(backup.mangas[0].title, "X");
    }

    #[test]
    fn rejects_non_protobuf_garbage() {
        assert!(Backup::decode(b"not a protobuf at all, sorry").is_err());
    }
}
