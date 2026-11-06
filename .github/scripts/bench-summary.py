#!/usr/bin/env python3
"""Summarise the `BENCH` lines of several benchmark runs.

Reads files of test output (one per run, as `cargo test -- --nocapture`
prints it) and writes a Markdown table per operating system with the median
and the worst of every benchmark, in the units BENCHMARKS.md uses. The
lines look like

    BENCH listing-first linux 23456 23.456ms (target 50ms)

and the first four fields are what is read: key, OS, microseconds.
"""

import glob
import re
import statistics
import sys
from collections import defaultdict

LINE = re.compile(r"^BENCH (\S+) (\S+) (\d+)")


def pretty(micros: float) -> str:
    """Microseconds as BENCHMARKS.md writes them: s above 10 s, else ms."""
    if micros >= 10_000_000:
        return f"{micros / 1_000_000:.1f} s"
    if micros >= 100_000:
        return f"{micros / 1_000:.0f} ms"
    return f"{micros / 1_000:.1f} ms"


def main(paths: list[str]) -> int:
    runs: dict[tuple[str, str], list[int]] = defaultdict(list)
    for pattern in paths:
        for path in sorted(glob.glob(pattern)):
            with open(path, encoding="utf-8", errors="replace") as handle:
                for line in handle:
                    match = LINE.match(line.strip())
                    if match:
                        key, os_name, micros = match.groups()
                        runs[(key, os_name)].append(int(micros))
    if not runs:
        print("no BENCH lines found", file=sys.stderr)
        return 1
    for os_name in sorted({os_name for _, os_name in runs}):
        print(f"### {os_name}\n")
        print("| Benchmark | Runs | Median | Worst | All runs (ms) |")
        print("| --- | --- | --- | --- | --- |")
        for key, name in sorted(runs):
            if name != os_name:
                continue
            values = runs[(key, name)]
            median = statistics.median(values)
            worst = max(values)
            all_runs = ", ".join(f"{v / 1000:.1f}" for v in values)
            print(
                f"| {key} | {len(values)} | {pretty(median)} | {pretty(worst)} | {all_runs} |"
            )
        print()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:] or ["bench-*.log"]))
