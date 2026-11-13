#!/usr/bin/env bash
# The README describes the release being cut (specs/readme.md, *Rules for
# the release*): the version in its status line, the first line of the
# `generated:status` block, is the tag's, without the `v` and without a
# pre-release suffix such as `-rc1`. `cargo xtask readme` writes it in the
# release pull request. Used by the release workflow before anything is
# built.
#
#   check-readme.sh <tag> [<readme>]
set -euo pipefail

tag="${1:?usage: check-readme.sh <tag> [<readme>]}"
readme="${2:-README.md}"
version="${tag#v}"
version="${version%%-*}"
line="$(sed -n '/^<!-- generated:status -->$/{n;p;q;}' "$readme")"
case "$line" in
  "v$version · "*)
    echo "$readme describes v$version"
    ;;
  *)
    echo "$readme describes another release than $tag: its status line is"
    echo "  ${line:-(missing)}"
    echo "Run \`cargo xtask readme\` in the release pull request."
    exit 1
    ;;
esac
