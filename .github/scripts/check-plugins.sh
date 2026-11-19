#!/usr/bin/env bash
# Checks a bundle or an unpacked release archive (issues 22 and 250): the
# plugins built for this platform have their plugin.toml under
# plugins/<id>/ and `rick --list-plugins` reports them loaded with the
# points their manifests declare; a plugin built for another platform only
# (the Spotlight connector on Linux) is not there at all, since the bundler
# leaves it out (QUESTIONS.md 121). Used by the ci, full and release
# workflows.
#
# A bundle holds every plugin of the platform. A release archive holds the
# default set and none of the separate plugins, which a release publishes
# as a package each (specs/plugin-distribution.md §4): Git, SVN, ZFS, SSH,
# the three object stores, Office, 3D and, on macOS and Windows, media.
#
#   check-plugins.sh <dir> [--sevenzip] [--archive] [--installed]
#       <dir> is the directory holding rick and plugins/.
#       --sevenzip also requires the 7-Zip library and its licence beside
#       the archive plugin (issue 51), as a release archive always carries
#       them and a bundle does when RICK_SEVENZIP_DIR named the library.
#       --archive checks a release archive: the default set is in plugins/
#       and loaded, no separate plugin and nothing else is in plugins/.
#       --installed, with --archive: every separate plugin of the platform
#       is loaded all the same, which it can only be from the per-user
#       plugins folder, where its package was unpacked.
set -euo pipefail

dir="${1:?usage: check-plugins.sh <dir> [--sevenzip] [--archive] [--installed]}"
shift
want_sevenzip=""
archive=""
installed=""
for flag in "$@"; do
  case "$flag" in
    --sevenzip) want_sevenzip="--sevenzip" ;;
    --archive) archive="yes" ;;
    --installed) installed="yes" ;;
    *) echo "unknown argument $flag"; exit 1 ;;
  esac
done
if [ -n "${RICK_SEVENZIP_DIR:-}" ]; then want_sevenzip="--sevenzip"; fi
exe=""
if [ "${RUNNER_OS:-$(uname -s)}" = "Windows" ]; then exe=".exe"; fi
cd "$dir"

# id|name|points, the name as the manifest spells it. First the default
# set, in every install; then the separate plugins.
default_set=(
  "vfs-trash|Trash|provider, decorator, action"
  "vfs-archive|Archives|provider"
  "preview-archive|[^|]*|previewer"
  "preview-markup|[^|]*|previewer"
  "preview-hex|[^|]*|previewer"
  "preview-icons|[^|]*|previewer"
  "search-walk|Subdirectory search|search-index"
  "action-compress|Compress and extract|action"
  "action-compare|Compare|action"
  "action-terminal|Console|action"
  "action-rename|Bulk rename|action"
  "installer|Installer|provider, decorator, action"
)
separate=(
  "git|Git|decorator, action"
  "svn|SVN|decorator, action"
  "ssh|SSH|provider, action"
  "vfs-s3|S3|provider"
  "vfs-azure|Azure Blob|provider"
  "vfs-gcs|Google Cloud Storage|provider"
  "preview-office|[^|]*|previewer"
  "preview-3d|[^|]*|previewer"
)
# The media previewer is in the default set on Linux, where GStreamer is
# the system's, and separate where the runtime travels with it.
media="preview-media|[^|]*|previewer"
case "${RUNNER_OS:-$(uname -s)}" in
  Linux)   default_set+=("search-tracker|Tracker|search-index" "search-baloo|Baloo|search-index" "$media")
           separate+=("zfs|ZFS|provider, decorator, action")
           elsewhere=(search-windows search-spotlight) ;;
  macOS|Darwin) default_set+=("search-spotlight|Spotlight|search-index")
           separate+=("$media")
           elsewhere=(search-windows search-tracker search-baloo zfs) ;;
  Windows) default_set+=("search-windows|Windows Search|search-index")
           separate+=("$media")
           elsewhere=(search-spotlight search-tracker search-baloo zfs) ;;
  *) echo "unknown platform ${RUNNER_OS:-$(uname -s)}"; exit 1 ;;
esac

ls -l . plugins
for id in "${elsewhere[@]}"; do
  if [ -e "plugins/$id" ]; then echo "plugins/$id is bundled but built for another platform"; exit 1; fi
done
out="$("./rick$exe" --list-plugins)"
echo "$out"

# The plugin of `spec` is listed as loaded with its points.
loaded() {
  local id="${1%%|*}" rest="${1#*|}"
  local name="${rest%%|*}" points="${rest#*|}"
  echo "$out" | grep -Eq "^$id +$name +[^ ]+ +$points +loaded +" || { echo "$id plugin not loaded with points '$points'"; exit 1; }
}

here=("${default_set[@]}")
if [ -z "$archive" ]; then here+=("${separate[@]}"); fi
for spec in "${here[@]}"; do
  id="${spec%%|*}"
  test -f "plugins/$id/plugin.toml" || { echo "no plugins/$id/plugin.toml"; exit 1; }
  loaded "$spec"
done
if [ -n "$archive" ]; then
  for spec in "${separate[@]}"; do
    id="${spec%%|*}"
    if [ -e "plugins/$id" ]; then echo "plugins/$id is in the archive, but it is a separate plugin"; exit 1; fi
    if [ -n "$installed" ]; then loaded "$spec"; fi
  done
  # Nothing but the default set: a folder the lists above do not know
  # would be a plugin nobody decided about.
  for folder in plugins/*/; do
    id="$(basename "$folder")"
    known=""
    for spec in "${default_set[@]}"; do
      if [ "${spec%%|*}" = "$id" ]; then known="yes"; fi
    done
    test -n "$known" || { echo "plugins/$id is in the archive, but not in the default set"; exit 1; }
  done
fi
if [ "$want_sevenzip" = "--sevenzip" ]; then
  test -f plugins/vfs-archive/7z.so || test -f plugins/vfs-archive/7z.dll || { echo "no 7-Zip library beside the archive plugin"; exit 1; }
  test -f plugins/vfs-archive/7-Zip-License.txt || { echo "no 7-Zip licence beside the library"; exit 1; }
  echo "the 7-Zip library and its licence are beside the archive plugin"
fi
if [ -n "$installed" ]; then
  echo "the default set and every separate plugin of this platform loaded"
elif [ -n "$archive" ]; then
  echo "the default set of this platform loaded, and no separate plugin is in the archive"
else
  echo "every plugin for this platform loaded"
fi
