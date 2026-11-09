#!/usr/bin/env bash
# Runs a command with the Linux file indexers up (issue 22): a private
# session bus from dbus-run-session, Tracker's miner and Baloo's file
# indexer started in it over a fixture directory under $HOME, and the
# command run once both own their bus names, with RICK_SEARCH_FIXTURE
# naming the directory, so the Tracker and Baloo connectors' tests in
# crates/tests/tests/search.rs run against the real services: a file
# written there must be found by name. RICK_REQUIRE_TRACKER and
# RICK_REQUIRE_BALOO make those tests fail rather than skip.
#
#   with-indexers.sh cargo test --workspace ...
set -euo pipefail

if [ "${WITH_INDEXERS_INNER:-}" != "1" ]; then
  export WITH_INDEXERS_INNER=1
  exec dbus-run-session -- "$0" "$@"
fi

fixture="$HOME/rick-search-fixture"
mkdir -p "$fixture" "$HOME/.config"
export RICK_SEARCH_FIXTURE="$fixture"
export RICK_REQUIRE_TRACKER=1
export RICK_REQUIRE_BALOO=1

# Baloo: its config file names the folder; the indexer reads it at start.
# `baloo_file` is on the PATH with KF5 (Ubuntu 24.04's baloo-kf5) and in
# the KF6 libexec directory with `baloo6` (Ubuntu 26.04).
cat > "$HOME/.config/baloofilerc" <<CFG
[Basic Settings]
Indexing-Enabled=true

[General]
folders[\$e]=$fixture/
exclude folders[\$e]=
only basic indexing=true
CFG
baloo_file_bin="$(command -v baloo_file || true)"
if [ -z "$baloo_file_bin" ]; then
  for candidate in /usr/lib/*/libexec/kf6/baloo_file /usr/libexec/kf6/baloo_file /usr/lib/*/libexec/baloo_file; do
    if [ -x "$candidate" ]; then baloo_file_bin="$candidate"; break; fi
  done
fi
if [ -z "$baloo_file_bin" ]; then
  echo "baloo_file not found: install baloo6 (Ubuntu 26.04) or baloo-kf5 (24.04)"
  exit 1
fi
QT_QPA_PLATFORM=offscreen "$baloo_file_bin" > "$HOME/baloo.log" 2>&1 &

# Tracker: the miner's settings through the keyfile backend, which needs
# no dconf service; only the miner process reads them.
(
  export GSETTINGS_BACKEND=keyfile
  gsettings set org.freedesktop.Tracker3.Miner.Files index-recursive-directories "['$fixture']"
  gsettings set org.freedesktop.Tracker3.Miner.Files index-single-directories "[]"
  gsettings set org.freedesktop.Tracker3.Miner.Files index-on-battery true
  gsettings set org.freedesktop.Tracker3.Miner.Files index-removable-devices false
  gsettings set org.freedesktop.Tracker3.Miner.Files initial-sleep 0
  exec /usr/libexec/tracker-miner-fs-3 > "$HOME/tracker.log" 2>&1
) &

has_owner() {
  gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
    --method org.freedesktop.DBus.NameHasOwner "$1" 2>/dev/null | grep -q true
}
for _ in $(seq 1 60); do
  if has_owner org.kde.baloo && has_owner org.freedesktop.Tracker3.Miner.Files; then break; fi
  sleep 1
done
has_owner org.kde.baloo || { echo "baloo_file did not come up"; cat "$HOME/baloo.log"; exit 1; }
has_owner org.freedesktop.Tracker3.Miner.Files || { echo "tracker-miner-fs-3 did not come up"; cat "$HOME/tracker.log"; exit 1; }
echo "indexers up over $fixture"
"$@"
