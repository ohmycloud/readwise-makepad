# ReadWise Makepad 重写 — Milestones

> 决策日期：2026-07-14
> 架构：纯 Makepad 2.0（Splash DSL + Rust），不再有 WebView/React/Vue。
> 参考资料：[Makepad 2.0 Book](https://zhanghandong.github.io/makepad-2.0-book/)、[makepad-skills](https://github.com/ZhangHanDong/makepad-skills)

## 目标平台（2026-07-15 确认：桌面 + 移动，不含 Web）

| 平台 | 状态 |
|------|------|
| macOS 桌面 | ✅ 已验证 |
| iOS（iPhone） | ✅ 已验证（真机 + 模拟器） |
| Windows / Linux 桌面 | 🟡 顺路支持，需补测（无 mac 专属代码；Windows 窗口装饰小差异待处理） |
| Android | 🟡 顺路支持，需补测（cargo makepad android 打包链路待走） |
| **Web (wasm)** | ❌ **明确不做**。Makepad 框架支持 wasm，但 readwise-core 的 sqlx + SQLite + tokio 在浏览器沙箱跑不了（无文件系统/原生线程）。支持 Web 需为 wasm 单独做数据层（IndexedDB/OPFS sqlite-wasm）或改远程架构，违背本地优先设计，不纳入路线图。 |

## 总体策略

- **Rust 业务逻辑最大化复用**：现有 `src-tauri/src/core`（books/threads/notes/tags/vocab/skills/statistics、sqlx+sqlite）和 `tauri-plugin-epub`（EPUB 解析、TOC、分词/切块/向量化）基本是纯 Rust，抽成独立 crate `readwise-core`，去掉 tauri 依赖后直接复用。
- **UI 全部重写**：React 页面（library/reader/chat/vocab/wordbook/statistics/skills）逐个用 Splash DSL 重建。
- **最大风险点是阅读器**：现在靠 foliate-js（WebView 里的 HTML/CSS 排版）渲染 EPUB。Makepad 没有浏览器引擎，只有 Html widget（HTML 子集）。注音（ruby）、高亮、划词、分页都要自己做。**所以 M0 先做技术验证，不通过就止损。**
- **AI 推理**：桌面端可保留 llama-server 子进程方案；iOS 禁止子进程，移动端 AI 一律走远程 API（smartFetch 链路对应的 reqwest 流式请求）。

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
- [x] App 外壳：窗口标题、侧栏导航、PageFlip 页面切换；主题显式注册为 dark（运行时深/浅切换移至 M8——前提是自定义色全部改用 theme.* 变量）
- [x] 日志（log 门面 → stderr 最小 logger）、应用数据目录（paths::app_data_dir，iOS 差异 M7 验证）
- [x] 开发日志机制（DEVLOG.md）
- [x] 应用图标（桌面 Dock + iOS，源文件 app/resources/icon.svg）

验收：空壳 App 各页面可导航，数据库初始化成功。✅ 用户实测通过。

## M2 书库（Library）

- [x] EPUB 导入：拖放进窗口 + import 投递箱目录（Makepad 文件对话框是空壳，原生选择器留 M7；rfd 因不支持移动端被否）
- [x] 复用 epub 解析：元数据、封面提取（epub crate，Rust 侧解析，替代 foliate-js）
- [x] 书架卡片 UI（封面/首字占位、书名、作者、进度），删除（直删，M8 可加确认）
- [ ] tags 逻辑：推迟到 M8 整理——当前书架无筛选 UI 消费场景

验收：导入若干 EPUB，书架正常展示并持久化。✅ 主链路用户实测通过（封面/删除待复测）

## M3 阅读器核心（最重）

- [ ] 章节 HTML → Makepad 渲染管线（基于 M0 验证的方案）
- [ ] 目录（TOC）侧栏、章节跳转
- [ ] 滚动/分页模式、阅读进度保存与恢复
- [ ] 字体、字号、行距、阅读主题设置
- [ ] 图片渲染

验收：完整读完一本中文/英文 EPUB 不出排版问题，重启恢复进度。

## M4 阅读增强

- [ ] 文本选择、高亮（highlights 持久化，复用 notes/public_highlights 逻辑）
- [ ] 笔记（notepad）
- [ ] 生词注音 ruby 渲染 + 释义弹窗（渲染层已提前验证 ✅，方案与复用映射见 [ruby-annotation.md](ruby-annotation.md)；弹窗布局与例句剔除注音的规则照搬老项目）
- [ ] 划词查词、加入生词本

验收：现有 Tauri 版的注音阅读体验在 Makepad 版等效复现。

## M5 AI 功能

- [ ] reqwest 流式 chat（打字机效果），thread 管理复用 threads 逻辑
- [ ] 阅读页 side-chat（选中文本发给 AI）
- [ ] skills（prompt 模板）管理
- [ ] 桌面端可选：llamacpp 本地推理（子进程方案，`#[cfg(desktop)]`）
- [ ] 移动端：仅远程 API

验收：三个 AI 功能（chat/side-chat/skills）流式输出不挂起。

## M6 词汇与统计

- [ ] 生词本（vocab/wordbook）：列表、复习、状态管理
- [ ] 统计页（statistics）：阅读时长、词汇量图表（Makepad shader/vector 绘制）

验收：与现有版本功能对齐。

## M7 移动端适配

- [ ] cargo-makepad 打包 iOS（真机）/ Android
- [ ] 触摸交互：翻页手势、长按选词、虚拟键盘适配
- [ ] 响应式布局：手机上侧栏折叠、单栏阅读
- [ ] 移动端文件导入、数据目录、生命周期（后台/恢复）

验收：iPhone 真机安装，完成一次完整的"导入→阅读→查词→AI 问答"流程。

## M8 打磨与发布

- [ ] i18n（中/英）
- [ ] 设置页整合
- [ ] 性能优化（draw batching、GC、启动时间）
- [ ] 桌面打包分发；iOS TestFlight
- [ ] 老版本数据迁移（sqlite 库直接兼容，验证 schema 一致性）

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
