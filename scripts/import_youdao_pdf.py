#!/usr/bin/env python3
# /// script
# requires-python = ">=3.10"
# dependencies = ["PyMuPDF==1.26.7"]
# ///
"""Import numbered Youdao PDF tables into an existing ReadWise database.

Preview by default; pass --apply to back up the database and import atomically.
Uses actual PDF table borders and text baselines, not plain-text line splitting.
"""

import argparse
import csv
import hashlib
import json
import os
import re
import sqlite3
import sys
import time
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path

import pymupdf

from generate_starter_glossary import short_gloss

ROOT = Path(__file__).resolve().parents[1]
CJK = re.compile(r"[\u3400-\u9fff]")


@dataclass
class Entry:
    source: str
    page: int
    number: int
    word: str
    phonetic: str
    gloss: str
    lemma: str = ""
    level: int = 5


def join_lines(lines: list[str]) -> str:
    """Join wrapped cells, retaining spaces in English but not between CJK."""
    result = ""
    for line in lines:
        line = line.strip()
        if not line:
            continue
        space = bool(
            result
            and result[-1].isascii()
            and result[-1].isalnum()
            and line[0].isascii()
            and line[0].isalnum()
        )
        result += (" " if space else "") + line
    return result


def cell_lines(spans: list[dict]) -> list[str]:
    rows: list[list[dict]] = []
    for span in sorted(spans, key=lambda s: (s["origin"][1], s["bbox"][0])):
        if not rows or abs(rows[-1][0]["origin"][1] - span["origin"][1]) > 1.0:
            rows.append([])
        rows[-1].append(span)
    lines = []
    for row in rows:
        text = ""
        right = None
        for span in sorted(row, key=lambda s: s["bbox"][0]):
            part = span["text"].replace("\u00a0", " ").replace("\u00ad", "")
            if right is not None and span["bbox"][0] - right > 1.0:
                text += " "
            text += part
            right = span["bbox"][2]
        lines.append(text.strip())
    return lines


def parse_page(page, source: str) -> list[Entry]:
    flags = pymupdf.TEXTFLAGS_DICT & ~pymupdf.TEXT_PRESERVE_IMAGES
    spans = [
        span
        for block in page.get_text("dict", flags=flags)["blocks"]
        for line in block.get("lines", [])
        for span in line["spans"]
    ]
    headers = {
        s["text"].strip(): s
        for s in spans
        if s["text"].strip() in ("NO.", "Word", "Meaning")
    }
    if set(headers) != {"NO.", "Word", "Meaning"}:
        raise ValueError(f"{source} page {page.number + 1}: missing table headers")
    header = headers["Word"]
    center = (header["bbox"][0] + header["bbox"][2]) / 2
    # The export draws borders as thin filled rectangles. Matching the Word
    # header selects this column even when column widths change across exports.
    borders = [
        item[1]
        for draw in page.get_drawings()
        for item in draw["items"]
        if item[0] == "re"
        and 0 < item[1].height <= 1.5
        and item[1].width > 30
        and item[1].x0 < center < item[1].x1
        and item[1].y0 > header["bbox"][3]
    ]
    if len(borders) < 2:
        raise ValueError(f"{source} page {page.number + 1}: missing row borders")
    left, right = borders[0].x0, borders[0].x1
    boundaries = sorted(
        {
            round((r.y0 + r.y1) / 2, 2)
            for r in borders
            if abs(r.x0 - left) < 1 and abs(r.x1 - right) < 1
        }
    )
    entries = []
    for top, bottom in zip(boundaries, boundaries[1:]):
        cells: list[list[dict]] = [[], [], []]
        for span in spans:
            x, y = span["origin"]
            if top < y < bottom:
                cells[0 if x < left else 1 if x < right else 2].append(span)
        number_text = join_lines(cell_lines(cells[0]))
        if not number_text.isdecimal():
            raise ValueError(
                f"{source} page {page.number + 1}: invalid row {number_text!r}"
            )
        word_lines = cell_lines(cells[1])
        phonetic_start = next(
            (i for i, line in enumerate(word_lines) if line.startswith("[")),
            len(word_lines),
        )
        word = join_lines(word_lines[:phonetic_start])
        if not word or "\ufffd" in word:
            raise ValueError(f"{source} row {number_text}: missing or undecodable word")
        entries.append(
            Entry(
                source,
                page.number + 1,
                int(number_text),
                word,
                join_lines(word_lines[phonetic_start:]),
                join_lines(cell_lines(cells[2])),
            )
        )
    # Account independently for every printed row number, including clipped or
    # unsupported rows that otherwise could silently disappear from the import.
    printed = [
        int(s["text"].strip())
        for s in spans
        if s["origin"][0] < left
        and s["origin"][1] > header["bbox"][3]
        and s["text"].strip().isdecimal()
    ]
    if printed != [entry.number for entry in entries]:
        raise ValueError(f"{source} page {page.number + 1}: row accounting mismatch")
    return entries


def parse_pdf(path: Path) -> tuple[list[Entry], dict]:
    entries = []
    with pymupdf.open(path) as document:
        for page in document:
            entries.extend(parse_page(page, str(path)))
            if (page.number + 1) % 100 == 0:
                print(
                    f"{path.name}: {page.number + 1}/{len(document)} pages",
                    file=sys.stderr,
                )
        pages = len(document)
    if not entries or [e.number for e in entries] != list(range(1, len(entries) + 1)):
        raise ValueError(f"{path}: row numbers must be complete and sequential from 1")
    with path.open("rb") as source:
        digest = hashlib.sha256()
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return entries, {
        "path": str(path),
        "sha256": digest.hexdigest(),
        "pages": pages,
        "rows": len(entries),
        "empty_glosses": sum(not e.gloss for e in entries),
        "cjk_headwords": sum(bool(CJK.search(e.word)) for e in entries),
    }


def normalize_token(word: str) -> str:
    """Match core/src/vocab/lemma.rs; phrases keep their full lookup key."""
    word = " ".join(word.lower().replace("\u2019", "'").split())
    if " " in word:
        return word
    if word.endswith("n't"):
        return word[:-3]
    for suffix in ("'s", "'ll", "'ve", "'re", "'d", "'m", "'t"):
        if word.endswith(suffix):
            return word[: -len(suffix)]
    return word


def group_entries(entries: list[Entry]) -> dict[str, list[Entry]]:
    data = ROOT / "core/src/vocab/data"
    lemmas = json.loads((data / "lemmas-en.json").read_text(encoding="utf-8"))
    levels = json.loads((data / "word-levels.json").read_text(encoding="utf-8"))
    grouped: dict[str, list[Entry]] = {}
    for entry in entries:
        normalized = normalize_token(entry.word)
        entry.lemma = lemmas.get(normalized, normalized)
        entry.level = levels.get(normalized, levels.get(entry.lemma, 5))
        if not entry.lemma:
            raise ValueError(f"{entry.source} row {entry.number}: empty normalized key")
        grouped.setdefault(entry.lemma, []).append(entry)
    return grouped


def default_database() -> Path:
    if sys.platform == "darwin":
        base = Path.home() / "Library/Application Support"
    elif sys.platform == "win32":
        base = Path(os.environ["APPDATA"])
    else:
        base = Path(os.environ.get("XDG_DATA_HOME", str(Path.home() / ".local/share")))
    return base / "ReadWise/database/app.db"


def connect_database(path: Path, writable: bool = False) -> sqlite3.Connection:
    # mode=rw prevents a misspelled path from silently creating an empty DB.
    connection = sqlite3.connect(
        path.resolve().as_uri() + ("?mode=rw" if writable else "?mode=ro"),
        uri=True,
        timeout=30,
        isolation_level=None,
    )
    connection.execute("PRAGMA foreign_keys=ON")
    for table, columns in {
        "vocab_book": {
            "lemma",
            "word",
            "gloss",
            "level",
            "context_sentence",
            "book_id",
            "status",
            "review_count",
            "created_at",
            "updated_at",
        },
        "known_words": {"lemma"},
        "word_glosses": {"lemma", "gloss", "level", "created_at"},
    }.items():
        actual = {row[1] for row in connection.execute(f"PRAGMA table_info({table})")}
        if not columns <= actual:
            connection.close()
            raise ValueError(f"{path}: incompatible ReadWise table {table}")
    return connection


def import_entries(
    connection: sqlite3.Connection, grouped: dict[str, list[Entry]], apply: bool
) -> tuple[dict, list[dict]]:
    counts = dict(
        new_words=0, filled_glosses=0, unchanged_words=0, new_reader_glosses=0,
        new_custom_words=0, filled_custom_glosses=0,
    )
    existing = {
        r[0]: r[1] for r in connection.execute("SELECT lemma, gloss FROM vocab_book")
    }
    known = {r[0] for r in connection.execute("SELECT lemma FROM known_words")}
    reader_glosses = {
        r[0] for r in connection.execute("SELECT lemma FROM word_glosses")
    }
    # Older databases have no provenance for previous Youdao imports. Re-import
    # adds C3 membership without relabelling difficulty or resetting progress.
    has_custom = bool(connection.execute("PRAGMA table_info(custom_words)").fetchall())
    custom = dict(connection.execute("SELECT lemma, gloss FROM custom_words")) if has_custom else {}
    if apply and not has_custom:
        connection.execute(
            "CREATE TABLE custom_words (lemma TEXT PRIMARY KEY, gloss TEXT, "
            "source TEXT NOT NULL DEFAULT 'youdao', created_at INTEGER NOT NULL)"
        )
    now = time.time_ns() // 1_000_000
    outcomes = []
    for lemma, entries in grouped.items():
        # Prefer a definition of the lemma itself over an inflected form. All
        # source definitions remain in the full gloss and the per-row audit.
        ordered = sorted(
            entries, key=lambda e: (e.word.lower() != lemma, not bool(e.gloss))
        )
        primary = ordered[0]
        gloss = "\n".join(dict.fromkeys(e.gloss for e in ordered if e.gloss)) or None
        if lemma not in existing:
            outcome = "new_words"
            if apply:
                connection.execute(
                    "INSERT INTO vocab_book (lemma,word,gloss,level,context_sentence,book_id,"
                    "status,review_count,created_at,updated_at) VALUES (?,?,?,?,NULL,NULL,?,0,?,?)",
                    (
                        lemma,
                        primary.word,
                        gloss,
                        primary.level,
                        "mastered" if lemma in known else "learning",
                        now,
                        now,
                    ),
                )
        elif not (existing[lemma] or "").strip() and gloss:
            outcome = "filled_glosses"
            if apply:
                connection.execute(
                    "UPDATE vocab_book SET gloss=?,updated_at=? WHERE lemma=?",
                    (gloss, now, lemma),
                )
        else:
            outcome = "unchanged_words"
        counts[outcome] += 1
        outcomes.append(
            {
                "lemma": lemma,
                "word": primary.word,
                "source_rows": len(entries),
                "result": outcome,
            }
        )
        # Only fill missing, short Chinese reading glosses for single English
        # words. Never replace the user's existing annotation cache.
        brief = (
            short_gloss(gloss or "")
            if re.fullmatch(r"[a-z]+(?:['-][a-z]+)*", lemma)
            else None
        )
        if lemma not in custom:
            counts["new_custom_words"] += 1
            if apply:
                connection.execute(
                    "INSERT INTO custom_words (lemma,gloss,source,created_at) VALUES (?,?,'youdao',?)",
                    (lemma, brief, now),
                )
        elif not (custom[lemma] or "").strip() and brief:
            counts["filled_custom_glosses"] += 1
            if apply:
                connection.execute("UPDATE custom_words SET gloss=? WHERE lemma=?", (brief, lemma))
        if brief and lemma not in reader_glosses:
            counts["new_reader_glosses"] += 1
            if apply:
                connection.execute(
                    "INSERT INTO word_glosses (lemma,gloss,level,created_at) "
                    "VALUES (?,?,?,?)",
                    (lemma, brief, primary.level, now),
                )
    return counts, outcomes


def write_json(path: Path, value) -> None:
    with path.open("w", encoding="utf-8") as output:
        json.dump(value, output, ensure_ascii=False, indent=2)
        output.write("\n")


def run(args) -> dict:
    entries, sources = [], []
    for path in args.pdfs:
        rows, metadata = parse_pdf(path.resolve())
        entries.extend(rows)
        sources.append(metadata)
    grouped = group_entries(entries)
    args.report_dir.mkdir(parents=True, exist_ok=False)
    with (args.report_dir / "entries.csv").open(
        "w", newline="", encoding="utf-8-sig"
    ) as output:
        writer = csv.DictWriter(output, fieldnames=list(Entry.__dataclass_fields__))
        writer.writeheader()
        writer.writerows(asdict(entry) for entry in entries)
    summary = {
        "applied": False,
        "database": str(args.database.resolve()),
        "sources": sources,
        "source_rows": len(entries),
        "unique_lemmas": len(grouped),
        "merged_rows": len(entries) - len(grouped),
    }
    connection = connect_database(args.database, writable=args.apply)
    try:
        if args.apply:
            backup_path = args.report_dir / "database-before.sqlite3"
            with sqlite3.connect(backup_path) as backup:
                connection.backup(backup)
            summary["backup"] = str(backup_path.resolve())
            connection.execute("BEGIN IMMEDIATE")
        else:
            connection.execute("BEGIN")
        summary["vocab_before"] = connection.execute(
            "SELECT count(*) FROM vocab_book"
        ).fetchone()[0]
        counts, outcomes = import_entries(connection, grouped, args.apply)
        summary.update(counts)
        summary["vocab_after"] = summary["vocab_before"] + counts["new_words"]
        write_json(args.report_dir / "outcomes.json", outcomes)
        if args.apply:
            actual = connection.execute("SELECT count(*) FROM vocab_book").fetchone()[0]
            if actual != summary["vocab_after"]:
                raise ValueError("post-import word count does not match plan")
            integrity = connection.execute("PRAGMA quick_check").fetchall()
            if integrity != [("ok",)]:
                raise ValueError(f"database check failed: {integrity}")
            connection.commit()
            summary["applied"] = True
        else:
            connection.rollback()
        write_json(args.report_dir / "summary.json", summary)
    except BaseException:
        connection.rollback()
        raise
    finally:
        connection.close()
    return summary


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pdfs", nargs="+", type=Path)
    parser.add_argument("--database", type=Path, default=default_database())
    parser.add_argument(
        "--apply", action="store_true", help="back up and write; default is preview"
    )
    parser.add_argument(
        "--report-dir",
        type=Path,
        default=ROOT
        / "target/youdao-import"
        / datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ"),
        help="new directory for row audit, summary and database backup",
    )
    args = parser.parse_args()
    try:
        summary = run(args)
    except (ValueError, OSError, sqlite3.Error, pymupdf.FileDataError) as error:
        parser.exit(1, f"Import failed: {error}\n")
    print(json.dumps(summary, ensure_ascii=False, indent=2))
    print(f"Report: {args.report_dir.resolve()}")


if __name__ == "__main__":
    main()
