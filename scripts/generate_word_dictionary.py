#!/usr/bin/env python3
"""Extract offline word-card data from the pinned ECDICT CSV (see attribution)."""
import csv
import json
import sys
from pathlib import Path

from gloss_brackets import correct_source_brackets


def main():
    if len(sys.argv) != 3:
        raise SystemExit("usage: generate_word_dictionary.py ECDICT_CSV OUTPUT_JSON")
    root = Path(__file__).resolve().parents[1] / "core/src/vocab/data"
    wanted = set(json.loads((root / "word-levels.json").read_text()))
    wanted.update(json.loads((root / "starter-glosses.json").read_text()))
    wanted.update(json.loads((root / "lemmas-en.json").read_text()).values())
    entries = {}
    with Path(sys.argv[1]).open(encoding="utf-8-sig", newline="") as source:
        for row in csv.DictReader(source):
            word = row["word"].strip().lower()
            translation = correct_source_brackets(
                word, row["translation"].replace("\\n", "\n").strip()
            )
            if word in wanted and word not in entries and translation:
                entries[word] = [row["phonetic"].strip(), translation]
    Path(sys.argv[2]).write_text(
        json.dumps(dict(sorted(entries.items())), ensure_ascii=False, separators=(",", ":")),
        encoding="utf-8",
    )
    print(f"wrote {len(entries)} entries")


if __name__ == "__main__":
    main()
