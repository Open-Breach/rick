#!/usr/bin/env bash
# Checks a bundle or an unpacked release archive (issue 22): every plugin
# built for this platform has its plugin.toml under plugins/<id>/ and
# `rick --list-plugins` reports it loaded with the points its manifest
# declares; a plugin built for another platform only (the Spotlight
# connector on Linux) is not there at all, since the bundler leaves it out
# (QUESTIONS.md 121). The action plugins of issue 23 and the installer of
# issue 44 load everywhere; the installer's actions hide off Linux. Used by
# the ci, full and release workflows.
#
#   check-plugins.sh <dir> [--sevenzip]
#       <dir> is the directory holding rick and plugins/; --sevenzip also
#       requires the 7-Zip library and its licence beside the archive
#       plugin (issue 51), as a release archive always carries them and a
#       bundle does when RICK_SEVENZIP_DIR named the library.
set -euo pipefail

dir="${1:?usage: check-plugins.sh <dir> [--sevenzip]}"
want_sevenzip="${2:-}"
if [ -n "${RICK_SEVENZIP_DIR:-}" ]; then want_sevenzip="--sevenzip"; fi
exe=""
if [ "${RUNNER_OS:-$(uname -s)}" = "Windows" ]; then exe=".exe"; fi
cd "$dir"

# id|name|points, the name as the manifest spells it, on every platform.
everywhere=(
  "git|Git|decorator, action"
  "svn|SVN|decorator, action"
  "vfs-trash|Trash|provider, decorator, action"
  "vfs-archive|Archives|provider"
  "ssh|SSH|provider, action"
  "vfs-s3|S3|provider"
  "vfs-azure|Azure Blob|provider"
  "vfs-gcs|Google Cloud Storage|provider"
  "preview-archive|[^|]*|previewer"
  "preview-markup|[^|]*|previewer"
  "preview-media|[^|]*|previewer"
  "preview-office|[^|]*|previewer"
  "preview-hex|[^|]*|previewer"
  "preview-3d|[^|]*|previewer"
  "preview-icons|[^|]*|previewer"
  "search-walk|Subdirectory search|search-index"
  "action-compress|Compress and extract|action"
  "action-compare|Compare|action"
  "action-terminal|Console|action"
  "action-rename|Bulk rename|action"
  "installer|Installer|provider, decorator, action"
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
if [ "$want_sevenzip" = "--sevenzip" ]; then
  test -f plugins/vfs-archive/7z.so || test -f plugins/vfs-archive/7z.dll || { echo "no 7-Zip library beside the archive plugin"; exit 1; }
  test -f plugins/vfs-archive/7-Zip-License.txt || { echo "no 7-Zip licence beside the library"; exit 1; }
  echo "the 7-Zip library and its licence are beside the archive plugin"
fi
echo "every plugin for this platform loaded"
