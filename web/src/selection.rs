use crate::settings::{MARKER, Preferences};
use topcoat::{
    Result,
    view::{Unescaped, View, component, view},
};

#[component]
pub async fn icon(name: &str) -> Result<impl View> {
    let svg = match name {
        "explain" => include_str!("../../app/resources/icons/selection_explain.svg"),
        "define" => include_str!("../../app/resources/icons/selection_define.svg"),
        "more" => include_str!("../../app/resources/icons/selection_more.svg"),
        "marker" => include_str!("../../app/resources/icons/selection_marker.svg"),
        "pen" => include_str!("../../app/resources/icons/selection_pen.svg"),
        _ => include_str!("../../app/resources/icons/selection_send.svg"),
    };
    Ok(view! { (Unescaped::new_unchecked(svg)) })
}

#[component]
pub async fn toolbar(preferences: &Preferences) -> Result<impl View> {
    Ok(view! {
        <div
            id="selection-toolbar"
            class="selection-toolbar"
            role="toolbar"
            aria-label="选文操作"
            hidden=(true)
        >
            <div class="selection-scroll">
                <div class="selection-actions">
                    <button type="button" id="ask-selection">"问 AI"</button>
                    <span class="selection-divider"></span>
                    <button
                        type="button"
                        data-selection-question="请解释这段文字，并指出重要词语和表达。"
                    >
                        icon(name: "explain")
                        <span>"解释"</span>
                    </button>
                    <button
                        type="button"
                        data-selection-question="请解释所选词语的含义、词性和在此处的用法。"
                    >
                        icon(name: "define")
                        <span>"释义"</span>
                    </button>
                    <button
                        type="button"
                        id="more-selection"
                        aria-label="更多选文操作"
                        aria-expanded="false"
                    >
                        icon(name: "more")
                    </button>
                    <button
                        type="button"
                        id="toggle-highlight"
                        aria-label="添加或移除高亮"
                    >
                        icon(name: "marker")
                    </button>
                    for (index, &(label, colour)) in MARKER.iter().enumerate() {
                        <button
                            type="button"
                            class="selection-colour"
                            data-marker=(index)
                            aria-label=(format!("高亮颜色：{label}"))
                            aria-pressed=(preferences.highlight_color_slot == index + 4)
                            style=(format!("--marker:{colour}"))
                        >
                            <i></i>
                        </button>
                    }
                    <button type="button" id="compose-note" aria-label="添加笔记">
                        icon(name: "pen")
                    </button>
                </div>
                <section id="selection-ai" hidden=(true)>
                    <form id="selection-question-form">
                        <input
                            id="selection-question"
                            aria-label="向 AI 提问"
                            placeholder="关于这段选文提问…"
                            maxlength="2000"
                        >
                        <button type="button" data-selection-utility="web">
                            "Web"
                        </button>
                        <button type="submit" aria-label="发送问题">
                            icon(name: "send")
                        </button>
                    </form>
                    <div class="selection-suggestions">
                        <button
                            type="button"
                            data-selection-question="为什么在这里使用这个词？"
                        >
                            "为什么用这个词？"
                        </button>
                        <button
                            type="button"
                            data-selection-question="这个词源自哪里？请解释它的词源。"
                        >
                            "这个词源自哪里？"
                        </button>
                        <button
                            type="button"
                            data-selection-question="请用所选词语造句，并解释用法。"
                        >
                            "用它造个句子"
                        </button>
                    </div>
                </section>
                <section id="selection-more" hidden=(true)>
                    <button
                        type="button"
                        data-selection-question="请解释所选词语的含义、词性和在此处的用法。"
                    >
                        "定义与用法"
                    </button>
                    <button type="button" data-selection-utility="fact">
                        "事实核查"
                    </button>
                    <button type="button" data-selection-utility="related">
                        "查找书中相关内容"
                    </button>
                    <button
                        type="button"
                        data-selection-question="请提出对所选观点最有力的反驳，区分事实、假设与意见。"
                    >
                        "提出反驳"
                    </button>
                    <button type="button" data-selection-utility="quote">
                        "复制引文及出处"
                    </button>
                    <div class="selection-utilities">
                        <button type="button" id="copy-selection">"复制"</button>
                        <button type="button" id="save-excerpt">"保存摘录"</button>
                        <button type="button" data-selection-utility="vocabulary">
                            "生词"
                        </button>
                        <button type="button" data-selection-utility="translate">
                            "翻译"
                        </button>
                        <button type="button" data-selection-utility="speak">
                            "发音"
                        </button>
                        <button type="button" data-selection-utility="remove">
                            "移除高亮"
                        </button>
                        <button
                            type="button"
                            id="dismiss-selection"
                            aria-label="取消选文"
                        >
                            "关闭"
                        </button>
                    </div>
                    <p class="selection-disclaimer">
                        "翻译与事实核查会在外部网站打开选文。"
                    </p>
                </section>
                <p id="selection-hint" role="status" hidden=(true)></p>
            </div>
        </div>
        <dialog id="note-dialog" class="panel-dialog">
            <div class="dialog-heading">
                <h2>"添加笔记"</h2>
                <button class="button quiet" data-close="" type="button">
                    "取消"
                </button>
            </div>
            <form id="note-form">
                <pre id="note-quote" class="excerpt-text"></pre>
                <label class="note-label">
                    "我的想法"
                    <textarea
                        id="note-comment"
                        maxlength="12000"
                        rows="5"
                        placeholder="写下想法（可选）"
                    ></textarea>
                </label>
                <button class="button primary" type="submit">"保存笔记"</button>
                <p id="note-status" role="status"></p>
            </form>
        </dialog>
        <dialog id="ai-dialog" class="panel-dialog">
            <div class="dialog-heading">
                <h2>"阅读助教"</h2>
                <button class="button quiet" data-close="" type="button">
                    "关闭"
                </button>
            </div>
            <blockquote id="ai-quote"></blockquote>
            <p id="ai-prompt" class="muted"></p>
            <pre id="ai-answer" class="excerpt-text" role="status"></pre>
            <p>
                <a class="button" href="/settings#ai-settings-form">
                    "配置 AI 接口"
                </a>
            </p>
        </dialog>
        <dialog id="search-dialog" class="panel-dialog">
            <div class="dialog-heading">
                <h2>"书内搜索"</h2>
                <button class="button quiet" data-close="" type="button">
                    "关闭"
                </button>
            </div>
            <p id="search-status" role="status"></p>
            <div id="search-results"></div>
        </dialog>
    })
}
