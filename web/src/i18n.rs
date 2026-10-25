//! Explicit interface messages; browser projections are generated from Fluent.
//! Chinese source aliases keep wire values and existing browser action APIs stable.
use std::{collections::BTreeMap, sync::LazyLock};
use topcoat::{
    context::Cx,
    l10n::{Locale, locale},
    view::Unescaped,
};

topcoat::l10n::fluent::messages!(pub(crate) "./l10n/{locale}.ftl");

fn catalog(locale: &Locale) -> BTreeMap<&'static str, String> {
    BTreeMap::from([
        (
            "注音颜色",
            t!(annotation_color).resolve_in(locale).into_owned(),
        ),
        (
            "自定义配色",
            t!(custom_colours).resolve_in(locale).into_owned(),
        ),
        (
            "内嵌取色卡片",
            t!(inline_colour_picker).resolve_in(locale).into_owned(),
        ),
        (
            "收起取色卡片",
            t!(collapse_colour_picker).resolve_in(locale).into_owned(),
        ),
        (
            "饱和度与明度",
            t!(saturation_and_brightness)
                .resolve_in(locale)
                .into_owned(),
        ),
        ("色相", t!(hue).resolve_in(locale).into_owned()),
        ("十六进制", t!(hex).resolve_in(locale).into_owned()),
        (
            "十六进制颜色",
            t!(hex_colour).resolve_in(locale).into_owned(),
        ),
        ("恢复预设", t!(use_preset).resolve_in(locale).into_owned()),
        (
            "推荐色",
            t!(suggested_colours).resolve_in(locale).into_owned(),
        ),
        (
            "我的搭配",
            t!(my_combinations).resolve_in(locale).into_owned(),
        ),
        (
            "♡ 收藏这套配色",
            t!(favorite_this_combination)
                .resolve_in(locale)
                .into_owned(),
        ),
        (
            "喜欢的配色可以收藏在这里。",
            t!(keep_your_favorite_combinations_here)
                .resolve_in(locale)
                .into_owned(),
        ),
        (
            "自定义颜色和收藏在点击「应用并保存」后保存。",
            t!(custom_colours_and_favorites_are_saved_only_with_apply_save)
                .resolve_in(locale)
                .into_owned(),
        ),
        ("列表", t!(list).resolve_in(locale).into_owned()),
        ("网格", t!(grid).resolve_in(locale).into_owned()),
        ("书架视图", t!(library_view).resolve_in(locale).into_owned()),
        ("分享", t!(share).resolve_in(locale).into_owned()),
        (
            "加入欲读清单",
            t!(add_to_want_to_read).resolve_in(locale).into_owned(),
        ),
        (
            "添加到藏书",
            t!(add_to_collection).resolve_in(locale).into_owned(),
        ),
        (
            "标记为已读完",
            t!(mark_as_finished).resolve_in(locale).into_owned(),
        ),
        ("重新命名", t!(rename).resolve_in(locale).into_owned()),
        ("删除", t!(delete).resolve_in(locale).into_owned()),
        ("书名", t!(book_title).resolve_in(locale).into_owned()),
        ("保存", t!(save).resolve_in(locale).into_owned()),
        (
            "删除书籍？",
            t!(delete_book).resolve_in(locale).into_owned(),
        ),
        (
            "确认删除",
            t!(confirm_delete).resolve_in(locale).into_owned(),
        ),
        (
            "阅读进度和书内高亮会一并删除，已保存的摘录笔记保留。不会删除应用外的原始 EPUB / PDF。此操作无法撤销。",
            t!(reading_progress_and_book_highlights_will_be_deleted_saved_excerp)
                .resolve_in(locale)
                .into_owned(),
        ),
        ("阅读", t!(read).resolve_in(locale).into_owned()),
        ("书架", t!(library).resolve_in(locale).into_owned()),
        ("词汇", t!(vocabulary).resolve_in(locale).into_owned()),
        ("我", t!(me).resolve_in(locale).into_owned()),
        ("我与设置", t!(me_settings).resolve_in(locale).into_owned()),
        (
            "阅读设置",
            t!(reading_settings).resolve_in(locale).into_owned(),
        ),
        ("设置", t!(settings).resolve_in(locale).into_owned()),
        (
            "让文字呈现你喜欢的样子。",
            t!(make_reading_your_own).resolve_in(locale).into_owned(),
        ),
        ("主题", t!(theme).resolve_in(locale).into_owned()),
        ("文本", t!(text).resolve_in(locale).into_owned()),
        ("注音", t!(annotations).resolve_in(locale).into_owned()),
        (
            "阅读外观",
            t!(reading_appearance).resolve_in(locale).into_owned(),
        ),
        (
            "界面语言",
            t!(interface_language).resolve_in(locale).into_owned(),
        ),
        ("中文", t!(chinese).resolve_in(locale).into_owned()),
        ("英文", t!(english).resolve_in(locale).into_owned()),
        (
            "英文水平",
            t!(english_level).resolve_in(locale).into_owned(),
        ),
        ("C3-自定义", t!(c3_custom).resolve_in(locale).into_owned()),
        ("外观主题", t!(appearance).resolve_in(locale).into_owned()),
        ("深色", t!(dark).resolve_in(locale).into_owned()),
        ("浅色", t!(light).resolve_in(locale).into_owned()),
        (
            "阅读背景",
            t!(reading_background).resolve_in(locale).into_owned(),
        ),
        ("原色", t!(original).resolve_in(locale).into_owned()),
        ("静谧", t!(quiet).resolve_in(locale).into_owned()),
        ("纸白", t!(paper).resolve_in(locale).into_owned()),
        ("米黄", t!(sepia).resolve_in(locale).into_owned()),
        ("浅绿", t!(calm).resolve_in(locale).into_owned()),
        ("雪白", t!(snow).resolve_in(locale).into_owned()),
        ("夜间", t!(night).resolve_in(locale).into_owned()),
        ("跟随主题", t!(follow_theme).resolve_in(locale).into_owned()),
        ("笔颜色", t!(pen).resolve_in(locale).into_owned()),
        ("朱红", t!(vermilion).resolve_in(locale).into_owned()),
        ("钴蓝", t!(cobalt).resolve_in(locale).into_owned()),
        ("苔绿", t!(moss).resolve_in(locale).into_owned()),
        ("石墨", t!(graphite).resolve_in(locale).into_owned()),
        (
            "高亮颜色",
            t!(highlight_color).resolve_in(locale).into_owned(),
        ),
        ("黄", t!(yellow).resolve_in(locale).into_owned()),
        ("绿", t!(green).resolve_in(locale).into_owned()),
        ("粉", t!(pink).resolve_in(locale).into_owned()),
        (
            "经典色（保留）",
            t!(classic_colour_preserved).resolve_in(locale).into_owned(),
        ),
        ("正文字体", t!(font).resolve_in(locale).into_owned()),
        ("正文字号", t!(text_size).resolve_in(locale).into_owned()),
        ("正文行距", t!(line_spacing).resolve_in(locale).into_owned()),
        ("正文宽度", t!(text_width).resolve_in(locale).into_owned()),
        ("窄", t!(narrow).resolve_in(locale).into_owned()),
        ("适中", t!(even).resolve_in(locale).into_owned()),
        ("宽", t!(wide).resolve_in(locale).into_owned()),
        ("对齐", t!(alignment).resolve_in(locale).into_owned()),
        ("左对齐", t!(left).resolve_in(locale).into_owned()),
        ("两端对齐", t!(justified).resolve_in(locale).into_owned()),
        ("段落", t!(paragraphs).resolve_in(locale).into_owned()),
        ("段间留白", t!(spaced).resolve_in(locale).into_owned()),
        ("首行缩进", t!(indented).resolve_in(locale).into_owned()),
        ("正文分栏", t!(columns).resolve_in(locale).into_owned()),
        ("单列", t!(one).resolve_in(locale).into_owned()),
        ("双列", t!(two).resolve_in(locale).into_owned()),
        ("自动", t!(auto).resolve_in(locale).into_owned()),
        ("翻页方式", t!(page_turn).resolve_in(locale).into_owned()),
        ("仿真翻页", t!(page_curl).resolve_in(locale).into_owned()),
        ("左右滑动", t!(slide).resolve_in(locale).into_owned()),
        ("上下滚动", t!(scroll).resolve_in(locale).into_owned()),
        (
            "显示注音",
            t!(show_annotations).resolve_in(locale).into_owned(),
        ),
        (
            "注音字号",
            t!(annotation_size).resolve_in(locale).into_owned(),
        ),
        (
            "注音字色",
            t!(annotation_color_2).resolve_in(locale).into_owned(),
        ),
        (
            "注音背景",
            t!(annotation_background).resolve_in(locale).into_owned(),
        ),
        ("词字色", t!(word_color).resolve_in(locale).into_owned()),
        (
            "词背景",
            t!(word_background).resolve_in(locale).into_owned(),
        ),
        ("青", t!(teal).resolve_in(locale).into_owned()),
        ("蓝", t!(blue).resolve_in(locale).into_owned()),
        ("橙", t!(orange).resolve_in(locale).into_owned()),
        ("灰", t!(gray).resolve_in(locale).into_owned()),
        ("无", t!(none).resolve_in(locale).into_owned()),
        ("默认", t!(default).resolve_in(locale).into_owned()),
        ("奶黄", t!(cream).resolve_in(locale).into_owned()),
        ("薄荷", t!(mint).resolve_in(locale).into_owned()),
        ("雾蓝", t!(mist_blue).resolve_in(locale).into_owned()),
        (
            "↶ 重置主题设置",
            t!(reset_theme).resolve_in(locale).into_owned(),
        ),
        (
            "↶ 重置文本设置",
            t!(reset_text).resolve_in(locale).into_owned(),
        ),
        (
            "↶ 重置注音设置",
            t!(reset_annotations).resolve_in(locale).into_owned(),
        ),
        ("应用并保存", t!(apply_save).resolve_in(locale).into_owned()),
        ("取消", t!(cancel).resolve_in(locale).into_owned()),
        ("关闭", t!(close).resolve_in(locale).into_owned()),
        (
            "阅读设置已保存",
            t!(reading_settings_saved).resolve_in(locale).into_owned(),
        ),
        (
            "载入桌面阅读设置",
            t!(load_desktop_reading_settings)
                .resolve_in(locale)
                .into_owned(),
        ),
        (
            "只读载入此电脑的桌面设置为草稿，点击应用后保存到 Web；不会修改桌面数据库或同步书籍。",
            t!(load_a_read_only_snapshot_from_this_computer_apply_to_save_to_web)
                .resolve_in(locale)
                .into_owned(),
        ),
        (
            "水平越高，注音越少；C3 仅注音有自定义释义的导入词，已认识词不注音。",
            t!(higher_levels_show_fewer_annotations_c3_uses_imported_custom_glos)
                .resolve_in(locale)
                .into_owned(),
        ),
        (
            "字体使用本机已安装版本；不可用时使用同类后备字体。",
            t!(uses_locally_installed_fonts_with_matching_fallbacks_when_unavail)
                .resolve_in(locale)
                .into_owned(),
        ),
        (
            "窄屏自动回退单栏；上下滚动采用单栏，分栏用于分页模式。分页可使用底部按钮或左右方向键，选文时不拦截方向键。",
            t!(columns_apply_to_paged_reading_narrow_screens_and_scrolling_use_o)
                .resolve_in(locale)
                .into_owned(),
        ),
        (
            "颜色即时预览，点击应用后保存。修改英语水平会重新加载当前章节；其他外观修改不重载正文。",
            t!(preview_changes_then_apply_to_save_only_an_english_level_change_r)
                .resolve_in(locale)
                .into_owned(),
        ),
        ("问 AI", t!(ask_ai).resolve_in(locale).into_owned()),
        ("解释", t!(explain).resolve_in(locale).into_owned()),
        ("释义", t!(define).resolve_in(locale).into_owned()),
        ("复制", t!(copy).resolve_in(locale).into_owned()),
        ("保存摘录", t!(save_excerpt).resolve_in(locale).into_owned()),
        ("生词", t!(vocabulary_2).resolve_in(locale).into_owned()),
        ("翻译", t!(translate).resolve_in(locale).into_owned()),
        ("发音", t!(speak).resolve_in(locale).into_owned()),
        (
            "移除高亮",
            t!(remove_highlight).resolve_in(locale).into_owned(),
        ),
        ("添加笔记", t!(add_note).resolve_in(locale).into_owned()),
        ("保存笔记", t!(save_note).resolve_in(locale).into_owned()),
        ("我的想法", t!(my_thoughts).resolve_in(locale).into_owned()),
        (
            "定义与用法",
            t!(definition_usage).resolve_in(locale).into_owned(),
        ),
        ("事实核查", t!(fact_check).resolve_in(locale).into_owned()),
        (
            "查找书中相关内容",
            t!(find_related_in_book).resolve_in(locale).into_owned(),
        ),
        ("提出反驳", t!(counterpoint).resolve_in(locale).into_owned()),
        (
            "复制引文及出处",
            t!(copy_quote_source).resolve_in(locale).into_owned(),
        ),
        (
            "翻译与事实核查会在外部网站打开选文。",
            t!(translation_and_fact_checking_open_your_selection_on_an_external_)
                .resolve_in(locale)
                .into_owned(),
        ),
        (
            "为什么用这个词？",
            t!(why_this_word).resolve_in(locale).into_owned(),
        ),
        (
            "这个词源自哪里？",
            t!(its_etymology).resolve_in(locale).into_owned(),
        ),
        (
            "用它造个句子",
            t!(use_it_in_a_sentence).resolve_in(locale).into_owned(),
        ),
        (
            "阅读助教",
            t!(reading_tutor).resolve_in(locale).into_owned(),
        ),
        (
            "配置 AI 接口",
            t!(configure_ai).resolve_in(locale).into_owned(),
        ),
        (
            "书内搜索",
            t!(search_in_book).resolve_in(locale).into_owned(),
        ),
        (
            "关于这段选文提问…",
            t!(ask_about_this_passage).resolve_in(locale).into_owned(),
        ),
        (
            "写下想法（可选）",
            t!(your_thoughts_optional).resolve_in(locale).into_owned(),
        ),
        ("AI 设置", t!(ai_settings).resolve_in(locale).into_owned()),
        ("接口地址", t!(endpoint).resolve_in(locale).into_owned()),
        ("模型", t!(model).resolve_in(locale).into_owned()),
        (
            "清除已保存密钥",
            t!(clear_saved_key).resolve_in(locale).into_owned(),
        ),
        (
            "保存 AI 设置",
            t!(save_ai_settings).resolve_in(locale).into_owned(),
        ),
        (
            "留空保留已保存的密钥",
            t!(leave_blank_to_keep_the_saved_key)
                .resolve_in(locale)
                .into_owned(),
        ),
        (
            "选文的问 AI、解释和释义共用 OpenAI 兼容接口。仅在点击发送时将选文发送到你配置的服务。",
            t!(ask_ai_explain_and_define_use_your_openai_compatible_service_sele)
                .resolve_in(locale)
                .into_owned(),
        ),
        (
            "密钥已保存（不回传浏览器）",
            t!(key_saved_never_sent_back_to_the_browser)
                .resolve_in(locale)
                .into_owned(),
        ),
        (
            "尚未保存密钥；本机无鉴权模型可留空",
            t!(no_key_saved_leave_blank_for_a_local_service_without_authenticati)
                .resolve_in(locale)
                .into_owned(),
        ),
        (
            "AI 设置已保存",
            t!(ai_settings_saved).resolve_in(locale).into_owned(),
        ),
        (
            "更多选文操作",
            t!(more_selection_actions).resolve_in(locale).into_owned(),
        ),
        (
            "添加或移除高亮",
            t!(add_or_remove_highlight).resolve_in(locale).into_owned(),
        ),
        (
            "添加高亮",
            t!(add_highlight).resolve_in(locale).into_owned(),
        ),
        (
            "取消选文",
            t!(dismiss_selection).resolve_in(locale).into_owned(),
        ),
        (
            "向 AI 提问",
            t!(ask_ai_a_question).resolve_in(locale).into_owned(),
        ),
        (
            "发送问题",
            t!(send_question).resolve_in(locale).into_owned(),
        ),
        (
            "返回书架",
            t!(back_to_library).resolve_in(locale).into_owned(),
        ),
        (
            "关闭词卡",
            t!(close_word_card).resolve_in(locale).into_owned(),
        ),
        (
            "选文操作",
            t!(selection_actions).resolve_in(locale).into_owned(),
        ),
        (
            "阅读设置分区",
            t!(reader_settings_sections).resolve_in(locale).into_owned(),
        ),
        (
            "主题设置",
            t!(theme_settings).resolve_in(locale).into_owned(),
        ),
        (
            "文本设置",
            t!(text_settings).resolve_in(locale).into_owned(),
        ),
        (
            "注音设置",
            t!(annotation_settings).resolve_in(locale).into_owned(),
        ),
        ("词汇本", t!(vocabulary_3).resolve_in(locale).into_owned()),
        (
            "把遇见的词留下来",
            t!(keep_the_words_you_meet).resolve_in(locale).into_owned(),
        ),
        (
            "阅读时点击带注音的单词，选择「加入词汇本」。",
            t!(tap_an_annotated_word_while_reading_and_choose_add_to_vocabulary)
                .resolve_in(locale)
                .into_owned(),
        ),
        (
            "个收藏词 ·",
            t!(saved_words).resolve_in(locale).into_owned(),
        ),
        (
            "个已认识词",
            t!(known_words).resolve_in(locale).into_owned(),
        ),
        (
            "已认识词（",
            t!(known_words_2).resolve_in(locale).into_owned(),
        ),
        (
            "）",
            t!(closing_parenthesis).resolve_in(locale).into_owned(),
        ),
        ("移除", t!(remove).resolve_in(locale).into_owned()),
        (
            "移出词汇本",
            t!(remove_from_vocabulary).resolve_in(locale).into_owned(),
        ),
        (
            "阅读记录",
            t!(reading_history).resolve_in(locale).into_owned(),
        ),
        (
            "← 返回我与设置",
            t!(back_to_me_settings).resolve_in(locale).into_owned(),
        ),
        (
            "来自当前 Web 数据库的真实统计。",
            t!(real_statistics_from_the_current_web_database)
                .resolve_in(locale)
                .into_owned(),
        ),
        ("藏书", t!(books).resolve_in(locale).into_owned()),
        ("在读", t!(reading).resolve_in(locale).into_owned()),
        (
            "词汇收藏",
            t!(saved_words_2).resolve_in(locale).into_owned(),
        ),
        ("笔记", t!(notes).resolve_in(locale).into_owned()),
        (
            "本阶段尚未接入 Web 阅读计时及已有笔记编辑；不会用模拟数据代替。",
            t!(reading_timers_and_editing_existing_notes_are_not_available_yet_n)
                .resolve_in(locale)
                .into_owned(),
        ),
        (
            "跳到正文",
            t!(skip_to_content).resolve_in(locale).into_owned(),
        ),
        (
            "主导航",
            t!(main_navigation).resolve_in(locale).into_owned(),
        ),
        (
            "正文翻页",
            t!(page_navigation).resolve_in(locale).into_owned(),
        ),
        (
            "章节目录",
            t!(table_of_contents).resolve_in(locale).into_owned(),
        ),
        (
            "从一本书开始",
            t!(start_with_a_book).resolve_in(locale).into_owned(),
        ),
        (
            "本书，慢慢读。",
            t!(books_take_your_time).resolve_in(locale).into_owned(),
        ),
        (
            "＋ 导入书籍",
            t!(import_books).resolve_in(locale).into_owned(),
        ),
        (
            "支持 EPUB / PDF，每本最大 200 MB。文件保存在当前 Web 服务的数据目录。",
            t!(epub_pdf_up_to_200_mb_per_book_files_are_stored_in_this_web_serve)
                .resolve_in(locale)
                .into_owned(),
        ),
        (
            "导入书籍后，即可阅读、显示生词注音，并建立自己的词汇本。",
            t!(import_a_book_to_start_reading_with_annotations_and_build_your_vo)
                .resolve_in(locale)
                .into_owned(),
        ),
        (
            "继续阅读",
            t!(continue_reading).resolve_in(locale).into_owned(),
        ),
        (
            "开始阅读",
            t!(start_reading).resolve_in(locale).into_owned(),
        ),
        (
            "选中正文后，使用悬浮工具栏保存摘录。",
            t!(select_text_and_use_the_floating_toolbar_to_save_an_excerpt)
                .resolve_in(locale)
                .into_owned(),
        ),
        (
            "已载入桌面设置草稿，点击应用后保存",
            t!(desktop_settings_loaded_as_a_draft_apply_to_save)
                .resolve_in(locale)
                .into_owned(),
        ),
        (
            "已加入词汇本",
            t!(added_to_vocabulary).resolve_in(locale).into_owned(),
        ),
        (
            "已保存摘录",
            t!(excerpt_saved).resolve_in(locale).into_owned(),
        ),
        ("已保存笔记", t!(note_saved).resolve_in(locale).into_owned()),
        (
            "已添加高亮",
            t!(highlight_added).resolve_in(locale).into_owned(),
        ),
        (
            "已移除高亮",
            t!(highlight_removed).resolve_in(locale).into_owned(),
        ),
        (
            "已标为认识",
            t!(marked_as_known).resolve_in(locale).into_owned(),
        ),
        ("← 书架", t!(library_2).resolve_in(locale).into_owned()),
        ("目录", t!(contents).resolve_in(locale).into_owned()),
        ("摘录", t!(excerpts).resolve_in(locale).into_owned()),
        (
            "摘录笔记",
            t!(excerpt_notes).resolve_in(locale).into_owned(),
        ),
        (
            "← 返回阅读",
            t!(back_to_reading).resolve_in(locale).into_owned(),
        ),
        (
            "← 上一页",
            t!(previous_page).resolve_in(locale).into_owned(),
        ),
        ("下一页 →", t!(next_page).resolve_in(locale).into_owned()),
        (
            "← 上一章",
            t!(previous_chapter).resolve_in(locale).into_owned(),
        ),
        ("下一章 →", t!(next_chapter).resolve_in(locale).into_owned()),
        ("本书完", t!(end_of_book).resolve_in(locale).into_owned()),
        (
            "标为已认识",
            t!(mark_as_known).resolve_in(locale).into_owned(),
        ),
        (
            "恢复注音",
            t!(restore_annotation).resolve_in(locale).into_owned(),
        ),
        (
            "加入词汇本",
            t!(add_to_vocabulary).resolve_in(locale).into_owned(),
        ),
        (
            "阅读记录 →",
            t!(reading_history_2).resolve_in(locale).into_owned(),
        ),
        (
            "更多操作：",
            t!(more_actions).resolve_in(locale).into_owned(),
        ),
        (
            "移出欲读清单",
            t!(remove_from_want_to_read).resolve_in(locale).into_owned(),
        ),
        ("欲读", t!(want_to_read).resolve_in(locale).into_owned()),
        (
            "书籍信息已复制，可粘贴到其他应用分享",
            t!(book_details_copied_paste_them_into_another_app_to_share)
                .resolve_in(locale)
                .into_owned(),
        ),
        (
            "复制失败，请检查浏览器剪贴板权限",
            t!(copy_failed_check_clipboard_permissions)
                .resolve_in(locale)
                .into_owned(),
        ),
        ("标签", t!(tags).resolve_in(locale).into_owned()),
        (
            "用逗号或分号分隔；最多 8 个，每个 24 字符。清空可移除全部标签。",
            t!(separate_with_commas_or_semicolons_up_to_8_tags_24_characters_eac)
                .resolve_in(locale)
                .into_owned(),
        ),
        (
            "只修改应用内书名，不改写原始文件。最多 300 个字符。",
            t!(only_changes_the_title_in_the_app_not_the_original_file_up_to_300)
                .resolve_in(locale)
                .into_owned(),
        ),
        ("饱和度", t!(saturation).resolve_in(locale).into_owned()),
        ("明度", t!(brightness).resolve_in(locale).into_owned()),
        ("自定义", t!(custom).resolve_in(locale).into_owned()),
        (
            "未设置自定义色",
            t!(no_custom_colour).resolve_in(locale).into_owned(),
        ),
        (
            "♥ 已收藏，点击取消",
            t!(favorited_click_to_remove)
                .resolve_in(locale)
                .into_owned(),
        ),
        (
            "最多收藏 32 套配色",
            t!(up_to_32_combinations).resolve_in(locale).into_owned(),
        ),
        (
            "应用配色",
            t!(apply_combination).resolve_in(locale).into_owned(),
        ),
        (
            "删除配色",
            t!(remove_combination).resolve_in(locale).into_owned(),
        ),
        (
            "请输入 #RRGGBB 格式的颜色",
            t!(enter_a_colour_as_rrggbb).resolve_in(locale).into_owned(),
        ),
        (
            "Web 预览版 · Topcoat 0.11 · 本机单用户。数据与原生客户端默认独立，暂不提供云同步。",
            t!(web_preview_topcoat_0_11_single_user_loopback_only_desktop_data_i)
                .resolve_in(locale)
                .into_owned(),
        ),
        ("未读", t!(unread).resolve_in(locale).into_owned()),
        ("读完", t!(finished).resolve_in(locale).into_owned()),
        ("已保存", t!(saved).resolve_in(locale).into_owned()),
        (
            "书籍已删除",
            t!(book_deleted).resolve_in(locale).into_owned(),
        ),
        (
            "更多操作",
            t!(more_actions_2).resolve_in(locale).into_owned(),
        ),
        (
            "正在请求 AI…",
            t!(requesting_ai).resolve_in(locale).into_owned(),
        ),
        ("正在搜索…", t!(searching).resolve_in(locale).into_owned()),
        (
            "已复制选中文本",
            t!(selected_text_copied).resolve_in(locale).into_owned(),
        ),
        (
            "shelf_count",
            t!(shelf_count, count = "{count}")
                .resolve_in(locale)
                .into_owned(),
        ),
        (
            "chapter_position",
            t!(chapter_position, chapter = "{chapter}", total = "{total}")
                .resolve_in(locale)
                .into_owned(),
        ),
        (
            "vocabulary_counts",
            t!(vocabulary_counts, saved = "{saved}", known = "{known}")
                .resolve_in(locale)
                .into_owned(),
        ),
        (
            "known_count",
            t!(known_count, count = "{count}")
                .resolve_in(locale)
                .into_owned(),
        ),
    ])
}

pub static CATALOGS: LazyLock<BTreeMap<&'static str, BTreeMap<&'static str, String>>> =
    LazyLock::new(|| {
        BTreeMap::from([
            ("zh-CN", catalog(&locale!("zh-CN"))),
            ("en", catalog(&locale!("en"))),
        ])
    });

pub fn translate(cx: &Cx, source: &str) -> String {
    let language = topcoat::l10n::locale(cx).id().to_string();
    CATALOGS
        .get(language.as_str())
        .and_then(|messages| messages.get(source))
        .cloned()
        .unwrap_or_else(|| source.to_owned())
}

/// Only opt-in UI nodes carry a translation marker. Never scan arbitrary text.
pub fn text(cx: &Cx, source: &str) -> Unescaped<String> {
    // Both the attribute and translated text are escaped before the fixed wrapper.
    let value = crate::content::escape(&translate(cx, source));
    let source = crate::content::escape(source);
    Unescaped::new_unchecked(format!("<span data-i18n=\"{source}\">{value}</span>"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fluent_catalogs_have_matching_keys_and_keep_unknown_user_text() {
        let english = Cx::default().with(locale!("en"));
        let chinese = Cx::default().with(locale!("zh-CN"));
        assert_eq!(
            CATALOGS["en"].keys().collect::<Vec<_>>(),
            CATALOGS["zh-CN"].keys().collect::<Vec<_>>()
        );
        assert_eq!(translate(&english, "阅读设置"), "Reading Settings");
        assert_eq!(translate(&chinese, "阅读设置"), "阅读设置");
        assert_eq!(
            translate(&english, "Not an interface message 中文🌿"),
            "Not an interface message 中文🌿"
        );
        let position = t!(chapter_position, chapter = 2, total = 8).resolve(&english);
        assert!(position.starts_with("Chapter "));
        assert!(position.contains('2') && position.contains('8'));
    }

    #[test]
    fn ui_text_escapes_both_attribute_and_content() {
        let cx = Cx::default().with(locale!("en"));
        let html = text(&cx, "\"><img src=x onerror=alert(1)> & 中文");
        assert!(!html.contains("<img"));
        assert!(html.contains("&quot;&gt;&lt;img"));
        assert!(html.contains("&amp; 中文</span>"));
    }
}
