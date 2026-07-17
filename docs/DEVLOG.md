# ReadWise Makepad 重写 — 开发日志

> 约定：每次开发会话在此追加一节，记录做了什么、遇到的问题、决策及理由。倒序排列（最新在上）。

## 2026-07-15 — M3 TOC 刀：目录抽屉 + 章节标题

- core：`load_chapters` 返回 `ChapterInfo{title, html}`——标题取消毒后首个 h1-h3 纯文本（40 字截断），fallback"第 N 节"。没用 epub crate 的 ncx toc（NavPoint→spine 下标映射跨书不稳），标题法对技术书足够；若遇标题质量差的书再升级。
- UI：头部左侧"目录"按钮开关左侧抽屉（260px SolidView，默认 visible:false）；TocList widget（PortalList，每项 ButtonFlat）；点章跳转并收起；当前章 "▸" 高亮（CURRENT_CHAPTER AtomicUsize 共享）。
- 坑：抽屉项按钮与头部按钮同名 `toc_btn` 会被深度查找撞车 → 重命名 toc_item_btn。**教训：ids! 是全树深搜，命名必须全局唯一。**
- 待验证：目录开关、点章跳转、当前章高亮、长目录滚动。
- 修复：`▸` 字形主题字体缺失渲染成豆腐块（用户截图）→ 改颜色高亮。**教训：UI 里的装饰字符只用主题字体确有的字形，不确定就用颜色/布局表达。**
- 二次修复：设按钮 draw_text.color 会被 ButtonFlat 的 animator（hover/默认状态动画）在重绘时用主题色覆盖——高亮时有时无（翻页/新书后消失）。改为**行背景高亮**（TocItem 换 SolidView，按项 apply draw_bg.color 青绿/底色），View 背景无动画竞争。**教训：不要 apply 会被 widget animator 管理的属性（Button 的 draw_text/draw_bg），要么改 View 层属性，要么走 animator 状态。**
- 崩溃修复：多章书点目录 abort——`MTLTextureDescriptor height (21510) > 16384`。根因：Para/GlossPara 用 `CachedView`（todo 示例抄来的）把列表项渲染进离屏纹理，超长章节块超过 Metal 纹理上限直接 assert。**文本块不用纹理缓存**：模板根换普通 View（宽 Fill 高 Fit）。**教训：CachedView 只适合小而固定的项（按钮卡片类），任何高度不可控的内容（长文本/表格/代码块）严禁包 CachedView。** 若后续单块布局本身过慢，方案是 split_blocks 里对超长块二次切分，而不是回缓存。

## 2026-07-15 — 阅读页去 chrome（用户反馈：学 iBooks）

- 用户点名：字号/注音色/字体切换不该显式摆在阅读页；上一章/下一章按钮多余。
- 改：阅读页头部只剩 书名 + 状态 + 章内进度（弱化小字灰色）；字号调节移入设置页"阅读"区（A-/A+ + 当前值标签）；上/下章按钮删除——章节导航全靠键盘（空格连读自动跨章、Shift+←/→）与后续 TOC。
- **设计原则沉淀：阅读页保持沉浸，控制项进设置或走手势/键盘（iBooks 范式）。** 后续做 TOC/图片渲染时同样遵守。
- 待验证：阅读页观感、设置页字号调节联动。

## 2026-07-15 — M3 翻页刀：快捷键 + 视口翻屏 + 章内进度 + 字号

- 翻屏：按 PortalList 可见块数翻页（`first_id + visible_items - 1` 重叠一块保上下文），到底自动进下一章、到顶回上一章；`set_first_id_and_scroll` 定位。
- 快捷键（仅阅读页 + 有书打开，`on_reader_page` 标志随导航维护）：空格/PgDn/→ 下屏，PgUp/← 上屏，Shift+←/→ 跨章，Home 回章首。KeyCode 来自 makepad_studio_protocol 再导出。
- 章内进度：`chapter_pos` 标签 `第x/y章 · z%`（z=已读到块/总块），翻屏/切章/滚轮（Event::Scroll）时刷新。
- 字号：A-/A+ 按钮调 `READER_FONT_SIZE`（None=主题默认，8-20 clamp）；ChapterList 绑定时 `script_apply_eval!` 设 Html 的 `font_size`——TextFlow 的标题字号按 font_size 缩放（text_flow.rs:1193），一个旋钮全局生效。
- 已知风险：CachedView 可能缓存旧字号纹理（改字号后需滚动才刷新）——待实测，若发生则换失效策略。
- 待验证：快捷键翻屏手感、跨章、百分比、字号调节即时性。

## 2026-07-15 — 修复：中文文件名拖放导入失败

- macOS 拖放的 `DragItem::FilePath` 是 URL 百分号编码（中文/空格变 %XX），makepad 未解码直接透传 → 文件系统找不到。Drop 处理时 `percent_decode_path`（15 行零依赖）解码后再导入。
- 注意 import 投递箱目录不受影响（直接读目录，无编码问题）。

## 2026-07-15 — M3 修复：真书空白根因 = 未知标签丢内容

- 用户实测：标题/章节数(1/22)加载正确但整章空白、偶发崩溃。根因即 M0 发现的 Html widget 行为：**未知标签连内容整体跳过**（handle_custom_widget 查不到模板就 jump_to_close）——真书章节全部包在 `<section>/<div>` 里，整章被吞。
- 修复（core/epub_meta）：`sanitize_for_reader`——白名单（Html widget 的 32 个 tag，grep some_id! 确认）保留；未知标签（div/section/span/a/figure…）**去壳留瓤**；script/style/head/svg/math/rt/rp 连内容删；注释/声明剔除。`split_blocks` 按标签深度归零切块（替代 `</p>` 粗切，不再把 blockquote/表格切断——疑似崩溃源之一）。
- 3 个单元测试全过（去壳留瓤/删 script/嵌套分块）。坑：raw string 里 `href="#f1"` 的 `"#` 会终止 `r#".."#`，要用 `r##`。
- 待复测：真书章节渲染、崩溃是否消失。

## 2026-07-15 — M3 第一刀：真书阅读闭环

- core：`epub_meta::load_chapter_htmls`（spine 遍历 `get_current_str` → `extract_body` 取 body 内容，空章跳过）；`books::get_status` + Db 包装（恢复进度用）。
- app：书卡加"阅读"按钮 → `spawn_open_book` 后台加载全书章节（逐章 strip_ruby）→ `CURRENT_BOOK` 静态区 → `ReaderAction::Opened{resume_chapter}`。
- `set_chapter`：章节 HTML 按 `</p>` split_inclusive 切块喂 PortalList（保持虚拟化）、标题/页码更新、`set_first_id_and_scroll(0,0)` 回顶、后台存进度（location=章节下标，progress=章/总章）。
- 阅读页头部改为 上一章/页码/下一章；流式测试与重载示例挪到设置页"开发工具"。
- 无书打开时阅读页仍显示 M0 示例章节（兜底）。
- 待验证：点"阅读"能否渲染真书、上下章切换、重启后从上次章节恢复、书架进度百分比变化、真书里的点词（`<a>` 无处不在？不，真书没包链接——点词在真书上暂不可用，M4 注音管线接上才有）。

## 2026-07-15 — M2 完成 ✅

- 收尾三项用户实测通过（真封面/删除/投递箱），已提交 75e947f。
- 编译修复三处（新坑记录）：① Script 派生宏不认字段类型的全限定路径（`std::collections::HashMap` → 必须 use 导入短名，报 `Unexpected field form` + 连锁 E0277/E0599）② `load_image_file_by_path` 收 `&Path` 非 `&str` ③ `#[live(2.0)]` 对 f32 字段要写 `2.0f32`（future-incompat）。
- 枚举 DSL 写法：`fit: Biggest` 运行时报 type mismatch（静默回退默认值），正确为 `fit: ImageFit.Biggest`（examples/uizoo/tab_image 确认；枚举一律 `类型名.变体`，与 skills 里 `@` 前缀说法不同——`@` 是 default 声明用）。
- **M2 关闭。下一步 M3 阅读器**：点击书卡片 → 打开真实 EPUB（迁移老项目 toc_parser/sanitizer 或用 epub crate 取章节 HTML）→ M0 管线渲染 → 进度保存。

## 2026-07-15 — M2 收尾：真封面 + 删除 + 投递箱语义

- M2 第一刀用户验收通过（导入/书架/持久化），已提交 e7964ab。
- 封面：Card 封面区改 `flow: Overlay`（首字色块打底 + `Image{fit: Biggest}` 叠加）；BookShelf 加 `loaded_covers: HashMap<usize, Option<String>>`，路径变化才 `load_image_file_by_path`，无封面 `set_texture(None)` 清旧纹理（PortalList item 跨书复用防串图）。
- 删除：卡片尾部 `del_btn`，`items_with_actions` 拿 item_id → SHELF 查 book id → spawn_delete 后台删+刷新。M2 先直删不确认（书可重新导入）。
- 投递箱：scan_import_dir 成功导入后删除 import/ 源文件——否则删书后重启会被重新扫回。
- 待验证：封面真图显示、无封面书显示首字、删除即时生效、import 目录文件导入后消失。
- tags 复用评估：老项目书库 tags 仅作筛选，当前书架无筛选 UI 消费场景 → 推迟到 M8 整理（MILESTONES 已注）。

## 2026-07-15 — M2 动手：books 迁移 + epub 解析 + 书架 + 拖放导入

- 导入方式定为拖放 + 导入目录（用户拍板）。
- core/books：models（SimpleBook/BookStatus/BookShelfItem，M2 只迁书库需要的，ReadingSession/BookNote 留 M3/M4）、epub_meta（epub crate 解析 title/author/language/cover，`mdata().value` + `get_cover()`）、queries（**import_book 新架构**：Rust 侧解析元数据→copy 文件与封面到 data_dir/books/<id>/→入库，源文件不删；get_shelf_items 拼绝对路径；delete_book 删目录+级联；update_progress；scan_import_dir 扫 data_dir/import/ 按 标题+大小 去重）。Db 加 data_dir 字段 + 5 个书库方法。
- UI：BookShelf widget（deref View + PortalList 卡片列表，封面暂用书名首字色块占位）；page_library 换成书架 + 顶部提示；空态卡片。启动 DbAction::Ready 后 spawn_load_shelf（扫描导入目录 + 读书目）；拖放 Event::Drag 设 DragResponse::Copy 接受、Event::Drop 取 DragItem::FilePath 过滤 .epub → spawn_import → 刷新。ShelfAction 回传计数。
- 核对：std::sync::Mutex → response.lock().unwrap()；DragItem/DragResponse 由 makepad_widgets 可达；epub crate 纯 Rust。
- 待验证：编译；拖 epub 进窗口是否入库并显示卡片；重启后书架持久；放文件到 ~/Library/Application Support/ReadWise/import/ 重启是否收录；封面占位显示首字。**封面图真实加载（Image widget 从运行时文件路径）是下一个子任务。**

## 2026-07-15 — rfd 集成评估：可行但不推荐

- rfd 平台：Windows/macOS/Linux/WASM，**iOS/Android 不支持**——桌面用 rfd 仍要为移动端另写 UIDocumentPicker/Intent，未省移动端工作量（致命）。
- Makepad 不实现 raw-window-handle（grep 无），rfd 无法 attach 父窗口 → 只能弹独立对话框，模态/焦点体验打折。
- 桌面集成技术可行：Makepad 内部有主线程 dispatch（permission 回调用的 NSBlockOperation）+ NSApp 在跑，AsyncFileDialog 内部派发弹窗能在其 runloop 转；路径 = 后台 tokio block_on future → post_action 回 UI。但 examples 无先例，中等试错成本。
- 结论：rfd 卖点（原生对话框）只覆盖桌面，恰在移动端帮不上，性价比低。推荐仍是拖放+导入目录（桌面移动通用、零依赖、原生接线）。追求一步到位则方案四自实现（NSOpenPanel+UIDocumentPicker）留 M7。
- 待用户拍板导入方式。

## 2026-07-15 — M2 调研：文件导入技术闸门（待决策）

- 老项目 books：models（SimpleBook/BookStatus/ReadingSession/BookNote 等）+ commands 900+ 行，纯 sqlx，迁移方式同 vocab（去 tauri、共享 pool）。EPUB 元数据在老项目是 foliate-js（JS）解析后传给 save_book。
- 关键发现：`epub` crate（2.1.5，纯 Rust）能解析 title/author/章节——**替代 foliate-js 在 Rust 侧解析元数据可行**，M2 架构与老项目不同（Rust 解析而非前端）。
- **技术闸门**：Makepad dev 分支内置文件对话框是**空壳**——macos_app.rs:873 `open_select_file_dialog` 只有 `println!("open select file dialog!")`，无 NSOpenPanel、无结果回传事件，examples 无一使用。标准"按钮弹选择器"走不通。
- 备选：① 拖放（`DragItem::FilePath` 已接线，能拿路径，仅桌面）② 扫描导入目录（零依赖，桌面移动通用）③ rfd crate（跨平台但需验证与 makepad 事件循环集成）④ 自实现 NSOpenPanel/UIDocumentPicker（工作量大）。
- 导入方式 = 影响架构的决策，已抛给用户（AskUserQuestion），用户 dismiss，**暂停等指示**。解析与书架主线不受导入方式阻塞。

## 2026-07-15 — M1 收尾：图标/标题/日志/主题

- 外壳验证通过（用户截图：侧栏导航、数据库就绪）。图标：方案 A（注音书页），源文件 app/resources/icon.svg；iOS 用 app/resources/icon_1024.png（cargo-makepad 自动打包），桌面 Dock 用 workspace 根 resources/icon_{32..1024}.png（makepad-platform build.rs 构建期内嵌；首次需 `cargo clean -p makepad-platform` 触发，因目录此前不存在无监听）。
- 窗口标题 `window.title: "ReadWise"`；主题改为官方三段式显式注册（theme_mod → mod.theme = mod.themes.dark → widgets_mod），运行时深/浅切换推迟到 M8（需先把自定义色改 theme.* 变量）。
- 日志：main.rs 内置 10 行 StderrLogger 接通 log 门面（core 的 log::info 可见），零新依赖。
- **M1 完成**。下一步 M2 书库：迁移 books 模块 + epub 解析（tauri-plugin-epub），书架 UI，文件导入。

## 2026-07-15 — M1 第一刀：readwise-core + App 外壳

- 新建 `core` crate（readwise-core）：`db.rs`（老 database.rs 去 tauri 化，数据目录改由 `paths::app_data_dir()` 提供 = 系统数据目录/ReadWise；schema.sql、default-skills.json 原样拷贝）、`vocab/`（models 原样 + queries 全量迁移，签名从 `AppHandle` 改为 `&SqlitePool`，修掉老代码每次调用新建连接的低效——现在共享 pool）。
- 异步策略：`Db` 同步门面持有 current_thread tokio runtime，方法内部 `block_on`。UI 侧在后台 `std::thread` 调用，结果 `Cx::post_action` 回 UI（M0 已验证的桥）——App 里零 async 管线。
- App 外壳：左侧 SolidView 侧栏（书库/阅读/生词本/设置 ButtonFlat + 底部 db 状态）+ `PageFlip`（uizoo 示例同款，`active_page: @page_reader`，Rust 侧 `page_flip().set_active_page`）。M0 demo 整体挪入阅读页；生词本页展示 DB 实际数据（计数 + 前 50 条预览），作为 core 迁移的冒烟验证。
- 启动流：Event::Startup → spawn_db_init 线程 → Db::open（建库/建 schema/默认 skills）→ DbAction::Ready/Failed → 侧栏状态与生词本页更新。
- 待用户验证：编译、四页导航、侧栏底部显示"数据库就绪"、数据库文件落在 ~/Library/Application Support/ReadWise/database/app.db。

## 2026-07-14 — Ruby 注音验证通过 ✅ + EPUB 复用方案落档

- 第三轮用户实测通过：释义悬浮词上居中、基线完美、行距拉开、点击正常（截图确认）。
- 复用评估（翻老项目代码核实）：注音数据层全在 Rust（schema 四张表 + core/vocab + tauri-plugin-epub），复用率 ~90%；只需移植两块纯 JS——`normalizeToken/lemmatize`（~50 行查表逻辑）和 `GlossAnnotator`（DOM 注入改写为章节 HTML 字符串预处理，反而更简单）。
- 设计、三轮教训、复用映射表落档 [ruby-annotation.md](ruby-annotation.md)；遗留优化（释义居中微偏、首行间距、长释义碰撞）记录在案，非阻塞。
- 至此 M0 + Ruby 专项全部完成，重写路线的最后一个技术问号拔除。下一步 M1。

## 2026-07-14 — Ruby 第三轮：字体与行距的正确来源

- 第二轮实测：基线对齐彻底解决 ✓、点击 ✓，但释义不可见（`WARNING: encountered empty font family`）、行距未生效。
- 释义字体：自建 DrawText 的 text_style 默认无字体族，必须从主题取完整 TextStyle——`text_style: theme.font_regular{ font_size: 7.5 }`（官方 browser 示例同款写法）。
- 行距真相：TextFlow 每个 run 都会用自身 `text_style_normal/bold/italic/fixed` 字段**整体覆盖** `draw_text.text_style`（text_flow.rs:1866），所以设 `draw_text.text_style.line_spacing` 无效；正确旋钮是 Html 实例的 `text_style_normal.line_spacing`（bold 同理）。行距 scale 经 layout options 的 `line_spacing_scale` 生效（draw_text.rs:2528）。

## 2026-07-14 — Ruby 第二轮：改为 TextFlow 内联绘制 + 释义叠加

- 首版两层 Label 盒子方案失败：TextFlow 把 widget 盒子顶对齐塞进行内，释义占了正文行、原词被挤到下一行（用户截图）。
- 重写（HtmlLink 同款画法）：原词交给 `tf.draw_text` 内联绘制——基线/字体/断行与正文像素级一致；`areas_tracker` push/pop 拿到词的绘制区域，释义用自有 `DrawText.draw_abs` 叠画在首段区域上方居中（`layout()` 测宽）。命中区域改用 drawn_areas 逐段 hits（跨行断词也能点）。
- 行高：新增 `GlossPara` 模板（`draw_text.text_style.line_spacing: 2.0`），ChapterList 按内容含 `<ruby` 自动选模板，为释义留出行间空间。
- 已知边界：段落首行的释义上方是段间距，极端情况可能与上一项贴近；后续在真实阅读器里统一调段距。

## 2026-07-14 — Ruby 注音 Demo（方案 A：Html 自定义标签 widget）

- 源码确认扩展点：Html 遇未知标签走 `handle_custom_widget`（html.rs:968）——按标签名查 Html 实例的同名子模板，实例化 widget 内联进 TextFlow，属性经 `Scope::with_props_index(doc, index)` 传入（HtmlLink 读 href 同机制）。**零侵入实现 ruby 成为可能。**
- 实现：`ruby_word.rs` 新增 `RubyWord` widget（flow Down + 居中：上小字释义 `gloss_label`、下原词 `word_label`），`ScriptHook::on_after_new_scoped` 从 `<ruby t="释义">` 的 `t` 属性读释义；点击发 `RubyWordAction::Clicked{word, gloss}`。
- 标记格式定为 `<ruby t="释义">word</ruby>`（释义走属性，非标准 `<rt>` 子标签——因扩展点把内层文本整体当 widget 文本）。
- Demo：`build_glossed_paragraph()` 生成英文段落（灯塔守夜人，自写文本），10 个生词带中文注音，插为章节第一项。
- 注册链：`makepad_widgets::script_mod` → `crate::ruby_word::script_mod` → app；Html 实例挂 `ruby := mod.widgets.RubyWord{}` 模板。
- 待验证：释义居中显示于词上方、行高撑高但不乱版、点击显示"词 → 释义"、与普通文本/链接混排正常。

## 2026-07-14 — iPhone 17 真机通过，M0 全线收官 ✅✅

- 真机部署完整链路打通：Homebrew Rust 冲突 → DDI 未挂载 → profile 设备不含 UDID → Bundle ID 不一致 → 开发者信任 → 在线验证，全部排完。真机运行流畅、各功能正常（用户确认）。
- 全部坑位整理成 [ios-deploy-guide.md](ios-deploy-guide.md)（9 个坑 + 三方一致原则 + 报错对照表）。
- 下一专题：Ruby 注音（英文单词上方显示中文释义）——本 App 核心特性，先方案讨论后 demo 验证。

## 2026-07-14 — 真机部署排障 + 热重载确认

- `--hot` 热重载用户实测 OK（M0 最后一项补齐）。
- 真机 run-device 失败：cargo-makepad 对 devicectl 失败是 `Err(_)` 吞掉原因再回退 ios-deploy（第三方工具，不支持 iOS 17+，对 iPhone 17 无效）。
- 真实根因：`xcrun devicectl list devices` 显示设备状态 `connected (no DDI)`——开发者磁盘映像未挂载，需 Xcode 对设备完成 "Preparing device for development"。
- 签名后的 .app 产物路径：`target/makepad-apple-app/aarch64-apple-ios/release/readwise-app.app`，可用 `xcrun devicectl device install app --device <ID> <path>` 手动安装看真实报错。
- 命令勘误：列签名资源是 `cargo makepad apple list`（不是 `apple ios list`）。

## 2026-07-14 — M0 收官：iOS 模拟器跑通，闸门通过 ✅

- 修完 Homebrew Rust 冲突后 iOS release 编译 74s 通过；启动模拟器后安装运行成功（iPhone 17 / iOS 26.5）：中文、流式、重渲染均正常（用户截图）。
- 模拟器卡顿：Makepad 纯 Metal 渲染在模拟器走 GPU 软件转译，属已知现象，真机预期流畅；真机验证待用户完成签名三件套。
- 遗留小项：app 图标（resources/icon_1024.png）、窗口标题、`--hot` 热重载专项测试、safe-area 适配（M7）。
- **M0 风险闸门全部通过，MILESTONES.md 已更新。下一步 M1：workspace 重组 + readwise-core 迁移 + App 外壳。**

## 2026-07-14 — M0 第八轮：iOS 构建失败根因——Homebrew Rust 劫持 rustc

- 症状：`rustup target add aarch64-apple-ios-sim` 已装、std rlib 齐全，但 run-sim 仍报 E0463 找不到 core/std。
- 定位：最小复现 + `rustup run stable sh -c 'which rustc'` → 解析到 `/opt/homebrew/bin/rustc`（Homebrew Rust 1.93.1）。机器上有两套 Rust：rustup 1.97.0 与 Homebrew 1.93.1，PATH 里 `/opt/homebrew/bin` 在 `~/.cargo/bin` 前。`rustup run stable cargo` 启动的是 rustup 的 cargo，但它按 PATH 找 `rustc` 时命中 Homebrew 版——它不认识 rustup 的 target 库目录 → E0463。
- 解决：卸载 Homebrew Rust（`brew uses --installed rust` 确认无包依赖它），或 PATH 里把 `~/.cargo/bin` 提到 homebrew 前。
- 教训：桌面构建一直"碰巧"能过是因为 host target 两套都有；交叉编译才暴露混用。

## 2026-07-14 — M0 第七轮：流式通过 ✅，进入 iOS 打包

- 用户实测流式打字机通过。M0 仅剩 iOS。
- 读 tools/cargo_makepad 源码确认 CLI 规则：`--org/--app/--cert/--profile/--device` 必须在 `run-sim`/`run-device` 之前（解析器遇非选项参数即停）；Bundle ID=`{org}.{app}`；iOS target 用 stable channel；`apple ios list` 列签名资源；run-device 缺参会自动打印可用 profile/cert/device。
- 真机免费签名路径：Xcode 建同 Bundle ID 空工程跑一次生成 profile+cert（个人团队 7 天有效）。README 已更新成完整教程。

## 2026-07-14 — M0 第六轮：虚拟化通过 ✅ + 流式打字机验证

- 用户实测：PortalList 200 段滚动流畅 ✓、点词正常 ✓、浅灰链接观感可接受 ✓、重新渲染正常 ✓。**M0 桌面渲染/交互全部通过。**
- 新增流式验证：后台 `std::thread` 每 80ms `Cx::post_action(StreamAction::Chunk)` → `handle_actions` 里 `downcast_ref::<StreamAction>()`（post_action 是 bare action，不能用 as_widget_action）→ Label 逐词上屏。
- 重要发现：makepad 仓库自带 `libs/makepad_ai`（Agent 抽象 + OpenAI 兼容流式后端 + AgentEvent::TextDelta，内部用 SignalToUI），官方 aichat 示例就是流式聊天。**M5 的 AI 链路直接用 makepad-ai，不必自己拼 reqwest+SSE。**
- 待验证：点「流式测试」逐词上屏不卡顿、期间滚动/点词不互相阻塞。

## 2026-07-14 — M0 第五轮：自注册 widget 的引用方式

- 报错 `variable ChapterList not found in scope`：`use mod.widgets.*` 是执行到该行时的快照，看不到同模块后续注册的 widget。官方 todo 的写法：使用处写全路径 `mod.widgets.TodoList{}`（todo/main.rs:234）。已改为 `mod.widgets.ChapterList{}`。

## 2026-07-14 — M0 第四轮：PortalList 虚拟化重构

- 用户反馈：点词 ✓、ruby 降级 ✓，但 3000 链接下滚动粘滞、满屏蓝色链接观感差。
- 重构：参照官方 examples/todo 的模式，自定义 `ChapterList` widget（`#[deref] view: View` + `register_widget`），内部 PortalList 按段虚拟化渲染——`set_item_range` + `next_visible_item` + `list.item(cx, id, id!(Para))` + `set_text` + `draw_all_unscoped`。段落数提到 200 段以证明虚拟化后成本只与可见段落相关。
- 链接样式：`Html{ a.color / a.hover_color / a.pressed_color }` 覆盖为浅灰/悬停蓝。下划线在 `HtmlLink::draw_walk` 里硬编码（`tf.underline.push()` 无条件），M0 接受；正式阅读器不走 `<a>` 包词，M3 做 TextFlow 级命中。
- 数据流：段落存 `static LazyLock<RwLock<Vec<String>>>`（todo 官方同款），draw_walk 读、按钮写。
- 待验证：200 段滚动是否顺滑、点词仍工作、观感改善。

## 2026-07-14 — M0 点词命中验证

- 读源码确认：Html 里 `<a>` 由 `HtmlLink` widget 渲染，点击发 `HtmlLinkAction::Clicked { url }`，且内置 `FingerLongPress`（对移动端长按查词是现成机制）。
- 实现：正文每个词（英文按单词、CJK 按单字）包成 `<a href="w:词">`，`handle_actions` 里 `action.as_widget_action()` + `cast()` 捕获，点词显示在状态栏。约 3000 个 HtmlLink，兼作性能压测。
- ruby 丢字修复：渲染前 `strip_ruby()` 剔除 `<rt>` 保留正文（M4 再做真正的注音渲染）。
- 待验证：点词是否命中、大量链接下滚动/重渲染性能、链接样式是否影响阅读观感。

## 2026-07-14 — M0 桌面渲染验证通过 ✅

- 用户实测（macOS）：章节渲染 ✓、**中文无豆腐字**（makepad 内置字体含 CJK）✓、标题/粗斜体/引用/列表排版 ✓、40 段滚动流畅 ✓、按钮事件计数 ✓（截图确认渲染 13 次）。
- 新发现：`<ruby>` 标签的全部内容被 Html widget **直接丢弃**（非退化为普通文本），"山月/旷野/虎"缺失。结论：进 Html 前必须预处理未知标签（短期：剥掉 `<rt>` 保留正文；M4：自研 ruby 渲染）。
- 小问题：窗口标题是默认 "Makepad"，后续设置。
- M0 剩余：点词命中、iOS 打包、reqwest 流式 → UI。

## 2026-07-14 — M0 第二轮：Rust 侧更新 widget 的正确姿势

- 运行时报 `field chapter not found`：`script_apply_eval!` 按对象字段路径严格解析，不深搜命名实例。官方做法（examples/aichat）是 `self.ui.widget(cx, ids!(chapter)).set_text(cx, &html)` / `self.ui.label(cx, ids!(status)).set_text(...)`——`ids!` 深度查找，`set_text` 对 Html widget 会重新解析 body。已改用此模式。
- 结论：`script_apply_eval!` 适合改样式属性（需完整字段路径），内容更新用 WidgetRef 访问器。

## 2026-07-14 — M0 首次运行：入口协议修正

- 首次 `cargo run` 编译通过，运行时 panic：`AppMain::script_mod not implemented`。dev 分支真实 API 与 makepad-skills 文档有偏差：入口不是 `App::run`，而是 `impl AppMain` 里实现 `fn script_mod(vm) -> ScriptValue`（对照官方 examples/counter 修正）。**教训：以 `~/.cargo/git/checkouts/makepad-*/` 本地源码为准，skills 文档只作参考。**
- 读源码确认：`Html` widget（widgets/src/html.rs）支持 h1-h6、b/i/u/s、code/pre、blockquote、ul/ol、table（含对齐）、a 链接、sub/sup、entities；`body` 属性 + `set_text()` 重新解析。**无 ruby 支持**——M4 注音需自研（方案：扩展 html.rs 的 tag 处理或基于 TextFlow 自定义 widget）。
- 待用户复测：`cargo run -p readwise-app`。

## 2026-07-14 — M0 开工：新项目脚手架

- 新建独立项目 `/Users/ohmycloud/opt/rust/rust-ui/readwise-makepad`（原 Tauri 版 readwise 原样保留），git 已初始化。
- 工程：cargo workspace + `app` crate，依赖 `makepad-widgets` git dev 分支（已确认该分支版本为 2.0.0，Html/Markdown widget 默认启用，无需 feature）。
- M0 demo：窗口 + 头部工具栏 + ScrollYView 内 Html widget 渲染模拟 EPUB 章节（中英混排、标题/引用/列表、`<ruby>` 注音探测、40 段长文滚动），"重新渲染"按钮验证 Rust↔Splash 事件链路（`script_apply_eval!` 更新 body 与状态文本）。
- 技术要点（来自 makepad-skills）：`script_mod!` 冒号语法、`:=` 命名实例、`+:` 合并、`#[source] source: ScriptObjectRef` 必需、含 `e` 的十六进制色需 `#x` 前缀、注册顺序 makepad_widgets 先于 App。
- 已知风险待验证：中文字体（可能豆腐字，需注册 CJK 字体）、ruby 大概率不支持（M4 前需自研 widget）、iOS 打包链路。
- 待用户执行：`cargo run -p readwise-app`，按 README 的 M0 清单逐项反馈。

## 2026-07-14 — 立项

- 决策：放弃 Tauri + React 架构，改用纯 Makepad 2.0（Splash DSL）重写，不引入任何 Web 前端（Vue 方案经确认不采用）。
- 动机：一套 Rust 代码覆盖桌面 + iOS/Android；此前评估 Tauri iOS 适配存在 llamacpp 子进程、sidecar 等多处阻碍。
- 产出：[MILESTONES.md](MILESTONES.md)，M0 为技术验证闸门。
- 学习资料：Makepad 2.0 Book、makepad-skills（14 个 skill，本地已克隆备用）。
- 下一步：等用户确认 milestone 后启动 M0。
