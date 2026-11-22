#!/usr/bin/env python3
"""Check that relative links and #anchors in the Markdown and the manual resolve.

    scripts/check-links.py

Covers README.md, CONTRIBUTING.md, CHANGELOG.md, docs/*.md and docs/manual/index.html. External URLs are not fetched.
"""
import re, sys, pathlib, urllib.parse
root = pathlib.Path(__file__).resolve().parent.parent
files = [root / n for n in ("README.md", "CONTRIBUTING.md", "CHANGELOG.md", "AGENTS.md")]
files += sorted((root / "docs").glob("*.md")) + [root / "docs/manual/index.html", root / "docs/index.html"]

def slug(heading):
    s = re.sub(r"[`*_]", "", heading.strip().lower())
    s = re.sub(r"[^\w\- ]", "", s)
    return s.replace(" ", "-")

def anchors(path):
    text = path.read_text(errors="replace")
    if path.suffix == ".html":
        return set(re.findall(r'id="([^"]+)"', text))
    return {slug(m) for m in re.findall(r"^#{1,6}\s+(.*)$", text, re.M)}

bad = 0
for f in files:
    if not f.exists():
        continue
    text = f.read_text(errors="replace")
    if f.suffix == ".md":
        links = re.findall(r"\]\(([^)\s]+)", text) + re.findall(r'(?:src|href|srcset)="([^"]+)"', text)
    else:
        links = re.findall(r'(?:src|href|srcset)="([^"]+)"', text)
    for link in links:
        if re.match(r"(https?:|mailto:|data:|javascript:)", link):
            continue
        target, _, frag = urllib.parse.unquote(link).partition("#")
        if target in ("media/favicon.png", "../media/favicon.png") or target.endswith("media/icon-256.png"):
            continue  # made by scripts/site.sh
        dest = (f.parent / target).resolve() if target else f
        if target and not dest.exists():
            print(f"{f.relative_to(root)}: missing {link}"); bad += 1; continue
        if frag and dest.is_file() and dest.suffix in (".md", ".html") and frag not in anchors(dest):
            print(f"{f.relative_to(root)}: no anchor #{frag} in {dest.relative_to(root)}"); bad += 1
sys.exit(1 if bad else 0)
