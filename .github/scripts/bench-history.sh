#!/usr/bin/env bash
# Records a benchmark run on the `benchmark-history` branch (issue 35): one
# Markdown file per run under nightly/, named by the date and the platform,
# with the commit measured, the runner's hardware line and the median and
# worst table of bench-summary.py. The nightly bench run on dev records
# here, so a regression can be traced to a day range, and so to the pull
# requests that landed in it. The branch is created on the first run;
# nothing else is on it.
#
#   bench-history.sh <summary.md> <hardware.md> <platform>
#
# Runs in the checkout of the measured commit, with the token
# actions/checkout left in its configuration, and GITHUB_RUN_ID and RUNS in
# the environment.
set -euo pipefail

summary="${1:?the summary table}"
hardware="${2:?the hardware line}"
platform="${3:?the platform, as linux-x64}"
branch="benchmark-history"
date="$(date -u +%Y-%m-%d)"
sha="$(git rev-parse --short HEAD)"
run="$GITHUB_SERVER_URL/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID"

git config user.name "github-actions[bot]"
git config user.email "41898282+github-actions[bot]@users.noreply.github.com"

# Three attempts: a manual run records every platform's job, and two jobs
# may push at once; the loser starts over from the new tip.
for attempt in 1 2 3; do
  rm -rf history
  git worktree prune
  git branch -D "$branch" 2>/dev/null || true
  if git fetch --depth 1 origin "$branch" 2>/dev/null; then
    git worktree add --detach history FETCH_HEAD
  else
    git worktree add --detach history HEAD
    git -C history checkout -q --orphan "$branch"
    git -C history rm -rfq .
    cat > history/README.md <<'README'
# Benchmark history

The summaries of the `bench` workflow's nightly runs
(`.github/workflows/bench.yml` on `main`, which checks `dev` out), one file
per run under `nightly/`, named by date and platform: the commit measured,
the runner's hardware line and the median and worst of every acceptance
benchmark. A manual run records here too when asked. The workflow writes
this branch; nothing else does.
README
  fi
  mkdir -p history/nightly
  file="history/nightly/$date-$platform.md"
  if [ -e "$file" ]; then
    file="history/nightly/$date-$platform-$GITHUB_RUN_ID.md"
  fi
  {
    echo "# $date, $platform"
    echo
    echo "Commit \`$sha\`, [run $GITHUB_RUN_ID]($run), ${RUNS:-?} runs per benchmark, measured with \`RICK_BENCH_RECORD\` (a run over its target is recorded, not failed)."
    echo
    cat "$hardware"
    cat "$summary"
  } > "$file"
  git -C history add -A
  git -C history commit -q -m "bench: $date $platform at $sha"
  if git -C history push -q origin "HEAD:refs/heads/$branch"; then
    echo "recorded $file on $branch"
    exit 0
  fi
  echo "push $attempt did not land; retrying from the new tip"
  sleep 5
done
echo "could not push to $branch"
exit 1
