#!/usr/bin/env bash
# Copies docs/wiki/ to the GitHub wiki of the public mirror.
#
# GitHub makes the wiki repository when the first page is saved. Before the
# first run, open the Wiki tab of the repository and save one page.
#
# Usage: scripts/publish-wiki.sh [--dry-run]
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
src="$root/docs/wiki"
remote="${WIKI_REMOTE:-$(git -C "$root" remote get-url github | sed 's/\.git$//').wiki.git}"
dry=0
[ "${1:-}" = "--dry-run" ] && dry=1

"$root/scripts/check-wiki.sh"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

git clone --quiet "$remote" "$work/wiki"
# The wiki has the same pages as docs/wiki/, and no others.
find "$work/wiki" -maxdepth 1 -name '*.md' -delete
cp "$src"/*.md "$work/wiki/"

cd "$work/wiki"
git add -A
if git diff --cached --quiet; then
    echo "The wiki is up to date."
    exit 0
fi
git status --short
if [ "$dry" = 1 ]; then
    echo "Dry run: no push."
    exit 0
fi
commit="$(git -C "$root" rev-parse --short HEAD)"
git commit --quiet -m "Update from docs/wiki at $commit"
git push --quiet origin HEAD
echo "Published to $remote"
