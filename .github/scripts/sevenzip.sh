#!/usr/bin/env bash
# Puts the 7-Zip library (7z.so / 7z.dll, LGPL) and its licence text into a
# directory, for `cargo xtask bundle` (RICK_SEVENZIP_DIR) and the tests of
# the disk images (issue 51). The library is bundled in plugins/vfs-archive
# beside the archive plugin on every platform (SPEC.md "Third-party
# libraries").
#
#   sevenzip.sh <out dir>
#
# Where it comes from:
#   Linux:   the distribution's 7zip package (/usr/lib/7zip/7z.so), the path
#            CI's tests use; with SEVENZIP_SOURCE=build, the pinned 7-Zip
#            sources, built as DOC/readme.txt says (release builds).
#   macOS:   the pinned sources, built with the mac makefile for the arch.
#   Windows: the pinned installer from 7-zip.org, 7z.dll and License.txt
#            extracted with the runner's own 7z.
# The pinned release and the checksums of what is downloaded are below;
# every download is verified before it is used.
set -euo pipefail

out="${1:?usage: sevenzip.sh <out dir>}"
version="${SEVENZIP_VERSION:-25.01}"
tag="${version//./}"
src_sha256="ed087f83ee789c1ea5f39c464c55a5c9d4008deb0efe900814f2df262b82c36e"
win_sha256="78afa2a1c773caf3cf7edf62f857d2a8a5da55fb0fff5da416074c0d28b2b55f"
os="${RUNNER_OS:-$(uname -s)}"
mkdir -p "$out"

verify() {
  local file="$1" expected="$2" actual
  if command -v sha256sum > /dev/null; then
    actual="$(sha256sum "$file" | cut -d' ' -f1)"
  else
    actual="$(shasum -a 256 "$file" | cut -d' ' -f1)"
  fi
  if [ "$actual" != "$expected" ]; then
    echo "checksum of $file: $actual, expected $expected" >&2
    exit 1
  fi
}

# Downloads and verifies the pinned sources, unpacks them and builds
# Format7zF, the full-format 7z.so, with the makefile DOC/readme.txt names
# for this platform; copies the library and DOC/License.txt to $out.
build_from_source() {
  local work="${RUNNER_TEMP:-/tmp}/sevenzip-src" mak object_dir
  rm -rf "$work"
  mkdir -p "$work"
  curl -sSL -o "$work/src.tar.xz" "https://7-zip.org/a/7z$tag-src.tar.xz"
  verify "$work/src.tar.xz" "$src_sha256"
  tar xJf "$work/src.tar.xz" -C "$work"
  local -a make_vars=()
  case "$os" in
    macOS|Darwin)
      if [ "$(uname -m)" = "arm64" ]; then mak=cmpl_mac_arm64.mak; else mak=cmpl_mac_x64.mak; fi
      # The mac makefile compiles with -Weverything -Werror, so every
      # warning a newer Apple clang adds stops the build: clang 17 brought
      # -Wc++-keyword on the `and` mnemonic in the arm64 assembly macros
      # and -Wimplicit-void-ptr-cast in LzmaEnc.c. The warnings are the
      # library's, not Rick's, so on macOS the build keeps -Wall -Wextra
      # and drops -Werror and -Weverything; the two variables are the
      # makefile's own (7zip_gcc.mak, warn_clang_mac.mak).
      make_vars=("CFLAGS_WARN_WALL=-Wall -Wextra" "CFLAGS_WARN=-Wno-poison-system-directories") ;;
    *) mak=cmpl_gcc.mak ;;
  esac
  (cd "$work/CPP/7zip/Bundles/Format7zF" && make -j"$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)" -f "../../$mak" "${make_vars[@]}" > "$work/build.log" 2>&1) \
    || { tail -50 "$work/build.log" >&2; echo "building 7z.so failed" >&2; exit 1; }
  object_dir="$(find "$work/CPP/7zip/Bundles/Format7zF/b" -name 7z.so -print -quit)"
  test -n "$object_dir" || { echo "no 7z.so under $work/CPP/7zip/Bundles/Format7zF/b" >&2; exit 1; }
  cp "$object_dir" "$out/7z.so"
  cp "$work/DOC/License.txt" "$out/License.txt"
}

case "$os" in
  Linux)
    if [ "${SEVENZIP_SOURCE:-package}" = "build" ]; then
      build_from_source
    else
      if [ ! -f /usr/lib/7zip/7z.so ]; then
        sudo apt-get update
        sudo apt-get install -y --no-install-recommends 7zip
      fi
      cp /usr/lib/7zip/7z.so "$out/7z.so"
      cp /usr/share/doc/7zip/copyright "$out/License.txt"
    fi ;;
  macOS|Darwin)
    build_from_source ;;
  Windows)
    installer="${RUNNER_TEMP:-/tmp}/7z$tag-x64.exe"
    curl -sSL -o "$installer" "https://7-zip.org/a/7z$tag-x64.exe"
    verify "$installer" "$win_sha256"
    # The runner's own 7-Zip unpacks the installer; the library and the
    # licence are plain files inside it.
    7z e -y "-o$out" "$installer" 7z.dll License.txt > /dev/null ;;
  *) echo "unknown platform $os" >&2; exit 1 ;;
esac
ls -l "$out"
echo "7-Zip library in $out"
