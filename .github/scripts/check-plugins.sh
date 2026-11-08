#!/usr/bin/env bash
# Checks a bundle or an unpacked release archive (issue 22): every plugin
# built for this platform has its plugin.toml under plugins/<id>/ and
# `rick --list-plugins` reports it loaded with the points its manifest
# declares; a plugin built for another platform only (the Spotlight
# connector on Linux) is not there at all, since the bundler leaves it out
# (QUESTIONS.md 121). Used by the ci, full and release workflows.
#
#   check-plugins.sh <dir>    the directory holding rick and plugins/
set -euo pipefail

dir="${1:?usage: check-plugins.sh <dir>}"
exe=""
if [ "${RUNNER_OS:-$(uname -s)}" = "Windows" ]; then exe=".exe"; fi
cd "$dir"

# id|name|points, the name as the manifest spells it, on every platform.
everywhere=(
  "git|Git|decorator, action"
  "svn|SVN|decorator, action"
  "vfs-trash|Trash|provider, decorator, action"
  "vfs-archive|Archives|provider"
  "preview-archive|[^|]*|previewer"
  "preview-csv|[^|]*|previewer"
  "preview-media|[^|]*|previewer"
  "preview-office|[^|]*|previewer"
  "preview-hex|[^|]*|previewer"
  "search-walk|Subdirectory search|search-index"
)
case "${RUNNER_OS:-$(uname -s)}" in
  Linux)   here=("search-tracker|Tracker|search-index" "search-baloo|Baloo|search-index")
           elsewhere=(search-windows search-spotlight) ;;
  macOS|Darwin) here=("search-spotlight|Spotlight|search-index")
           elsewhere=(search-windows search-tracker search-baloo) ;;
  Windows) here=("search-windows|Windows Search|search-index")
           elsewhere=(search-spotlight search-tracker search-baloo) ;;
  *) echo "unknown platform ${RUNNER_OS:-$(uname -s)}"; exit 1 ;;
esac

ls -l . plugins
for id in "${elsewhere[@]}"; do
  if [ -e "plugins/$id" ]; then echo "plugins/$id is bundled but built for another platform"; exit 1; fi
done
out="$("./rick$exe" --list-plugins)"
echo "$out"
for spec in "${everywhere[@]}" "${here[@]}"; do
  id="${spec%%|*}"; rest="${spec#*|}"; name="${rest%%|*}"; points="${rest#*|}"
  test -f "plugins/$id/plugin.toml" || { echo "no plugins/$id/plugin.toml"; exit 1; }
  echo "$out" | grep -Eq "^$id +$name +[^ ]+ +$points +loaded +" || { echo "$id plugin not loaded with points '$points'"; exit 1; }
done
echo "every plugin for this platform loaded"
