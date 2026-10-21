//! First-page thumbnails only. Never used by reader text extraction/pagination.
use hayro::{
    hayro_interpret::InterpreterSettings, hayro_syntax::Pdf, vello_cpu::color::palette::css::WHITE,
    RenderCache, RenderSettings,
};
use std::{fs, io::Read, path::Path, sync::Mutex};

pub(crate) const FILE: &str = "cover-pdf-v1.png";
const MAX_EDGE: f32 = 600.0;
const MAX_BYTES: u64 = 200 * 1024 * 1024;
// Import and legacy backfill share this bound on concurrent renderer memory.
static RENDER_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn generate(source: &Path) -> Result<Vec<u8>, String> {
    let _guard = RENDER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut bytes = Vec::new();
    fs::File::open(source)
        .and_then(|f| f.take(MAX_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|e| format!("读取 PDF 封面源文件失败: {e}"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("PDF 超过封面渲染大小限制".into());
    }
    // A renderer limitation must not prevent text import or stop the backfill.
    std::panic::catch_unwind(|| {
        let pdf = Pdf::new(bytes).map_err(|e| format!("打开 PDF 封面失败: {e:?}"))?;
        let page = pdf.pages().first().ok_or("PDF 没有首页")?;
        // Includes page rotation and crop box, rather than raw MediaBox sizes.
        let (width, height) = page.render_dimensions();
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return Err("PDF 首页尺寸无效".into());
        }
        let scale = MAX_EDGE / width.max(height);
        let settings = RenderSettings {
            x_scale: scale,
            y_scale: scale,
            width: Some((width * scale).round().clamp(1.0, MAX_EDGE) as u16),
            height: Some((height * scale).round().clamp(1.0, MAX_EDGE) as u16),
            bg_color: WHITE,
        };
        hayro::render(
            page,
            &RenderCache::new(),
            &InterpreterSettings::default(),
            &settings,
        )
        .into_png()
        .map_err(|e| format!("编码 PDF 封面失败: {e}"))
    })
    .unwrap_or_else(|_| Err("PDF 首页渲染失败".into()))
}

/// Destination is always an app-owned book directory; never create sidecars by
/// an arbitrary source PDF. A removed book directory is not recreated here.
pub(crate) fn save(source: &Path, directory: &Path) -> Result<(), String> {
    let bytes = generate(source)?;
    let temporary = directory.join(format!(".pdf-cover-{}.tmp", uuid::Uuid::new_v4()));
    let result =
        fs::write(&temporary, bytes).and_then(|_| fs::rename(&temporary, directory.join(FILE)));
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(format!("保存 PDF 封面失败: {error}"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "cover_tests.rs"]
mod tests;
