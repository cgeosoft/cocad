#!/usr/bin/env bash
# Checks the links of docs/wiki/: each page link must have a file, each
# anchor must have a heading, and each repository image must exist.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
python3 - "$root" <<'EOF'
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])
wiki = root / "docs" / "wiki"
raw = "https://raw.githubusercontent.com/cgeosoft/cocad/main/"
pages = {p.stem: p.read_text(encoding="utf-8") for p in wiki.glob("*.md")}


def anchors(text):
    out = set()
    for line in text.splitlines():
        m = re.match(r"#+\s+(.*)", line)
        if m:
            slug = re.sub(r"[^\w\- ]", "", m.group(1).strip().lower())
            out.add(slug.replace(" ", "-"))
    return out


heads = {name: anchors(text) for name, text in pages.items()}
errors = []
linked = set()
for name, text in pages.items():
    for target in re.findall(r"\]\(([^)\s]+)\)", text):
        if target.startswith(raw):
            if not (root / target[len(raw):]).is_file():
                errors.append(f"{name}: no file for {target}")
            continue
        if re.match(r"[a-z]+:", target):
            continue
        page, _, anchor = target.partition("#")
        page = page or name
        linked.add(page)
        if page not in pages:
            errors.append(f"{name}: no page {page}")
        elif anchor and anchor not in heads[page]:
            errors.append(f"{name}: no heading #{anchor} in {page}")

for name in pages:
    if name not in linked and not name.startswith("_") and name != "Home":
        errors.append(f"{name}: no page links to it")
    if "—" in pages[name]:
        errors.append(f"{name}: em-dash")

print("\n".join(errors) if errors else f"{len(pages)} wiki files: links OK")
sys.exit(1 if errors else 0)
EOF
