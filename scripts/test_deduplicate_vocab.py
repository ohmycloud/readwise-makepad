import sqlite3
import tempfile
import unittest
from pathlib import Path
from contextlib import closing

from deduplicate_vocab import connect, plan_cleanup, protected_fingerprint, run
from vocab_glosses import deduplicate_glosses


class GlossCleanupTests(unittest.TestCase):
    def test_format_variants_keep_first_original(self):
        original = "adj. 详尽的，彻底的；消耗的"
        cleaned, removed = deduplicate_glosses([
            original, "adj.详尽的，彻底的；消耗的", "adj．详尽的,彻底的;消耗的;"
        ])
        self.assertEqual((cleaned, removed), (original, 2))

    def test_duplicate_separator_noise(self):
        self.assertEqual(deduplicate_glosses([
            "n. 疯狂；v. 使发狂", "n. 疯狂; ；v. 使发狂;"
        ]), ("n. 疯狂；v. 使发狂", 1))

    def test_distinct_meanings_pos_and_english_boundaries_survive(self):
        lines = ["n. 工作", "v. 工作", "n. 工作；职业", "a part", "apart", "US", "us"]
        self.assertEqual(deduplicate_glosses(lines), ("\n".join(lines), 0))

    def test_backup_dry_run_preservation_and_second_run(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            database = root / "app.db"
            with closing(sqlite3.connect(database, isolation_level=None)) as connection:
                connection.executescript(
                    (Path(__file__).resolve().parents[1] / "core/src/schema.sql").read_text()
                )
                connection.execute(
                    "INSERT INTO vocab_book VALUES ('exhaustive','exhaustive',?,5,"
                    "'例句',NULL,'mastered',12,100,200)",
                    ("adj. 详尽的，彻底的；消耗的\nadj.详尽的，彻底的；消耗的",),
                )
                connection.execute("INSERT INTO word_glosses VALUES ('exhaustive','详尽的',5,1)")
                connection.execute("INSERT INTO known_words VALUES ('exhaustive','marked',1)")
                before = protected_fingerprint(connection)
            preview = run(database, root / "preview")
            self.assertEqual(preview["changed_words"], 1)
            with closing(connect(database)) as connection:
                self.assertEqual(len(plan_cleanup(connection)), 1)
            applied = run(database, root / "applied", True)
            self.assertEqual(applied["removed_lines"], 1)
            with closing(connect(database)) as connection:
                self.assertEqual(protected_fingerprint(connection), before)
                self.assertEqual(connection.execute("SELECT gloss FROM vocab_book").fetchone(),
                                 ("adj. 详尽的，彻底的；消耗的",))
            with closing(connect(Path(applied["backup"]))) as backup:
                self.assertEqual(len(plan_cleanup(backup)), 1)
            self.assertEqual(run(database, root / "again", True)["changed_words"], 0)


if __name__ == "__main__":
    unittest.main()
