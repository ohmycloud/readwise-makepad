#!/usr/bin/env python3
"""Repair audited bracket defects from full local sources; dry-run by default."""

import argparse
import json
import sqlite3
from contextlib import closing
from pathlib import Path

from deduplicate_vocab import connect, protected_fingerprint
from generate_starter_glossary import short_gloss
from gloss_brackets import balanced_brackets, correct_source_brackets


ROOT = Path(__file__).resolve().parents[1]
TABLE_KEYS = {"word_glosses": ("lemma",), "book_glosses": ("book_id", "lemma"),
              "vocab_book": ("lemma",)}


def gloss_snapshot(connection):
    return {
        table: {row[:-1]: row[-1] for row in connection.execute(
            f"SELECT {','.join(keys)},gloss FROM {table}"
        )} for table, keys in TABLE_KEYS.items()
    }


def plan_repairs(connection, dictionary):
    before = gloss_snapshot(connection)
    changes, unresolved = [], []
    for table, rows in before.items():
        for key, gloss in rows.items():
            if not gloss or balanced_brackets(gloss):
                continue
            lemma = key[-1]
            source = gloss
            after = correct_source_brackets(lemma, gloss)
            reason = "source bracket typo / mixed-width pair"
            if table == "word_glosses":
                # Only replace a truncated short gloss if its exact text is
                # present in the local complete entry. No inferred translations.
                candidates = [before["vocab_book"].get((lemma,)),
                              dictionary.get(lemma, [None, None])[1]]
                source = next((s for s in candidates if s and gloss.strip() in s), None)
                after = short_gloss(correct_source_brackets(lemma, source)) if source else None
                reason = "re-extracted from matching full local entry"
            item = dict(table=table, key=key, lemma=lemma, before=gloss,
                        after=after, source=source, reason=reason)
            if not after or not balanced_brackets(after):
                unresolved.append(item)
            else:
                changes.append(item)
    return changes, unresolved


def run(database, report_dir, dictionary, apply=False):
    report_dir.mkdir(parents=True, exist_ok=False)
    with closing(connect(database, writable=apply)) as connection:
        connection.execute("BEGIN IMMEDIATE" if apply else "BEGIN")
        try:
            protected = protected_fingerprint(connection, TABLE_KEYS)
            snapshot = gloss_snapshot(connection)
            changes, unresolved = plan_repairs(connection, dictionary)
            backup = report_dir / "database-before.sqlite3"
            if apply:
                if unresolved:
                    raise ValueError(f"{len(unresolved)} entries still need source review")
                with closing(connect(database)) as source, closing(sqlite3.connect(backup)) as target:
                    source.backup(target)
                    assert target.execute("PRAGMA integrity_check").fetchall() == [("ok",)]
                    assert protected_fingerprint(target, TABLE_KEYS) == protected
                    assert gloss_snapshot(target) == snapshot
                expected = {table: dict(rows) for table, rows in snapshot.items()}
                for item in changes:
                    table, key = item["table"], item["key"]
                    where = " AND ".join(f"{column}=?" for column in TABLE_KEYS[table])
                    result = connection.execute(
                        f"UPDATE {table} SET gloss=? WHERE {where} AND gloss=?",
                        (item["after"], *key, item["before"]),
                    )
                    assert result.rowcount == 1
                    expected[table][key] = item["after"]
                assert gloss_snapshot(connection) == expected
                assert protected_fingerprint(connection, TABLE_KEYS) == protected
                assert plan_repairs(connection, dictionary) == ([], [])
                assert connection.execute("PRAGMA integrity_check").fetchall() == [("ok",)]
            summary = dict(
                applied=False, database=str(database.resolve()),
                changes={table: sum(item["table"] == table for item in changes) for table in TABLE_KEYS},
                unresolved=len(unresolved), backup=str(backup.resolve()) if apply else None,
                protected_sha256=protected,
            )
            for name, data in [("changes.json", changes), ("unresolved.json", unresolved),
                               ("summary.json", summary)]:
                (report_dir / name).write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n")
            if apply:
                connection.commit()
                summary["applied"] = True
                (report_dir / "summary.json").write_text(
                    json.dumps(summary, ensure_ascii=False, indent=2) + "\n"
                )
            else:
                connection.rollback()
            return summary
        except BaseException:
            connection.rollback()
            raise


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--report-dir", type=Path, required=True)
    parser.add_argument("--dictionary", type=Path,
                        default=ROOT / "core/src/vocab/data/word-dictionary.json")
    parser.add_argument("--apply", action="store_true")
    args = parser.parse_args()
    dictionary = json.loads(args.dictionary.read_text())
    print(json.dumps(run(args.database, args.report_dir, dictionary, args.apply),
                     ensure_ascii=False, indent=2))
