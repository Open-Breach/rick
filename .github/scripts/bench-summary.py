#!/usr/bin/env python3
"""Summarise the `BENCH` lines of several benchmark runs.

Reads files of test output (one per run, as `cargo test -- --nocapture`
prints it) and writes, per operating system, a Markdown table with the
target, the median and the worst run of every benchmark, in the units
BENCHMARKS.md uses, and whether the benchmark is over its target. The
lines look like

    BENCH listing-first linux 23456 23.456ms (target 50ms; runs [23.456ms])

and what is read is the key, the OS, the microseconds and the target in
the brackets, written as Rust prints a duration (`tests::bench::line`). A
test that prints nothing else lands its line after the harness's
`test name ...` prefix, so the marker is searched for anywhere in the line.

A benchmark is **over** when the median of its runs is over the target,
which is what `tests::bench::record` asserts where it asserts. One whose
worst run alone is over is not counted; its row says so. A line above the
table counts and names the benchmarks that are over. All of it is in
words, so that it reads the same in the job summary and in the file on
the benchmark-history branch. The summary reports; it never fails a run
for a number.
"""

import glob
import re
import statistics
import sys
from collections import defaultdict

LINE = re.compile(
    r"\bBENCH (\S+) (\S+) (\d+)(?: \S+ \(target (\d+(?:\.\d+)?)(s|ms|µs|ns)[;)])?"
)

# One of a duration's units, as Rust prints it, in microseconds.
UNITS = {"s": 1_000_000.0, "ms": 1_000.0, "µs": 1.0, "ns": 0.001}


def pretty(micros: float) -> str:
    """Microseconds as BENCHMARKS.md writes them: s above 10 s, else ms."""
    if micros >= 10_000_000:
        return f"{micros / 1_000_000:.1f} s"
    if micros >= 100_000:
        return f"{micros / 1_000:.0f} ms"
    return f"{micros / 1_000:.1f} ms"


def names(keys: list[str]) -> str:
    """The keys as a list in a sentence."""
    return ", ".join(f"`{key}`" for key in keys)


def verdict(total: int, over: list[str], worst_over: list[str], bare: list[str]) -> str:
    """The lines above a table: how many benchmarks are over their target."""
    if not over:
        text = f"No benchmark is over its target: {total} measured."
    elif len(over) == 1:
        text = f"**1 of {total} benchmarks is over its target:** {names(over)}."
    else:
        text = f"**{len(over)} of {total} benchmarks are over their target:** {names(over)}."
    if worst_over:
        text += (
            f" Not counted, the median within the target and the worst run"
            f" over it: {names(worst_over)}."
        )
    if bare:
        text += f" No target on the line, so not compared: {names(bare)}."
    return text


def main(paths: list[str]) -> int:
    # The same text on every runner: Windows would write its own code page
    # and line ends.
    sys.stdout.reconfigure(encoding="utf-8", newline="\n")
    runs: dict[tuple[str, str], list[int]] = defaultdict(list)
    # Per benchmark and OS, the target in microseconds and as it is shown.
    targets: dict[tuple[str, str], tuple[float, str]] = {}
    for pattern in paths:
        for path in sorted(glob.glob(pattern)):
            with open(path, encoding="utf-8", errors="replace") as handle:
                for line in handle:
                    match = LINE.search(line)
                    if match:
                        key, os_name, micros, number, unit = match.groups()
                        runs[(key, os_name)].append(int(micros))
                        if number is not None:
                            targets[(key, os_name)] = (
                                float(number) * UNITS[unit],
                                f"{number} {unit}",
                            )
    if not runs:
        print("no BENCH lines found", file=sys.stderr)
        return 1
    for os_name in sorted({os_name for _, os_name in runs}):
        rows = []
        over, worst_over, bare = [], [], []
        for key, name in sorted(runs):
            if name != os_name:
                continue
            values = runs[(key, name)]
            median = statistics.median(values)
            worst = max(values)
            all_runs = ", ".join(f"{v / 1000:.1f}" for v in values)
            target = targets.get((key, name))
            if target is None:
                shown, state = "—", "—"
                bare.append(key)
            elif median > target[0]:
                shown, state = target[1], "**over**"
                over.append(key)
            elif worst > target[0]:
                shown, state = target[1], "within; worst run over"
                worst_over.append(key)
            else:
                shown, state = target[1], "within"
            rows.append(
                f"| {key} | {len(values)} | {shown} | {pretty(median)} | {pretty(worst)}"
                f" | {state} | {all_runs} |"
            )
        print(f"### {os_name}\n")
        print(verdict(len(rows), over, worst_over, bare))
        print()
        print("| Benchmark | Runs | Target | Median | Worst | Against the target | All runs (ms) |")
        print("| --- | --- | --- | --- | --- | --- | --- |")
        for row in rows:
            print(row)
        print()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:] or ["bench-*.log"]))
