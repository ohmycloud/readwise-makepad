#!/usr/bin/env python3
"""Extract short level 2-4 Chinese glosses from an ECDICT CSV snapshot."""

import csv
import json
import re
import sys
from pathlib import Path

from gloss_brackets import (
    balanced_brackets,
    first_sense,
    normalize_bracket_pairs,
    without_bracket_notes,
    without_leading_notes,
)


CJK = re.compile(r"[\u3400-\u9fff]")
POS = re.compile(
    r"^(?:(?:n|v|vt|vi|adj|adv|prep|pron|conj|num|art|aux|abbr|int|a|ad|s)\.\s*)+",
    re.IGNORECASE,
)
BRACKET_TAG = re.compile(r"^\s*[\[【][^\]】]{1,8}[\]】]\s*")


def short_gloss(raw: str) -> str | None:
    lines = raw.replace("\\n", "\n").splitlines()
    candidates = [line.strip() for line in lines if CJK.search(line)]
    candidates.sort(key=lambda line: line.startswith(("[网络]", "【网络】")))
    for line in candidates:
        line = normalize_bracket_pairs(line)
        line = BRACKET_TAG.sub("", line)
        line = POS.sub("", line)
        line = BRACKET_TAG.sub("", line)
        line = without_leading_notes(first_sense(line)).strip(" ，。,.；;")
        if len(line) > 12 or not balanced_brackets(line):
            line = without_bracket_notes(line).strip(" ，。,.；;")
        if CJK.search(line):
            return line[:12]
    return None


def main() -> None:
    if len(sys.argv) != 4:
        raise SystemExit("usage: generate_starter_glossary.py LEVELS_JSON ECDICT_CSV OUTPUT_JSON")
    levels_path, csv_path, output_path = map(Path, sys.argv[1:])
    levels = json.loads(levels_path.read_text(encoding="utf-8"))
    wanted = {word.lower() for word, level in levels.items() if 2 <= level <= 4}
    glosses: dict[str, str] = {}
    with csv_path.open(encoding="utf-8-sig", newline="") as source:
        for row in csv.DictReader(source):
            word = (row.get("word") or "").strip().lower()
            if word not in wanted or word in glosses:
                continue
            gloss = short_gloss(row.get("translation") or "")
            if gloss:
                glosses[word] = gloss
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(
        json.dumps(dict(sorted(glosses.items())), ensure_ascii=False, separators=(",", ":")),
        encoding="utf-8",
    )
    print(f"wrote {len(glosses)} of {len(wanted)} requested glosses to {output_path}")


if __name__ == "__main__":
    main()
