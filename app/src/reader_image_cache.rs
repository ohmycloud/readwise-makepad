//! Bounded CPU preparation shared by PDF workers and visible image widgets.
//! Measurement needs dimensions only, never decoded/tinted pixels or GPU upload.
use makepad_widgets::{
    image_cache::{decode_image_from_data, image_size_by_data, ImageBuffer},
    Vec4f,
};
use std::{
    collections::VecDeque,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
    time::SystemTime,
};

const MAX_FILE: u64 = 16 * 1024 * 1024;
const MAX_PIXELS: usize = 16 * 1024 * 1024;
const CACHE_BYTES: usize = 32 * 1024 * 1024;
const CACHE_ENTRIES: usize = 24;
#[derive(Clone, PartialEq, Eq)]
struct Key {
    path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
    palette: [u32; 8],
}
#[derive(Default)]
struct PreparedCache {
    entries: VecDeque<(Key, Arc<ImageBuffer>)>,
    bytes: usize,
}
impl PreparedCache {
    fn get(&mut self, key: &Key) -> Option<Arc<ImageBuffer>> {
        let index = self.entries.iter().position(|(k, _)| k == key)?;
        let entry = self.entries.remove(index)?;
        let image = entry.1.clone();
        self.entries.push_back(entry);
        Some(image)
    }
    fn insert(&mut self, key: Key, image: Arc<ImageBuffer>) {
        let bytes = image.data.len().saturating_mul(4);
        if bytes > CACHE_BYTES {
            return;
        }
        if let Some(index) = self.entries.iter().position(|(k, _)| *k == key) {
            let (_, old) = self.entries.remove(index).unwrap();
            self.bytes -= old.data.len() * 4;
        }
        while self.entries.len() >= CACHE_ENTRIES || self.bytes + bytes > CACHE_BYTES {
            let Some((_, old)) = self.entries.pop_front() else {
                break;
            };
            self.bytes -= old.data.len() * 4;
        }
        self.bytes += bytes;
        self.entries.push_back((key, image));
    }
}
static PREPARED: LazyLock<Mutex<PreparedCache>> =
    LazyLock::new(|| Mutex::new(PreparedCache::default()));
fn key(path: &Path, bg: Vec4f, fg: Vec4f) -> Option<Key> {
    let meta = path.metadata().ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE {
        return None;
    }
    Some(Key {
        path: path.into(),
        len: meta.len(),
        modified: meta.modified().ok(),
        palette: [bg.x, bg.y, bg.z, bg.w, fg.x, fg.y, fg.z, fg.w].map(f32::to_bits),
    })
}
/// Scan bounded chunk headers, seeking OVER compressed pixels. Animation uses
/// atlas dimensions; EXIF can rotate width/height. Both need the original loader,
/// including metadata placed after IDAT. Never infer geometry from IHDR alone.
fn png_dimensions(reader: &mut (impl Read + Seek), length: u64) -> Option<(usize, usize)> {
    let mut signature = [0; 8];
    reader.read_exact(&mut signature).ok()?;
    if signature != *b"\x89PNG\r\n\x1a\n" || length > MAX_FILE {
        return None;
    }
    let mut at = 8u64;
    let mut size = None;
    let mut has_pixels = false;
    for _ in 0..1024 {
        let mut header = [0; 8];
        reader.read_exact(&mut header).ok()?;
        let len = u32::from_be_bytes(header[..4].try_into().ok()?) as u64;
        let next = at.checked_add(12)?.checked_add(len)?;
        if next > length {
            return None;
        }
        match &header[4..] {
            b"acTL" | b"eXIf" => return None,
            b"IHDR" => {
                if at != 8 || len != 13 {
                    return None;
                }
                let mut ihdr = [0; 13];
                reader.read_exact(&mut ihdr).ok()?;
                let w = u32::from_be_bytes(ihdr[..4].try_into().ok()?) as usize;
                let h = u32::from_be_bytes(ihdr[4..8].try_into().ok()?) as usize;
                if w == 0 || h == 0 || w > 16384 || h > 16384 || w.checked_mul(h)? > MAX_PIXELS {
                    return None;
                }
                size = Some((w, h));
            }
            b"IDAT" => has_pixels = true,
            b"IEND" => return if len == 0 && has_pixels { size } else { None },
            _ => {}
        }
        reader.seek(SeekFrom::Start(next)).ok()?;
        at = next;
    }
    None
}
fn static_png_size(bytes: &[u8]) -> Option<(usize, usize)> {
    png_dimensions(&mut std::io::Cursor::new(bytes), bytes.len() as u64)
}
pub(crate) fn measurement_size(path: &Path) -> Option<(usize, usize)> {
    // No bitmap decode, theme loop, ImageCache insert or texture allocation.
    let mut file = std::fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    png_dimensions(&mut file, length)
}

pub(crate) fn prepared_image(path: &Path, bg: Vec4f, fg: Vec4f) -> Option<Arc<ImageBuffer>> {
    let key = key(path, bg, fg)?;
    if let Some(image) = PREPARED.lock().unwrap().get(&key) {
        return Some(image);
    }
    // Never hold the shared cache lock while reading, decoding or theming.
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_FILE {
        return None;
    }
    if bytes.starts_with(b"\x89PNG") {
        static_png_size(&bytes)?;
    } else if !bytes.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let (w, h) = image_size_by_data(&bytes, path).ok()?;
    let pixels = w.checked_mul(h)?;
    // An uncacheable image must not be decoded speculatively and then decoded
    // again on the UI thread. Keep the original loader as its only fallback.
    if pixels > MAX_PIXELS || pixels > CACHE_BYTES / 4 {
        return None;
    }
    let mut image = decode_image_from_data(&bytes).ok()?;
    if image.animation.is_some() {
        return None;
    }
    crate::reader_image::theme_static_image(&mut image, path, bg, fg);
    let image = Arc::new(image);
    PREPARED.lock().unwrap().insert(key, image.clone());
    Some(image)
}

pub(crate) fn prepare_chapter(html: &str, bg: Vec4f, fg: Vec4f) {
    use makepad_widgets::{live_id, makepad_html::parse_html, InternLiveId, LiveId};
    if !html.contains("<img") {
        return;
    }
    let doc = parse_html(html, &mut None, InternLiveId::No);
    let mut walker = doc.new_walker();
    let mut prepared = 0;
    while !walker.done() && prepared < 4 {
        if walker.open_tag_lc() == Some(live_id!(img)) {
            if let Some(src) = walker.find_attr_lc(live_id!(src)) {
                let path = Path::new(src);
                if path.is_absolute() {
                    prepared_image(path, bg, fg);
                    prepared += 1;
                }
            }
        }
        walker.walk();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn test_key(n: u64) -> Key {
        Key {
            path: format!("{n}.png").into(),
            len: n,
            modified: None,
            palette: [0; 8],
        }
    }
    fn image(pixels: usize) -> Arc<ImageBuffer> {
        Arc::new(ImageBuffer {
            width: pixels,
            height: 1,
            data: vec![0; pixels],
            animation: None,
        })
    }
    #[test]
    fn prepared_cache_is_lru_byte_bounded_and_oversized_entries_do_not_evict() {
        let mut cache = PreparedCache::default();
        let a = image(CACHE_BYTES / 8);
        cache.insert(test_key(1), a.clone());
        cache.insert(test_key(2), a.clone());
        assert!(Arc::ptr_eq(&cache.get(&test_key(1)).unwrap(), &a));
        cache.insert(test_key(3), a);
        assert!(cache.get(&test_key(2)).is_none());
        assert!(cache.get(&test_key(1)).is_some());
        cache.insert(test_key(4), image(CACHE_BYTES / 4 + 1));
        assert!(cache.get(&test_key(4)).is_none());
        assert_eq!(cache.bytes, CACHE_BYTES);
        let mut small = PreparedCache::default();
        for n in 0..100 {
            small.insert(test_key(n), image(1));
        }
        assert_eq!(small.entries.len(), CACHE_ENTRIES);
    }
    #[test]
    fn prepared_pixels_reuse_the_exact_theme_projection_without_modifying_the_source() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/reader-diagram.png");
        let original = std::fs::read(&path).unwrap();
        let mut first = None;
        for (bg, fg) in [(0xf2e3c6ff, 0x3b3025ff), (0x20252bff, 0xd8d4caff)] {
            let bg = Vec4f::from_u32(bg);
            let fg = Vec4f::from_u32(fg);
            let expected = crate::reader_image::themed_paper_image(&path, bg, fg).unwrap();
            let prepared = prepared_image(&path, bg, fg).unwrap();
            assert_eq!(prepared.data, expected.data);
            assert_eq!((prepared.width, prepared.height), (96, 96));
            assert!(Arc::ptr_eq(
                &prepared,
                &prepared_image(&path, bg, fg).unwrap()
            ));
            if let Some(first) = first {
                assert!(!Arc::ptr_eq(&prepared, &first));
            }
            first = Some(prepared);
        }
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
    #[test]
    fn png_header_rejects_animation_truncation_and_oversized_dimensions() {
        let original = include_bytes!("../tests/fixtures/reader-diagram.png");
        assert_eq!(static_png_size(original), Some((96, 96)));
        assert!(static_png_size(&original[..25]).is_none());
        let mut oversized = original.to_vec();
        oversized[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(static_png_size(&oversized).is_none());
        let mut animated = original.to_vec();
        let control = [
            0, 0, 0, 8, b'a', b'c', b'T', b'L', 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        animated.splice(33..33, control);
        let mut exif = original.to_vec();
        let mut control = control;
        control[4..8].copy_from_slice(b"eXIf");
        exif.splice(exif.len() - 12..exif.len() - 12, control);
        assert!(
            static_png_size(&exif).is_none(),
            "EXIF after IDAT must not change measured geometry"
        );
        assert!(
            static_png_size(&animated).is_none(),
            "APNG atlas must use the original loader"
        );
    }
    #[test]
    fn exif_rotated_png_keeps_original_loader_geometry() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/reader-diagram-rotated.png");
        let decoded = decode_image_from_data(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!((decoded.width, decoded.height), (64, 96));
        assert!(measurement_size(&path).is_none());
        assert!(prepared_image(
            &path,
            Vec4f::from_u32(0xffffffff),
            Vec4f::from_u32(0x000000ff)
        )
        .is_none());
    }
    #[test]
    fn palette_and_file_revision_are_separate_cache_keys() {
        let original = test_key(1);
        let mut palette = original.clone();
        palette.palette[0] = 1;
        let mut revision = original.clone();
        revision.len = 2;
        let mut cache = PreparedCache::default();
        cache.insert(original.clone(), image(1));
        assert!(cache.get(&palette).is_none());
        assert!(cache.get(&revision).is_none());
        assert!(cache.get(&original).is_some());
    }
    #[test]
    fn measurement_reads_chunk_headers_but_never_compressed_pixels() {
        struct HeadersOnly {
            inner: std::io::Cursor<&'static [u8]>,
            read: usize,
        }
        impl Read for HeadersOnly {
            fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
                let read = self.inner.read(bytes)?;
                self.read += read;
                Ok(read)
            }
        }
        impl Seek for HeadersOnly {
            fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
                self.inner.seek(pos)
            }
        }
        let bytes = include_bytes!("../tests/fixtures/reader-diagram.png");
        let mut reader = HeadersOnly {
            inner: std::io::Cursor::new(bytes),
            read: 0,
        };
        assert_eq!(
            png_dimensions(&mut reader, bytes.len() as u64),
            Some((96, 96))
        );
        assert_eq!(
            reader.read,
            8 + 3 * 8 + 13,
            "signature, three chunk headers, IHDR only"
        );
    }
    #[test]
    fn png_measurement_matches_decoded_pixels_without_theming_or_cache_population() {
        let png = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/icon_128.png");
        let decoded = decode_image_from_data(&std::fs::read(&png).unwrap()).unwrap();
        assert_eq!(
            measurement_size(&png),
            Some((decoded.width, decoded.height))
        );
        assert_eq!((decoded.width, decoded.height), (128, 128));
    }
}
