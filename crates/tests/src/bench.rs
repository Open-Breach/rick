//! The acceptance benchmarks: their targets per operating system and the
//! line each timing test prints for `BENCHMARKS.md`.
//!
//! Every timing test ends in [`record`], which prints one `BENCH` line that
//! the bench workflow collects and asserts the measurement against the
//! target of the operating system it runs on. The Linux targets are the
//! spec's (Decisions table, QUESTIONS.md 5, 24 and 30); the macOS and
//! Windows targets were set from five measured runs on the CI runners, as
//! `BENCHMARKS.md` records. Setting `RICK_BENCH_RECORD` turns the assertion
//! off, for measuring before a target exists.

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
        macos: ms(40_000),
        windows: ms(30_000),
    },
    Benchmark {
        key: "listing-first",
        linux: ms(50),
        macos: ms(400),
        windows: ms(50),
    },
    Benchmark {
        key: "listing-complete",
        linux: ms(400),
        macos: ms(3_000),
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
        macos: ms(1_200),
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
        linux: ms(20),
        macos: ms(150),
        windows: ms(40),
    },
    Benchmark {
        key: "tree-first",
        linux: ms(50),
        macos: ms(80),
        windows: ms(50),
    },
    Benchmark {
        key: "search-first",
        linux: ms(100),
        macos: ms(100),
        windows: ms(150),
    },
    Benchmark {
        key: "recent",
        linux: ms(500),
        macos: ms(500),
        windows: ms(500),
    },
    Benchmark {
        key: "cancel",
        linux: ms(20),
        macos: ms(50),
        windows: ms(20),
    },
    Benchmark {
        key: "session-start",
        linux: ms(5),
        macos: ms(15),
        windows: ms(5),
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

/// Prints the measurement of `key` and asserts it meets the target of this
/// operating system.
///
/// The line is `BENCH <key> <os> <microseconds> <readable>`; the bench
/// workflow collects the first four fields. The targets are release
/// numbers: a debug build prints and does not assert, and so does a run
/// with `RICK_BENCH_RECORD` set, which the bench workflow uses to measure
/// before a target exists.
///
/// # Panics
///
/// When the measurement is over the target in a release build without
/// `RICK_BENCH_RECORD`.
pub fn record(key: &str, measured: Duration) {
    let os = std::env::consts::OS;
    let target = target(key);
    println!(
        "BENCH {key} {os} {} {measured:?} (target {target:?})",
        measured.as_micros()
    );
    if !cfg!(debug_assertions) && std::env::var_os("RICK_BENCH_RECORD").is_none() {
        assert!(
            measured <= target,
            "{key}: {measured:?} is over the {target:?} target on {os}"
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

    #[test]
    fn durations_parse() {
        assert_eq!(parse("16.6 ms"), Some(Duration::from_micros(16_600)));
        assert_eq!(parse("30 s"), Some(Duration::from_secs(30)));
        assert_eq!(parse(" 400 ms "), Some(Duration::from_millis(400)));
        assert_eq!(parse("—"), None);
    }
}
