"""Conservative comparison of imported glosses, preserving original wording."""

import re


PUNCTUATION = str.maketrans("，；．。：（）［］【】", ",;..:()[][]")
CJK = r"\u3400-\u9fff"


def gloss_key(line: str) -> str:
    text = re.sub(r"\s+", " ", line.translate(PUNCTUATION)).strip()
    # Ignore layout whitespace, but keep English word boundaries (a part/apart).
    text = re.sub(r"\s*([,;.:()\[\]])\s*", r"\1", text)
    text = re.sub(rf"(?<=[{CJK}])\s+(?=[{CJK}])", "", text)
    text = re.sub(r";+", ";", text)
    return text.rstrip(";")


def deduplicate_glosses(glosses) -> tuple[str, int]:
    """Drop equivalent whole lines; retain distinct senses and the first spelling."""
    seen, kept, removed = set(), [], 0
    for gloss in glosses:
        for line in (gloss or "").splitlines():
            if not line.strip():
                continue
            key = gloss_key(line)
            if key in seen:
                removed += 1
            else:
                seen.add(key)
                kept.append(line.strip())
    return "\n".join(kept), removed
