#!/usr/bin/env python3
"""Build local reading difficulty bands and short glosses (not official CEFR).

Usage: python scripts/generate_reader_vocabulary.py ECDICT_CSV OUTPUT_JSON
Uses the checked-in dictionary headwords, existing short-gloss cleanup, and
ECDICT BNC/COCA ranks. See core/src/vocab/data/ECDICT-LICENSE.md.
"""
import csv
import json
import sys
from pathlib import Path

from generate_starter_glossary import short_gloss

ROOT = Path(__file__).resolve().parents[1]
RANK_LIMITS = (1000, 2500, 5000, 10000, 20000, 50000)


def reading_band(bnc: str, frq: str, legacy: int | None) -> int:
    ranks = [int(value) for value in (bnc, frq) if value.isdecimal() and int(value) > 0]
    if ranks:
        rank = min(ranks)
        return next((i + 1 for i, limit in enumerate(RANK_LIMITS) if rank <= limit), 7)
    # The legacy school/exam tiers are a fallback, not CEFR labels.
    return {1: 2, 2: 3, 3: 4, 4: 5}.get(legacy, 7)


def main():
    source_path, output_path = map(Path, sys.argv[1:])
    data = ROOT / 'core/src/vocab/data'
    dictionary = json.loads((data / 'word-dictionary.json').read_text())
    legacy = json.loads((data / 'word-levels.json').read_text())
    bands = {}
    with source_path.open(encoding='utf-8-sig', newline='') as source:
        for row in csv.DictReader(source):
            word = row['word'].strip().lower()
            if word in dictionary and word not in bands:
                bands[word] = reading_band(row.get('bnc', ''), row.get('frq', ''), legacy.get(word))
    vocabulary = {}
    for word, (_, translation) in dictionary.items():
        gloss = short_gloss(translation)
        if gloss:
            vocabulary[word] = [bands.get(word, reading_band('', '', legacy.get(word))), gloss]
    output_path.write_text(json.dumps(dict(sorted(vocabulary.items())), ensure_ascii=False, separators=(',', ':')))
    print(f'wrote {len(vocabulary)} reading entries to {output_path}')


if __name__ == '__main__':
    main()
