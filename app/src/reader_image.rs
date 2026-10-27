//! Theme the paper and ink of flat EPUB diagrams, not photographs or originals.
//! Derived textures are private to ChapterImage and never enter ImageCache.
use makepad_widgets::image_cache::{decode_image_from_data, ImageBuffer};
use makepad_widgets::Vec4f;
use std::{collections::HashMap, io::Read, path::Path};

fn rgb(pixel: u32) -> [u8; 3] {
    [(pixel >> 16) as u8, (pixel >> 8) as u8, pixel as u8]
}

fn is_paper(pixel: u32) -> bool {
    pixel >> 24 >= 250 && rgb(pixel).iter().all(|channel| *channel >= 245)
}

/// Conservative flat-art detection: substantial white paper including the
/// border, opaque pixels, and a small dominant palette in the NON-white art.
/// Looking only at white coverage would also recolor white-framed photos.
fn is_paper_diagram(image: &ImageBuffer) -> bool {
    let (width, height) = (image.width, image.height);
    if width < 32
        || height < 32
        || image.animation.is_some()
        || width.checked_mul(height) != Some(image.data.len())
    {
        return false;
    }
    let mut samples = 0usize;
    let mut paper = 0usize;
    let mut palette = HashMap::<u16, usize>::new();
    for y in (0..height).step_by((height / 96).max(1)) {
        for x in (0..width).step_by((width / 96).max(1)) {
            let pixel = image.data[y * width + x];
            // Transparent drawings already expose the reading background.
            if pixel >> 24 < 250 {
                return false;
            }
            samples += 1;
            if is_paper(pixel) {
                paper += 1;
            } else {
                let [r, g, b] = rgb(pixel);
                let key = ((r as u16 >> 3) << 10) | ((g as u16 >> 3) << 5) | (b as u16 >> 3);
                *palette.entry(key).or_default() += 1;
            }
        }
    }
    let ink = samples - paper;
    if paper * 4 < samples || ink < 16 {
        return false;
    }
    let mut border_paper = 0usize;
    for step in 0..64 {
        let x = step * (width - 1) / 63;
        let y = step * (height - 1) / 63;
        for index in [
            x,
            (height - 1) * width + x,
            y * width,
            y * width + width - 1,
        ] {
            border_paper += usize::from(is_paper(image.data[index]));
        }
    }
    if border_paper < 128 {
        return false;
    }
    let mut counts: Vec<_> = palette.into_values().collect();
    counts.sort_unstable_by(|a, b| b.cmp(a));
    counts.iter().take(8).sum::<usize>() * 100 >= ink * 80
}

fn theme_pixel(pixel: u32, surface: Vec4f, text: Vec4f) -> u32 {
    let [r, g, b] = rgb(pixel).map(f32::from);
    let dark = surface.x + surface.y + surface.z < text.x + text.y + text.z;
    if dark {
        // Decompose into black ink, white paper and a chromatic remainder.
        // This is affine along black -> colored-fill edges, so antialiased
        // lettering on a colored box keeps its full stroke, not just its black
        // center pixels. A saturation threshold here would make small text thin.
        let paper = r.min(g).min(b) / 255.0;
        let ink = 1.0 - r.max(g).max(b) / 255.0;
        let channel = |original: f32, bg: f32, fg: f32| -> u32 {
            ((ink * fg + paper * bg) * 255.0 + (original - paper * 255.0) * 0.7)
                .round()
                .clamp(0.0, 255.0) as u32
        };
        return (pixel & 0xff00_0000)
            | (channel(r, surface.x, text.x) << 16)
            | (channel(g, surface.y, text.y) << 8)
            | channel(b, surface.z, text.z);
    }
    let chroma = r.max(g).max(b) - r.min(g).min(b);
    let color_weight = ((chroma - 4.0) / 12.0).clamp(0.0, 1.0);
    let color_weight = color_weight * color_weight * (3.0 - 2.0 * color_weight);
    // Preserve antialiasing: gray edge pixels interpolate between ink and paper.
    let paper_weight = ((r + g + b) / (3.0 * 250.0)).min(1.0);
    let channel = |original: f32, bg: f32, fg: f32| -> u32 {
        let neutral = (fg + (bg - fg) * paper_weight) * 255.0;
        (neutral + (original - neutral) * color_weight)
            .round()
            .clamp(0.0, 255.0) as u32
    };
    (pixel & 0xff00_0000)
        | (channel(r, surface.x, text.x) << 16)
        | (channel(g, surface.y, text.y) << 8)
        | channel(b, surface.z, text.z)
}

pub(crate) fn theme_static_image(
    image: &mut ImageBuffer,
    path: &Path,
    surface: Vec4f,
    text: Vec4f,
) {
    let cover = path.file_stem().is_some_and(|name| {
        name.to_string_lossy()
            .to_ascii_lowercase()
            .contains("cover")
    });
    if !cover && is_paper_diagram(image) {
        for pixel in &mut image.data {
            *pixel = theme_pixel(*pixel, surface, text);
        }
    }
}

/// Decode only on first display / palette changes, never on hover or click.
/// Do not keep a second CPU bitmap alive after uploading the derived texture.
pub fn themed_paper_image(path: &Path, surface: Vec4f, text: Vec4f) -> Option<ImageBuffer> {
    const MAX_BYTES: u64 = 16 * 1024 * 1024;
    // Cover art must retain its identity, including minimalist white covers.
    if path
        .file_stem()?
        .to_string_lossy()
        .to_ascii_lowercase()
        .contains("cover")
    {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_BYTES {
        return None;
    }
    let mut image = decode_image_from_data(&bytes).ok()?;
    if !is_paper_diagram(&image) {
        return None;
    }
    for pixel in &mut image.data {
        *pixel = theme_pixel(*pixel, surface, text);
    }
    Some(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(content: impl Fn(usize, usize) -> u32) -> ImageBuffer {
        let mut image = ImageBuffer {
            width: 96,
            height: 96,
            data: vec![0xffff_ffff; 96 * 96],
            animation: None,
        };
        for y in 12..84 {
            for x in 12..84 {
                image.data[y * 96 + x] = content(x, y);
            }
        }
        image
    }

    #[test]
    fn recognizes_white_paper_with_monochrome_or_colored_flat_art() {
        for fill in [0xff00_0000, 0xffb3_d7ec, 0xffb0_b0b0] {
            assert!(is_paper_diagram(&fixture(|x, y| {
                if x % 12 == 0 || y % 12 == 0 {
                    0xff00_0000
                } else {
                    fill
                }
            })));
        }
    }

    #[test]
    fn rejects_photos_even_with_a_white_frame_and_grayscale_photos() {
        assert!(!is_paper_diagram(&fixture(|x, y| {
            0xff00_0000 | ((x as u32 * 3) << 16) | ((y as u32 * 3) << 8) | (x * y % 255) as u32
        })));
        assert!(!is_paper_diagram(&fixture(|x, y| {
            let gray = ((x * 7 + y * 13) % 240) as u32;
            0xff00_0000 | gray * 0x010101
        })));
    }

    #[test]
    fn rejects_transparent_nonpaper_empty_and_invalid_images() {
        for fill in [0x0000_0000, 0x80ff_ffff, 0xffff_ffff] {
            assert!(!is_paper_diagram(&fixture(|_, _| fill)));
        }
        let mut image = fixture(|_, _| 0xff00_0000);
        image.data.fill(0xff12_5678);
        assert!(!is_paper_diagram(&image));
        image.data.pop();
        assert!(!is_paper_diagram(&image));
        assert!(!is_paper_diagram(&ImageBuffer::default()));
    }

    #[test]
    fn paper_and_ink_follow_each_palette_without_changing_daytime_colors() {
        for (surface, text) in [
            (0xf7f4_eaff, 0x2b29_25ff),
            (0xf2e3_c6ff, 0x3b30_25ff),
            (0xe7ef_e3ff, 0x2734_2bff),
            (0x2025_2bff, 0xd8d4_caff),
            (0x4d4d_4dff, 0xd9d9_d9ff),
        ] {
            let bg = Vec4f::from_u32(surface);
            let fg = Vec4f::from_u32(text);
            assert_eq!(theme_pixel(0xffff_ffff, bg, fg), 0xff00_0000 | surface >> 8);
            assert_eq!(theme_pixel(0xff00_0000, bg, fg), 0xff00_0000 | text >> 8);
        }
        let bg = Vec4f::from_u32(0xf2e3_c6ff);
        let fg = Vec4f::from_u32(0x3b30_25ff);
        for pixel in [0xffb3_d7ec, 0xffff_4080, 0xff20_70ff] {
            assert_eq!(theme_pixel(pixel, bg, fg), pixel);
        }
    }

    #[test]
    fn night_mode_preserves_ink_contrast_and_antialiased_edges() {
        let bg = Vec4f::from_u32(0x2025_2bff);
        let fg = Vec4f::from_u32(0xd8d4_caff);
        let fill = rgb(theme_pixel(0xffb3_d7ec, bg, fg));
        let ink = rgb(theme_pixel(0xff00_0000, bg, fg));
        assert!(ink.iter().zip(fill).all(|(a, b)| *a > b + 100));
        let mut previous = 256;
        for gray in 0..=255u32 {
            let pixel = theme_pixel(0xff00_0000 | gray * 0x010101, bg, fg);
            let red = rgb(pixel)[0] as u32;
            assert!(red <= previous);
            previous = red;
        }
        assert_eq!(theme_pixel(0x80ff_ffff, bg, fg) >> 24, 0x80);
    }

    #[test]
    fn night_mode_keeps_antialiased_text_on_colored_boxes() {
        let bg = Vec4f::from_u32(0x2025_2bff);
        let fg = Vec4f::from_u32(0xd8d4_caff);
        let fill = 0xffb3_d7ec;
        let ink = rgb(theme_pixel(0xff00_0000, bg, fg));
        let themed_fill = rgb(theme_pixel(fill, bg, fg));
        for fraction in [0.1, 0.25, 0.5, 0.75, 0.9] {
            let [r, g, b] = rgb(fill).map(|c| (c as f32 * fraction).round() as u32);
            let original = 0xff00_0000 | (r << 16) | (g << 8) | b;
            let actual = rgb(theme_pixel(original, bg, fg));
            for i in 0..3 {
                let expected = ink[i] as f32 * (1.0 - fraction) + themed_fill[i] as f32 * fraction;
                assert!((actual[i] as f32 - expected).abs() <= 2.0);
            }
        }
    }
}
