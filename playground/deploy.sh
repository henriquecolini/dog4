#!/usr/bin/env bash
# Builds the playground and commits it to the gh-pages branch (without
# touching your working tree). Push afterwards with `git push origin gh-pages`.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(dirname "$here")"
"$here/build.sh"

pages="$(mktemp -d)"
trap 'git -C "$root" worktree remove --force "$pages" 2>/dev/null || true' EXIT
if git -C "$root" show-ref --verify --quiet refs/heads/gh-pages; then
  git -C "$root" worktree add --quiet "$pages" gh-pages
else
  git -C "$root" worktree add --quiet --orphan -b gh-pages "$pages"
fi
find "$pages" -mindepth 1 -maxdepth 1 ! -name .git -exec rm -rf {} +
cp "$here"/web/{index.html,style.css,app.js,worker.js,highlight.js,dog4-wasm.js,examples.js,dog4.wasm} "$pages/"
touch "$pages/.nojekyll"
source_commit="$(git -C "$root" rev-parse --short HEAD)"
git -C "$pages" add -A
if git -C "$pages" diff --cached --quiet; then
  echo "gh-pages is already up to date"
else
  git -C "$pages" commit --quiet -m "Deploy the playground from $source_commit"
  echo "committed to gh-pages; push with: git push origin gh-pages"
fi
