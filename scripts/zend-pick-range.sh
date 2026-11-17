#!/usr/bin/env bash
# zend-pick-range: take an upstream range (Zeron's commits BASE..TARGET) into
# this worktree as one change, file by file, by a 3-way merge whose base and
# upstream side are rebranded first (scripts/zend-rebrand.pl --filter).
#
#   scripts/zend-pick-range.sh <base> <target> [path...]
#
# <base> and <target> are commits in this repository (fetch upstream first);
# the paths limit it to those files. Each file: "new" (upstream added it),
# "clean", "CONFLICTS=<n>" (conflict markers left in the file, fork side
# first), "DELETED" (upstream removed it: not done, see to it by hand) or
# "MISSING-IN-FORK" (upstream changed a file the fork doesn't have: written as
# upstream has it). Nothing is staged or committed.
#
# Why not `git format-patch | zend-rebrand.pl --filter | git am -3`: for a
# branch upstream made before the fork's merge-base, the rebranded patch names
# pre-image blobs the fork never had, so -3 has nothing to fall back to and
# the patch fails outright. Here the merge base is the range's own start,
# rebranded, and the fork's file is the other side.
set -euo pipefail
[ $# -ge 2 ] || { sed -n 2,16p "$0" >&2; exit 2; }
base=$1; target=$2; shift 2
root="$(git rev-parse --show-toplevel)"
rebrand="$root/scripts/zend-rebrand.pl"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
cd "$root"
status=0
while IFS= read -r f; do
  ours="$(printf '%s' "$f" | "$rebrand" --filter)"
  if ! git cat-file -e "$target:$f" 2>/dev/null; then echo "DELETED $ours"; continue; fi
  git show "$target:$f" | "$rebrand" --filter > "$tmp/theirs"
  if git cat-file -e "$base:$f" 2>/dev/null; then
    git show "$base:$f" | "$rebrand" --filter > "$tmp/base"
  else
    : > "$tmp/base"
  fi
  if [ ! -e "$ours" ]; then
    [ -s "$tmp/base" ] && echo "MISSING-IN-FORK $ours" || echo "new $ours"
    mkdir -p "$(dirname "$ours")"; cp "$tmp/theirs" "$ours"; continue
  fi
  cp "$ours" "$tmp/ours"
  n=0; git merge-file -L fork -L base -L upstream "$tmp/ours" "$tmp/base" "$tmp/theirs" || n=$?
  cp "$tmp/ours" "$ours"
  if [ "$n" -eq 0 ]; then echo "clean $ours"; else echo "CONFLICTS=$n $ours"; status=1; fi
done < <(git diff --name-only "$base" "$target" -- "$@")
exit $status
