import json
import sqlite3
import tempfile
import unittest
from pathlib import Path
from contextlib import closing

from deduplicate_vocab import connect, protected_fingerprint
from generate_starter_glossary import short_gloss
from gloss_brackets import balanced_brackets, correct_source_brackets, first_sense
from repair_gloss_brackets import ROOT, TABLE_KEYS, gloss_snapshot, run


class ShortGlossTests(unittest.TestCase):
    def test_usage_notes_do_not_become_truncated_definitions(self):
        self.assertEqual(short_gloss(
            "adj. （尤指英国大学里的课程 ）分单元的（由独立单元组成，学生可选修）；组合式的；模块化的"
        ), "分单元的")
        self.assertEqual(short_gloss("adj. （物质，物体）坚韧的，坚固的"), "坚韧的")
        self.assertEqual(short_gloss("n. （仪式上庆祝或欢迎重要人物到场时）号角齐鸣，嘹亮短曲"), "号角齐鸣")

    def test_commas_inside_brackets_do_not_split_the_sense(self):
        self.assertEqual(first_sense("特征（如眼、口等，见正文），特色"), "特征（如眼、口等，见正文）")
        self.assertEqual(short_gloss("n. 特征（如眼、口），特色"), "特征（如眼、口）")

    def test_length_limit_removes_whole_notes_not_closing_brackets(self):
        self.assertEqual(short_gloss("vi. 道歉（等于apologize）"), "道歉")
        self.assertEqual(short_gloss("n. 朱庇物(罗马神话中的主神), 木星"), "朱庇物")
        self.assertEqual(short_gloss("n. 雪人（等于 Abominable Snowman）"), "雪人")
        self.assertEqual(short_gloss("n. 声音（很长的未闭合注释"), "声音")
        self.assertIsNone(short_gloss("adj. （很长的未闭合注释"))
        self.assertEqual(short_gloss("n. [计] 模块，组件"), "模块")
        self.assertEqual(short_gloss("n. [计] 模块，组件\\nv. 组合"), "模块")

    def test_source_typos_are_scoped_and_wording_is_preserved(self):
        self.assertEqual(correct_source_brackets("whole", "(强调大小）整个的"), "(强调大小)整个的")
        self.assertEqual(correct_source_brackets("receipt", "n. 企业、银行等）收到的款"),
                         "n. （企业、银行等）收到的款")
        self.assertEqual(correct_source_brackets("snapshotting", "快照(照片))(snapshot的现在分词)"),
                         "快照(照片)(snapshot的现在分词)")
        self.assertEqual(correct_source_brackets("other", "n. 企业、银行等）收到的款"),
                         "n. 企业、银行等）收到的款")

    def test_checked_in_gloss_assets_have_complete_brackets(self):
        for name in ["starter-glosses", "word-dictionary"]:
            data = json.loads((ROOT / f"core/src/vocab/data/{name}.json").read_text())
            broken = [lemma for lemma, value in data.items()
                      if not balanced_brackets(value if isinstance(value, str) else value[1])]
            self.assertEqual(broken, [], name)


class RepairDatabaseTests(unittest.TestCase):
    def test_preview_backup_exact_updates_progress_preservation_and_idempotency(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            database = root / "app.db"
            full = "adj. （尤指英国大学里的课程 ）分单元的（由独立单元组成，学生可选修）；模块化的"
            with closing(sqlite3.connect(database, isolation_level=None)) as connection:
                connection.executescript((ROOT / "core/src/schema.sql").read_text())
                connection.execute(
                    "INSERT INTO vocab_book VALUES ('modular','modular',?,5,'原句',NULL,'mastered',7,1,2)",
                    (full,),
                )
                connection.execute("INSERT INTO word_glosses VALUES ('modular',?,5,1)",
                                   ("（尤指英国大学里的课程 ",))
                connection.execute("INSERT INTO vocab_book VALUES "
                                   "('receipt','receipt',?,3,NULL,NULL,'learning',4,10,20)",
                                   ("n. 企业、银行等）收到的款",))
                original = gloss_snapshot(connection)
                protected = protected_fingerprint(connection, TABLE_KEYS)
            preview = run(database, root / "preview", {})
            self.assertEqual(preview["changes"], dict(word_glosses=1, book_glosses=0, vocab_book=1))
            with closing(connect(database)) as connection:
                self.assertEqual(gloss_snapshot(connection), original)
            actual = run(database, root / "actual", {}, True)
            with closing(connect(Path(actual["backup"]))) as backup:
                self.assertEqual(gloss_snapshot(backup), original)
            with closing(connect(database)) as connection:
                self.assertEqual(protected_fingerprint(connection, TABLE_KEYS), protected)
                self.assertEqual(connection.execute("SELECT gloss FROM word_glosses").fetchone(),
                                 ("分单元的",))
                self.assertEqual(connection.execute("SELECT gloss FROM vocab_book WHERE lemma='modular'").fetchone(),
                                 (full,))
            again = run(database, root / "again", {}, True)
            self.assertEqual(sum(again["changes"].values()), 0)

    def test_unknown_source_defect_blocks_apply_without_mutation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            database = root / "app.db"
            with closing(sqlite3.connect(database, isolation_level=None)) as connection:
                connection.executescript((ROOT / "core/src/schema.sql").read_text())
                connection.execute("INSERT INTO word_glosses VALUES ('custom','（未知残片',2,1)")
            self.assertEqual(run(database, root / "preview", {})["unresolved"], 1)
            with self.assertRaisesRegex(ValueError, "source review"):
                run(database, root / "apply", {}, True)
            with closing(connect(database)) as connection:
                self.assertEqual(connection.execute("SELECT gloss FROM word_glosses").fetchone(),
                                 ("（未知残片",))


if __name__ == "__main__":
    unittest.main()
