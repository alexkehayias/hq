#!/usr/bin/env python3
"""Flag inline `crate::` paths in staged Rust changes.

Writing a path such as `crate::core::db::async_db(..)` inline hides where the
current module reaches into the rest of the crate. Prefer a `use` import at the
top of the file so a module's dependencies are visible at a glance.

Only lines added in the staged diff are checked, so existing code is
grandfathered. Vendored code (src/bash/), doc comments, and `#[cfg(test)]`
modules are skipped.
"""

from __future__ import annotations

import re
import subprocess
import sys

VENDOR_PREFIX = "src/bash/"

CRATE_PATH = re.compile(r"\bcrate::")
USE_LINE = re.compile(r"^\s*(?:pub(?:\s*\([^)]*\))?\s+)?use\b")
CFG_TEST = re.compile(r"^\s*#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]")
ATTRIBUTE = re.compile(r"^\s*#\[")
PATH_TOKEN = re.compile(r"crate::[A-Za-z_][A-Za-z0-9_:]*")
HUNK = re.compile(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@")


def git(*args: str) -> str:
    return subprocess.run(
        ["git", *args], check=True, capture_output=True, text=True
    ).stdout


def staged_rust_files() -> list[str]:
    out = git("diff", "--cached", "--name-only", "--diff-filter=ACMR", "--", "*.rs")
    return [
        path for path in out.splitlines() if path and not path.startswith(VENDOR_PREFIX)
    ]


def added_line_numbers(path: str) -> list[int]:
    """New-file line numbers of lines added by the staged diff for `path`."""
    diff = git("diff", "--cached", "--no-color", "--unified=0", "--", path)
    numbers: list[int] = []
    new_line = 0
    for raw in diff.splitlines():
        hunk = HUNK.match(raw)
        if hunk:
            new_line = int(hunk.group(1))
            continue
        if raw.startswith(("+++", "---")):
            continue
        if raw.startswith("+"):
            numbers.append(new_line)
            new_line += 1
        elif raw.startswith("-"):
            continue
        elif raw.startswith(" "):
            new_line += 1
    return numbers


def _skip_raw_string(text: str, start: int) -> int | None:
    """If a raw string literal starts at `start`, return the index past it."""
    for prefix in ("br", "cr", "r"):
        if text.startswith(prefix, start):
            pos = start + len(prefix)
            hashes = 0
            while pos < len(text) and text[pos] == "#":
                hashes += 1
                pos += 1
            if pos < len(text) and text[pos] == '"':
                terminator = '"' + "#" * hashes
                end = text.find(terminator, pos + 1)
                return len(text) if end == -1 else end + len(terminator)
    return None


def _skip_normal_string(text: str, start: int) -> int:
    pos = start + 1
    while pos < len(text):
        if text[pos] == "\\":
            pos += 2
            continue
        if text[pos] == '"':
            return pos + 1
        pos += 1
    return len(text)


def _skip_char_or_lifetime(text: str, start: int) -> int:
    if text.startswith("\\", start + 1):
        pos = start + 2
        while pos < len(text) and text[pos] != "'":
            pos += 1
        return min(pos + 1, len(text))
    if start + 2 < len(text) and text[start + 2] == "'":
        return start + 3
    return start + 1


def strip_code(text: str, in_block_comment: bool) -> tuple[str, bool]:
    """Drop comments and literals so braces can be counted reliably."""
    out: list[str] = []
    pos = 0
    while pos < len(text):
        if in_block_comment:
            end = text.find("*/", pos)
            if end == -1:
                return "".join(out), True
            pos = end + 2
            in_block_comment = False
            continue
        if text.startswith("//", pos):
            break
        if text.startswith("/*", pos):
            in_block_comment = True
            pos += 2
            continue
        raw_end = _skip_raw_string(text, pos)
        if raw_end is not None:
            pos = raw_end
            continue
        char = text[pos]
        if char == '"':
            pos = _skip_normal_string(text, pos)
            continue
        if char == "'":
            pos = _skip_char_or_lifetime(text, pos)
            continue
        out.append(char)
        pos += 1
    return "".join(out), in_block_comment


def cfg_test_lines(lines: list[str]) -> set[int]:
    """Line numbers inside a `#[cfg(test)]` item."""
    inside: set[int] = set()
    depth = 0
    pending = False
    in_block_comment = False
    for number, raw in enumerate(lines, 1):
        code, in_block_comment = strip_code(raw, in_block_comment)
        if depth > 0:
            inside.add(number)
            depth += code.count("{") - code.count("}")
            continue
        if pending:
            if "{" in code:
                inside.add(number)
                depth = code.count("{") - code.count("}")
                pending = False
            elif not ATTRIBUTE.match(code):
                pending = False
            continue
        if CFG_TEST.match(raw):
            if "{" in code:
                inside.add(number)
                depth = code.count("{") - code.count("}")
            else:
                pending = True
    return inside


def main() -> int:
    problems: list[tuple[str, int, str, str]] = []
    for path in staged_rust_files():
        lines = git("show", f":{path}").splitlines()
        test_lines = cfg_test_lines(lines)
        for number in added_line_numbers(path):
            if number > len(lines):
                continue
            text = lines[number - 1]
            code, _ = strip_code(text, False)
            if not CRATE_PATH.search(code):
                continue
            if number in test_lines or USE_LINE.match(code):
                continue
            token = PATH_TOKEN.search(code)
            problems.append(
                (path, number, text.strip(), token.group(0) if token else "crate::")
            )

    if not problems:
        return 0

    print(
        "pre-commit: inline `crate::` paths found — import them at the top of the file instead:",
        file=sys.stderr,
    )
    for path, number, text, token in problems:
        print(f"  {path}:{number}: {token}", file=sys.stderr)
        print(f"      {text}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
