#!/usr/bin/env bash
# A stand-in for polkit's pkexec on the Linux CI runner, where no desktop
# can show a prompt: runs the program as root through the runner's
# passwordless sudo, so the elevation test of the permission jobs
# (crates/rick/tests/perms.rs, behind RICK_ELEVATE_FIXTURE) goes through
# the real helper the real way, prompt excepted. Installed as
# /usr/local/bin/pkexec by the test job of ci.yml.
#
# sudo, like pkexec, starts the program with a clean environment. The
# installed helper finds the kernel dylibs and libstd beside itself in the
# bundle; the one cargo built for the tests finds them through the library
# path cargo set, which sudo cannot be asked to keep (the loader strips it
# from a setuid program before sudo runs), so it is handed on as an argument
# to env.
set -euo pipefail
exec sudo -n -- env LD_LIBRARY_PATH="${LD_LIBRARY_PATH:-}" "$@"
