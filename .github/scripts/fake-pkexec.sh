#!/usr/bin/env bash
# A stand-in for polkit's pkexec on the Linux CI runner, where no desktop
# can show a prompt: runs the program as root through the runner's
# passwordless sudo, so the elevation test of the permission jobs
# (crates/rick/tests/perms.rs, behind RICK_ELEVATE_FIXTURE) goes through
# the real helper the real way, prompt excepted. Installed as
# /usr/local/bin/pkexec by the test job of ci.yml.
set -euo pipefail
exec sudo -n -- "$@"
