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
cat > "$HOME/.config/baloofilerc" <<CFG
[Basic Settings]
Indexing-Enabled=true

[General]
folders[\$e]=$fixture/
exclude folders[\$e]=
only basic indexing=true
CFG
QT_QPA_PLATFORM=offscreen baloo_file > "$HOME/baloo.log" 2>&1 &

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
