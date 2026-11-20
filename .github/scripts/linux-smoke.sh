#!/usr/bin/env bash
# Rick's own part of the Linux smoke test: the `smoke-command` of the call
# to Trebuchet's linux-packages.yml in the release workflow
# (Open-Breach/trebuchet, docs/linux-packages.md). Trebuchet's test installs
# the package in a fresh container of the distribution release and runs
# `rick --version`. This script then checks what only Rick knows, there, in
# a container that has the package's declared dependencies and nothing of
# the build:
#
#   - the files are where docs/building.md says, with the licence texts;
#   - every plugin of the package loads, and no other is in it;
#   - pdfium, from beside the binary, opens a PDF and renders a page;
#   - the media previewer plays two videos through the system's GStreamer,
#     one in Motion JPEG and one in H.264, whose decoder the package has
#     to have brought;
#   - where this build's plugin packages are at hand, each installs into
#     the per-user folder and loads from there.
#
# It runs once after the package is installed and, in the job whose build
# is also the archive, once more for the unpacked archive:
#
#   TREBUCHET_SMOKE        package or tarball
#   TREBUCHET_SMOKE_DIR    the unpacked archive, for tarball
#   TREBUCHET_DIST         the build job's files for the release: the
#                          package, and the archive and the plugin
#                          packages where the job made them
#   TREBUCHET_SMOKE_FILES  the build job's files for this test only: the
#                          tar file of test programs and videos that
#                          linux-package.sh put there. No release gets it.
#   TREBUCHET_FORMAT       deb or rpm
#   TREBUCHET_TARGET, TREBUCHET_ARCH, TREBUCHET_VERSION
#
# The order matters. Everything that could miss a library runs first, with
# nothing installed but the package. Only the last check installs a tool,
# unzip, to unpack the plugin packages.
#
# What it cannot check, in a container: a window. Rick's user interface
# loads its libraries for X11, Wayland, Vulkan and EGL when it opens one,
# and nothing here opens one. Nor does a container play sound.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."
repo="$PWD"

fail() {
  echo "linux-smoke.sh: $*" >&2
  exit 1
}

what="${TREBUCHET_SMOKE:?}"
dist="${TREBUCHET_DIST:?}"
files="${TREBUCHET_SMOKE_FILES:?}"
format="${TREBUCHET_FORMAT:?}"
target="${TREBUCHET_TARGET:?}"
version="${TREBUCHET_VERSION:?}"
case "${TREBUCHET_ARCH:?}" in
  x86_64) platform=linux-x64 ;;
  aarch64) platform=linux-arm64 ;;
  *) fail "no Rick build for $TREBUCHET_ARCH" ;;
esac

# Where the binary and everything beside it is, the command a user starts,
# the XDG data folder, and where the licence texts are.
case "$what" in
  package)
    root=/usr/lib/rick
    rick=rick
    share=/usr/share
    case "$format" in
      deb) texts=(/usr/share/doc/rick/copyright /usr/share/doc/rick/THIRD-PARTY-LICENSES.md) ;;
      rpm) texts=(/usr/share/licenses/rick/LICENSE /usr/share/licenses/rick/THIRD-PARTY-LICENSES.md) ;;
      *) fail "unknown package format $format" ;;
    esac
    ;;
  tarball)
    root="${TREBUCHET_SMOKE_DIR:?}"
    rick="$root/rick"
    share="$root/share"
    texts=("$root/LICENSE" "$root/THIRD-PARTY-LICENSES.md" "$root/README.md" "$root/CHANGELOG.md" "$root/7-Zip-License.txt")
    ;;
  *) fail "TREBUCHET_SMOKE is '$what', neither package nor tarball" ;;
esac
echo "Rick's smoke test of the $what on $target, $platform, in $root"

# The licence texts. For a package the list of its files is asked as well
# as the disk, because an image may be set up to install less than the
# package holds: Ubuntu's container images tell dpkg to leave
# /usr/share/doc out, all of it but the copyright files. A text that is in
# the package and left out for that reason is said so and not held against
# the package; a desktop installs it.
listed=""
docs_left_out=""
if [ "$what" = package ]; then
  case "$format" in
    deb)
      listed="$(dpkg-query --listfiles rick)"
      if grep -qs '^path-exclude=/usr/share/doc/' /etc/dpkg/dpkg.cfg.d/*; then docs_left_out=yes; fi
      ;;
    rpm) listed="$(rpm --query --list rick)" ;;
  esac
fi
for file in "${texts[@]}"; do
  if [ "$what" = package ]; then
    grep -qxF "$file" <<< "$listed" || fail "the package does not hold $file"
  fi
  if [ ! -s "$file" ]; then
    case "$docs_left_out:$file" in
      yes:*/copyright) fail "no $file" ;;
      yes:/usr/share/doc/*)
        echo "$file is in the package and not on disk: this image's dpkg leaves /usr/share/doc out"
        continue
        ;;
      *) fail "no $file" ;;
    esac
  fi
  case "$file" in
    *THIRD-PARTY-LICENSES.md)
      grep -q '^## Bundled components$' "$file" || fail "$file does not name the bundled components"
      ;;
  esac
done

# What sits beside the binary.
for file in "$root/third-party.json" "$root/libpdfium.so" \
  "$root/licenses/pdfium/LICENSE" "$root/licenses/rust/COPYRIGHT-library.html"; do
  [ -s "$file" ] || fail "no $file"
done
[ "$(find "$root/licenses/pdfium" -type f | wc -l)" -gt 5 ] \
  || fail "$root/licenses/pdfium holds no texts of the libraries built into pdfium"
find "$root" -maxdepth 1 -name 'libstd-*.so' | grep -q . || fail "no libstd beside the binary"
[ -f "$root/libext.so" ] || fail "no ext library beside the binary"
case "$what" in
  package)
    # The file that tells Rick which package manager it came from
    # (specs/plugin-distribution.md §7).
    [ "$(cat "$root/package" 2> /dev/null)" = "$format" ] || fail "$root/package does not say $format"
    [ -x /usr/bin/rick ] || fail "no /usr/bin/rick"
    ;;
  tarball)
    [ ! -e "$root/package" ] || fail "the archive has a file named package, as if it were a deb or an rpm"
    [ -f "$share/polkit-1/actions/org.openbreach.rick.zfs.policy" ] || fail "no polkit action for the ZFS helper under share/"
    ;;
esac

# The desktop entry names the app id `rick` as its icon and its window
# class, and every icon of the repository's hicolor set is at its place
# (specs/app-icons.md). Trebuchet's test validates the entry itself.
entry="$share/applications/rick.desktop"
grep -qx 'Icon=rick' "$entry" || fail "the desktop entry's Icon is not rick"
grep -qx 'StartupWMClass=rick' "$entry" || fail "the desktop entry's StartupWMClass is not rick"
while IFS= read -r icon; do
  [ -f "$share/icons/hicolor/$icon" ] || fail "no $share/icons/hicolor/$icon"
done < <(cd "$repo/assets/icons/linux/hicolor" && find . -type f | sed 's|^\./||' | sort)

# The version text, the About dialog's details (specs/about-dialog.md): the
# name and the version, the build line, the toolchain, then the plugins.
out="$("$rick" --version)"
echo "$out"
sed -n 1p <<< "$out" | grep -Eq "^Rick ${version%%-*}(-[0-9A-Za-z.]+)?$" || fail "unexpected version line"
sed -n 2p <<< "$out" | grep -Eq "^[0-9a-f]{7,} · Linux [a-z0-9_]+ · plugin ABI [0-9]+ · built [0-9]{4}-[0-9]{2}-[0-9]{2}$" \
  || fail "unexpected build line"
sed -n 3p <<< "$out" | grep -q "^rustc " || fail "no toolchain line"

# Every plugin of the package: the default set is in plugins/ and loaded,
# no separate plugin and nothing else is there, and the 7-Zip library is
# beside the archive plugin. The per-user folder has to be empty for that
# to say anything about the package.
user_plugins="${XDG_CONFIG_HOME:-$HOME/.config}/rick/plugins"
if [ -d "$user_plugins" ] && [ -n "$(ls -A "$user_plugins")" ]; then
  fail "$user_plugins is not empty in this container"
fi
"$repo/.github/scripts/check-plugins.sh" "$root" --sevenzip --archive

# The two test programs of this build, and the video. They sit beside the
# binary while they run: that is where they look for pdfium and for
# plugins/, and where the libraries they share with Rick are.
tar="$(find "$files" -maxdepth 1 -name "rick-*-smoke-$target-$platform.tar")"
if [ -z "$tar" ] || [ "$(wc -l <<< "$tar")" != 1 ]; then
  fail "not exactly one tar file of test programs for $target $platform in $files"
fi
# They are not release files, and none is among the files for the release.
[ -z "$(find "$dist" -name '*-smoke-*')" ] || fail "test files among the files for the release in $dist"
programs="$(mktemp -d)"
tar --extract --file "$tar" --directory "$programs"
cleanup() {
  rm -f "$root/pdfium_check" "$root/preview_check"
  rm -rf "$programs"
}
trap cleanup EXIT
cp "$programs/pdfium_check" "$programs/preview_check" "$root/"
chmod 755 "$root/pdfium_check" "$root/preview_check"

# pdfium, loaded from beside the binary with no setting and no system
# pdfium: it opens a PDF of three pages and renders the first.
pdf="$(LD_LIBRARY_PATH="$root" "$root/pdfium_check")" || fail "pdfium did not open the PDF"
echo "$pdf"
grep -q "^pdfium ok: 3 pages" <<< "$pdf" || fail "unexpected answer from pdfium_check"

# Two videos, played for real: the test program loads the plugins as Rick
# does and plays each clip through the previewer that claims it, with the
# GStreamer the package asked for. Motion JPEG needs the base and good
# plug-in sets. H.264 needs the decoder the package depends on beside them:
# without it this is where the test stops.
for clip in sample.avi sample-h264.mp4; do
  if ! media="$(LD_LIBRARY_PATH="$root" "$root/preview_check" "$programs/$clip" --play)"; then
    echo "$clip did not play; once more with GStreamer's warnings:"
    GST_DEBUG=2 LD_LIBRARY_PATH="$root" "$root/preview_check" "$programs/$clip" --play 2>&1 \
      | grep -v "^plugin .*: loaded, " | tail -40 || true
    fail "the media previewer did not play $clip"
  fi
  echo "$media"
  grep -q "^plugin preview-media: loaded, beside the binary, " <<< "$media" \
    || fail "the media plugin was not loaded from beside the binary"
  grep -q "^preview ok: .*first frame 320x240, played to " <<< "$media" || fail "$clip did not play"
done

# The plugin packages, installed the way Install from file… does it: each
# is unpacked into the per-user plugins folder, its folder whole, and every
# separate plugin then loads, which it can only do from there. A package
# fits the build it was made with and no other, so this runs where the
# build's own packages are at hand: in the job whose build is also the
# archive, for its deb and for the archive.
mapfile -t packages < <(find "$dist" -maxdepth 1 -name "*-$platform.rkp" | sort)
if [ "${#packages[@]}" -eq 0 ]; then
  echo "no plugin packages of this build here: installing one into the per-user folder is not checked for $target"
else
  [ -f "$dist/plugins-$platform.json" ] || fail "plugin packages without their list, plugins-$platform.json"
  case "$format" in
    deb)
      export DEBIAN_FRONTEND=noninteractive
      apt-get install -y --no-install-recommends unzip > /dev/null
      ;;
    rpm) dnf -y --setopt=install_weak_deps=False install unzip > /dev/null ;;
  esac
  mkdir -p "$user_plugins"
  for package in "${packages[@]}"; do
    echo "installing $(basename "$package")"
    unzip -q "$package" -d "$user_plugins"
  done
  ls "$user_plugins"
  "$repo/.github/scripts/check-plugins.sh" "$root" --archive --installed
  # Empty again, for the test of the archive that follows in this container.
  rm -rf "$user_plugins"
fi

echo "Rick's smoke test passed for the $what on $target"
