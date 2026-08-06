#!/bin/sh
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$here/upstream.env"

dir=${1:?usage: apply.sh <checkout-dir> [remote]}
remote=${2:-$UPSTREAM_REMOTE}

if [ ! -d "$dir/.git" ]; then
    git -c core.autocrlf=false -c core.eol=lf \
        clone --quiet --depth 1 --branch "$UPSTREAM_TAG" "$remote" "$dir"
fi

git -C "$dir" config core.autocrlf false
git -C "$dir" config core.eol lf

at=$(git -C "$dir" rev-parse HEAD)
if [ "$at" != "$UPSTREAM_COMMIT" ]; then
    echo "$dir is at $at, not the pinned $UPSTREAM_COMMIT" >&2
    exit 1
fi

git -C "$dir" apply --check "$here/headless.patch"
git -C "$dir" apply "$here/headless.patch"

for tool in eql_headless.py eql_quest_cli.py eql_fights_cli.py; do
    test -f "$dir/$tool" || { echo "$tool is missing after the patch" >&2; exit 1; }
done
echo "patched $dir at $UPSTREAM_COMMIT"
