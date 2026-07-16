use crate::ruby_word::RubyWordAction;
use crate::sample_chapter;
use makepad_widgets::*;
use std::collections::HashMap;
use std::sync::{LazyLock, OnceLock, RwLock};

app_main!(App);

const PARA_COUNT: usize = 200;

static PARAGRAPHS: LazyLock<RwLock<Vec<String>>> =
    LazyLock::new(|| RwLock::new(sample_chapter::build_paragraphs(PARA_COUNT)));

/// 全局数据库句柄：后台线程初始化后写入，各处只读使用。
static DB: OnceLock<readwise_core::Db> = OnceLock::new();

/// 书架数据（UI 侧只读渲染，后台线程写）。
static SHELF: LazyLock<RwLock<Vec<ShelfItem>>> = LazyLock::new(|| RwLock::new(Vec::new()));

#[derive(Clone)]
struct ShelfItem {
    id: String,
    title: String,
    author: String,
    status_line: String,
    cover_path: Option<String>,
    file_path: String,
}

/// 当前打开的书（后台线程写入，UI 读）。
static CURRENT_BOOK: LazyLock<RwLock<Option<OpenBook>>> = LazyLock::new(|| RwLock::new(None));

struct OpenBook {
    id: String,
    title: String,
    /// 各章 body HTML（已剔除 ruby 降级处理）。
    chapters: Vec<String>,
}

/// 打开书的结果（后台 → UI）。
#[derive(Debug)]
enum ReaderAction {
    Opened { resume_chapter: usize },
    OpenError(String),
}

/// 后台加载整本书章节 + 恢复进度。
fn spawn_open_book(item: ShelfItem) {
    std::thread::spawn(move || {
        let chapters = match readwise_core::books::epub_meta::load_chapter_htmls(
            std::path::Path::new(&item.file_path),
        ) {
            Ok(list) => list,
            Err(e) => {
                Cx::post_action(ReaderAction::OpenError(e));
                return;
            }
        };
        let total = chapters.len();
        let resume = DB
            .get()
            .and_then(|db| db.get_status(&item.id).ok().flatten())
            .and_then(|s| s.location.parse::<usize>().ok())
            .filter(|idx| *idx < total)
            .unwrap_or(0);
        *CURRENT_BOOK.write().unwrap() = Some(OpenBook {
            id: item.id,
            title: item.title,
            chapters,
        });
        Cx::post_action(ReaderAction::Opened { resume_chapter: resume });
    });
}

/// 后台保存阅读进度（章节粒度，location 存章节下标）。
fn spawn_save_progress(id: String, chapter_idx: usize, total: usize) {
    std::thread::spawn(move || {
        if let Some(db) = DB.get() {
            let _ = db.update_progress(
                &id,
                (chapter_idx + 1) as i64,
                total as i64,
                &chapter_idx.to_string(),
            );
        }
    });
}

fn status_cn(status: &str, cur: i64, total: i64) -> String {
    let name = match status {
        "reading" => "在读",
        "completed" => "读完",
        _ => "未读",
    };
    if total > 0 {
        format!("{} · {}%", name, (cur * 100 / total).min(100))
    } else {
        name.to_string()
    }
}

/// 书架加载结果（后台 → UI）。
#[derive(Debug)]
enum ShelfAction {
    Loaded { items: usize, imported: usize },
    Error(String),
}

/// 从 DB 读书架写入静态区，返回本数（后台线程内调用）。
fn refresh_shelf(db: &readwise_core::Db) -> Result<usize, String> {
    let items = db.get_shelf_items()?;
    let ui_items: Vec<ShelfItem> = items
        .iter()
        .map(|b| ShelfItem {
            id: b.id.clone(),
            title: b.title.clone(),
            author: b.author.clone(),
            status_line: status_cn(&b.status, b.progress_current, b.progress_total),
            cover_path: b.cover_abs_path.clone(),
            file_path: b.file_abs_path.clone(),
        })
        .collect();
    let n = ui_items.len();
    *SHELF.write().unwrap() = ui_items;
    Ok(n)
}

/// 后台加载书架：先扫描导入目录，再读全部书目。
fn spawn_load_shelf() {
    std::thread::spawn(|| {
        let Some(db) = DB.get() else { return };
        let imported = db.scan_import_dir().unwrap_or(0);
        match refresh_shelf(db) {
            Ok(n) => Cx::post_action(ShelfAction::Loaded { items: n, imported }),
            Err(e) => Cx::post_action(ShelfAction::Error(e)),
        }
    });
}

/// 后台删除书籍后刷新书架。
fn spawn_delete(id: String) {
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        if let Err(e) = db.delete_book(&id) {
            println!("删除失败 {}: {}", id, e);
        }
        match refresh_shelf(db) {
            Ok(n) => Cx::post_action(ShelfAction::Loaded {
                items: n,
                imported: 0,
            }),
            Err(e) => Cx::post_action(ShelfAction::Error(e)),
        }
    });
}

fn is_epub_path(path: &str) -> bool {
    path.to_lowercase().ends_with(".epub")
}

/// macOS 拖放给的是 URL 百分号编码路径（中文/空格会变 %XX），解码成真实路径。
fn percent_decode_path(s: &str) -> String {
    fn hex(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn is_epub_item(item: &DragItem) -> bool {
    matches!(item, DragItem::FilePath { path, .. } if is_epub_path(path))
}

/// 拖入的 epub 文件在后台导入，完成后刷新书架。
fn spawn_import(paths: Vec<String>) {
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        for p in paths {
            if let Err(e) = db.import_book(std::path::Path::new(&p)) {
                //log::error!("导入失败 {}: {}", p, e);
                println!("导入失败 {}: {}", p, e);
            }
        }
        match refresh_shelf(db) {
            Ok(n) => Cx::post_action(ShelfAction::Loaded {
                items: n,
                imported: 0,
            }),
            Err(e) => Cx::post_action(ShelfAction::Error(e)),
        }
    });
}

/// 数据库初始化结果（后台线程 → UI）。
#[derive(Debug)]
enum DbAction {
    Ready {
        vocab_count: usize,
        known_count: usize,
        vocab_preview: String,
    },
    Failed(String),
}

fn spawn_db_init() {
    std::thread::spawn(|| match readwise_core::Db::open() {
        Ok(db) => {
            let entries = db.vocab_book_list().unwrap_or_default();
            let known = db.get_known_words().unwrap_or_default();
            let vocab_preview = entries
                .iter()
                .take(50)
                .map(|e| {
                    format!(
                        "{}  —  {}   [{} · 复习{}次]",
                        e.word,
                        e.gloss.as_deref().unwrap_or("(无释义)"),
                        e.status,
                        e.review_count
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            let action = DbAction::Ready {
                vocab_count: entries.len(),
                known_count: known.len(),
                vocab_preview,
            };
            let _ = DB.set(db);
            Cx::post_action(action);
        }
        Err(e) => Cx::post_action(DbAction::Failed(e)),
    });
}

script_mod! {
    use mod.prelude.widgets.*
    use mod.widgets.*

    mod.widgets.BookShelfBase = #(BookShelf::register_widget(vm))
    mod.widgets.BookShelf = set_type_default() do mod.widgets.BookShelfBase{
        width: Fill height: Fill
        list := PortalList{
            width: Fill height: Fill
            padding: Inset{ left: 24 right: 24 top: 8 bottom: 24 }
            spacing: 10
            scroll_bar: ScrollBar{}
            Card := RoundedView{
                width: Fill height: 76
                flow: Right spacing: 14 padding: 12
                align: Align{ y: 0.5 }
                draw_bg.color: #x2a2a2a
                draw_bg.border_radius: 8.0
                cover := View{
                    width: 44 height: 52
                    flow: Overlay
                    cover_ph := SolidView{
                        width: Fill height: Fill
                        align: Align{ x: 0.5 y: 0.5 }
                        draw_bg.color: #x1f6f63
                        draw_bg.border_radius: 4.0
                        cover_letter := Label{
                            text: ""
                            draw_text.text_style.font_size: 20
                        }
                    }
                    cover_img := Image{
                        width: Fill height: Fill
                        fit: ImageFit.Biggest
                    }
                }
                info := View{
                    width: Fill height: Fit
                    flow: Down spacing: 5
                    card_title := Label{
                        width: Fill
                        text: ""
                        draw_text.text_style.font_size: 14
                    }
                    card_author := Label{
                        width: Fill
                        text: ""
                        draw_text.color: #x9a9a9a
                    }
                    card_status := Label{
                        width: Fill
                        text: ""
                        draw_text.color: #x7fb2a6
                        draw_text.text_style.font_size: 9
                    }
                }
                read_btn := ButtonFlat{ text: "阅读" }
                del_btn := ButtonFlat{ text: "删除" }
            }
        }
    }

    mod.widgets.ChapterListBase = #(ChapterList::register_widget(vm))
    mod.widgets.ChapterList = set_type_default() do mod.widgets.ChapterListBase{
        width: Fill height: Fill
        list := PortalList{
            width: Fill height: Fill
            padding: Inset{ left: 32 right: 32 top: 8 bottom: 32 }
            spacing: 6
            scroll_bar: ScrollBar{}
            Para := CachedView{
                para_html := Html{
                    width: Fill height: Fit
                    a.color: #xc9c9c9
                    a.hover_color: #x62b6ff
                    a.pressed_color: #x2a8fe0
                }
            }
            GlossPara := CachedView{
                para_html := Html{
                    width: Fill height: Fit
                    text_style_normal.line_spacing: 2.0
                    text_style_bold.line_spacing: 2.0
                    a.color: #xc9c9c9
                    a.hover_color: #x62b6ff
                    a.pressed_color: #x2a8fe0
                    ruby := mod.widgets.RubyWord{}
                }
            }
        }
    }

    startup() do #(App::script_component(vm)){
        ui: Root{
            main_window := Window{
                window.title: "ReadWise"
                window.inner_size: vec2(1080, 720)
                body +: {
                    flow: Right
                    sidebar := SolidView{
                        width: 150 height: Fill
                        flow: Down padding: 12 spacing: 8
                        draw_bg.color: #x1f1f1f
                        app_title := Label{
                            text: "ReadWise"
                            draw_text.text_style.font_size: 15
                        }
                        nav_library := ButtonFlat{ width: Fill text: "书库" }
                        nav_reader := ButtonFlat{ width: Fill text: "阅读" }
                        nav_vocab := ButtonFlat{ width: Fill text: "生词本" }
                        nav_settings := ButtonFlat{ width: Fill text: "设置" }
                        sidebar_fill := View{ width: Fill height: Fill }
                        db_status := Label{
                            width: Fill
                            text: "数据库初始化中…"
                            draw_text.text_style.font_size: 8.5
                        }
                    }
                    pages := PageFlip{
                        width: Fill height: Fill
                        active_page: @page_reader

                        page_library := View{
                            width: Fill height: Fill
                            flow: Down
                            lib_header := View{
                                width: Fill height: Fit
                                padding: Inset{ left: 24 right: 24 top: 20 bottom: 8 }
                                flow: Down spacing: 4
                                Label{ text: "书库" draw_text.text_style.font_size: 18 }
                                library_hint := Label{
                                    width: Fill
                                    text: "把 EPUB 拖进窗口，或放入导入目录后重启。"
                                    draw_text.color: #x9a9a9a
                                    draw_text.text_style.font_size: 9
                                }
                            }
                            book_shelf := mod.widgets.BookShelf{}
                        }

                        page_reader := View{
                            width: Fill height: Fill
                            flow: Down
                            header := View{
                                width: Fill height: Fit
                                padding: 12 spacing: 12
                                align: Align{ y: 0.5 }
                                title := Label{
                                    text: "阅读 · 示例章节"
                                    draw_text.text_style.font_size: 15
                                }
                                spacer := View{ width: Fill height: Fit }
                                status := Label{ text: "" }
                                prev_btn := ButtonFlat{ text: "上一章" }
                                chapter_pos := Label{ text: "" }
                                next_btn := ButtonFlat{ text: "下一章" }
                            }
                            chapter_list := mod.widgets.ChapterList{}
                        }

                        page_vocab := View{
                            width: Fill height: Fill
                            flow: Down padding: 24 spacing: 12
                            vocab_title := Label{ text: "生词本" draw_text.text_style.font_size: 18 }
                            vocab_summary := Label{ width: Fill text: "等待数据库…" }
                            vocab_scroll := ScrollYView{
                                width: Fill height: Fill
                                vocab_list := Label{ width: Fill text: "" }
                            }
                        }

                        page_settings := View{
                            width: Fill height: Fill
                            flow: Down padding: 24 spacing: 12
                            Label{ text: "设置" draw_text.text_style.font_size: 18 }
                            settings_hint := Label{
                                width: Fill
                                text: "M8 整合：主题、字体、AI 配置。"
                            }
                            Label{
                                text: "开发工具"
                                draw_text.text_style.font_size: 13
                                draw_text.color: #x9a9a9a
                            }
                            dev_tools := View{
                                width: Fill height: Fit
                                spacing: 12
                                stream_button := Button{ text: "流式测试" }
                                reload_button := Button{ text: "重载示例章节" }
                            }
                            stream_panel := View{
                                width: Fill height: Fit
                                stream_label := Label{
                                    width: Fill
                                    text: "点「流式测试」验证后台线程 → UI 打字机效果"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Script, ScriptHook, Widget)]
struct ChapterList {
    #[deref]
    view: View,
}

impl Widget for ChapterList {
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let paragraphs = PARAGRAPHS.read().unwrap();

        while let Some(step) = self.view.draw_walk(cx, scope, walk).step() {
            if let Some(mut list) = step.as_portal_list().borrow_mut() {
                list.set_item_range(cx, 0, paragraphs.len());
                while let Some(item_id) = list.next_visible_item(cx) {
                    let Some(para) = paragraphs.get(item_id) else {
                        continue;
                    };
                    let template = if para.contains("<ruby") {
                        id!(GlossPara)
                    } else {
                        id!(Para)
                    };
                    let item = list.item(cx, item_id, template);
                    item.widget(cx, ids!(para_html)).set_text(cx, para);
                    item.draw_all_unscoped(cx);
                }
            }
        }

        DrawStep::done()
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        self.view.handle_event(cx, event, scope);
    }
}

#[derive(Script, ScriptHook, Widget)]
struct BookShelf {
    #[deref]
    view: View,
    /// item_id → 已加载的封面路径（PortalList 复用 item，路径变了才重载）。
    #[rust]
    loaded_covers: HashMap<usize, Option<String>>,
}

impl Widget for BookShelf {
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let shelf = SHELF.read().unwrap();

        while let Some(step) = self.view.draw_walk(cx, scope, walk).step() {
            if let Some(mut list) = step.as_portal_list().borrow_mut() {
                if shelf.is_empty() {
                    list.set_item_range(cx, 0, 1);
                    while let Some(item_id) = list.next_visible_item(cx) {
                        let item = list.item(cx, item_id, id!(Card));
                        item.label(cx, ids!(cover_letter)).set_text(cx, "书");
                        item.label(cx, ids!(card_title)).set_text(cx, "书架为空");
                        item.label(cx, ids!(card_author))
                            .set_text(cx, "拖入 EPUB 文件，或放进 ReadWise/import 目录");
                        item.label(cx, ids!(card_status)).set_text(cx, "");
                        item.draw_all_unscoped(cx);
                    }
                } else {
                    list.set_item_range(cx, 0, shelf.len());
                    while let Some(item_id) = list.next_visible_item(cx) {
                        let Some(book) = shelf.get(item_id) else {
                            continue;
                        };
                        let letter = book
                            .title
                            .chars()
                            .next()
                            .map(|c| c.to_string())
                            .unwrap_or_default();
                        let item = list.item(cx, item_id, id!(Card));
                        item.label(cx, ids!(cover_letter)).set_text(cx, &letter);
                        item.label(cx, ids!(card_title)).set_text(cx, &book.title);
                        item.label(cx, ids!(card_author)).set_text(cx, &book.author);
                        item.label(cx, ids!(card_status))
                            .set_text(cx, &book.status_line);
                        if self.loaded_covers.get(&item_id) != Some(&book.cover_path) {
                            let img = item.image(cx, ids!(cover_img));
                            match &book.cover_path {
                                Some(path) => {
                                    let p = std::path::Path::new(path);
                                    if let Err(e) = img.load_image_file_by_path(cx, p) {
                                        println!("封面加载失败 {}: {:?}", path, e);
                                        img.set_texture(cx, None);
                                    }
                                }
                                None => img.set_texture(cx, None),
                            }
                            self.loaded_covers.insert(item_id, book.cover_path.clone());
                        }
                        item.draw_all_unscoped(cx);
                    }
                }
            }
        }

        DrawStep::done()
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        self.view.handle_event(cx, event, scope);
    }
}

#[derive(Script, ScriptHook)]
pub struct App {
    #[source]
    source: ScriptObjectRef,
    #[live]
    ui: WidgetRef,
    #[rust]
    reload_count: usize,
    #[rust]
    stream_text: String,
    #[rust]
    chapter_idx: usize,
}

impl App {
    /// 切换到指定章节：按 </p> 切块喂给 PortalList，更新标题/页码，保存进度，滚回顶部。
    fn set_chapter(&mut self, cx: &mut Cx, idx: usize) {
        let (id, title, total, chunks) = {
            let guard = CURRENT_BOOK.read().unwrap();
            let Some(book) = guard.as_ref() else { return };
            if idx >= book.chapters.len() {
                return;
            }
            let chunks = readwise_core::books::epub_meta::split_blocks(&book.chapters[idx]);
            (book.id.clone(), book.title.clone(), book.chapters.len(), chunks)
        };
        self.chapter_idx = idx;
        *PARAGRAPHS.write().unwrap() = chunks;
        self.ui
            .label(cx, ids!(title))
            .set_text(cx, &format!("《{}》", title));
        self.ui
            .label(cx, ids!(chapter_pos))
            .set_text(cx, &format!("{}/{}", idx + 1, total));
        self.ui
            .widget(cx, ids!(chapter_list))
            .portal_list(cx, ids!(list))
            .set_first_id_and_scroll(0, 0.0);
        spawn_save_progress(id, idx, total);
        self.ui.redraw(cx);
    }
}

/// 后台线程 → UI 的流式消息（模拟 AI 打字机，M5 换成真实 SSE）。
#[derive(Debug)]
enum StreamAction {
    Chunk(String),
    Done,
}

fn spawn_stream_demo() {
    std::thread::spawn(|| {
        let words = [
            "月光",
            "如水",
            "，",
            "虎影",
            "徘徊",
            "。",
            "Streaming ",
            "tokens ",
            "arrive ",
            "one ",
            "by ",
            "one",
            "，",
            "后台",
            "线程",
            "经",
            "Cx::post_action",
            "推送",
            "，",
            "UI",
            "线程",
            "逐词",
            "上屏",
            "。",
        ];
        for w in words {
            std::thread::sleep(std::time::Duration::from_millis(80));
            Cx::post_action(StreamAction::Chunk(w.to_string()));
        }
        Cx::post_action(StreamAction::Done);
    });
}

impl MatchEvent for App {
    fn handle_actions(&mut self, cx: &mut Cx, actions: &Actions) {
        for action in actions {
            if let Some(db_action) = action.downcast_ref::<DbAction>() {
                match db_action {
                    DbAction::Ready {
                        vocab_count,
                        known_count,
                        vocab_preview,
                    } => {
                        let status = format!(
                            "数据库就绪\n生词本 {} · 已认识 {}",
                            vocab_count, known_count
                        );
                        self.ui.label(cx, ids!(db_status)).set_text(cx, &status);
                        let summary = format!("共 {} 词（预览前 50 条）", vocab_count);
                        self.ui
                            .label(cx, ids!(vocab_summary))
                            .set_text(cx, &summary);
                        let list = if vocab_preview.is_empty() {
                            "生词本为空——去阅读页点注音词收藏（M4 打通）。".to_string()
                        } else {
                            vocab_preview.clone()
                        };
                        self.ui.label(cx, ids!(vocab_list)).set_text(cx, &list);
                        spawn_load_shelf();
                    }
                    DbAction::Failed(e) => {
                        let status = format!("数据库失败:\n{}", e);
                        self.ui.label(cx, ids!(db_status)).set_text(cx, &status);
                    }
                }
                continue;
            }
            if let Some(reader) = action.downcast_ref::<ReaderAction>() {
                match reader {
                    ReaderAction::Opened { resume_chapter } => {
                        self.set_chapter(cx, *resume_chapter);
                    }
                    ReaderAction::OpenError(e) => {
                        self.ui
                            .label(cx, ids!(title))
                            .set_text(cx, &format!("打开失败: {}", e));
                    }
                }
                continue;
            }
            if let Some(shelf) = action.downcast_ref::<ShelfAction>() {
                match shelf {
                    ShelfAction::Loaded { items, imported } => {
                        let hint = if *imported > 0 {
                            format!("共 {} 本（导入目录新增 {} 本）", items, imported)
                        } else {
                            format!("共 {} 本 · 拖入 EPUB 或放进 import 目录添加", items)
                        };
                        self.ui.label(cx, ids!(library_hint)).set_text(cx, &hint);
                        self.ui.redraw(cx);
                    }
                    ShelfAction::Error(e) => {
                        self.ui
                            .label(cx, ids!(library_hint))
                            .set_text(cx, &format!("书架加载失败: {}", e));
                    }
                }
                continue;
            }
            if let Some(stream) = action.downcast_ref::<StreamAction>() {
                match stream {
                    StreamAction::Chunk(text) => self.stream_text.push_str(text),
                    StreamAction::Done => self.stream_text.push_str(" ✅"),
                }
                self.ui
                    .label(cx, ids!(stream_label))
                    .set_text(cx, &self.stream_text);
                continue;
            }
            let Some(action) = action.as_widget_action() else {
                continue;
            };
            if let HtmlLinkAction::Clicked { url, .. } = action.cast() {
                if let Some(word) = url.strip_prefix("w:") {
                    let status = format!("点词：{}", word);
                    self.ui.label(cx, ids!(status)).set_text(cx, &status);
                }
            }
            if let RubyWordAction::Clicked { word, gloss } = action.cast() {
                let status = format!("注音词：{} → {}", word.trim_end_matches(['.', ',']), gloss);
                self.ui.label(cx, ids!(status)).set_text(cx, &status);
            }
        }
        let shelf_list = self
            .ui
            .widget(cx, ids!(book_shelf))
            .portal_list(cx, ids!(list));
        let pages = self.ui.page_flip(cx, ids!(pages));
        for (item_id, item) in shelf_list.items_with_actions(actions) {
            if item.button(cx, ids!(del_btn)).clicked(actions) {
                let book_id = SHELF.read().unwrap().get(item_id).map(|b| b.id.clone());
                if let Some(id) = book_id {
                    spawn_delete(id);
                }
            }
            if item.button(cx, ids!(read_btn)).clicked(actions) {
                let book = SHELF.read().unwrap().get(item_id).cloned();
                if let Some(book) = book {
                    self.ui
                        .label(cx, ids!(title))
                        .set_text(cx, &format!("《{}》加载中…", book.title));
                    pages.set_active_page(cx, id!(page_reader));
                    spawn_open_book(book);
                }
            }
        }
        if self.ui.button(cx, ids!(nav_library)).clicked(actions) {
            pages.set_active_page(cx, id!(page_library));
        }
        if self.ui.button(cx, ids!(nav_reader)).clicked(actions) {
            pages.set_active_page(cx, id!(page_reader));
        }
        if self.ui.button(cx, ids!(nav_vocab)).clicked(actions) {
            pages.set_active_page(cx, id!(page_vocab));
        }
        if self.ui.button(cx, ids!(nav_settings)).clicked(actions) {
            pages.set_active_page(cx, id!(page_settings));
        }
        if self.ui.button(cx, ids!(prev_btn)).clicked(actions) {
            if self.chapter_idx > 0 && CURRENT_BOOK.read().unwrap().is_some() {
                let idx = self.chapter_idx - 1;
                self.set_chapter(cx, idx);
            }
        }
        if self.ui.button(cx, ids!(next_btn)).clicked(actions) {
            if CURRENT_BOOK.read().unwrap().is_some() {
                let idx = self.chapter_idx + 1;
                self.set_chapter(cx, idx);
            }
        }
        if self.ui.button(cx, ids!(stream_button)).clicked(actions) {
            self.stream_text = "▶ ".to_string();
            self.ui
                .label(cx, ids!(stream_label))
                .set_text(cx, &self.stream_text);
            spawn_stream_demo();
        }
        if self.ui.button(cx, ids!(reload_button)).clicked(actions) {
            self.reload_count += 1;
            *PARAGRAPHS.write().unwrap() = sample_chapter::build_paragraphs(PARA_COUNT);
            let status = format!("已重新渲染 {} 次", self.reload_count);
            self.ui.label(cx, ids!(status)).set_text(cx, &status);
            self.ui.redraw(cx);
        }
    }
}

impl AppMain for App {
    fn script_mod(vm: &mut ScriptVm) -> ScriptValue {
        makepad_widgets::theme_mod(vm);
        script_eval!(vm, {
            mod.theme = mod.themes.dark
        });
        makepad_widgets::widgets_mod(vm);
        crate::ruby_word::script_mod(vm);
        self::script_mod(vm)
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event) {
        match event {
            Event::Startup => spawn_db_init(),
            // 拖入悬停：声明接受拷贝，否则系统不触发 Drop。
            Event::Drag(de) => {
                if de.items.iter().any(is_epub_item) {
                    *de.response.lock().unwrap() = DragResponse::Copy;
                }
            }
            // 松手放下：收集 epub 路径后台导入。
            Event::Drop(de) => {
                let paths: Vec<String> = de
                    .items
                    .iter()
                    .filter_map(|it| match it {
                        DragItem::FilePath { path, .. } if is_epub_path(path) => {
                            Some(percent_decode_path(path))
                        }
                        _ => None,
                    })
                    .collect();
                if !paths.is_empty() {
                    spawn_import(paths);
                }
            }
            _ => {}
        }
        self.match_event(cx, event);
        self.ui.handle_event(cx, event, &mut Scope::empty());
    }
}
