#!/usr/bin/env python3
"""Repeatable Rust size and complexity index for Crow workspace crates.

Counts tracked `.rs` files under `crates/` at a given git revision (or working tree):
- Physical lines (including comments and blank lines), split into non-test and test.
- Decision points inside non-test named functions/methods:
  `if`, `while`, `for`, `loop`, `match` arms (`=>` at match-body depth), match guards,
  `&&`, `||`, `?`, and `let ... else`. Macro bodies (`ident!(...)`) are opaque.

Usage:
  python3 scripts/code-index.py [--rev REV] [--crates] [--top N] [--files N]
  python3 scripts/code-index.py --series REV1:Label1 REV2:Label2 ...
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import PurePosixPath
from typing import Iterator


@dataclass
class Token:
    kind: str  # 'ident', 'punct', 'str', 'char', 'num', 'eof'
    text: str
    line: int


def tokenize(src: str) -> list[Token]:
    tokens: list[Token] = []
    n = len(src)
    i = 0
    line = 1

    while i < n:
        c = src[i]
        # Whitespace
        if c in " \t\r\n":
            if c == "\n":
                line += 1
            i += 1
            continue

        # Line comment
        if c == "/" and i + 1 < n and src[i + 1] == "/":
            i += 2
            while i < n and src[i] != "\n":
                i += 1
            continue

        # Nested block comment
        if c == "/" and i + 1 < n and src[i + 1] == "*":
            depth = 1
            i += 2
            while i < n and depth > 0:
                if src[i] == "\n":
                    line += 1
                    i += 1
                elif src[i] == "/" and i + 1 < n and src[i + 1] == "*":
                    depth += 1
                    i += 2
                elif src[i] == "*" and i + 1 < n and src[i + 1] == "/":
                    depth -= 1
                    i += 2
                else:
                    i += 1
            continue

        # Raw strings: r"...", r#"..."#, br"...", br#"..."#, cr"...", cr#"..."#
        if (
            c == "r"
            or (c in ("b", "c") and i + 1 < n and src[i + 1] == "r")
        ):
            j = i + (2 if c in ("b", "c") else 1)
            hashes = 0
            while j < n and src[j] == "#":
                hashes += 1
                j += 1
            if j < n and src[j] == '"':
                start_line = line
                j += 1
                while j < n:
                    if src[j] == "\n":
                        line += 1
                        j += 1
                    elif src[j] == '"':
                        k = j + 1
                        h = 0
                        while h < hashes and k < n and src[k] == "#":
                            h += 1
                            k += 1
                        if h == hashes:
                            j = k
                            break
                        j += 1
                    else:
                        j += 1
                tokens.append(Token("str", '""', start_line))
                i = j
                continue

        # Byte/C string: b"...", c"..."
        if c in ("b", "c") and i + 1 < n and src[i + 1] == '"':
            start_line = line
            i += 2
            while i < n:
                if src[i] == "\\":
                    if i + 1 < n and src[i + 1] == "\n":
                        line += 1
                    i += 2
                elif src[i] == '"':
                    i += 1
                    break
                else:
                    if src[i] == "\n":
                        line += 1
                    i += 1
            tokens.append(Token("str", '""', start_line))
            continue

        # Normal string
        if c == '"':
            start_line = line
            i += 1
            while i < n:
                if src[i] == "\\":
                    if i + 1 < n and src[i + 1] == "\n":
                        line += 1
                    i += 2
                elif src[i] == '"':
                    i += 1
                    break
                else:
                    if src[i] == "\n":
                        line += 1
                    i += 1
            tokens.append(Token("str", '""', start_line))
            continue

        # Byte char literal: b'...'
        if c == "b" and i + 1 < n and src[i + 1] == "'":
            start_line = line
            i += 2
            if i < n and src[i] == "\\":
                i += 2
            while i < n and src[i] != "'" and src[i] != "\n":
                i += 1
            if i < n and src[i] == "'":
                i += 1
            tokens.append(Token("char", "''", start_line))
            continue

        # Char literal vs lifetime
        if c == "'":
            start_line = line
            # Check if it's a char literal: '\...' or 'x'
            if i + 1 < n and src[i + 1] == "\\":
                j = i + 2
                while j < n and src[j] != "'" and src[j] != "\n":
                    if src[j] == "\\":
                        j += 2
                    else:
                        j += 1
                if j < n and src[j] == "'":
                    tokens.append(Token("char", "''", start_line))
                    i = j + 1
                    continue
            elif i + 2 < n and src[i + 2] == "'" and src[i + 1] != "'":
                tokens.append(Token("char", "''", start_line))
                i += 3
                continue
            # Lifetime: 'ident or '_
            j = i + 1
            while j < n and (src[j].isalnum() or src[j] == "_"):
                j += 1
            tokens.append(Token("lifetime", src[i:j], start_line))
            i = j
            continue

        # Raw identifier r#ident
        if c == "r" and i + 1 < n and src[i + 1] == "#" and i + 2 < n and (src[i + 2].isalpha() or src[i + 2] == "_"):
            j = i + 2
            while j < n and (src[j].isalnum() or src[j] == "_"):
                j += 1
            tokens.append(Token("ident", src[i:j], line))
            i = j
            continue

        # Identifier / keyword
        if c.isalpha() or c == "_":
            j = i + 1
            while j < n and (src[j].isalnum() or src[j] == "_"):
                j += 1
            tokens.append(Token("ident", src[i:j], line))
            i = j
            continue

        # Number
        if c.isdigit():
            j = i + 1
            while j < n and (src[j].isalnum() or src[j] == "_"):
                j += 1
            tokens.append(Token("num", src[i:j], line))
            i = j
            continue

        # Multi-char punctuation
        if i + 1 < n:
            two = src[i : i + 2]
            if two in ("&&", "||", "=>", "::", "->", "..", "==", "!=", "<=", ">="):
                tokens.append(Token("punct", two, line))
                i += 2
                continue

        tokens.append(Token("punct", c, line))
        i += 1

    tokens.append(Token("eof", "", line))
    return tokens


def is_test_file(rel_path: str) -> bool:
    p = PurePosixPath(rel_path)
    parts = p.parts
    if "tests" in parts:
        return True
    name = p.name
    if name == "tests.rs" or name.endswith("_tests.rs") or name.endswith("_test.rs"):
        return True
    return False


def find_matching(tokens: list[Token], idx: int, open_ch: str, close_ch: str) -> int:
    depth = 0
    n = len(tokens)
    i = idx
    while i < n:
        t = tokens[i]
        if t.kind == "punct":
            if t.text == open_ch:
                depth += 1
            elif t.text == close_ch:
                depth -= 1
                if depth == 0:
                    return i
        i += 1
    return n - 1


def attr_is_test(tokens: list[Token], start: int, end: int) -> bool:
    """Check if attribute #[ ... ] marks a test item or cfg(test)."""
    inner = [t.text for t in tokens[start + 2 : end]]
    joined = "".join(inner)
    if "cfg(test)" in joined:
        return True
    if "cfg(all(test," in joined or ",test))" in joined:
        return True
    # #[test], #[gpui::test], #[tokio::test], #[gpui::test(...)]
    if inner and inner[-1] == "test":
        return True
    if "test" in inner:
        # e.g. gpui :: test ( ... )
        for idx, tok in enumerate(inner):
            if tok == "test":
                prev = inner[idx - 1] if idx > 0 else None
                nxt = inner[idx + 1] if idx + 1 < len(inner) else None
                if (prev is None or prev == "::") and (nxt is None or nxt == "("):
                    return True
    return False


def count_fn_decisions(tokens: list[Token], body_start: int, body_end: int) -> int:
    """Count decision points in a function body [body_start..=body_end]."""
    decisions = 0

    def is_logical_or(idx: int, lo: int) -> bool:
        if idx <= lo:
            return False
        prev = tokens[idx - 1]
        if prev.kind in ("num", "str", "char"):
            return True
        if prev.kind == "punct" and prev.text in (")", "]", "}", "?"):
            return True
        if prev.kind == "ident" and prev.text not in (
            "move",
            "async",
            "static",
            "return",
            "break",
            "yield",
            "else",
            "in",
            "match",
            "if",
            "while",
        ):
            return True
        return False

    def walk(lo: int, hi: int, in_match_body: bool = False) -> None:
        nonlocal decisions
        k = lo
        brace_depth = 0
        paren_depth = 0
        bracket_depth = 0
        match_arms = 0
        # Stack of (depth_tuple, has_unmatched_if) for active `let` statements
        active_lets: list[tuple[int, int, int]] = []
        active_ifs: list[tuple[int, int, int]] = []
        while k < hi:
            t = tokens[k]

            # Skip nested `#[cfg(test)]` or `#[test]` items inside functions
            if t.kind == "punct" and t.text == "#" and k + 1 < hi and tokens[k + 1].text == "[":
                attr_end = find_matching(tokens, k + 1, "[", "]")
                if attr_is_test(tokens, k, attr_end):
                    k = skip_item(tokens, attr_end + 1, hi)
                    continue
                k = attr_end + 1
                continue

            # Skip opaque macro invocations: `ident ! ( ... )` / `[...]` / `{...}`
            if (
                t.kind == "ident"
                and k + 2 < hi
                and tokens[k + 1].text == "!"
                and tokens[k + 2].text in ("(", "[", "{")
            ):
                open_ch = tokens[k + 2].text
                close_ch = {"(": ")", "[": "]", "{": "}"}[open_ch]
                k = find_matching(tokens, k + 2, open_ch, close_ch) + 1
                continue

            if t.kind == "punct":
                if t.text == "{":
                    brace_depth += 1
                elif t.text == "}":
                    brace_depth -= 1
                elif t.text == "(":
                    paren_depth += 1
                elif t.text == ")":
                    paren_depth -= 1
                elif t.text == "[":
                    bracket_depth += 1
                elif t.text == "]":
                    bracket_depth -= 1
                elif t.text in ("&&", "?"):
                    decisions += 1
                elif t.text == "||":
                    if is_logical_or(k, lo):
                        decisions += 1
                elif t.text == "=>" and in_match_body and brace_depth == 0 and paren_depth == 0 and bracket_depth == 0:
                    match_arms += 1
                elif t.text == ";":
                    cur = (brace_depth, paren_depth, bracket_depth)
                    while active_lets and active_lets[-1] == cur:
                        active_lets.pop()
                    while active_ifs and active_ifs[-1] == cur:
                        active_ifs.pop()
                k += 1
                continue

            if t.kind == "ident":
                cur = (brace_depth, paren_depth, bracket_depth)
                if t.text == "if":
                    decisions += 1
                    active_ifs.append(cur)
                    k += 1
                    continue
                if t.text in ("while", "for", "loop"):
                    decisions += 1
                    k += 1
                    continue
                if t.text == "let":
                    prev = tokens[k - 1].text if k > lo else ""
                    if prev not in ("if", "while", "&&"):
                        active_lets.append(cur)
                    k += 1
                    continue
                if t.text == "else":
                    if active_ifs and active_ifs[-1] == cur:
                        active_ifs.pop()
                    elif (
                        active_lets
                        and active_lets[-1] == cur
                        and k + 1 < hi
                        and tokens[k + 1].text == "{"
                    ):
                        decisions += 1
                        active_lets.pop()
                    k += 1
                    continue
                if t.text == "match":
                    j = k + 1
                    pd = 0
                    bd = 0
                    while j < hi:
                        tj = tokens[j]
                        if tj.text == "(":
                            pd += 1
                        elif tj.text == ")":
                            pd -= 1
                        elif tj.text == "[":
                            bd += 1
                        elif tj.text == "]":
                            bd -= 1
                        elif tj.text == "{" and pd == 0 and bd == 0:
                            break
                        j += 1
                    walk(k + 1, j, in_match_body=False)
                    if j < hi and tokens[j].text == "{":
                        m_end = find_matching(tokens, j, "{", "}")
                        walk(j + 1, m_end, in_match_body=True)
                        k = m_end + 1
                        continue
                    k = j
                    continue

            k += 1

        if in_match_body and match_arms > 1:
            decisions += match_arms - 1

    walk(body_start + 1, body_end, in_match_body=False)
    return decisions


def skip_item(tokens: list[Token], start: int, limit: int) -> int:
    """Skip one Rust item (mod, fn, struct, enum, impl, use, const, static, type, macro) starting at `start`."""
    i = start
    # Skip any stacked attributes
    while i < limit and tokens[i].text == "#" and i + 1 < limit and tokens[i + 1].text == "[":
        i = find_matching(tokens, i + 1, "[", "]") + 1
    while i < limit:
        t = tokens[i]
        if t.text == ";":
            return i + 1
        if t.text == "{":
            return find_matching(tokens, i, "{", "}") + 1
        if t.text == "(":
            i = find_matching(tokens, i, "(", ")") + 1
            continue
        if t.text == "[":
            i = find_matching(tokens, i, "[", "]") + 1
            continue
        i += 1
    return limit


@dataclass
class FnStat:
    file: str
    name: str
    line: int
    lines: int
    decisions: int


@dataclass
class FileStat:
    path: str
    crate: str
    total_lines: int
    non_test_lines: int
    test_lines: int
    decisions: int
    functions: list[FnStat] = field(default_factory=list)


def analyze_file(rel_path: str, src: str) -> FileStat:
    parts = PurePosixPath(rel_path).parts
    crate = parts[1] if len(parts) > 1 and parts[0] == "crates" else parts[0]
    total_lines = src.count("\n") + (1 if src and not src.endswith("\n") else 0)

    if is_test_file(rel_path):
        return FileStat(rel_path, crate, total_lines, 0, total_lines, 0)

    tokens = tokenize(src)
    n = len(tokens) - 1  # exclude eof
    test_line_set: set[int] = set()
    functions: list[FnStat] = []

    def scan_items(lo: int, hi: int, scope_prefix: str = "") -> None:
        i = lo
        pending_test_attr_start: int | None = None
        while i < hi:
            t = tokens[i]

            # Attribute #[...]
            if t.kind == "punct" and t.text == "#" and i + 1 < hi and tokens[i + 1].text == "[":
                attr_end = find_matching(tokens, i + 1, "[", "]")
                if attr_is_test(tokens, i, attr_end) and pending_test_attr_start is None:
                    pending_test_attr_start = t.line
                i = attr_end + 1
                continue

            # Inner attribute #![...]
            if t.kind == "punct" and t.text == "#" and i + 1 < hi and tokens[i + 1].text == "!":
                if i + 2 < hi and tokens[i + 2].text == "[":
                    i = find_matching(tokens, i + 2, "[", "]") + 1
                    continue

            # If pending_test_attr_start is set, skip the entire annotated item and count its lines as test lines
            if pending_test_attr_start is not None:
                end_idx = skip_item(tokens, i, hi)
                end_line = tokens[min(end_idx - 1, hi - 1)].line if end_idx > i else pending_test_attr_start
                for ln in range(pending_test_attr_start, end_line + 1):
                    test_line_set.add(ln)
                pending_test_attr_start = None
                i = end_idx
                continue

            # Visibility / qualifiers before item
            if t.kind == "ident" and t.text in ("pub", "async", "unsafe", "const", "extern", "default"):
                if t.text == "pub" and i + 1 < hi and tokens[i + 1].text == "(":
                    i = find_matching(tokens, i + 1, "(", ")") + 1
                    continue
                if t.text == "extern" and i + 1 < hi and tokens[i + 1].kind == "str":
                    i += 2
                    continue
                i += 1
                continue

            # Named function
            if t.kind == "ident" and t.text == "fn" and i + 1 < hi and tokens[i + 1].kind == "ident":
                fn_name = tokens[i + 1].text
                fn_line = t.line
                # Find signature end (`{` or `;`)
                j = i + 2
                angle = 0
                while j < hi:
                    tj = tokens[j]
                    if tj.text == "(":
                        j = find_matching(tokens, j, "(", ")") + 1
                        continue
                    if tj.text == "[":
                        j = find_matching(tokens, j, "[", "]") + 1
                        continue
                    if tj.text == ";" and angle == 0:
                        j += 1
                        break
                    if tj.text == "{" and angle == 0:
                        body_end = find_matching(tokens, j, "{", "}")
                        dec = count_fn_decisions(tokens, j, body_end)
                        full_name = f"{scope_prefix}::{fn_name}" if scope_prefix else fn_name
                        functions.append(
                            FnStat(
                                file=rel_path,
                                name=full_name,
                                line=fn_line,
                                lines=tokens[body_end].line - fn_line + 1,
                                decisions=dec,
                            )
                        )
                        j = body_end + 1
                        break
                    j += 1
                i = j
                continue

            # `impl ... { ... }` or `trait ... { ... }` or `mod name { ... }`
            if t.kind == "ident" and t.text in ("impl", "trait", "mod"):
                kind = t.text
                j = i + 1
                label = ""
                if kind in ("mod", "trait") and j < hi and tokens[j].kind == "ident":
                    label = tokens[j].text
                elif kind == "impl":
                    # Grab the main type name before `{` or `where`
                    k = j
                    if k < hi and tokens[k].text == "<":
                        # skip generics
                        depth = 0
                        while k < hi:
                            if tokens[k].text == "<":
                                depth += 1
                            elif tokens[k].text == ">":
                                depth -= 1
                                if depth == 0:
                                    k += 1
                                    break
                            k += 1
                    names: list[str] = []
                    while k < hi and tokens[k].text not in ("{", "where", ";"):
                        if tokens[k].kind == "ident" and tokens[k].text != "for":
                            names.append(tokens[k].text)
                        elif tokens[k].text == "for":
                            names.clear()
                        k += 1
                    label = names[-1] if names else "impl"
                while j < hi and tokens[j].text not in ("{", ";"):
                    if tokens[j].text == "(":
                        j = find_matching(tokens, j, "(", ")") + 1
                    elif tokens[j].text == "[":
                        j = find_matching(tokens, j, "[", "]") + 1
                    else:
                        j += 1
                if j < hi and tokens[j].text == "{":
                    blk_end = find_matching(tokens, j, "{", "}")
                    sub = f"{scope_prefix}::{label}" if (scope_prefix and label) else (label or scope_prefix)
                    scan_items(j + 1, blk_end, sub)
                    i = blk_end + 1
                    continue
                i = j + 1
                continue

            # Macro invocation at item level: skip body
            if (
                t.kind == "ident"
                and i + 2 < hi
                and tokens[i + 1].text == "!"
                and tokens[i + 2].text in ("(", "[", "{")
            ):
                open_ch = tokens[i + 2].text
                close_ch = {"(": ")", "[": "]", "{": "}"}[open_ch]
                i = find_matching(tokens, i + 2, open_ch, close_ch) + 1
                continue

            if t.text == "{":
                i = find_matching(tokens, i, "{", "}") + 1
                continue
            if t.text == "(":
                i = find_matching(tokens, i, "(", ")") + 1
                continue
            if t.text == "[":
                i = find_matching(tokens, i, "[", "]") + 1
                continue

            i += 1

    scan_items(0, n)
    test_lines = len(test_line_set)
    non_test_lines = total_lines - test_lines
    decisions = sum(f.decisions for f in functions)
    return FileStat(rel_path, crate, total_lines, non_test_lines, test_lines, decisions, functions)


def iter_files(repo: str, rev: str | None) -> Iterator[tuple[str, str]]:
    if rev is None:
        out = subprocess.check_output(
            ["git", "-C", repo, "ls-files", "crates/**/*.rs", "crates/*.rs"],
            text=True,
        )
        paths = [p for p in out.splitlines() if p.endswith(".rs")]
        for p in sorted(paths):
            with open(os.path.join(repo, p), "r", encoding="utf-8") as fh:
                yield p, fh.read()
    else:
        out = subprocess.check_output(
            ["git", "-C", repo, "ls-tree", "-r", "--name-only", rev, "crates"],
            text=True,
        )
        paths = [p for p in out.splitlines() if p.endswith(".rs")]
        for p in sorted(paths):
            content = subprocess.check_output(
                ["git", "-C", repo, "show", f"{rev}:{p}"],
                text=True,
            )
            yield p, content


def summarize_rev(repo: str, rev: str | None) -> list[FileStat]:
    return [analyze_file(p, src) for p, src in iter_files(repo, rev)]


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", default=".")
    ap.add_argument("--rev", default=None)
    ap.add_argument("--crates", action="store_true")
    ap.add_argument("--top", type=int, default=0)
    ap.add_argument("--files", type=int, default=0)
    ap.add_argument("--series", nargs="*", default=[])
    args = ap.parse_args()

    if args.series:
        print("| Revision | Rust files | Rust lines | Non-test lines | Test lines | Decision points outside tests |")
        print("| --- | ---: | ---: | ---: | ---: | ---: |")
        for spec in args.series:
            if ":" in spec:
                rev, label = spec.split(":", 1)
            else:
                rev, label = spec, spec
            stats = summarize_rev(args.repo, rev)
            files = len(stats)
            tot = sum(s.total_lines for s in stats)
            nt = sum(s.non_test_lines for s in stats)
            tl = sum(s.test_lines for s in stats)
            dp = sum(s.decisions for s in stats)
            print(f"| {label} (`{rev}`) | {files} | {tot:,} | {nt:,} | {tl:,} | {dp:,} |")
        return

    stats = summarize_rev(args.repo, args.rev)
    files = len(stats)
    tot = sum(s.total_lines for s in stats)
    nt = sum(s.non_test_lines for s in stats)
    tl = sum(s.test_lines for s in stats)
    dp = sum(s.decisions for s in stats)
    print(f"Total: {files} files, {tot:,} lines ({nt:,} non-test, {tl:,} test), {dp:,} decision points")

    if args.crates:
        by_crate: dict[str, list[FileStat]] = {}
        for s in stats:
            by_crate.setdefault(s.crate, []).append(s)
        print("\n| Crate | Files | Rust lines | Non-test lines | Test lines | Non-test decision points |")
        print("| --- | ---: | ---: | ---: | ---: | ---: |")
        for crate, items in sorted(by_crate.items(), key=lambda kv: -sum(x.total_lines for x in kv[1])):
            print(
                f"| `{crate}` | {len(items)} | {sum(x.total_lines for x in items):,} | "
                f"{sum(x.non_test_lines for x in items):,} | {sum(x.test_lines for x in items):,} | "
                f"{sum(x.decisions for x in items):,} |"
            )

    if args.files:
        print(f"\nTop {args.files} files by total lines:")
        print("| File | Total lines | Non-test | Test | Decision points |")
        print("| --- | ---: | ---: | ---: | ---: |")
        for s in sorted(stats, key=lambda x: -x.total_lines)[: args.files]:
            print(f"| `{s.path}` | {s.total_lines:,} | {s.non_test_lines:,} | {s.test_lines:,} | {s.decisions:,} |")

    if args.top:
        all_fns = [f for s in stats for f in s.functions]
        print(f"\nTop {args.top} functions by decision points:")
        print("| # | Function | Location | Lines | Decision points |")
        print("| ---: | --- | --- | ---: | ---: |")
        for idx, f in enumerate(sorted(all_fns, key=lambda x: (-x.decisions, -x.lines))[: args.top], 1):
            print(f"| {idx} | `{f.name}` | `{f.file}:{f.line}` | {f.lines} | {f.decisions} |")


if __name__ == "__main__":
    main()
