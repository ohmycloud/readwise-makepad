# 导入有道词典 PDF 生词表

`scripts/import_youdao_pdf.py` 读取带 `NO. / Word / Meaning` 三列的有道词典 PDF 导出表，导入现有 ReadWise SQLite 数据库。需要 Python 3.10+ 和 PyMuPDF 1.26.7；使用 `uv run` 时会根据脚本内的依赖声明自动安装。

## 使用

先预览，不写数据库：

```bash
uv run scripts/import_youdao_pdf.py \
  '/Users/ohmycloud/Downloads/有道词典655.pdf' \
  '/Users/ohmycloud/Downloads/有道词典1w.pdf'
```

正式导入，加 `--apply`：

```bash
uv run scripts/import_youdao_pdf.py \
  '/Users/ohmycloud/Downloads/有道词典655.pdf' \
  '/Users/ohmycloud/Downloads/有道词典1w.pdf' \
  --apply
```

本次工作区已经安装依赖，也可将 `uv run` 换成 `target/youdao-import-venv/bin/python`。

默认数据库是系统应用数据目录下的 `ReadWise/database/app.db`，macOS 对应 `~/Library/Application Support/ReadWise/database/app.db`。可用 `--database /完整路径/app.db` 指定其他数据库；路径必须已存在且表结构匹配，脚本不会新建应用数据库。

默认每次生成独立的 `target/youdao-import/<UTC时间>/` 目录。可通过 `--report-dir /新目录` 指定长期保存位置；目录已存在时会拒绝覆盖。`target` 不进入 Git，清理构建产物前可自行另存备份和明细。

## 数据规则

- 按 PDF 表格横线切分词条、按文字基线拼接释义，支持长释义、换行、音标、中文查询词、短语及空释义。逐页检查所有编号，再核对全书从 1 开始连续递增；缺行、重复编号或无法解码的词头会中止整个导入。
- 英文单词复用 `core/src/vocab/data/lemmas-en.json` 的词形归并和 `word-levels.json` 的等级；词表外等级为 5。短语保留完整内容，不逐词拆分；中文查询词按原样保留，不自动反向翻译。
- 新词写入 `vocab_book`，默认“学习中”；若已在 `known_words` 中，则沿用“已掌握”。已有词保留释义、状态、复习次数和来源，仅在释义为空时补全。
- 相同原形的多条 PDF 记录合并，新词保留去重后的完整释义。音标、原词形、每条原始释义以及对应 PDF、页码、编号都保留在 `entries.csv` 中；当前数据库没有音标字段。
- 给单个英文词补充 `word_glosses` 中缺失的简短中文释义，供阅读时的生词注音使用。沿用项目的简短释义提取规则，已有注音缓存保持不变。
- 所有导入原形单独写入 `custom_words`，来源为 `youdao`，归入「C3-自定义」。这是独立的词库成员关系，不把原有 `vocab_book.level` 改成 7，也不改变已认识、学习状态或复习次数。短中文注音使用导入释义；无可用短释义时由内置词库兜底；短语和中文查询词保留成员关系，不拆成多个单词注音。
- 旧导入记录没有来源字段，不能仅凭生词本内容可靠猜出有道词。对旧版数据库重新执行同一 PDF 的 `--apply`，会在事务中创建/补齐 `custom_words` 并登记 C3 成员，已有学习记录不变；预览不创建表。
- 正式写入前使用 SQLite backup API 创建一致性备份，随后在单个事务中导入；异常时回滚，成功前检查行数和数据库完整性。重复导入不会新增重复单词，也不会重置学习进度。

## 阅读水平与注音

设置 → 阅读 → 英文水平支持 A1、A2、B1、B2、C1、C2、C3。A1–C2 是自评入口，使用本地阅读难度分档：等级越高，注音集合越小；C3 不是正式 CEFR 等级，只按有道导入成员注音。两种模式都排除 `known_words`，关闭生词注音时均显示原文。

本地难度表 `reader-vocabulary.json` 由 ECDICT 的 BNC/COCA 词频排名近似分档，不冒充官方 CEFR 词表。取两套语料中的较小非零排名，以 1000、2500、5000、10000、20000、50000 为边界形成 1–6 档，超过或无已知难度的词为 7 档；缺排名时旧四档映射到 2–5 档。A1–C2 分别注音高于 1–6 档的词。离线短释义与生成方式见 `core/src/vocab/data/ECDICT-LICENSE.md`。

切换水平会重新生成本书注音章节及全书分页，按原文锚点保留阅读位置；标为已认识仍走原地隐藏释义路径，保留 ruby 槽位、当前页布局和翻页历史。

## 产物与验证

每次预览/导入输出 `entries.csv`（完整原始词条）、`outcomes.json`（每个原形的处理结果）、`summary.json`（数量、PDF SHA-256、数据库路径）。正式导入额外输出 `database-before.sqlite3`。预览的 `vocab_after` 为预计总量，正式导入后为已核对的实际总量；`applied` 标识是否已提交数据库事务。

应用会缓存生词列表及注音词库，导入后重新启动 ReadWise 再查看。备份恢复需先退出应用，并妥善处理 SQLite WAL 文件，不要在运行时直接覆盖数据库。

运行回归测试：

```bash
target/youdao-import-venv/bin/python -m unittest discover -s scripts -p 'test_import_youdao_pdf.py'
```
