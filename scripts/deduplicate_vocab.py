#!/usr/bin/env python3
"""Audit/clean repeated gloss lines. Dry-run by default; --apply backs up first."""

import argparse
import hashlib
import json
import sqlite3
from contextlib import closing
from pathlib import Path

from vocab_glosses import deduplicate_glosses


def connect(path, writable=False):
    return sqlite3.connect(
        path.resolve().as_uri() + ("?mode=rw" if writable else "?mode=ro"),
        uri=True, isolation_level=None, timeout=30,
    )


def protected_fingerprint(connection, excluded_gloss_tables=("vocab_book",)):
    """Every persisted value except the gloss field we are authorized to clean."""
    digest = hashlib.sha256()
    for (table,) in connection.execute(
        "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"
    ):
        quoted = '"' + table.replace('"', '""') + '"'
        columns = [
            row[1] for row in connection.execute(f"PRAGMA table_info({quoted})")
            if not (table in excluded_gloss_tables and row[1] == "gloss")
        ]
        names = ",".join('"' + c.replace('"', '""') + '"' for c in columns)
        digest.update(repr((table, columns)).encode())
        rows = sorted(repr(row) for row in connection.execute(f"SELECT {names} FROM {quoted}"))
        for row in rows:
            digest.update((row + "\n").encode())
    return digest.hexdigest()


def plan_cleanup(connection):
    changes = []
    for lemma, gloss in connection.execute("SELECT lemma,gloss FROM vocab_book ORDER BY lemma"):
        cleaned, removed = deduplicate_glosses([gloss])
        if removed:
            changes.append(dict(lemma=lemma, before=gloss, after=cleaned, removed=removed))
    return changes


def run(database, report_dir, apply=False):
    report_dir.mkdir(parents=True, exist_ok=False)
    with closing(connect(database, writable=apply)) as connection:
        connection.execute("BEGIN IMMEDIATE" if apply else "BEGIN")
        try:
            before = protected_fingerprint(connection)
            changes = plan_cleanup(connection)
            count = connection.execute("SELECT count(*) FROM vocab_book").fetchone()[0]
            backup = report_dir / "database-before.sqlite3"
            if apply:
                # A separate reader can back up while this connection holds the
                # writer lock, so the backup matches the exact pre-clean state.
                with closing(connect(database)) as source, closing(sqlite3.connect(backup)) as target:
                    source.backup(target)
                    assert target.execute("PRAGMA quick_check").fetchall() == [("ok",)]
                    assert protected_fingerprint(target) == before
                    assert plan_cleanup(target) == changes
                for change in changes:
                    result = connection.execute(
                        "UPDATE vocab_book SET gloss=? WHERE lemma=? AND gloss=?",
                        (change["after"], change["lemma"], change["before"]),
                    )
                    assert result.rowcount == 1
                assert not plan_cleanup(connection), "cleanup must be idempotent"
                assert protected_fingerprint(connection) == before, "unrelated data changed"
                assert connection.execute("PRAGMA integrity_check").fetchall() == [("ok",)]
            summary = dict(
                applied=apply, database=str(database.resolve()), words=count,
                changed_words=len(changes), removed_lines=sum(c["removed"] for c in changes),
                backup=str(backup.resolve()) if apply else None,
                protected_sha256=before,
            )
            for name, value in [("changes.json", changes), ("summary.json", summary)]:
                (report_dir / name).write_text(
                    json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
                )
            if apply:
                connection.commit()
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
    parser.add_argument("--apply", action="store_true")
    args = parser.parse_args()
    print(json.dumps(run(args.database, args.report_dir, args.apply), ensure_ascii=False, indent=2))
