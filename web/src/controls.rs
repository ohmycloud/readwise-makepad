//! Shared, accessible controls. Native enum values are the wire/storage values.
use crate::settings::{MARKER, PEN, Preferences, SCENES};
use readwise_core::settings::ReaderFont;
use topcoat::{
    Result,
    view::{View, component, view},
};

#[component]
pub async fn choices(
    label: &str,
    field: &str,
    selected: &str,
    options: &[(&str, &str)],
) -> Result<impl View> {
    Ok(view! {
        <fieldset class="setting-choice">
            <legend>(label)</legend>
            <div class="choice-options">
                for &(value, title) in options {
                    <label>
                        <input
                            type="radio"
                            name=(field)
                            value=(value)
                            checked=(selected == value)
                        >
                        <span>(title)</span>
                    </label>
                }
            </div>
        </fieldset>
    })
}

#[component]
pub async fn range_control(
    label: &str,
    field: &str,
    value: f32,
    min: f32,
    max: f32,
    step: f32,
    #[default(1.0)] scale: f32,
) -> Result<impl View> {
    Ok(view! {
        <label class="setting-row">
            <span>(label)</span>
            <input
                name=(field)
                type="range"
                min=(min)
                max=(max)
                step=(step)
                value=(value)
                data-scale=(scale)
            >
            <output data-output=(field)>(format!("{:.1}", value / scale))</output>
        </label>
    })
}

#[component]
pub async fn global_controls(preferences: &Preferences) -> Result<impl View> {
    Ok(view! {
        <section class="global-reading-settings">
            <h2>"阅读"</h2>
            choices(
                label: "界面语言",
                field: "language",
                selected: &preferences.language,
                options: &[("zh-CN", "中文"), ("en", "英文")]
            )
            <label class="setting-row">
                <span>"英文水平"</span>
                <select name="english_level">
                    for level in ["A1", "A2", "B1", "B2", "C1", "C2", "C3"] {
                        <option
                            value=(level)
                            selected=(preferences.english_level == level)
                        >
                            (if level == "C3" { "C3-自定义" } else { level })
                        </option>
                    }
                </select>
            </label>
            <p class="small muted">
                "水平越高，注音越少；C3 仅注音有自定义释义的导入词，已认识词不注音。"
            </p>
            <button class="button" type="button" data-import-desktop="">
                "载入桌面阅读设置"
            </button>
            <p class="small muted">
                "只读载入此电脑的桌面设置为草稿，点击应用后保存到 Web；不会修改桌面数据库或同步书籍。"
            </p>
        </section>
    })
}

#[component]
pub async fn theme_controls(preferences: &Preferences) -> Result<impl View> {
    Ok(view! {
        choices(
            label: "外观主题",
            field: "theme",
            selected: &preferences.theme,
            options: &[("dark", "深色"), ("light", "浅色")]
        )
        <fieldset class="scene-picker">
            <legend>"阅读背景"</legend>
            <div class="scene-options">
                for &(value, title, paper, ink) in SCENES {
                    <label style=(format!("--scene-paper:{paper};--scene-ink:{ink}"))>
                        <input
                            type="radio"
                            name="background"
                            value=(value)
                            checked=(preferences.background == value)
                        >
                        <span>
                            <b>"Aa"</b>
                            (title)
                        </span>
                    </label>
                }
            </div>
        </fieldset>
        for (field, title, selected, colours) in [
            ("selection_color_slot", "笔颜色", preferences.selection_color_slot, PEN),
            (
                "highlight_color_slot",
                "高亮颜色",
                preferences.highlight_color_slot,
                MARKER,
            ),
        ] {
            <fieldset class="setting-choice">
                <legend>(title)</legend>
                <div class="choice-options colour-options">
                    for (index, &(name, colour)) in colours.iter().enumerate() {
                        <label>
                            <input
                                type="radio"
                                name=(field)
                                value=(index + 4)
                                checked=(selected == index + 4)
                            >
                            <span>
                                <i style=(format!("background:{colour}"))></i>
                                (name)
                            </span>
                        </label>
                    }
                </div>
            </fieldset>
        }
        <button
            type="button"
            class="button quiet reset-settings"
            data-reset-settings="theme"
        >
            "↶ 重置主题设置"
        </button>
    })
}

#[component]
pub async fn text_controls(preferences: &Preferences) -> Result<impl View> {
    Ok(view! {
        <div
            class="annotation-preview reading-content"
            data-preview=""
            style=(preferences.style())
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
        <label class="setting-row">
            <span>"正文字体"</span>
            <select name="font">
                for font in ReaderFont::ALL {
                    <option
                        value=(font.as_str())
                        selected=(preferences.font == font.as_str())
                    >
                        (font.label())
                    </option>
                }
            </select>
        </label>
        <p class="small muted">
            "字体使用本机已安装版本；不可用时使用同类后备字体。"
        </p>
        range_control(
            label: "正文字号",
            field: "font_size",
            value: preferences.font_size,
            min: 16.0,
            max: 40.0,
            step: 2.0,
            scale: 2.0
        )
        range_control(
            label: "正文行距",
            field: "spacing",
            value: preferences.spacing,
            min: 0.8,
            max: 2.5,
            step: 0.05
        )
        choices(
            label: "正文宽度",
            field: "text_width",
            selected: &preferences.text_width,
            options: &[("narrow", "窄"), ("even", "适中"), ("wide", "宽")]
        )
        choices(
            label: "对齐",
            field: "justify_text",
            selected: if preferences.justify_text { "true" } else { "false" },
            options: &[("false", "左对齐"), ("true", "两端对齐")]
        )
        choices(
            label: "段落",
            field: "indent_paragraphs",
            selected: if preferences.indent_paragraphs { "true" } else { "false" },
            options: &[("false", "段间留白"), ("true", "首行缩进")]
        )
        choices(
            label: "正文分栏",
            field: "columns",
            selected: &preferences.columns,
            options: &[("one", "单列"), ("two", "双列"), ("auto", "自动")]
        )
        choices(
            label: "翻页方式",
            field: "turn_mode",
            selected: &preferences.turn_mode,
            options: &[
                ("curl", "仿真翻页"),
                ("slide", "左右滑动"),
                ("scroll", "上下滚动"),
            ]
        )
        <p class="small muted">
            "窄屏自动回退单栏；上下滚动采用单栏，分栏用于分页模式。分页可使用底部按钮或左右方向键，选文时不拦截方向键。"
        </p>
        <button
            type="button"
            class="button quiet reset-settings"
            data-reset-settings="text"
        >
            "↶ 重置文本设置"
        </button>
    })
}

#[component]
pub async fn custom_colours() -> Result<impl View> {
    Ok(view! {
        <section class="custom-colours" aria-label="自定义配色">
            <div class="custom-colour-row">
                for (index, title) in [
                    "注音颜色",
                    "注音背景",
                    "词字色",
                    "词背景",
                ].iter().enumerate() {
                    <button
                        type="button"
                        class="custom-colour"
                        data-custom-colour=(index)
                        aria-expanded="false"
                        aria-controls="inline-colour-picker"
                    >
                        <i class="colour-dot" data-empty="true"></i>
                        <span>(title)</span>
                    </button>
                }
            </div>
            <section
                id="inline-colour-picker"
                class="inline-colour-picker"
                hidden=(true)
                aria-label="内嵌取色卡片"
            >
                <div class="colour-picker-heading">
                    <h3 data-picker-title="">"注音颜色"</h3>
                    <button
                        type="button"
                        class="button quiet"
                        data-close-picker=""
                        aria-label="收起取色卡片"
                    >
                        "×"
                    </button>
                </div>
                <div
                    class="colour-plane"
                    tabindex="0"
                    role="slider"
                    aria-label="饱和度与明度"
                    aria-valuemin="0"
                    aria-valuemax="100"
                    aria-valuenow="100"
                >
                    <span class="colour-cursor"></span>
                </div>
                <label class="colour-hue-label">
                    <span>"色相"</span>
                    <input
                        type="range"
                        class="colour-hue"
                        min="0"
                        max="360"
                        step="1"
                        value="160"
                        aria-label="色相"
                    >
                </label>
                <div class="colour-hex-row">
                    <label>
                        "十六进制"
                        <input
                            type="text"
                            class="colour-hex"
                            maxlength="7"
                            spellcheck="false"
                            autocomplete="off"
                            placeholder="#7FB2A6"
                            aria-label="十六进制颜色"
                        >
                    </label>
                    <button type="button" class="button" data-clear-colour="">
                        "恢复预设"
                    </button>
                </div>
                <p class="colour-error small" role="status" aria-live="polite"></p>
                <p class="small muted">"推荐色"</p>
                <div class="recommended-colours">
                    for colour in [
                        "#7FB2A6",
                        "#62B6FF",
                        "#E8875A",
                        "#8B6948",
                        "#3D6D59",
                        "#3F6F98",
                        "#FFE9A8",
                        "#CFE8D5",
                        "#D3E6F7",
                        "#F6D6CC",
                        "#55534D",
                        "#FFFFFF",
                    ] {
                        <button
                            type="button"
                            data-recommend-colour=(colour)
                            class="recommended-colour"
                            style=(format!("--colour:{colour}"))
                            aria-label=(colour)
                            title=(colour)
                        ></button>
                    }
                </div>
            </section>
            <button
                type="button"
                class="button favorite-toggle"
                data-favorite-toggle=""
                aria-pressed="false"
            >
                "♡ 收藏这套配色"
            </button>
            <div class="favorite-heading">
                <h3>"我的搭配"</h3>
                <span class="small muted" data-favorite-count=""></span>
            </div>
            <div class="favorite-schemes"></div>
            <p class="favorite-empty small muted">
                "喜欢的配色可以收藏在这里。"
            </p>
            <p class="small muted">
                "自定义颜色和收藏在点击「应用并保存」后保存。"
            </p>
        </section>
    })
}

#[component]
pub async fn ai_controls() -> Result<impl View> {
    Ok(view! {
        <section class="settings-card ai-settings">
            <h2>"AI 设置"</h2>
            <p class="small muted">
                "选文的问 AI、解释和释义共用 OpenAI 兼容接口。仅在点击发送时将选文发送到你配置的服务。"
            </p>
            <form id="ai-settings-form">
                <label>
                    "接口地址"
                    <input
                        type="url"
                        name="endpoint"
                        required=(true)
                        autocomplete="off"
                    >
                </label>
                <label>
                    "模型"
                    <input
                        name="model"
                        required=(true)
                        maxlength="256"
                        autocomplete="off"
                    >
                </label>
                <label>
                    "API Key"
                    <input
                        type="password"
                        name="api_key"
                        maxlength="4096"
                        autocomplete="new-password"
                        placeholder="留空保留已保存的密钥"
                    >
                </label>
                <label class="check-line">
                    <input type="checkbox" name="clear_key">
                    "清除已保存密钥"
                </label>
                <button type="submit" class="button">"保存 AI 设置"</button>
                <p id="ai-config-status" class="small muted" role="status"></p>
            </form>
        </section>
    })
}
