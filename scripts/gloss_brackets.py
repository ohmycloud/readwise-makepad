"""Bracket checks and narrowly scoped corrections for dictionary source typos."""

OPENERS = {"(": ")", "（": "）", "[": "]", "【": "】", "{": "}"}
CLOSERS = {close: opening for opening, close in OPENERS.items()}

# Keep wording intact; these are exact, audited fragments, never fuzzy repairs.
SOURCE_CORRECTIONS = {
    "gutter": [("流，淌（(gutter down）", "流，淌（gutter down）")],
    "snapshotting": [("快照(照片))(snapshot的现在分词)", "快照(照片)(snapshot的现在分词)")],
    "receipt": [("n. 企业、银行等）收到的款", "n. （企业、银行等）收到的款")],
    "prescription": [("（=positive prescription)）", "（=positive prescription）")],
    "mold": [("塑造成；使）紧贴于", "塑造成；（使）紧贴于")],
    "grunt": [("（步兵或海军陆战队中的)）低等兵", "（步兵或海军陆战队中的）低等兵")],
    "ranting": [("气愤地)大叫大嚷", "(气愤地)大叫大嚷")],
}


def normalize_bracket_pairs(text: str) -> str:
    """A round closing bracket must use the style of its matching opener."""
    stack, result = [], []
    for char in text:
        if char in OPENERS:
            stack.append(char)
        elif char in CLOSERS and stack:
            opener = stack[-1]
            if (opener in "(（" and char in ")）") or OPENERS[opener] == char:
                stack.pop()
                char = OPENERS[opener]
        result.append(char)
    return "".join(result)


def balanced_brackets(text: str) -> bool:
    stack = []
    for char in text:
        if char in OPENERS:
            stack.append(char)
        elif char in CLOSERS:
            if not stack or OPENERS[stack.pop()] != char:
                return False
    return not stack


def correct_source_brackets(lemma: str, text: str) -> str:
    for before, after in SOURCE_CORRECTIONS.get(lemma, []):
        text = text.replace(before, after)
    return normalize_bracket_pairs(text)


def first_sense(text: str) -> str:
    """Only outer commas/semicolons separate senses; notes may contain commas."""
    stack = []
    for index, char in enumerate(text):
        if char in OPENERS:
            stack.append(char)
        elif char in CLOSERS:
            if stack and OPENERS[stack[-1]] == char:
                stack.pop()
        elif char in "；;,，" and not stack:
            return text[:index]
    return text


def without_bracket_notes(text: str) -> str:
    """Remove entire notes, including any truncated note tail, without inventing text."""
    stack, result = [], []
    for char in text:
        if char in OPENERS:
            stack.append(char)
        elif char in CLOSERS:
            if stack and OPENERS[stack[-1]] == char:
                stack.pop()
        elif not stack:
            result.append(char)
    return "".join(result)


def without_leading_notes(text: str) -> str:
    text = text.strip()
    while text and text[0] in OPENERS:
        stack = []
        end = None
        for index, char in enumerate(text):
            if char in OPENERS:
                stack.append(char)
            elif char in CLOSERS:
                if not stack or OPENERS[stack.pop()] != char:
                    return text
                if not stack:
                    end = index + 1
                    break
        if end is None:
            return text
        rest = text[end:].strip()
        if not has_cjk(rest):
            return text
        text = rest
    return text


def has_cjk(text: str) -> bool:
    return any("\u3400" <= char <= "\u9fff" for char in text)
