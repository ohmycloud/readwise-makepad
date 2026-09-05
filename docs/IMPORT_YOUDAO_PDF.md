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
- 相同原形的多条 PDF 记录合并，新词保留去重后的完整释义。音标、原词形、每条原始释义以及对应 PDF、页码、编号都保留在 `entries.csv` 中；当前数据库没有音标字段，脚本不修改表结构。
- 给单个英文词补充 `word_glosses` 中缺失的简短中文释义，供阅读时的生词注音使用。沿用项目的简短释义提取规则，已有注音缓存保持不变。
- 正式写入前使用 SQLite backup API 创建一致性备份，随后在单个事务中导入；异常时回滚，成功前检查行数和数据库完整性。重复导入不会新增重复单词，也不会重置学习进度。

## 产物与验证

每次预览/导入输出 `entries.csv`（完整原始词条）、`outcomes.json`（每个原形的处理结果）、`summary.json`（数量、PDF SHA-256、数据库路径）。正式导入额外输出 `database-before.sqlite3`。预览的 `vocab_after` 为预计总量，正式导入后为已核对的实际总量；`applied` 标识是否已提交数据库事务。

应用会缓存生词列表及注音词库，导入后重新启动 ReadWise 再查看。备份恢复需先退出应用，并妥善处理 SQLite WAL 文件，不要在运行时直接覆盖数据库。

运行回归测试：

```bash
target/youdao-import-venv/bin/python -m unittest discover -s scripts -p 'test_import_youdao_pdf.py'
```
