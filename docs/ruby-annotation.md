# Ruby 注音：设计、教训与 EPUB 复用方案

> 2026-07-14 验证通过（桌面）。本文档是 M3/M4 阅读器注音功能的实现依据。

## 最终架构（已验证）

```
章节 HTML（含 <ruby t="释义">word</ruby>）
   ↓ Html widget 解析，遇到 ruby 标签
   ↓ handle_custom_widget：按标签名查同名子模板（html.rs:968）
RubyWord widget（app/src/ruby_word.rs）
   ├─ 原词：交给 TextFlow.draw_text 内联绘制 → 基线/字体/断行与正文像素级一致
   ├─ 释义：areas_tracker 拿到词的绘制区域，自有 DrawText.draw_abs 叠画在首段上方居中
   └─ 点击/长按：drawn_areas 逐段 hits → RubyWordAction::Clicked{word, gloss}
```

配套：含注音的段落用大行距模板（Html 实例 `text_style_normal.line_spacing: 2.0`，bold 同设）给释义留空间；PortalList 按内容含 `<ruby` 自动选模板。

**标记格式约定**：`<ruby t="释义">word</ruby>`——释义放属性。因为扩展点把标签内层文本整体作为 widget 文本，标准 `<rt>` 子标签反而难处理。EPUB 原文自带的标准 `<ruby><rt>` 用 `strip_ruby()` 降级为纯文本。

## 三轮试错的教训（Makepad 通用）

1. **内联富文本不要做成盒子**。两层 Label 的 widget 盒子被 TextFlow 顶对齐塞进行内，释义占正文行、原词被挤下去。凡是要和正文混排的内容，**必须经 `tf.draw_text` 绘制**（HtmlLink 是标准范本），装饰物用 `draw_abs` 叠加。
2. **自建 DrawText 必须从主题取字体**：`text_style: theme.font_regular{ font_size: 7.5 }`。只设字号/颜色会得到空字体族（`WARNING: encountered empty font family`），内容静默不渲染。
3. **TextFlow 的行距旋钮是 `text_style_normal/bold/italic/fixed`**。它每个 run 都会用这些字段整体覆盖 `draw_text.text_style`（text_flow.rs:1866），直接设 `draw_text.text_style.line_spacing` 无效。
4. Html 自定义标签 widget 读属性的姿势：`ScriptHook::on_after_new_scoped` 里 `scope.props.get::<HtmlDoc>()` + `new_walker_with_index(scope.index + 1)` + `while_attr_lc()`（HtmlLink 读 href 同款）。
5. 命中区域用 `drawn_areas`（跨行断词每段都能点）；`Cx::update_area_refs` 保持区域随重绘更新。

## EPUB 注音管线：老项目复用方案

老项目（Tauri 版）链路：**数据层全在 Rust，注入层在前端 JS**。

| 老项目模块 | 内容 | 复用方式 |
|-----------|------|---------|
| `schema.sql` 四张表 | `word_glosses`（全局 lemma→释义缓存）、`book_words`（每书生词+首现例句）、`book_glosses`（每书语境释义）、`known_words`（已认识） | **原样复用**（sqlx 纯 Rust），M1 随 readwise-core 迁移 |
| `core/vocab`（commands/models） | 增删查 glosses/known/vocab_book 的 Rust 逻辑 | **近乎原样**，去掉 tauri 命令宏改为普通 pub fn |
| `tauri-plugin-epub`（reader/toc_parser/sanitizer） | EPUB 解压、章节抽取、HTML 清洗 | **原样复用**，是注音的上游 |
| `vocab-service.ts` 的 `normalizeToken/lemmatize/getWordLevel` | 弯引号统一、缩写剥离（n't/'s/'ll…）、查 lemma 表、词表等级判定 | **JS→Rust 移植**（~50 行纯函数 + 加载 lemma/词级 JSON 数据文件，逻辑照抄） |
| `gloss-annotator.ts`（GlossAnnotator） | 遍历 DOM 文本节点、`/[A-Za-z][A-Za-z'’-]*/` 分词、跳过 code/pre/ruby 等容器、包 `<ruby><rt>` | **JS→Rust 重写为字符串变换**：章节 HTML → 走标签流跳过 skip 标签 → 文本节点分词 → 生词包 `<ruby t="">`。比老版逐 iframe DOM 修改更简单（一次预处理，无 DOM 生命周期问题） |
| 点词气泡（已认识/查详情） | DOM 气泡 | M4 用 Makepad Modal/Popup 重做，事件源就是 `RubyWordAction::Clicked` |
| AI 释义生成队列（gloss-queue） | 前端队列调 AI | M5 用 makepad_ai 重做，DB 缓存逻辑不变 |

**结论**：数据层（四张表 + vocab 逻辑 + EPUB 解析）复用率 ~90%；需要移植的只有两块小而纯的 JS（词形还原 ~50 行、注入器改为 HTML 字符串变换），且新架构比旧的 DOM 注入更简单。注音渲染层（本 demo 的 RubyWord）已验证完毕。

## 遗留优化项（非阻塞）

- 释义与词的水平居中在部分词上略有偏移，怀疑测量宽度与实绘宽度的 font_scale 差异，做真实阅读器时校准。
- 段落首行的释义上方是段间距，与上一元素的间距在真实排版里统一调。
- 长释义（比词宽很多）可能与相邻词的释义碰撞，注入时可限制释义长度（老项目 gloss 本来就要求"简短中文释义"）。
