use readwise_core::settings::ReaderFont;

#[derive(Clone)]
pub struct ReaderFontFace {
    pub path: String,
    pub weight: f32,
}

#[derive(Clone)]
pub struct ReaderFontFiles {
    pub regular: ReaderFontFace,
    pub italic: ReaderFontFace,
    pub bold: ReaderFontFace,
    pub bold_italic: ReaderFontFace,
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{ReaderFont, ReaderFontFace, ReaderFontFiles};
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    #[derive(Clone)]
    struct SourceFace {
        path: PathBuf,
        index: u32,
        weight: f32,
    }

    fn system_font(name: &str) -> PathBuf {
        Path::new("/System/Library/Fonts").join(name)
    }

    fn supplemental_font(name: &str) -> PathBuf {
        system_font(&format!("Supplemental/{name}"))
    }

    fn apple_font_asset(name: &str) -> Option<PathBuf> {
        let root = Path::new("/System/Library/AssetsV2/com_apple_MobileAsset_Font8");
        fs::read_dir(root)
            .ok()?
            .flatten()
            .map(|entry| entry.path().join("AssetData").join(name))
            .find(|path| path.is_file())
    }

    fn face(path: &Path, index: u32) -> SourceFace {
        SourceFace {
            path: path.to_path_buf(),
            index,
            weight: 0.0,
        }
    }

    fn weighted_face(path: &Path, weight: f32) -> SourceFace {
        SourceFace {
            path: path.to_path_buf(),
            index: 0,
            weight,
        }
    }

    fn source_faces(font: ReaderFont) -> Option<[SourceFace; 4]> {
        let (path, indices) = match font {
            ReaderFont::Canela => (apple_font_asset("CanelaText.ttc")?, [0, 2, 1, 3]),
            ReaderFont::Athelas => (supplemental_font("Athelas.ttc"), [0, 1, 3, 2]),
            ReaderFont::AvenirNext => (system_font("Avenir Next.ttc"), [7, 4, 0, 1]),
            ReaderFont::Charter => (supplemental_font("Charter.ttc"), [0, 1, 3, 2]),
            ReaderFont::Iowan => (supplemental_font("Iowan Old Style.ttc"), [0, 2, 1, 3]),
            ReaderFont::Palatino => (system_font("Palatino.ttc"), [0, 1, 2, 3]),
            ReaderFont::ProximaNova => (apple_font_asset("ProximaNova.ttc")?, [0, 1, 2, 3]),
            ReaderFont::Publico => (apple_font_asset("PublicoText.ttc")?, [0, 1, 2, 3]),
            ReaderFont::Seravek => (supplemental_font("Seravek.ttc"), [0, 1, 9, 8]),
            ReaderFont::Georgia => {
                let root = Path::new("/System/Library/Fonts/Supplemental");
                return Some([
                    face(&root.join("Georgia.ttf"), 0),
                    face(&root.join("Georgia Italic.ttf"), 0),
                    face(&root.join("Georgia Bold.ttf"), 0),
                    face(&root.join("Georgia Bold Italic.ttf"), 0),
                ]);
            }
            ReaderFont::TimesNewRoman => {
                let root = Path::new("/System/Library/Fonts/Supplemental");
                return Some([
                    face(&root.join("Times New Roman.ttf"), 0),
                    face(&root.join("Times New Roman Italic.ttf"), 0),
                    face(&root.join("Times New Roman Bold.ttf"), 0),
                    face(&root.join("Times New Roman Bold Italic.ttf"), 0),
                ]);
            }
            ReaderFont::SanFrancisco => {
                let regular = system_font("SFNS.ttf");
                let italic = system_font("SFNSItalic.ttf");
                return Some([
                    weighted_face(&regular, 400.0),
                    weighted_face(&italic, 400.0),
                    weighted_face(&regular, 700.0),
                    weighted_face(&italic, 700.0),
                ]);
            }
            ReaderFont::NewYork => {
                let regular = system_font("NewYork.ttf");
                let italic = system_font("NewYorkItalic.ttf");
                return Some([
                    weighted_face(&regular, 400.0),
                    weighted_face(&italic, 400.0),
                    weighted_face(&regular, 700.0),
                    weighted_face(&italic, 700.0),
                ]);
            }
            ReaderFont::Original => return None,
        };
        Some(indices.map(|index| face(&path, index)))
    }

    fn u16_be(bytes: &[u8], offset: usize) -> Option<usize> {
        Some(u16::from_be_bytes(bytes.get(offset..offset + 2)?.try_into().ok()?) as usize)
    }

    fn u32_be(bytes: &[u8], offset: usize) -> Option<usize> {
        Some(u32::from_be_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?) as usize)
    }

    /// Makepad currently loads face 0 of a TTC. Materialize the requested face as
    /// an sfnt in the user's temporary directory; system font bytes stay local.
    fn extract_collection_face(bytes: &[u8], index: u32) -> Option<Vec<u8>> {
        if bytes.get(..4)? != b"ttcf" {
            return (index == 0).then(|| bytes.to_vec());
        }
        let count = u32_be(bytes, 8)?;
        if index as usize >= count {
            return None;
        }
        let start = u32_be(bytes, 12 + index as usize * 4)?;
        let tables = u16_be(bytes, start + 4)?;
        let directory_end = start.checked_add(12 + tables * 16)?;
        let mut output = bytes.get(start..directory_end)?.to_vec();
        for table in 0..tables {
            let record = start + 12 + table * 16;
            let offset = u32_be(bytes, record + 8)?;
            let length = u32_be(bytes, record + 12)?;
            let table_bytes = bytes.get(offset..offset.checked_add(length)?)?;
            let new_offset = (output.len() + 3) & !3;
            output.resize(new_offset, 0);
            let output_record = 12 + table * 16 + 8;
            output[output_record..output_record + 4]
                .copy_from_slice(&(new_offset as u32).to_be_bytes());
            output.extend_from_slice(table_bytes);
        }
        Some(output)
    }

    fn prepare_face(font: ReaderFont, role: &str, face: SourceFace) -> Option<ReaderFontFace> {
        if !face.path.is_file() {
            return None;
        }
        let path = if face.path.extension().is_some_and(|ext| ext == "ttc") {
            let cache = std::env::temp_dir().join("readwise-reader-fonts");
            fs::create_dir_all(&cache).ok()?;
            let target = cache.join(format!("{}-{role}-{}.ttf", font.as_str(), face.index));
            if !target.is_file() {
                let bytes = fs::read(&face.path).ok()?;
                let extracted = extract_collection_face(&bytes, face.index)?;
                let staging = cache.join(format!(
                    "{}-{role}-{}.tmp",
                    font.as_str(),
                    std::process::id()
                ));
                fs::write(&staging, extracted).ok()?;
                fs::rename(&staging, &target).ok()?;
            }
            target
        } else {
            face.path
        };
        Some(ReaderFontFace {
            path: path.to_string_lossy().into_owned(),
            weight: face.weight,
        })
    }

    pub fn prepare(font: ReaderFont) -> Option<ReaderFontFiles> {
        let [regular, italic, bold, bold_italic] = source_faces(font)?;
        Some(ReaderFontFiles {
            regular: prepare_face(font, "regular", regular)?,
            italic: prepare_face(font, "italic", italic)?,
            bold: prepare_face(font, "bold", bold)?,
            bold_italic: prepare_face(font, "bold-italic", bold_italic)?,
        })
    }
}

pub fn prepare_reader_font(font: ReaderFont) -> Option<ReaderFontFiles> {
    #[cfg(target_os = "macos")]
    {
        macos::prepare(font)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = font;
        None
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn every_listed_system_font_has_four_readable_faces() {
        for font in ReaderFont::ALL {
            if font == ReaderFont::Original {
                continue;
            }
            let faces =
                prepare_reader_font(font).unwrap_or_else(|| panic!("{} is missing", font.label()));
            for face in [faces.regular, faces.italic, faces.bold, faces.bold_italic] {
                let bytes = std::fs::read(&face.path).unwrap();
                assert!(bytes.len() > 12, "{} has an empty face", font.label());
                assert_ne!(
                    &bytes[..4],
                    b"ttcf",
                    "{} still uses a collection",
                    font.label()
                );
                let tables = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
                assert!(
                    tables > 0 && 12 + tables * 16 <= bytes.len(),
                    "{} has no table directory",
                    font.label()
                );
                for record in 0..tables {
                    let at = 12 + record * 16;
                    let offset =
                        u32::from_be_bytes(bytes[at + 8..at + 12].try_into().unwrap()) as usize;
                    let length =
                        u32::from_be_bytes(bytes[at + 12..at + 16].try_into().unwrap()) as usize;
                    assert!(
                        offset
                            .checked_add(length)
                            .is_some_and(|end| end <= bytes.len()),
                        "{} has an invalid table",
                        font.label()
                    );
                }
            }
        }
    }
}
