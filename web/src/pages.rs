use crate::{
    i18n::{t, text, translate},
    library::Location,
    routes::{self, BookId, ChapterNo},
    settings::{BACKGROUND, GLOSS, Preferences, WORD},
};
use topcoat::{
    Result,
    asset::{Asset, asset},
    context::{Cx, request_context},
    router::{
        error::not_found, error::see_other, href, page, path_param, response::IntoResponse, route,
    },
    runtime::{Signal, signal},
    view::{Child, Unescaped, View, component, view},
};

pub(super) const STYLE: Asset = asset!("./style.css");
pub(super) const SCRIPT: Asset = asset!("./browser.js");
pub(super) const APPEARANCE_SCRIPT: Asset = asset!("./appearance.js");
pub(super) const I18N_SCRIPT: Asset = asset!("./i18n.js");
pub(super) const COLOURS_SCRIPT: Asset = asset!("./custom-colours.js");
pub(super) const PAGER_SCRIPT: Asset = asset!("./pager.js");
pub(super) const SELECTION_SCRIPT: Asset = asset!("./selection.js");
pub(super) const HIGHLIGHT_SCRIPT: Asset = asset!("./highlight.mjs");

#[component]
async fn shell(
    cx: &Cx,
    title: &str,
    active: &str,
    #[default] book_title: bool,
    #[default] child: Child<'_>,
) -> Result<impl View> {
    let preferences = request_context::<Preferences>(cx);
    Ok(view! {
        <!DOCTYPE html>
        <html
            lang=(&preferences.language)
            dir=(topcoat::l10n::locale(cx).id().direction())
        >
            <head>
                <meta charset="utf-8">
                <meta
                    name="viewport"
                    content="width=device-width, initial-scale=1, viewport-fit=cover"
                >
                <meta name="color-scheme" content=(&preferences.theme)>
                <title data-i18n-title=(if book_title { None } else { Some(title) })>
                    (if book_title { title.to_owned() } else { translate(cx, title) })
                    " · ReadWise"
                </title>
                <link rel="stylesheet" href=(STYLE)>
                <script type="module" src=(SCRIPT)></script>
                topcoat::dev::script()
                topcoat::runtime::script()
            </head>
            <body
                style=(preferences.ui_style(active == "reader"))
                data-preferences=(serde_json::to_string(&preferences)?)
                data-ui-messages=(serde_json::to_string(&*crate::i18n::CATALOGS)?)
                data-appearance-src=(APPEARANCE_SCRIPT)
                data-i18n-src=(I18N_SCRIPT)
                data-colours-src=(COLOURS_SCRIPT)
                data-selection-src=(SELECTION_SCRIPT)
                data-highlight-src=(HIGHLIGHT_SCRIPT)
            >
                <a class="skip-link" href="#main-content">(text(cx, "跳到正文"))</a>
                if active != "reader" {
                    <header class="app-header">
                        <a class="brand" href=(href!(home))>
                            "ReadWise"
                            <small>"WEB"</small>
                        </a>
                        <nav
                            aria-label=(translate(cx, "主导航"))
                            data-i18n-aria-label="主导航"
                        >
                            <a href=(href!(continue_reading))>
                                nav_icon(name: "read")
                                <span>(text(cx, "阅读"))</span>
                            </a>
                            <a
                                href=(href!(home))
                                aria-current=(if active == "books" {
                                    Some("page")
                                } else {
                                    None
                                })
                            >
                                nav_icon(name: "books")
                                <span>(text(cx, "书架"))</span>
                            </a>
                            <a
                                href=(href!(vocabulary))
                                aria-current=(if active == "vocabulary" {
                                    Some("page")
                                } else {
                                    None
                                })
                            >
                                nav_icon(name: "words")
                                <span>(text(cx, "词汇"))</span>
                            </a>
                            <a
                                href=(href!(settings))
                                aria-current=(if active == "settings" {
                                    Some("page")
                                } else {
                                    None
                                })
                            >
                                nav_icon(name: "person")
                                <span>(text(cx, "我"))</span>
                            </a>
                        </nav>
                    </header>
                }
                (child)
                <div id="toast" role="status" aria-live="polite" hidden=(true)></div>
            </body>
        </html>
    })
}

#[component]
async fn nav_icon(name: &str) -> Result<impl View> {
    let path = match name {
        "read" => {
            "M3 4h6a3 3 0 0 1 3 3v14a4 4 0 0 0-4-3H3V4Zm18 0h-6a3 3 0 0 0-3 3v14a4 4 0 0 1 4-3h5V4Z"
        }
        "books" => "M4 3h4v18H4V3Zm6 0h4v18h-4V3Zm6 1 4-1 4 17-4 1-4-17Z",
        "words" => "M3 20 10 3l7 17M6 13h8M17 8h5m-2-3v9m-3 3 5 4m0-4-5 4",
        _ => "M16 7a4 4 0 1 1-8 0 4 4 0 0 1 8 0ZM4 21v-2a8 8 0 0 1 16 0v2",
    };
    Ok(view! {
        <svg
            viewBox="0 0 24 24"
            width="18"
            height="18"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
        >
            <path d=(path)></path>
        </svg>
    })
}

#[page("/")]
async fn home(cx: &Cx) -> Result<impl View> {
    let books = routes::state(cx)
        .call(|library| library.db.get_shelf_items())
        .await?;
    let count = books.len();
    Ok(view! {
        shell(
            title: "书架",
            active: "books",
            <main
                id="main-content"
                class="page shelf-page"
                data-shelf-src=(crate::shelf::SCRIPT)
            >
                <section class="page-heading">
                    <div>
                        <p class="eyebrow">"YOUR READING SPACE"</p>
                        <h1>(text(cx, "书架"))</h1>
                        <p
                            id="shelf-count"
                            class="muted"
                            data-i18n-format="shelf_count"
                            data-i18n-args=(serde_json::json!({"count": count}).to_string(

                            ))
                        >
                            (t!(shelf_count, count))
                        </p>
                    </div>
                    <label class="button primary upload-button">
                        (text(cx, "＋ 导入书籍"))
                        <input
                            id="book-upload"
                            type="file"
                            accept=".epub,.pdf"
                            multiple=(true)
                        >
                    </label>
                </section>
                <div class="shelf-toolbar">
                    <p class="muted small">
                        (text(
                            cx,
                            "支持 EPUB / PDF，每本最大 200 MB。文件保存在当前 Web 服务的数据目录。",
                        ))
                    </p>
                    <div
                        class="shelf-view-switch"
                        role="group"
                        aria-label=(translate(cx, "书架视图"))
                        data-i18n-aria-label="书架视图"
                    >
                        <button
                            type="button"
                            class="button"
                            data-shelf-view="list"
                            aria-pressed="false"
                        >
                            (text(cx, "列表"))
                        </button>
                        <button
                            type="button"
                            class="button"
                            data-shelf-view="grid"
                            aria-pressed="true"
                        >
                            (text(cx, "网格"))
                        </button>
                    </div>
                </div>
                <ol id="import-results" class="import-results" aria-live="polite"></ol>
                if books.is_empty() {
                    <section class="empty-state">
                        <span class="empty-icon" aria-hidden="true">"本"</span>
                        <h2>(text(cx, "从一本书开始"))</h2>
                        <p>
                            (text(
                                cx,
                                "导入书籍后，即可阅读、显示生词注音，并建立自己的词汇本。",
                            ))
                        </p>
                    </section>
                } else {
                    <div class="book-grid">
                        for book in books {
                            <article
                                class="book-card"
                                data-book-id=(&book.id)
                                data-title=(&book.title)
                                data-author=(&book.author)
                                data-want-to-read=(book.want_to_read.to_string())
                                data-tags=(serde_json::to_string(&book.tags)?)
                            >
                                <a
                                    class="book-link"
                                    href=(href!(resume_reader, BookId(&book.id)))
                                >
                                    <div class="cover">
                                        if book.cover_abs_path.is_some() {
                                            <img
                                                src=(format!("/cover/{}", book.id))
                                                alt=""
                                                loading="lazy"
                                            >
                                        } else {
                                            <span class="cover-title">(&book.title)</span>
                                        }
                                    </div>
                                    <div class="book-details">
                                        <h2>(&book.title)</h2>
                                        <p class="muted">(&book.author)</p>
                                    </div>
                                </a>
                                <div class="book-footer">
                                    <span class="book-status">
                                        (crate::shelf::localized_status(cx, &book))
                                    </span>
                                    <button
                                        type="button"
                                        class="button quiet book-more"
                                        aria-label=(format!(
                                            "{}: {}",
                                            translate(cx, "更多操作"),
                                            book.title,
                                        ))
                                        aria-haspopup="dialog"
                                        aria-controls="book-menu"
                                    >
                                        "⋯"
                                    </button>
                                </div>
                                <p class="book-tags muted small">
                                    (if book.want_to_read {
                                        format!("{} · ", translate(cx, "欲读"))
                                    } else {
                                        String::new()
                                    })
                                    (book.tags.join(" · "))
                                </p>
                            </article>
                        }
                    </div>
                }
            </main>
            crate::shelf::dialogs()
        )
    })
}

#[route(GET "/continue")]
async fn continue_reading(cx: &Cx) -> Result<impl IntoResponse> {
    let id = routes::state(cx)
        .call(|library| {
            Ok(library
                .db
                .get_shelf_items()?
                .into_iter()
                .max_by_key(|book| book.last_read_at)
                .map(|book| book.id))
        })
        .await?;
    Ok(see_other(
        id.map(|id| format!("/read/{id}"))
            .unwrap_or_else(|| "/".into()),
    ))
}

#[route(GET "/read/{book_id}")]
async fn resume_reader(cx: &Cx) -> Result<impl IntoResponse> {
    let id = routes::book_id(cx)?;
    let url = routes::state(cx)
        .call(move |library| {
            let Some(book) = library.book(&id)? else {
                return Ok(None);
            };
            let content = library.content(&book)?;
            let location = library
                .db
                .get_status(&id)?
                .map(|status| Location::decode(&status.location))
                .unwrap_or_default();
            Ok(Some(format!(
                "/read/{id}/{}",
                location
                    .chapter
                    .min(content.chapters.len().saturating_sub(1))
            )))
        })
        .await?
        .ok_or_else(not_found)?;
    Ok(see_other(url))
}

#[page("/read/{book_id}/{chapter_no}")]
async fn read_chapter(cx: &Cx) -> Result<impl View> {
    let id = routes::book_id(cx)?;
    let chapter = *path_param::<ChapterNo>(cx)?;
    let preferences = request_context::<Preferences>(cx).clone();
    let reader = routes::state(cx)
        .call(move |library| library.reader_with_preferences(&id, chapter, preferences))
        .await?
        .ok_or_else(not_found)?;
    let id = reader.book.id.clone();
    let title = reader.book.title.clone();
    let preferences = reader.preferences;
    Ok(view! {
        shell(
            title: &title,
            active: "reader",
            book_title: true,
            <header class="reader-toolbar">
                <a
                    class="button quiet"
                    href=(href!(home))
                    aria-label=(translate(cx, "返回书架"))
                    data-i18n-aria-label="返回书架"
                >
                    (text(cx, "← 书架"))
                </a>
                <span class="reader-book-title">(&title)</span>
                <a class="button quiet" href=(href!(excerpts, BookId(&id)))>
                    (text(cx, "摘录"))
                </a>
                <button type="button" class="button quiet" data-open="toc-dialog">
                    (text(cx, "目录"))
                </button>
                <button
                    type="button"
                    class="button quiet"
                    data-open="appearance-dialog"
                    aria-label=(translate(cx, "阅读外观"))
                    data-i18n-aria-label="阅读外观"
                >
                    "aA"
                </button>
            </header>
            <main
                id="main-content"
                class="reader-main"
                data-reader-main=""
                data-turn-mode=(&preferences.turn_mode)
                data-text-width=(&preferences.text_width)
            >
                <div id="reader-viewport">
                    <article
                        id="reading-content"
                        class="reading-content"
                        style=(preferences.style())
                        data-book-id=(&id)
                        data-pager-src=(PAGER_SCRIPT)
                        data-preferences=(serde_json::to_string(&preferences)?)
                        data-turn-mode=(&preferences.turn_mode)
                        data-columns=(&preferences.columns)
                        data-chapter=(reader.chapter)
                        data-total=(reader.total)
                        data-resume-block=(reader.resume.block)
                        data-resume-offset=(reader.resume.offset)
                        data-gloss-enabled=(if preferences.gloss_enabled {
                            "true"
                        } else {
                            "false"
                        })
                    >
                        <p
                            class="chapter-kicker"
                            data-i18n-format="chapter_position"
                            data-i18n-args=(serde_json::json!({"chapter": reader.chapter + 1, "total": reader.total}).to_string(

                            ))
                        >
                            (t!(
                                chapter_position,
                                chapter = reader.chapter + 1,
                                total = reader.total,
                            ))
                        </p>
                        for (index, block) in reader.blocks.into_iter().enumerate() {
                            <section
                                class="reader-block"
                                data-block=(index)
                                id=(format!("web-block-{index}"))
                            >
                                (Unescaped::new_unchecked(block))
                            </section>
                        }
                    </article>
                </div>
                <nav
                    id="page-navigation"
                    class="page-navigation"
                    aria-label=(translate(cx, "正文翻页"))
                    data-i18n-aria-label="正文翻页"
                    hidden=(true)
                >
                    <button type="button" class="button quiet" id="previous-page">
                        (text(cx, "← 上一页"))
                    </button>
                    <output id="page-indicator"></output>
                    <button type="button" class="button quiet" id="next-page">
                        (text(cx, "下一页 →"))
                    </button>
                </nav>
                <footer class="chapter-navigation">
                    if reader.chapter > 0 {
                        <a
                            class="button"
                            href=(href!(
                                read_chapter,
                                BookId(&id),
                                ChapterNo(reader.chapter - 1),
                            ))
                        >
                            (text(cx, "← 上一章"))
                        </a>
                    } else {
                        <span></span>
                    }
                    <span class="muted">(reader.title)</span>
                    if reader.chapter + 1 < reader.total {
                        <a
                            class="button"
                            href=(href!(
                                read_chapter,
                                BookId(&id),
                                ChapterNo(reader.chapter + 1),
                            ))
                        >
                            (text(cx, "下一章 →"))
                        </a>
                    } else {
                        <span class="muted">(text(cx, "本书完"))</span>
                    }
                </footer>
            </main>
            crate::selection::toolbar(preferences: &preferences)
            <dialog id="toc-dialog" class="panel-dialog">
                <div class="dialog-heading">
                    <h2>(text(cx, "目录"))</h2>
                    <button class="button quiet" data-close="" type="button">
                        (text(cx, "关闭"))
                    </button>
                </div>
                <nav
                    class="toc"
                    aria-label=(translate(cx, "章节目录"))
                    data-i18n-aria-label="章节目录"
                >
                    for (label, level, index, fragment) in reader.toc {
                        <a
                            href=(href!(read_chapter, BookId(&id), ChapterNo(index)).fragment(
                                fragment
                                    .map(|f| format!("content-{f}"))
                                    .unwrap_or_default(),
                            ))
                            style=(format!("--level:{level}"))
                            aria-current=(if index == chapter {
                                Some("page")
                            } else {
                                None
                            })
                        >
                            (label)
                        </a>
                    }
                </nav>
            </dialog>
            <dialog id="appearance-dialog" class="panel-dialog appearance-dialog">
                <div class="dialog-heading">
                    <h2>(text(cx, "阅读外观"))</h2>
                    <button class="button quiet" data-close="" type="button">
                        (text(cx, "取消"))
                    </button>
                </div>
                appearance(preferences: &preferences)
            </dialog>
            <dialog id="word-card" class="word-card">
                <div class="dialog-heading">
                    <h2 id="card-word"></h2>
                    <button
                        class="button quiet"
                        data-close=""
                        type="button"
                        aria-label=(translate(cx, "关闭词卡"))
                        data-i18n-aria-label="关闭词卡"
                    >
                        "✕"
                    </button>
                </div>
                <p id="card-gloss"></p>
                <div class="card-actions">
                    <button type="button" class="button primary" id="mark-known">
                        (text(cx, "标为已认识"))
                    </button>
                    <button type="button" class="button" id="add-vocabulary">
                        (text(cx, "加入词汇本"))
                    </button>
                </div>
            </dialog>
        )
    })
}

#[page("/excerpts/{book_id}")]
async fn excerpts(cx: &Cx) -> Result<impl View> {
    let id = routes::book_id(cx)?;
    let data = routes::state(cx)
        .call(move |library| {
            let Some(book) = library.book(&id)? else {
                return Ok(None);
            };
            let notes = library.db.get_notes_by_book(&id)?;
            Ok(Some((book, notes)))
        })
        .await?;
    let (book, notes) = data.ok_or_else(not_found)?;
    Ok(view! {
        shell(
            title: "摘录笔记",
            active: "reader",
            <main id="main-content" class="page excerpts-page">
                <a class="button quiet" href=(href!(resume_reader, BookId(&book.id)))>
                    (text(cx, "← 返回阅读"))
                </a>
                <h1>(text(cx, "摘录笔记"))</h1>
                <p class="muted">(book.title)</p>
                if notes.is_empty() {
                    <p>
                        (text(
                            cx,
                            "选中正文后，使用悬浮工具栏保存摘录。",
                        ))
                    </p>
                }
                for note in notes {
                    <article class="excerpt-entry" data-note-id=(note.id)>
                        <pre class="excerpt-text">(note.content)</pre>
                    </article>
                }
            </main>
        )
    })
}

#[component]
async fn appearance(
    cx: &Cx,
    preferences: &Preferences,
    #[default] global: bool,
) -> Result<impl View> {
    let ann_fg = signal(cx, || GLOSS[preferences.gloss_color].1.to_owned());
    let ann_bg = signal(cx, || BACKGROUND[preferences.gloss_background].1.to_owned());
    let word_fg = signal(cx, || WORD[preferences.word_color].1.to_owned());
    let word_bg = signal(cx, || BACKGROUND[preferences.word_background].1.to_owned());
    let initial = serde_json::to_string(preferences)?;
    Ok(view! {
        <section
            class="appearance"
            data-appearance=""
            data-initial=(initial)
            data-defaults=(serde_json::to_string(&Preferences::default())?)
            data-global=(global)
            :style=$((
                "--gloss-color:",
                ann_fg.get(),
                ";--gloss-background:",
                ann_bg.get(),
                ";--word-color:",
                word_fg.get(),
                ";--word-background:",
                word_bg.get(),
            ))
        >
            if global {
                crate::controls::global_controls(preferences: &preferences)
            }
            <div
                class="appearance-tabs"
                role="tablist"
                aria-label=(translate(cx, "阅读设置分区"))
                data-i18n-aria-label="阅读设置分区"
                hidden=(global)
            >
                <button
                    type="button"
                    role="tab"
                    id="tab-theme"
                    aria-selected="true"
                    aria-controls="appearance-theme"
                    data-settings-tab="theme"
                >
                    (text(cx, "主题"))
                </button>
                <button
                    type="button"
                    role="tab"
                    id="tab-text"
                    aria-selected="false"
                    aria-controls="appearance-text"
                    data-settings-tab="text"
                    tabindex="-1"
                >
                    (text(cx, "文本"))
                </button>
                <button
                    type="button"
                    role="tab"
                    id="tab-annotations"
                    aria-selected="false"
                    aria-controls="appearance-annotations"
                    data-settings-tab="annotations"
                    tabindex="-1"
                >
                    (text(cx, "注音"))
                </button>
            </div>
            <section
                id="appearance-theme"
                class="settings-section"
                data-settings-section="theme"
                role="region"
                aria-label=(translate(cx, "主题设置"))
                data-i18n-aria-label="主题设置"
            >
                <h2 hidden=(!global)>(text(cx, "主题"))</h2>
                crate::controls::theme_controls(preferences: &preferences)
            </section>
            <section
                id="appearance-text"
                class="settings-section"
                data-settings-section="text"
                role="region"
                aria-label=(translate(cx, "文本设置"))
                data-i18n-aria-label="文本设置"
                hidden=(!global)
            >
                <h2 hidden=(!global)>(text(cx, "文本"))</h2>
                crate::controls::text_controls(preferences: &preferences)
            </section>
            <section
                id="appearance-annotations"
                class="settings-section"
                data-settings-section="annotations"
                role="region"
                aria-label=(translate(cx, "注音设置"))
                data-i18n-aria-label="注音设置"
                hidden=(!global)
            >
                <h2 hidden=(!global)>(text(cx, "注音"))</h2>
                <div
                    class="annotation-preview reading-content"
                    data-preview=""
                    style=(preferences.style())
                    data-gloss-enabled=(if preferences.gloss_enabled {
                        "true"
                    } else {
                        "false"
                    })
                >
                    <p>
                        "In the "
                        <ruby>
                            <span class="word">"solitude"</span>
                            <rt>"独处"</rt>
                        </ruby>
                        " of the forest, every "
                        <ruby>
                            <span class="word">"murmur"</span>
                            <rt>"低语"</rt>
                        </ruby>
                        " tells a story."
                    </p>
                </div>
                swatch_row(
                    label: "注音字色",
                    field: "gloss_color",
                    target: &ann_fg,
                    options: GLOSS
                )
                swatch_row(
                    label: "注音背景",
                    field: "gloss_background",
                    target: &ann_bg,
                    options: BACKGROUND
                )
                swatch_row(
                    label: "词字色",
                    field: "word_color",
                    target: &word_fg,
                    options: WORD
                )
                swatch_row(
                    label: "词背景",
                    field: "word_background",
                    target: &word_bg,
                    options: BACKGROUND
                )
                crate::controls::custom_colours()
                <label class="setting-row">
                    <span>(text(cx, "显示注音"))</span>
                    <input
                        name="gloss_enabled"
                        type="checkbox"
                        checked=(preferences.gloss_enabled)
                    >
                </label>
                <label class="setting-row">
                    <span>(text(cx, "注音字号"))</span>
                    <input
                        name="gloss_size"
                        data-scale="1.4"
                        type="range"
                        min="8.4"
                        max="16.8"
                        step="0.1"
                        value=(preferences.gloss_size)
                    >
                    <output data-output="gloss_size">(preferences.gloss_size)</output>
                </label>
                <button
                    type="button"
                    class="button quiet reset-settings"
                    data-reset-settings="annotations"
                >
                    (text(cx, "↶ 重置注音设置"))
                </button>
            </section>
            <p class="small muted">
                (text(
                    cx,
                    "颜色即时预览，点击应用后保存。修改英语水平会重新加载当前章节；其他外观修改不重载正文。",
                ))
            </p>
            <div class="settings-actions">
                <button class="button primary" type="button" data-save-appearance="">
                    (text(cx, "应用并保存"))
                </button>
            </div>
        </section>
    })
}

#[component]
async fn swatch_row(
    cx: &Cx,
    label: &str,
    field: &str,
    target: &Signal<String>,
    options: &[(&str, &str)],
) -> Result<impl View> {
    Ok(view! {
        <div class="swatch-row" data-field=(field)>
            <span>(text(cx, label))</span>
            <div class="swatches">
                for (slot, &(name, value)) in options.iter().enumerate() {
                    <button
                        type="button"
                        class="swatch"
                        title=(translate(cx, name))
                        data-i18n-title-attribute=(name)
                        aria-label=(format!(
                            "{}: {}",
                            translate(cx, label),
                            translate(cx, name),
                        ))
                        data-i18n-aria-label=(format!("{label}：{name}"))
                        data-slot=(slot)
                        data-css-color=(value)
                        style=(format!(
                            "--swatch:{}",
                            if value == "inherit" { "#302F2A" } else { value },
                        ))
                        data-none=(if value == "transparent" { "true" } else { "false" })
                        :data-on=$(target.get() == value.to_owned())
                        :aria-pressed=$(target.get() == value.to_owned())
                        @click=$(|_e| target.set(value.to_owned()))
                    >
                        <span class="visually-hidden">(text(cx, name))</span>
                    </button>
                }
            </div>
        </div>
    })
}

#[page("/settings")]
async fn settings(cx: &Cx) -> Result<impl View> {
    let preferences = request_context::<Preferences>(cx);
    Ok(view! {
        shell(
            title: "我与设置",
            active: "settings",
            <main id="main-content" class="page settings-page">
                <section class="page-heading">
                    <div>
                        <p class="eyebrow">"MAKE IT YOURS"</p>
                        <h1>(text(cx, "阅读设置"))</h1>
                        <p class="muted">
                            (text(cx, "让文字呈现你喜欢的样子。"))
                        </p>
                    </div>
                    <a class="button" href=(href!(statistics))>
                        (text(cx, "阅读记录 →"))
                    </a>
                </section>
                <div class="settings-card">
                    appearance(preferences: &preferences, global: true)
                </div>
                crate::controls::ai_controls()
                <p class="muted small">
                    (text(
                        cx,
                        "Web 预览版 · Topcoat 0.11 · 本机单用户。数据与原生客户端默认独立，暂不提供云同步。",
                    ))
                </p>
            </main>
        )
    })
}

#[page("/vocabulary")]
async fn vocabulary(cx: &Cx) -> Result<impl View> {
    let (entries, known) = routes::state(cx)
        .call(|library| Ok((library.db.vocab_book_list()?, library.db.get_known_words()?)))
        .await?;
    Ok(view! {
        shell(
            title: "词汇",
            active: "vocabulary",
            <main id="main-content" class="page vocabulary-page">
                <section class="page-heading">
                    <div>
                        <p class="eyebrow">"WORDS TO KEEP"</p>
                        <h1>(text(cx, "词汇本"))</h1>
                        <p
                            class="muted"
                            data-i18n-format="vocabulary_counts"
                            data-i18n-args=(serde_json::json!({"saved": entries.len(), "known": known.len()}).to_string(

                            ))
                        >
                            (t!(
                                vocabulary_counts,
                                saved = entries.len(),
                                known = known.len(),
                            ))
                        </p>
                    </div>
                </section>
                if entries.is_empty() {
                    <section class="empty-state">
                        <h2>(text(cx, "把遇见的词留下来"))</h2>
                        <p>
                            (text(
                                cx,
                                "阅读时点击带注音的单词，选择「加入词汇本」。",
                            ))
                        </p>
                    </section>
                }
                <div class="vocabulary-list">
                    for entry in entries {
                        <article class="vocabulary-entry">
                            <div>
                                <h2>(entry.word)</h2>
                                <p>(entry.gloss.unwrap_or_default())</p>
                            </div>
                            <button
                                type="button"
                                class="button quiet"
                                data-remove-vocabulary=(entry.lemma)
                            >
                                (text(cx, "移除"))
                            </button>
                        </article>
                    }
                </div>
                <details class="known-list">
                    <summary
                        data-i18n-format="known_count"
                        data-i18n-args=(serde_json::json!({"count": known.len()}).to_string(

                        ))
                    >
                        (t!(known_count, count = known.len()))
                    </summary>
                    <ul>
                        for lemma in known {
                            <li>
                                <span>(&lemma)</span>
                                <button
                                    type="button"
                                    class="button quiet"
                                    data-unmark-known=(&lemma)
                                >
                                    (text(cx, "恢复注音"))
                                </button>
                            </li>
                        }
                    </ul>
                </details>
            </main>
        )
    })
}

#[page("/statistics")]
async fn statistics(cx: &Cx) -> Result<impl View> {
    let snapshot = routes::state(cx)
        .call(|library| library.db.statistics_snapshot())
        .await?;
    Ok(view! {
        shell(
            title: "阅读记录",
            active: "settings",
            <main id="main-content" class="page">
                <a class="back-link" href=(href!(settings))>
                    (text(cx, "← 返回我与设置"))
                </a>
                <section class="page-heading">
                    <div>
                        <p class="eyebrow">"YOUR COLLECTION"</p>
                        <h1>(text(cx, "阅读记录"))</h1>
                        <p class="muted">
                            (text(cx, "来自当前 Web 数据库的真实统计。"))
                        </p>
                    </div>
                </section>
                <div class="stats-grid">
                    <article>
                        <strong>(snapshot.total_books)</strong>
                        <span>(text(cx, "藏书"))</span>
                    </article>
                    <article>
                        <strong>(snapshot.reading_books)</strong>
                        <span>(text(cx, "在读"))</span>
                    </article>
                    <article>
                        <strong>(snapshot.vocab_total)</strong>
                        <span>(text(cx, "词汇收藏"))</span>
                    </article>
                    <article>
                        <strong>(snapshot.total_notes)</strong>
                        <span>(text(cx, "笔记"))</span>
                    </article>
                </div>
                <p class="muted small">
                    (text(
                        cx,
                        "本阶段尚未接入 Web 阅读计时及已有笔记编辑；不会用模拟数据代替。",
                    ))
                </p>
            </main>
        )
    })
}
