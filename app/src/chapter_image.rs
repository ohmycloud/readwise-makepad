use makepad_widgets::makepad_html::HtmlDoc;
use makepad_widgets::*;

script_mod! {
    use mod.prelude.widgets.*

    mod.widgets.ChapterImageBase = #(ChapterImage::register_widget(vm))
    mod.widgets.ChapterImage = set_type_default() do mod.widgets.ChapterImageBase{
        width: Fill height: Fit
        align: Align{ x: 0.5 }
        padding: Inset{ top: 8 bottom: 8 }
        chapter_img := Image{
            width: 100 height: 100
            fit: ImageFit.Stretch
        }
    }
}

const INLINE_IMAGE_MAX_HEIGHT: f64 = 520.0;

thread_local! {
    static PAGE_HEIGHT: std::cell::Cell<f64> = const { std::cell::Cell::new(INLINE_IMAGE_MAX_HEIGHT) };
}

/// Measurement and visible drawing must use the same image size, including
/// on a short landscape screen. The original stays available in the viewer.
pub(crate) fn with_page_height<T>(height: f64, draw: impl FnOnce() -> T) -> T {
    PAGE_HEIGHT.with(|limit| {
        let previous = limit.replace(if height.is_finite() && height > 0.0 {
            (height - 48.0).max(20.0).min(INLINE_IMAGE_MAX_HEIGHT)
        } else {
            INLINE_IMAGE_MAX_HEIGHT
        });
        let result = draw();
        limit.set(previous);
        result
    })
}

fn inline_image_display_size(
    pixel_size: (usize, usize),
    available_width: f64,
    dpi_factor: f64,
) -> DVec2 {
    let pixel_width = pixel_size.0.max(1) as f64;
    let pixel_height = pixel_size.1.max(1) as f64;
    let dpi_factor = dpi_factor.max(1.0);
    let natural_scale = 1.0 / dpi_factor;
    let width_scale = if available_width.is_finite() && available_width > 0.0 {
        available_width / pixel_width
    } else {
        natural_scale
    };
    let height_scale = PAGE_HEIGHT.get() / pixel_height;
    let scale = natural_scale.min(width_scale).min(height_scale).max(0.01);
    dvec2(pixel_width * scale, pixel_height * scale)
}

#[derive(Clone, Debug, Default)]
pub enum ChapterImageAction {
    Open(String),
    #[default]
    None,
}

/// 章节内嵌图片：由 Html 自定义标签机制按 <img src="绝对路径"> 实例化，
/// src 已在 core 侧改写为提取到 res/ 的本地文件。
#[derive(Script, Widget)]
pub struct ChapterImage {
    #[deref]
    view: View,
    #[rust]
    src: String,
    #[rust]
    load_attempted: bool,
    #[rust]
    pixel_size: Option<(usize, usize)>,
    #[rust]
    image_palette: Option<(Vec4f, Vec4f)>,
    #[rust]
    preserve_original: bool,
    #[rust]
    press_origin: Option<DVec2>,
}

impl ScriptHook for ChapterImage {
    fn on_after_new_scoped(&mut self, _vm: &mut ScriptVm, scope: &mut Scope) {
        if let Some(doc) = scope.props.get::<HtmlDoc>() {
            let mut walker = doc.new_walker_with_index(scope.index + 1);
            while let Some((lc, attr)) = walker.while_attr_lc() {
                if lc == live_id!(src) {
                    self.src = attr.into();
                    break;
                }
            }
        }
    }
}

impl Widget for ChapterImage {
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let img = self.view.image(cx, ids!(chapter_img));
        if !self.load_attempted && !self.src.is_empty() {
            self.load_attempted = true;
            let path = std::path::Path::new(&self.src);
            if let Err(e) = img.load_image_file_by_path(cx, path) {
                ::log::error!("章节图片加载失败 {}: {:?}", self.src, e);
            } else {
                self.pixel_size = img.size_in_pixels(cx);
            }
        }
        if let Some(pixel_size) = self.pixel_size {
            // The original loader has already resolved the texture dimensions
            // (including animation atlases). Bound extra decode/upload work.
            if pixel_size.0.saturating_mul(pixel_size.1) > 16 * 1024 * 1024 {
                self.preserve_original = true;
            }
            let palette = crate::app::current_reader_image_palette();
            if !self.preserve_original && self.image_palette != Some(palette) {
                self.image_palette = Some(palette);
                if let Some(bitmap) = crate::reader_image::themed_paper_image(
                    std::path::Path::new(&self.src),
                    palette.0,
                    palette.1,
                ) {
                    // Never replace the cached original: the image viewer and
                    // other Image instances must still load unmodified pixels.
                    let texture = bitmap.into_new_texture(cx);
                    img.set_texture(cx, Some(texture));
                } else {
                    self.preserve_original = true;
                    // A later read failure must not leave a texture tinted for
                    // the previous theme. The original is still in ImageCache.
                    let _ = img.load_image_file_by_path(cx, std::path::Path::new(&self.src));
                }
            }
            let available_width = cx.peek_walk_turtle(walk).size.x;
            let display =
                inline_image_display_size(pixel_size, available_width, cx.current_dpi_factor());
            if let Some(mut inner) = img.borrow_mut() {
                inner.walk.width = Size::Fixed(display.x);
                inner.walk.height = Size::Fixed(display.y);
            }
        }
        self.view.draw_walk(cx, scope, walk)
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        self.view.handle_event(cx, event, scope);
        let area = self.view.image(cx, ids!(chapter_img)).area();
        match event.hits(cx, area) {
            Hit::FingerDown(event) => self.press_origin = Some(event.abs),
            Hit::FingerMove(event) => {
                if self
                    .press_origin
                    .is_some_and(|origin| (event.abs - origin).length() > 8.0)
                {
                    self.press_origin = None;
                }
            }
            Hit::FingerHoverIn(_) | Hit::FingerHoverOver(_) => {
                cx.set_cursor(MouseCursor::Hand);
            }
            Hit::FingerUp(event) => {
                let origin = self.press_origin.take();
                let clicked = event.is_over
                    && origin.is_some_and(|origin| (event.abs - origin).length() <= 8.0);
                if clicked && !self.src.is_empty() {
                    cx.widget_action(
                        self.widget_uid(),
                        ChapterImageAction::Open(self.src.clone()),
                    );
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paged_images_fit_short_viewports_and_restore_the_normal_limit() {
        let original = inline_image_display_size((600, 1200), 500.0, 1.0);
        let short = with_page_height(240.0, || inline_image_display_size((600, 1200), 500.0, 1.0));
        assert_size_close(short, dvec2(96.0, 192.0));
        assert_size_close(inline_image_display_size((600, 1200), 500.0, 1.0), original);
    }

    fn assert_size_close(actual: DVec2, expected: DVec2) {
        assert!((actual.x - expected.x).abs() < 0.001, "{actual:?}");
        assert!((actual.y - expected.y).abs() < 0.001, "{actual:?}");
    }

    #[test]
    fn inline_images_never_upscale_past_one_source_pixel_per_device_pixel() {
        assert_size_close(
            inline_image_display_size((24, 24), 600.0, 2.0),
            dvec2(12.0, 12.0),
        );
        assert_size_close(
            inline_image_display_size((1200, 800), 500.0, 2.0),
            dvec2(500.0, 500.0 / 1.5),
        );
    }

    #[test]
    fn inline_images_preserve_aspect_ratio_when_height_is_capped() {
        assert_size_close(
            inline_image_display_size((800, 1600), 700.0, 1.0),
            dvec2(260.0, 520.0),
        );
    }
}
