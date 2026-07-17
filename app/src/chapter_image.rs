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
            width: Fill height: 320
            fit: ImageFit.Biggest
        }
    }
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
        if !self.load_attempted && !self.src.is_empty() {
            self.load_attempted = true;
            let img = self.view.image(cx, ids!(chapter_img));
            let path = std::path::Path::new(&self.src);
            if let Err(e) = img.load_image_file_by_path(cx, path) {
                ::log::error!("章节图片加载失败 {}: {:?}", self.src, e);
            }
        }
        self.view.draw_walk(cx, scope, walk)
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        self.view.handle_event(cx, event, scope);
    }
}
