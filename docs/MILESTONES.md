# ReadWise Makepad 重写 — Milestones

> 决策日期：2026-07-14
> 原生架构：Makepad 2.0（Splash DSL + Rust），不嵌入 WebView/React/Vue。
> 2026-10-23 新增：独立 Topcoat Web 客户端，复用 readwise-core，详见 [WEB.md](WEB.md)。
> 参考资料：[Makepad 2.0 Book](https://zhanghandong.github.io/makepad-2.0-book/)、[makepad-skills](https://github.com/ZhangHanDong/makepad-skills)

## 目标平台（原桌面 + 移动；2026-10-23 增加 Topcoat Web）

| 平台 | 状态 |
|------|------|
| macOS 桌面 | ✅ 已验证 |
| iOS（iPhone） | ✅ 已验证（真机 + 模拟器） |
| Windows / Linux 桌面 | 🟡 原生打包脚本已实现，待对应主机实跑与 UI 补测（Windows 窗口装饰小差异待处理） |
| Android | 🟡 arm64 release APK 已真实构建并安装，EPUB 系统选择器已通过真机验证，横竖屏状态保持待复测 |
| **Web（Topcoat SSR）** | 🟡 第一阶段已实现：本机单用户书架、EPUB/PDF 导入、章节/目录/图片、注音词卡、已认识词、词汇、aA/设置及阅读锚点恢复。已增加桌面风格选文工具栏、持久化 Web 高亮/笔记、单次 AI、完整 aA 三页签/全局阅读偏好、只读桌面设置载入与 CSS 分页；默认独立数据目录，不含认证、自动双向同步和完整原生 AI。验证与限制见 [WEB.md](WEB.md)。 |
| **Web（Makepad WASM）** | ❌ 仍不纳入路线图。不是将当前原生 SQLite、文件和线程路径直接编译进浏览器；新增 Web 采用服务端运行 readwise-core 的独立架构。 |

## 总体策略

- **Rust 业务逻辑最大化复用**：现有 `src-tauri/src/core`（books/threads/notes/tags/vocab/skills/statistics、sqlx+sqlite）和 `tauri-plugin-epub`（EPUB 解析、TOC、分词/切块/向量化）基本是纯 Rust，抽成独立 crate `readwise-core`，去掉 tauri 依赖后直接复用。
- **UI 全部重写**：React 页面（library/reader/chat/vocab/wordbook/statistics/skills）逐个用 Splash DSL 重建。
- **最大风险点是阅读器**：现在靠 foliate-js（WebView 里的 HTML/CSS 排版）渲染 EPUB。Makepad 没有浏览器引擎，只有 Html widget（HTML 子集）。注音（ruby）、高亮、划词、分页都要自己做。**所以 M0 先做技术验证，不通过就止损。**
- **AI 推理**：桌面端可保留 llama-server 子进程方案；iOS 禁止子进程，移动端 AI 一律走远程 API（smartFetch 链路对应的 reqwest 流式请求）。

## W1 Topcoat Web 第一阶段

- [x] 独立 `readwise-web` crate，固定 Topcoat 0.10.0，资源打包与启动脚本。
- [x] 复用核心导入/解析/词库/数据库，隔离原生数据，原生客户端不迁移。
- [x] 书架、流式上传、章节/目录/图片、滚动阅读及文本锚点保存/恢复。
- [x] 原生 ruby、Topcoat signal 色板、词卡、标为已认识、词汇本、真实统计。
- [x] 原地隐藏注音保持几何、失败恢复、返回章节一致性；窄/矮视口草稿与取消回归。
- [x] 代码块语法高亮、选文复制/保存摘录工具栏与真实摘录列表。
- [x] 桌面风格选文工具栏、四种持久化高亮、笔记撰写/摘录列表、引文/词汇/书内搜索操作。
- [x] aA「主题／文本／注音」与统一全局阅读偏好；只读载入桌面设置草稿，明确应用，不修改桌面数据库。
- [x] CSS 多栏分页与滑动/轻量卷页过渡、窄屏回退、旋转锚点、首/后续页交互回归。
- [x] OpenAI 兼容配置及选文单次问答/解释/释义；回环 mock 验收，密钥不回传。
- [ ] 物理跟手卷页、全书精确页码、书签、已有笔记编辑/管理。
- [ ] AI 流式多轮/历史/skills、阅读计时、完整书架管理。
- [ ] Android/iOS 真机浏览器、Safari/Firefox 专项验收。
- [ ] 认证、多用户隔离、跨端同步及离线能力，须另行设计后开放远程部署。

## M0 技术验证（风险闸门）✅ 2026-07-14 通过

目标：证明 Makepad 能承载 ReadWise 的核心体验，任何一条不通过都要重新评估。

- [x] makepad-widgets（dev 分支）Hello App 跑通（热重载 `--hot` 未专项测试，留 M1）
- [x] 用 Html widget 渲染模拟 EPUB 章节（中文排版 ✓ 内置 CJK 字体、标题/引用/列表/表格 ✓；图片留 M3）
- [x] 长文滚动性能：单 Html 全量渲染 3000 链接会粘滞 → PortalList 按段虚拟化后 200 段流畅
- [x] 点击词命中：`<a>` 包词 + `HtmlLinkAction::Clicked`，含移动端长按事件
- [x] ruby 注音结论：Html widget 不支持且会**丢弃**未知标签内容；短期 `strip_ruby` 降级，M4 自研（扩展 html tag 处理或基于 TextFlow）
- [x] cargo-makepad iOS：模拟器（iPhone 17 / iOS 26.5）跑通；模拟器卡顿系 GPU 转译所致，真机验证待签名
- [x] 流式：后台线程 `Cx::post_action` → `downcast_ref` 打字机上屏；M5 直接用官方 `libs/makepad_ai`（OpenAI 兼容流式后端）

验收结论：demo 渲染整章、点词、iOS 可跑，全部达成。**闸门通过，重写路线成立。**

## M1 工程骨架 ✅ 2026-07-15

- [x] cargo workspace：`readwise-core`（纯业务逻辑）+ `readwise-app`（Makepad UI）
- [x] 迁移 database（schema.sql、sqlx、默认 skills）+ vocab 全模块，去 tauri 化；`Db` 同步门面（内置 tokio runtime）
- [x] App 外壳：窗口标题、侧栏导航、PageFlip 页面切换；深/浅主题在 M8 接入运行时切换与持久化
- [x] 日志（log 门面 → stderr 最小 logger）、应用数据目录（paths::app_data_dir，iOS 差异 M7 验证）
- [x] 开发日志机制（DEVLOG.md）
- [x] 应用图标（桌面 Dock + iOS，源文件 app/resources/icon.svg）

验收：空壳 App 各页面可导航，数据库初始化成功。✅ 用户实测通过。

## M2 书库（Library）✅ 2026-07-15（tags 项移 M8）

- [x] EPUB 导入：macOS 原生多选按钮、桌面拖放、import 投递箱目录与 iOS Files 多选（Makepad 文件对话框仍是空壳，应用按 Apple 平台提供轻量原生桥接；rfd 因不支持移动端被否）
- [x] 复用 epub 解析：元数据、封面提取（epub crate，Rust 侧解析，替代 foliate-js）
- [x] 书架卡片 UI（封面/首字占位、书名、作者、进度）：支持持久化的列表/网格切换，网格按窗口宽度每行 2/3/4 本；点击卡片直接阅读，三点菜单集中提供分享、欲读清单、藏书、读完、重命名和删除，删除前二次确认并反馈成功/失败（新菜单待用户实测）
- [ ] tags 逻辑：编辑、持久化、卡片展示与书库筛选已于 2026-08-29 实现，待用户实测后关闭

验收：导入若干 EPUB，书架正常展示并持久化。✅ 主链路用户实测通过（封面/删除待复测）

## M3 阅读器核心 ✅ 2026-07-16

- [x] 章节渲染管线：epub crate 取章 → extract_body → sanitize_for_reader（白名单/去壳留瓤）→ split_blocks 深度分块 → PortalList+Html；顶层 `pre/code` 使用只读 CodeView 保留缩进/换行/注释并重做常用语法着色；表格标题独立分块，表格按桌面/手机可用宽度居中换行完整展示（待真书复测）
- [x] 目录抽屉（章内首标题提取 + 行背景高亮）、章节跳转
- [x] 视口翻屏 + 快捷键（空格/PgUp/PgDn/←→/Shift 跨章/Home）、章内进度%；持久化章节、首个可见段落和段内偏移，滚动防抖保存并在离页/后台/退出时同步落盘，兼容旧版章节位置
- [x] iBooks 式页脚：当前页码、悬停 `当前页 of 总页数`、本章剩余页数；分栏版按实际行边界分页。本章使用实测页数，全书未排版章节暂按密度估算，悬停以 `≈` 标识
- [x] 正文分栏第一阶段（2026-09-14）：设置页提供单列 / 双列 / 自动，按正文可用宽度、字号和高度适配，窄屏回退单列；两列连续阅读、跨列选区与高亮共用原文偏移，v3 阅读位置支持重排恢复。桌面主路径已验证，移动端真机待复测
- [x] 翻页动画第一阶段（2026-09-14）：原生 DPI 快照卷纸过渡、正反方向与阅读纸色一致，单/双列共用，不改变选区与高亮布局；拖选跨页跳过动画，布局变化取消过渡。macOS 主路径验证，Android APK 构建通过，移动端真机待验收
- [x] 翻页方式（2026-09-14）：新增左右滑动和上下滚动；左右滑动复用原生 DPI 页面快照与水平过渡，上下滚动切换为连续单列正文并隐藏页码；三种方式均保存为 `reader.turn_mode`，切换时按原文字符位置恢复阅读位置
- [x] 快速翻页解阻塞（2026-09-15）：当前页就绪与整章页数解耦，优先预排下一屏；页码索引按已测量前缀增量推进，每帧测量仍受 8 块 / 10ms 预算约束。连续输入合并保留一次待翻页，支持页数未完成时前进及返回已读页面；修正对齐后的就绪状态，并使用紧凑测量缓存编号避免切换滚动模式时遍历巨大 ID 区间。新增 6 项回归，工作区 175 项测试通过；macOS 真书快速翻页/等待后继续/返回及模式切换已检查，Android 真机性能待复测
- [x] 触摸板一次手势只翻一屏（2026-09-15）：将 250ms 重复触发改为手势级去重；macOS 从当前 AppKit 滚动事件补读手势/惯性阶段，新的主动滑动可立即触发，惯性尾部不进入待翻页队列；无阶段的平台按整个事件流的静默间隔识别下一次手势。鼠标滚轮保持原有节流，代码/表格横向滚动不透传翻页。新增 7 项回归，工作区 182 项测试通过；原生窗口注入连续触摸板事件验证双列只前进一屏（本章剩余 19→17），新手势继续到 15；真实触摸板手感待用户复测
- [ ] 翻页动画后续：移动端跟手卷页/松手取消、双页书脊模型、减少动态效果开关，以及 Android/iOS GPU 方向与性能实测
- [ ] 分栏后续：全书精确页数缓存、超高表格的跨页行边界、标题孤行/段落孤行排版优化，以及 Android/iOS 真机长按跨页与横竖屏验收
- [x] 字号调节（设置页，8-20，标题等比缩放）；字体/行距/主题已在 M8 完成并统一持久化
- [x] 全应用阅读场景调色板：macOS 红黄绿按钮所在的窗口标题栏、桌面/移动导航以及书库、阅读、AI 对话、AI 技能、生词本、统计、设置七个页面，共享阅读背景的 surface 与控件语义色；卡片、输入框、按钮、动态列表和图表不再回落到灰色主题（待用户实测）
- [x] 图片渲染（打开时提取到 res/ + src 改写 + ChapterImage 自定义标签 widget；内嵌图按 DPI 保清晰且可点击查看原图，代码 callout 数字图转文本，待真书复测）
- 沉浸式阅读页原则（学 iBooks）：控制项进设置/键盘，不占阅读界面

验收：真书（多章技术书）渲染/翻读/恢复进度全通过。✅ 用户实测

## M4 阅读增强 ✅ 2026-08-26

- [x] 文本选择、高亮（跨块/滚轮延伸、离屏选区投影、持久化、逐条颜色）
- [x] 笔记（选区引用、补充内容、按书侧栏、持久化/删除）
- [x] 生词注音 ruby 渲染 + 释义弹窗（4437 条 ECDICT 离线基础释义与空库初始化；方案见 [ruby-annotation.md](ruby-annotation.md)）
- [x] 划词查词、加入生词本（选区浮层含复制/笔记/生词/翻译/发音/高亮）

验收：现有 Tauri 版的注音阅读体验在 Makepad 版等效复现。✅ 用户实测通过。

## M5 AI 功能（进行中）

- [x] OpenAI-compatible 流式 chat（官方 `makepad_ai` SSE、停止/新对话、错误反馈、设置与 threads 持久化；真实端点实测通过）
- [x] 阅读页 side-chat（选区浮层“问 AI”在阅读页右侧栏流式交互；真实端点实测通过）
- [x] skills（Prompt 模板）管理：增删改、启停、系统技能全局生效、自定义技能一次性应用到下一条提问
- [ ] 桌面端可选：llama.cpp 本地推理（进程管理、设置持久化、健康检查、取消加载、异常退出回退均已完成；待真实 `llama-server` + GGUF 实测）
- [x] 移动端：仅远程 API（子进程实现按目标平台隔离，`aarch64-apple-ios` 交叉编译通过）

验收：三个 AI 功能（chat/side-chat/skills）流式输出不挂起。

## M6 词汇与统计

- [x] 生词本（vocab/wordbook）：虚拟化列表、搜索/状态筛选、发音/移除、队列复习与状态管理
- [x] 统计页（statistics）：阅读时长、词汇量图表（Makepad shader 绘制）

验收：与现有版本功能对齐。

## M7 移动端适配

- [ ] cargo-makepad 打包 iOS（真机）/ Android（Android arm64 release APK、产品包名/版本、最小权限清单、五档图标、开发签名校验及可选 Play Store AAB 流程已完成，APK 已真机安装；iOS 分发仍待凭据/上游归档出口）
- [ ] 触摸交互：横向翻屏、长按选词后拖动扩选已实现（区分纵向滚动/长按/多指）；虚拟键盘显示时隐藏移动底栏并禁用翻屏手势；上述交互均待真机专项验收
- [ ] 响应式布局：手机上侧栏折叠、单栏阅读（窄屏底栏、阅读面板单栏与安全区已实现；手机横竖屏均保持移动端布局，待真机验收）
- [ ] 移动端文件导入、数据目录、生命周期（iOS Files 多选导入、Android SAF 多选导入、原生沙箱目录与后台/恢复计时已实现；Android 使用应用级 Activity/JNI 桥接弥补当前 `cargo-makepad` 未注入 Java 插件的缺口，SAF 导入已通过真机验证，横竖屏状态保持修复待复测）

验收：iPhone 真机安装，完成一次完整的"导入→阅读→查词→AI 问答"流程。

## M8 打磨与发布

- [x] i18n（中/英；应用语言可在设置页切换并持久化，动态列表与运行时状态同步本地化）
- [x] 设置页整合（正文字号、标准/等宽字体、默认/舒适/宽松行距、纸白/米黄/浅绿/夜间/跟随主题场景背景、注音开关/字号/配色、选区色、高亮色与深/浅主题已统一持久化；所有功能页和自定义 UI 共享场景化 `theme.*` token）
- [ ] 性能优化（三轮已完成正文热路径/AI 重绘收窄、整书注音惰性缓存及 PortalList 章节块缓存；draw batching 与 GC 已审计，512 段合成长章回归已加入，启动基线为数据库设置 258ms、书库 850ms，待真实长书 GPU 滚动体感验收后关闭）
- [ ] 桌面打包分发；iOS TestFlight（macOS `.app`/ZIP、公证/装订/Gatekeeper 流程已完成；Linux 原生 tar.gz + XDG 用户安装器、Windows 原生 ZIP + 可选 Authenticode 签名流程已实现；实际公证、Windows/Linux 原生实跑及 TestFlight 待发布凭据或目标环境）
- [x] 老版本数据迁移（保留旧 SQLite 数据原位升级；schema v2、缺列幂等补齐、完整表/列验证、未来版本拒绝与外键约束已完成）

## 明确放弃/降级的功能

- WebView 相关一切（foliate-js、React 组件库、CSS 主题）
- 自动更新（updater）：桌面端后续再评估，iOS 走 App Store
- woff2 sidecar 字体压缩：改为构建期预处理或纯 Rust 实现

## 风险登记

| 风险 | 影响 | 对策 |
|------|------|------|
| Html widget 覆盖的 HTML/CSS 子集不足以排版真实 EPUB | M3 阻塞 | M0 提前验证；必要时自研排版层，范围锁定在"常见 EPUB 子集" |
| ruby 注音无现成组件 | M4 延期 | M0 出可行方案原型 |
| makepad dev 分支 API 不稳定 | 全程 | 锁定 commit，集中升级 |
| iOS 打包链路（签名、cargo-makepad）不成熟 | M7 阻塞 | M0 就先跑 iOS demo，不留到最后 |
