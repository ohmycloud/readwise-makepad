"""Run with: python -m unittest discover -s scripts -p 'test_import_youdao_pdf.py'."""

import sqlite3
import tempfile
import unittest
from pathlib import Path

import pymupdf

from import_youdao_pdf import Entry, ROOT, group_entries, import_entries, parse_pdf


class PdfImportTests(unittest.TestCase):
    def test_real_cell_borders_handle_unequal_row_heights_and_phonetics(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "table.pdf"
            with pymupdf.open() as document:
                page = document.new_page()
                for x, value in [(50, "NO."), (150, "Word"), (350, "Meaning")]:
                    page.insert_text((x, 50), value)
                for y in [60, 180, 220]:
                    page.draw_rect(pymupdf.Rect(85, y, 255, y + 0.75), fill=(0, 0, 0))
                for point, value in [
                    ((55, 122), "1"),
                    ((150, 115), "hollow"),
                    ((150, 135), "[ipa]"),
                    ((260, 78), "first line"),
                    ((260, 95), "continued"),
                    ((260, 164), "last line of the tall first row"),
                    ((55, 200), "2"),
                    ((150, 200), "empty"),
                ]:
                    page.insert_text(point, value)
                document.save(path)
            entries, metadata = parse_pdf(path)
            self.assertEqual(metadata["rows"], 2)
            self.assertEqual(entries[0].word, "hollow")
            self.assertEqual(entries[0].phonetic, "[ipa]")
            self.assertEqual(
                entries[0].gloss, "first line continued last line of the tall first row"
            )
            self.assertEqual(entries[1].word, "empty")
            self.assertEqual(entries[1].gloss, "")

    def test_missing_row_border_aborts_instead_of_dropping_a_word(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "broken.pdf"
            with pymupdf.open() as document:
                page = document.new_page()
                for x, value in [(50, "NO."), (150, "Word"), (350, "Meaning")]:
                    page.insert_text((x, 50), value)
                for y in [60, 100]:
                    page.draw_rect(pymupdf.Rect(85, y, 255, y + 0.75), fill=(0, 0, 0))
                page.insert_text((55, 85), "1")
                page.insert_text((150, 85), "first")
                page.insert_text((55, 140), "2")
                page.insert_text((150, 140), "unaccounted")
                document.save(path)
            with self.assertRaisesRegex(ValueError, "row accounting mismatch"):
                parse_pdf(path)

    def test_legacy_database_preview_does_not_create_custom_table_and_apply_backfills_it(self):
        connection = sqlite3.connect(":memory:", isolation_level=None)
        self.addCleanup(connection.close)
        connection.executescript((ROOT / "core/src/schema.sql").read_text())
        connection.execute("DROP TABLE custom_words")
        grouped = group_entries([Entry("a.pdf", 1, 1, "forests", "", "n.森林")])
        preview, _ = import_entries(connection, grouped, False)
        self.assertEqual(preview["new_custom_words"], 1)
        self.assertEqual(connection.execute("PRAGMA table_info(custom_words)").fetchall(), [])
        connection.execute("BEGIN IMMEDIATE")
        actual, _ = import_entries(connection, grouped, True)
        connection.commit()
        self.assertEqual(preview, actual)
        self.assertEqual(connection.execute("SELECT lemma,gloss,source FROM custom_words").fetchall(), [("forest", "森林", "youdao")])

    def test_import_preserves_progress_fills_missing_glosses_and_is_idempotent(self):
        connection = sqlite3.connect(":memory:", isolation_level=None)
        self.addCleanup(connection.close)
        connection.executescript((ROOT / "core/src/schema.sql").read_text())
        connection.execute(
            "INSERT INTO vocab_book VALUES ('uncover','uncovered','自定义释义',2,'例句',"
            "NULL,'mastered',9,1,2)"
        )
        connection.execute(
            "INSERT INTO vocab_book VALUES ('vibrate','vibrate',NULL,3,NULL,"
            "NULL,'learning',4,1,2)"
        )
        connection.execute("INSERT INTO known_words VALUES ('sacred','marked',1)")
        connection.execute(
            "INSERT INTO word_glosses VALUES ('hollow','保留的注音',2,1)"
        )
        entries = [
            Entry("a.pdf", 1, 1, "uncovered", "", "揭露"),
            Entry("a.pdf", 1, 2, "hollow", "[ipa]", "adj.空心的；凹陷的"),
            Entry("b.pdf", 1, 1, "Hollow", "", "adj.空心的；凹陷的"),
            Entry("b.pdf", 1, 2, "vibrate", "", "v.振动；摇摆"),
            Entry("b.pdf", 1, 3, "sacred", "", "adj.神圣的"),
            Entry("b.pdf", 1, 4, "a big no-no", "", ""),
            Entry("b.pdf", 1, 5, "频率", "", "frequency;"),
        ]
        grouped = group_entries(entries)
        preserved = connection.execute(
            "SELECT * FROM vocab_book WHERE lemma='uncover'"
        ).fetchone()
        preview, _ = import_entries(connection, grouped, False)
        self.assertEqual(
            connection.execute("SELECT count(*) FROM vocab_book").fetchone()[0], 2
        )
        connection.execute("BEGIN IMMEDIATE")
        actual, _ = import_entries(connection, grouped, True)
        connection.commit()
        self.assertEqual(preview, actual)
        self.assertEqual(actual["new_words"], 4)
        self.assertEqual(actual["filled_glosses"], 1)
        self.assertEqual(actual["new_custom_words"], len(grouped))
        self.assertEqual(connection.execute("SELECT count(*) FROM custom_words").fetchone()[0], len(grouped))
        self.assertEqual(connection.execute("SELECT gloss,source FROM custom_words WHERE lemma='hollow'").fetchone(), ("空心的", "youdao"))
        self.assertEqual(connection.execute("SELECT source FROM custom_words WHERE lemma='sacred'").fetchone(), ("youdao",))
        self.assertEqual(
            preserved,
            connection.execute(
                "SELECT * FROM vocab_book WHERE lemma='uncover'"
            ).fetchone(),
        )
        self.assertEqual(
            connection.execute(
                "SELECT status,review_count,created_at,gloss FROM vocab_book WHERE lemma='vibrate'"
            ).fetchone(),
            ("learning", 4, 1, "v.振动；摇摆"),
        )
        self.assertEqual(
            connection.execute(
                "SELECT status FROM vocab_book WHERE lemma='sacred'"
            ).fetchone()[0],
            "mastered",
        )
        self.assertEqual(
            connection.execute(
                "SELECT gloss FROM word_glosses WHERE lemma='hollow'"
            ).fetchone()[0],
            "保留的注音",
        )
        before = connection.total_changes
        repeated, _ = import_entries(connection, grouped, True)
        self.assertEqual(connection.total_changes, before)
        self.assertEqual(repeated["new_words"], 0)
        self.assertEqual(repeated["filled_glosses"], 0)
        self.assertEqual(repeated["new_reader_glosses"], 0)
        self.assertEqual(repeated["new_custom_words"], 0)
        self.assertEqual(repeated["filled_custom_glosses"], 0)


if __name__ == "__main__":
    unittest.main()
