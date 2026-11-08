#!/usr/bin/env bash
# The docs-only decision of the ci and full workflows (issue 35): writes
# `code=false` to the step's outputs when every changed file is
# documentation, `code=true` otherwise. Documentation is Markdown anywhere
# and docs/, except BENCHMARKS.md, whose tables a unit test in crates/tests
# checks. On `code=false` the workflows skip their build jobs while the
# tier's gate job still reports success, so a docs-only pull request is
# green without a build and never waits for a check that cannot come.
#
# A pull request lists its files through the API; a push compares the
# commit before it with the one pushed. Whatever cannot be listed (a new or
# force-pushed branch, a comparison the API truncates at 300 files, any
# other event) builds.
set -euo pipefail

files=""
listed=true
case "${GITHUB_EVENT_NAME:-}" in
  pull_request)
    number="$(jq -r '.pull_request.number' "$GITHUB_EVENT_PATH")"
    files="$(gh api --paginate "repos/$GITHUB_REPOSITORY/pulls/$number/files?per_page=100" --jq '.[].filename')"
    ;;
  push)
    before="$(jq -r '.before' "$GITHUB_EVENT_PATH")"
    forced="$(jq -r '.forced' "$GITHUB_EVENT_PATH")"
    if [[ "$before" =~ ^0+$ || "$forced" == "true" ]]; then
      listed=false
    else
      compare="$(gh api "repos/$GITHUB_REPOSITORY/compare/$before...$GITHUB_SHA")"
      if [ "$(jq '.files | length' <<<"$compare")" -ge 300 ]; then
        listed=false
      else
        files="$(jq -r '.files[].filename' <<<"$compare")"
      fi
    fi
    ;;
  *)
    listed=false
    ;;
esac

if [ "$listed" = false ]; then
  echo "code=true" >> "$GITHUB_OUTPUT"
  echo "The change cannot be listed for a ${GITHUB_EVENT_NAME:-?} event: building." | tee -a "$GITHUB_STEP_SUMMARY"
  exit 0
fi

# Everything that is not documentation, BENCHMARKS.md included.
code="$(grep -v -E '^docs/|(^|/)[^/]*\.md$' <<<"$files" | grep -v '^$' || true)"
if grep -qx 'BENCHMARKS.md' <<<"$files"; then
  code="BENCHMARKS.md${code:+$'\n'$code}"
fi

if [ -n "$code" ]; then
  echo "code=true" >> "$GITHUB_OUTPUT"
  { echo "Files that need a build:"; echo; sed 's/^/- /' <<<"$code"; } | tee -a "$GITHUB_STEP_SUMMARY"
else
  echo "code=false" >> "$GITHUB_OUTPUT"
  { echo "Documentation only, nothing to build:"; echo; sed 's/^/- /' <<<"$files"; } | tee -a "$GITHUB_STEP_SUMMARY"
fi
