#!/usr/bin/env bash
# Builds what Rick's Linux packages and its Linux archive are made from. It
# is the build command of Trebuchet's reusable workflow linux-packages.yml
# (Open-Breach/trebuchet, docs/linux-packages.md), which the release
# workflow calls: the workflow runs it once per target — a Debian, Ubuntu
# or Fedora release — and architecture, inside that release's container,
# after it installed the build packages of the call and the pinned
# toolchain and put the artifact `third-party-licenses` into the working
# directory. Then it packages what this script leaves, with the metadata
# of crates/rick/Cargo.toml.
#
#   linux-package.sh
#
#   TREBUCHET_ARCH      x86_64 or aarch64
#   TREBUCHET_TARGET    the distribution release, such as debian-12
#   TREBUCHET_VERSION   the release's version
#   TREBUCHET_TARBALL   true in the one job per architecture whose build
#                       is also the release archive
#   TREBUCHET_DIST      the folder the workflow uploads whole: its files go
#                       to the smoke test and to the release job
#
# What it leaves, under target/release/:
#
#   bundle/     The tree of the archive: `cargo xtask bundle --release` with
#               the default set of plugins (specs/plugin-distribution.md
#               §4), third-party.json beside the binary, and the texts and
#               share/polkit-1 the archive carries at its root. The
#               workflow adds the desktop entry and the icons under share/
#               and makes rick-<version>-linux-<arch>.tar.gz from it.
#   package/    The same files for the deb and the rpm, split by the mode
#               they are installed with: bin/ (the programs, 755) and lib/
#               (everything else that sits beside the binary, 644), both
#               installed to /usr/lib/rick/; deb/package and rpm/package,
#               the file that tells Rick which package manager it came
#               from (specs/plugin-distribution.md §7).
#   packages/   `cargo xtask package --release`: the .rkp of every separate
#               plugin and plugins-linux-<arch>.json. They fit the archive
#               of this very build and no other, so the job that makes the
#               archive hands them out with it.
#   smoke/      What .github/scripts/linux-smoke.sh needs from this build
#               to test the package where it is installed: the test
#               programs pdfium_check and preview_check, which only run
#               against the libraries they were built with, and two short
#               videos. They leave as one tar file in TREBUCHET_DIST, which
#               the release job does not publish.
#
# Before it packages, it checks the bundle the way the release's smoke test
# checks an archive: the default set loads and no separate plugin is left.
# That check is of the build. Whether the package asks for every library
# the binary needs is the smoke test's to find out, in a container that
# has none of the build's.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."
root="$PWD"

case "${TREBUCHET_ARCH:-$(uname -m)}" in
  x86_64) platform=linux-x64 ;;
  aarch64) platform=linux-arm64 ;;
  *)
    echo "linux-package.sh: no Rick build for ${TREBUCHET_ARCH:-$(uname -m)}" >&2
    exit 1
    ;;
esac
for file in THIRD-PARTY-LICENSES.md third-party.json; do
  [ -s "$file" ] || {
    echo "linux-package.sh: no $file: the release's licenses job makes it and the call names its artifact" >&2
    exit 1
  }
done

# The checkout belongs to the runner's user and the build runs as root in
# the container; git refuses such a repository until it is told, and the
# About dialog's credits come from `git shortlog` (crates/rick/build.rs).
git config --global --add safe.directory "$root"

# pdfium, loaded from beside the binary (SPEC.md "Third-party libraries"):
# the same build the other platforms' archives take, named in the PDF
# previewer's setting, which is where `cargo xtask bundle` reads it.
pdfium="$root/target/pdfium"
rm -rf "$pdfium"
mkdir -p "$pdfium"
curl -fsSL "https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-$platform.tgz" | tar xz -C "$pdfium"
config="${XDG_CONFIG_HOME:-$HOME/.config}/rick"
mkdir -p "$config"
printf 'library = "%s"\n' "$pdfium/lib/libpdfium.so" > "$config/preview-pdf.toml"

# The 7-Zip library for the disk images, from the pinned sources.
export RICK_SEVENZIP_DIR="$root/target/sevenzip"
SEVENZIP_SOURCE=build RUNNER_OS=Linux .github/scripts/sevenzip.sh "$RICK_SEVENZIP_DIR"

cargo xtask bundle --release --features ext-preview-media/gstreamer
cargo xtask package --release

bundle=target/release/bundle
packages=target/release/packages
manifest="$packages/plugins-$platform.json"

# The archive and the `rick` package carry the default set: the separate
# plugins, the ones the manifest lists a package for, leave the tree. The
# ZFS plugin's helper rick-zfs stays beside the binary.
for id in $(jq -r '.plugins[].id' "$manifest"); do
  [ -d "$bundle/plugins/$id" ] || {
    echo "linux-package.sh: no plugins/$id in the bundle to take out" >&2
    exit 1
  }
  rm -r "${bundle:?}/plugins/$id"
done
cp third-party.json "$bundle/third-party.json"

.github/scripts/check-plugins.sh "$bundle" --sevenzip --archive
"$bundle/rick" --version

# The packages' tree, before the archive's texts join the bundle.
package=target/release/package
rm -rf "$package"
mkdir -p "$package/bin" "$package/deb" "$package/rpm"
cp -a "$bundle" "$package/lib"
for program in rick rick-perms rick-zfs; do
  mv "$package/lib/$program" "$package/bin/"
done
echo deb > "$package/deb/package"
echo rpm > "$package/rpm/package"

# What the archive has at its root besides the bundle.
cp LICENSE README.md CHANGELOG.md THIRD-PARTY-LICENSES.md "$bundle/"
cp "$bundle/plugins/vfs-archive/7-Zip-License.txt" "$bundle/7-Zip-License.txt"
mkdir -p "$bundle/share/polkit-1/actions"
cp packaging/linux/org.openbreach.rick.zfs.policy "$bundle/share/polkit-1/actions/"

# For the smoke test: the two test programs of this build, and two videos
# for preview_check to play, where the package's dependencies are all there
# is. Both are the test pattern, three seconds of it, written by the
# GStreamer of this container.
#   sample.avi         Motion JPEG, which GStreamer's base and good plug-in
#                      sets decode by themselves.
#   sample-h264.mp4    H.264, as most videos are: it plays only where the
#                      package brought a decoder for it. Debian's and
#                      Ubuntu's containers have the x264 encoder to write
#                      it with, Fedora's the OpenH264 one, whose output
#                      the parser turns into what an MP4 holds.
smoke=target/release/smoke
rm -rf "$smoke"
mkdir -p "$smoke"
cp target/release/examples/pdfium_check target/release/examples/preview_check "$smoke/"
pattern="videotestsrc num-buffers=90 pattern=smpte ! video/x-raw,format=I420,width=320,height=240,framerate=30/1"
# The pipelines are words for gst-launch, split on purpose.
# shellcheck disable=SC2086
gst-launch-1.0 -q $pattern ! jpegenc ! avimux ! filesink location="$smoke/sample.avi"
if gst-inspect-1.0 --exists x264enc; then
  h264="x264enc speed-preset=ultrafast tune=zerolatency key-int-max=15 bitrate=300"
elif gst-inspect-1.0 --exists openh264enc && gst-inspect-1.0 --exists h264parse; then
  h264="openh264enc bitrate=300000 ! h264parse"
else
  echo "linux-package.sh: no H.264 encoder in this container to write the smoke test's clip with" >&2
  exit 1
fi
# shellcheck disable=SC2086
gst-launch-1.0 -q $pattern ! $h264 ! mp4mux ! filesink location="$smoke/sample-h264.mp4"
ls -l "$smoke"

if [ -n "${TREBUCHET_DIST:-}" ]; then
  mkdir -p "$TREBUCHET_DIST"
  # One per job, named after it: the release job gathers the files of
  # every job into one folder.
  tar --create --file "$TREBUCHET_DIST/rick-${TREBUCHET_VERSION:?}-smoke-${TREBUCHET_TARGET:?}-$platform.tar" \
    --directory "$smoke" pdfium_check preview_check sample.avi sample-h264.mp4
  # The plugin packages and their manifest belong to the archive of this
  # build, so they leave with it, once per architecture.
  if [ "${TREBUCHET_TARBALL:-}" = true ]; then
    cp "$packages"/*.rkp "$manifest" "$TREBUCHET_DIST/"
  fi
  ls -l "$TREBUCHET_DIST"
fi
