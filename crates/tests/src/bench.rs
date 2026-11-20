//! The acceptance benchmarks: their targets per operating system and the
//! line each timing test prints for `BENCHMARKS.md`.
//!
//! Every timing test ends in [`record`], which prints one `BENCH` line that
//! the bench workflow collects and asserts the measurement against the
//! target of the operating system it runs on. The Linux targets are the
//! spec's (Decisions table, QUESTIONS.md 5, 24 and 30); the macOS and
//! Windows targets were set from five measured runs on the CI runners, as
//! `BENCHMARKS.md` records, last from the 0.2 run of issue 25 (QUESTIONS.md
//! 188). Setting `RICK_BENCH_RECORD` turns the assertion off: the bench
//! workflow measures that way, and its summary marks every benchmark whose
//! median is over the target (issue 277).
//!
//! A timing test holds what it measured to a figure through [`record`]
//! alone: an assertion of its own would fail the bench run and cost it the
//! numbers of every benchmark.

use std::time::Duration;

/// One benchmark: its key, as `BENCHMARKS.md` and the `BENCH` lines name
/// it, and the target per operating system.
#[derive(Debug, Clone, Copy)]
pub struct Benchmark {
    /// The key, unique over the table.
    pub key: &'static str,
    /// The target on Linux.
    pub linux: Duration,
    /// The target on macOS.
    pub macos: Duration,
    /// The target on Windows.
    pub windows: Duration,
}

const fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

const fn us(n: u64) -> Duration {
    Duration::from_micros(n)
}

/// Every benchmark and its targets. The order is the order of the tables
/// in `BENCHMARKS.md`, which says how each number was set.
pub const BENCHMARKS: &[Benchmark] = &[
    Benchmark {
        key: "fixture",
        linux: ms(30_000),
        macos: ms(35_000),
        windows: ms(45_000),
    },
    Benchmark {
        key: "listing-first",
        linux: ms(50),
        macos: ms(50),
        windows: ms(50),
    },
    Benchmark {
        key: "listing-complete",
        linux: ms(400),
        macos: ms(1_600),
        windows: ms(400),
    },
    Benchmark {
        key: "copy-sparse",
        linux: ms(10_000),
        macos: ms(10_000),
        windows: ms(10_000),
    },
    Benchmark {
        key: "details-frame",
        linux: us(16_600),
        macos: us(16_600),
        windows: us(16_600),
    },
    Benchmark {
        key: "image-first",
        linux: ms(200),
        macos: ms(200),
        windows: ms(200),
    },
    Benchmark {
        key: "pdf-first",
        linux: ms(200),
        macos: ms(200),
        windows: ms(200),
    },
    Benchmark {
        key: "preview-frame",
        linux: us(16_600),
        macos: us(16_600),
        windows: us(16_600),
    },
    Benchmark {
        key: "git-batch-cached",
        linux: ms(16),
        macos: ms(16),
        windows: ms(16),
    },
    Benchmark {
        key: "git-batch-pending",
        linux: ms(16),
        macos: ms(16),
        windows: ms(16),
    },
    Benchmark {
        key: "git-complete",
        linux: ms(500),
        macos: ms(500),
        windows: ms(500),
    },
    Benchmark {
        key: "thumbnails",
        linux: ms(500),
        macos: ms(500),
        windows: ms(500),
    },
    Benchmark {
        key: "thumbnails-frame",
        linux: us(16_600),
        macos: us(16_600),
        windows: us(16_600),
    },
    Benchmark {
        key: "wrap-frame",
        linux: us(16_600),
        macos: us(16_600),
        windows: us(16_600),
    },
    Benchmark {
        key: "startup",
        linux: ms(25),
        macos: ms(25),
        windows: ms(25),
    },
    Benchmark {
        key: "plugin-load",
        linux: ms(50),
        macos: ms(50),
        windows: ms(50),
    },
    Benchmark {
        key: "archive-zip-first",
        linux: ms(50),
        macos: ms(50),
        windows: ms(50),
    },
    Benchmark {
        key: "tree-first",
        linux: ms(50),
        macos: ms(100),
        windows: ms(50),
    },
    Benchmark {
        key: "search-first",
        linux: ms(100),
        macos: ms(100),
        windows: ms(100),
    },
    Benchmark {
        key: "cancel",
        linux: ms(20),
        macos: ms(80),
        windows: ms(20),
    },
    Benchmark {
        key: "session-start",
        linux: ms(5),
        macos: ms(12),
        windows: ms(5),
    },
    Benchmark {
        key: "office-convert",
        linux: ms(5_000),
        macos: ms(5_000),
        windows: ms(5_000),
    },
    Benchmark {
        key: "waveform",
        linux: ms(500),
        macos: ms(500),
        windows: ms(500),
    },
    Benchmark {
        key: "zip-first",
        linux: ms(100),
        macos: ms(100),
        windows: ms(100),
    },
    Benchmark {
        key: "csv-window",
        linux: us(16_600),
        macos: us(16_600),
        windows: us(16_600),
    },
    Benchmark {
        key: "previewers-frame",
        linux: us(16_600),
        macos: us(16_600),
        windows: us(16_600),
    },
    Benchmark {
        key: "3d-first",
        linux: ms(300),
        macos: ms(300),
        windows: ms(300),
    },
    Benchmark {
        key: "settings-reload",
        linux: ms(50),
        macos: ms(50),
        windows: ms(50),
    },
];

/// The benchmark named `key`.
///
/// # Panics
///
/// For a key the table does not have: a typo in a test.
#[must_use]
pub fn benchmark(key: &str) -> &'static Benchmark {
    BENCHMARKS
        .iter()
        .find(|b| b.key == key)
        .unwrap_or_else(|| panic!("no benchmark {key}"))
}

/// The target of `key` on the operating system the tests run on.
///
/// # Panics
///
/// For an unknown key or an operating system without a target.
#[must_use]
pub fn target(key: &str) -> Duration {
    target_on(key, std::env::consts::OS)
}

/// The target of `key` on `os`, named as `std::env::consts::OS` names it.
///
/// # Panics
///
/// For an unknown key or an operating system without a target.
#[must_use]
pub fn target_on(key: &str, os: &str) -> Duration {
    let benchmark = benchmark(key);
    match os {
        "linux" => benchmark.linux,
        "macos" => benchmark.macos,
        "windows" => benchmark.windows,
        other => panic!("no benchmark target for {other}"),
    }
}

/// How many times a timed test repeats its measured part: once on Linux,
/// where the spec's numbers hold on a single run, three times on macOS and
/// Windows, whose shared runners are too noisy for one run against a 1.5×
/// budget; [`record`] then asserts the median of the runs. `RICK_BENCH_RUNS`
/// overrides the count, to try the repeat paths on any machine.
#[must_use]
pub fn runs() -> usize {
    if let Some(runs) = std::env::var("RICK_BENCH_RUNS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0)
    {
        return runs;
    }
    if std::env::consts::OS == "linux" {
        1
    } else {
        3
    }
}

/// The median of `samples`: the middle one, the upper middle of an even
/// count.
///
/// # Panics
///
/// On no samples.
#[must_use]
pub fn median(samples: &[Duration]) -> Duration {
    assert!(!samples.is_empty(), "no samples");
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    sorted[sorted.len() / 2]
}

/// The line [`record`] prints for a measurement of `key` on `os`:
/// `BENCH <key> <os> <microseconds> <readable> (target <readable>; runs
/// [..])`, the median of `samples` first in microseconds and then as a
/// person reads it, then the target and every run's sample.
///
/// `.github/scripts/bench-summary.py` reads the key, the operating
/// system, the microseconds and the target from it, the target as it
/// stands in the brackets; a unit test here runs the script on lines this
/// function wrote, so the two cannot drift apart unseen.
///
/// # Panics
///
/// On no samples.
#[must_use]
pub fn line(key: &str, os: &str, target: Duration, samples: &[Duration]) -> String {
    let measured = median(samples);
    format!(
        "BENCH {key} {os} {} {measured:?} (target {target:?}; runs {samples:?})",
        measured.as_micros()
    )
}

/// Prints the measurement of `key`, the median of `samples` (one per run,
/// see [`runs`]), and asserts it meets the target of this operating system.
///
/// The line is [`line()`]'s; the bench workflow collects it from the output
/// of every run. The targets are release numbers: a debug build prints and
/// does not assert, and so does a run with `RICK_BENCH_RECORD` set, which
/// the bench workflow sets: it measures, and its summary marks a
/// benchmark whose median is over the target instead of failing the run.
///
/// # Panics
///
/// On no samples, and when the median is over the target in a release
/// build without `RICK_BENCH_RECORD`.
pub fn record(key: &str, samples: &[Duration]) {
    let os = std::env::consts::OS;
    let target = target(key);
    let measured = median(samples);
    println!("{}", line(key, os, target, samples));
    if !cfg!(debug_assertions) && std::env::var_os("RICK_BENCH_RECORD").is_none() {
        assert!(
            measured <= target,
            "{key}: median {measured:?} of {samples:?} is over the {target:?} target on {os}"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use super::BENCHMARKS;

    /// `"16.6 ms"`, `"30 s"` or `"400 ms"` as a duration, to the microsecond.
    fn parse(cell: &str) -> Option<Duration> {
        let (number, unit) = cell.trim().rsplit_once(' ')?;
        let number: f64 = number.trim().parse().ok()?;
        let micros = match unit.trim() {
            "s" => number * 1_000_000.0,
            "ms" => number * 1_000.0,
            "µs" => number,
            _ => return None,
        };
        // Rounded to the microsecond, so the cast is exact in range.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let micros = micros.round() as u64;
        Some(Duration::from_micros(micros))
    }

    /// The targets `BENCHMARKS.md` prints, keyed by benchmark then OS.
    fn documented() -> BTreeMap<String, BTreeMap<String, Duration>> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../BENCHMARKS.md");
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let mut out: BTreeMap<String, BTreeMap<String, Duration>> = BTreeMap::new();
        let mut key: Option<String> = None;
        for line in text.lines() {
            if let Some(heading) = line.strip_prefix("### ") {
                key = heading.split('`').nth(1).map(ToOwned::to_owned);
                continue;
            }
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            let os = match cells.get(1).copied() {
                Some("Linux") => "linux",
                Some("macOS") => "macos",
                Some("Windows") => "windows",
                _ => continue,
            };
            let Some(key) = key.as_ref() else {
                continue;
            };
            let cell = cells.get(2).copied().unwrap_or_default();
            let target = parse(cell).unwrap_or_else(|| panic!("{key} {os}: bad target {cell:?}"));
            out.entry(key.clone())
                .or_default()
                .insert(os.to_owned(), target);
        }
        out
    }

    /// `BENCHMARKS.md` names exactly the benchmarks and targets the tests
    /// assert.
    #[test]
    fn benchmarks_md_matches_the_targets() {
        let documented = documented();
        let keys: Vec<&str> = BENCHMARKS.iter().map(|b| b.key).collect();
        assert_eq!(
            documented.keys().map(String::as_str).collect::<Vec<_>>(),
            {
                let mut sorted = keys.clone();
                sorted.sort_unstable();
                sorted
            },
            "the tables in BENCHMARKS.md and the keys in bench.rs differ"
        );
        for benchmark in BENCHMARKS {
            let rows = &documented[benchmark.key];
            for (os, target) in [
                ("linux", benchmark.linux),
                ("macos", benchmark.macos),
                ("windows", benchmark.windows),
            ] {
                assert_eq!(
                    rows.get(os).copied(),
                    Some(target),
                    "{} on {os}: BENCHMARKS.md says {:?}, bench.rs says {target:?}",
                    benchmark.key,
                    rows.get(os)
                );
            }
        }
    }

    /// `bench-summary.py` on lines [`super::line()`] wrote, three runs of
    /// four benchmarks: every target is shown as `BENCHMARKS.md` writes it,
    /// a median over its target is marked and counted in the line above
    /// the table, and a worst run alone over it is said and not counted
    /// (issue 277).
    #[test]
    fn the_summary_marks_a_median_over_its_target() {
        let ms = Duration::from_millis;
        let us = Duration::from_micros;
        let runs = [
            [
                ("fixture", ms(1_900)),
                ("listing-complete", ms(450)),
                ("pdf-first", ms(285)),
                ("details-frame", us(3_100)),
            ],
            [
                ("fixture", ms(2_000)),
                ("listing-complete", ms(390)),
                ("pdf-first", ms(11)),
                ("details-frame", us(3_200)),
            ],
            [
                ("fixture", ms(2_100)),
                ("listing-complete", ms(460)),
                ("pdf-first", ms(12)),
                ("details-frame", us(3_300)),
            ],
        ];
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        for (i, run) in runs.iter().enumerate() {
            let mut log = String::new();
            for (key, sample) in run {
                // As the harness prints it for a test with no other output.
                log.push_str("test some::test ... ");
                log.push_str(&super::line(
                    key,
                    "linux",
                    super::target_on(key, "linux"),
                    &[*sample],
                ));
                log.push('\n');
            }
            std::fs::write(dir.path().join(format!("bench-{}.log", i + 1)), log)
                .unwrap_or_else(|e| panic!("{e}"));
        }

        let script = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../.github/scripts/bench-summary.py"
        );
        let output = std::process::Command::new("python3")
            .arg(script)
            .arg(dir.path().join("bench-*.log"))
            .output();
        let output = match output {
            Ok(output) => output,
            Err(error) => {
                eprintln!("python3 is not available ({error}); the check is not run here");
                return;
            }
        };
        assert!(
            output.status.success(),
            "bench-summary failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = text.lines().collect();
        for expected in [
            "### linux",
            "**1 of 4 benchmarks is over its target:** `listing-complete`. Not counted, the \
             median within the target and the worst run over it: `pdf-first`.",
            "| Benchmark | Runs | Target | Median | Worst | Against the target | All runs (ms) |",
            "| details-frame | 3 | 16.6 ms | 3.2 ms | 3.3 ms | within | 3.1, 3.2, 3.3 |",
            "| fixture | 3 | 30 s | 2000 ms | 2100 ms | within | 1900.0, 2000.0, 2100.0 |",
            "| listing-complete | 3 | 400 ms | 450 ms | 460 ms | **over** | 450.0, 390.0, 460.0 |",
            "| pdf-first | 3 | 200 ms | 12.0 ms | 285 ms | within; worst run over | 285.0, 11.0, 12.0 |",
        ] {
            assert!(
                lines.contains(&expected),
                "no line {expected:?} in:\n{text}"
            );
        }
    }

    #[test]
    fn the_median_is_the_middle_or_upper_middle_sample() {
        let ms = |n| Duration::from_millis(n);
        assert_eq!(super::median(&[ms(3), ms(1), ms(2)]), ms(2));
        assert_eq!(super::median(&[ms(1), ms(9)]), ms(9));
        assert_eq!(super::median(&[ms(7)]), ms(7));
    }

    #[test]
    fn durations_parse() {
        assert_eq!(parse("16.6 ms"), Some(Duration::from_micros(16_600)));
        assert_eq!(parse("30 s"), Some(Duration::from_secs(30)));
        assert_eq!(parse(" 400 ms "), Some(Duration::from_millis(400)));
        assert_eq!(parse("—"), None);
    }
}
