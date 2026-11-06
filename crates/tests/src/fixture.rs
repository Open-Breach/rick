//! Deterministic fixture tree generator.
//!
//! [`generate`] writes a tree of N entries (directories and files) into a
//! fresh temporary directory. The plan is derived from the seed alone with a
//! small built-in PRNG, so the same seed produces the same names, layout,
//! sizes and file contents on every platform and every run; only the writing
//! is parallel. Nothing is ever committed: the tree lives as long as the
//! returned [`Fixture`].
//!
//! Creating and removing a file is latency-bound in the kernel and its filter
//! drivers, not CPU-bound, so the file phases run on the runtime's blocking
//! pool with [`WORKERS`] workers rather than on the compute pool. Generating or
//! dropping a fixture from inside the runtime is not supported: [`Config::generate_in`]
//! panics there, and [`Fixture`]'s drop falls back to the serial removal of
//! `TempDir`.
//!
//! ```no_run
//! let fixture = tests::fixture::generate(1_000, 42).unwrap();
//! assert!(fixture.root().join("d00001").is_dir() || fixture.files() > 0);
//! ```

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use tempfile::TempDir;

/// Number of entries [`generate`] produces unless told otherwise.
pub const DEFAULT_ENTRIES: usize = 200_000;

/// Blocking-pool workers that create or remove files concurrently. More than
/// the core count on purpose: each call mostly waits in the kernel.
pub const WORKERS: usize = 32;

/// How file sizes are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeDistribution {
    /// Every file is empty.
    Empty,
    /// Every file has exactly this many bytes.
    Fixed(u64),
    /// Uniform over `min..=max`.
    Uniform {
        /// Smallest size.
        min: u64,
        /// Largest size.
        max: u64,
    },
    /// Log-uniform over `min..=max`: each power of two is equally likely, so
    /// small files dominate the way they do on real disks.
    LogUniform {
        /// Smallest size.
        min: u64,
        /// Largest size.
        max: u64,
    },
}

impl SizeDistribution {
    fn draw(self, rng: &mut SplitMix64) -> u64 {
        match self {
            Self::Empty => 0,
            Self::Fixed(size) => size,
            Self::Uniform { min, max } => rng.below(max.saturating_sub(min) + 1) + min.min(max),
            Self::LogUniform { min, max } => {
                let (min, max) = (min.min(max), min.max(max));
                // Pick a bit length between those of min+1 and max+1, then a
                // value with that bit length, clamped into range. Integer
                // only, so every platform draws the same sizes.
                let bit_width = |value: u64| value.checked_ilog2().map_or(0, |log| log + 1);
                let low_bits = bit_width(min.saturating_add(1));
                let high_bits = bit_width(max.saturating_add(1));
                let bits = low_bits
                    + u32::try_from(rng.below(u64::from(high_bits - low_bits) + 1)).unwrap_or(0);
                let floor = 1u64 << (bits - 1);
                let value = floor + rng.below(floor);
                (value - 1).clamp(min, max)
            }
        }
    }
}

/// Shape of the generated tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Total number of entries, directories and files together, below the
    /// root.
    pub entries: usize,
    /// How deep directories may nest below the root; `0` puts every entry
    /// directly in the root, which is the layout the listing performance
    /// target is measured on.
    pub max_depth: u32,
    /// One directory is created for every this many entries when
    /// `max_depth > 0`.
    pub entries_per_dir: usize,
    /// File sizes.
    pub sizes: SizeDistribution,
    /// Seed for every random choice.
    pub seed: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            entries: DEFAULT_ENTRIES,
            max_depth: 3,
            entries_per_dir: 64,
            sizes: SizeDistribution::LogUniform { min: 0, max: 4096 },
            seed: 0,
        }
    }
}

impl Config {
    /// `entries` entries drawn from `seed`, everything else as
    /// [`Config::default`].
    #[must_use]
    pub fn new(entries: usize, seed: u64) -> Self {
        Self {
            entries,
            seed,
            ..Self::default()
        }
    }

    /// Writes the tree into a fresh temporary directory.
    ///
    /// # Errors
    ///
    /// Fails when the temporary directory or any entry cannot be created.
    pub fn generate(&self) -> Result<Fixture, FixtureError> {
        let dir = tempfile::Builder::new()
            .prefix("rick-fixture-")
            .tempdir()
            .map_err(FixtureError::TempDir)?;
        let stats = self.generate_in(dir.path())?;
        Ok(Fixture {
            dir,
            config: self.clone(),
            stats,
        })
    }

    /// Writes the tree below `root`, which must exist and should be empty.
    ///
    /// # Errors
    ///
    /// Fails when an entry cannot be created; entries written so far are
    /// left in place.
    ///
    /// # Panics
    ///
    /// Panics when called from inside the runtime (from a job or another
    /// `block_on`): the file phase blocks on the runtime to drive its workers.
    pub fn generate_in(&self, root: &Path) -> Result<Stats, FixtureError> {
        let span =
            tracing::info_span!("fixture.generate", entries = self.entries, seed = self.seed);
        let _enter = span.enter();
        let plan = Plan::new(self);

        for dir in &plan.dirs {
            let path = root.join(dir);
            fs::create_dir(&path).map_err(|source| FixtureError::Create { path, source })?;
        }

        let stats = Stats {
            dirs: plan.dirs.len(),
            files: plan.files.len(),
            bytes: plan.files.iter().map(|f| f.size).sum(),
        };
        for_each_file(plan.files, root, |path, file| {
            write_file(path, file.size, file.seed)
        })?;
        tracing::info!(
            dirs = stats.dirs,
            files = stats.files,
            bytes = stats.bytes,
            "fixture ready"
        );
        Ok(stats)
    }
}

/// Generates `entries` entries from `seed` with the default shape into a
/// temporary directory. Shorthand for `Config::new(entries, seed).generate()`.
///
/// # Errors
///
/// Fails when the temporary directory or any entry cannot be created.
pub fn generate(entries: usize, seed: u64) -> Result<Fixture, FixtureError> {
    Config::new(entries, seed).generate()
}

/// What was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    /// Directories created below the root.
    pub dirs: usize,
    /// Files created.
    pub files: usize,
    /// Bytes written across all files.
    pub bytes: u64,
}

/// A generated tree. Deleted when dropped.
#[derive(Debug)]
pub struct Fixture {
    dir: TempDir,
    config: Config,
    stats: Stats,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // `TempDir` removes the tree serially, which takes minutes for 200 000
        // files on Windows. Remove the files concurrently first, from the same
        // plan that created them, and leave the directories to `TempDir`.
        // Inside the runtime the blocking pool cannot be driven from here, so
        // the serial path is the only one.
        if tokio::runtime::Handle::try_current().is_ok() {
            return;
        }
        let files = Plan::new(&self.config).files;
        let removed = for_each_file(files, self.dir.path(), |path, _| {
            match fs::remove_file(path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            }
        });
        if let Err(error) = removed {
            tracing::warn!(%error, "parallel fixture removal failed; TempDir finishes it");
        }
    }
}

impl Fixture {
    /// The directory the tree was written into.
    #[must_use]
    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    /// The configuration the tree was generated from.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Entries below the root, directories and files together.
    #[must_use]
    pub fn entries(&self) -> usize {
        self.stats.dirs + self.stats.files
    }

    /// Directories below the root.
    #[must_use]
    pub fn dirs(&self) -> usize {
        self.stats.dirs
    }

    /// Files in the tree.
    #[must_use]
    pub fn files(&self) -> usize {
        self.stats.files
    }

    /// Bytes written across all files.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.stats.bytes
    }
}

/// Why a fixture could not be generated.
#[derive(Debug, thiserror::Error)]
pub enum FixtureError {
    /// The temporary directory could not be created.
    #[error("cannot create temporary directory: {0}")]
    TempDir(#[source] io::Error),
    /// An entry could not be created or removed.
    #[error("cannot create or remove {path}: {source}")]
    Create {
        /// The entry that failed.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: io::Error,
    },
    /// A worker on the blocking pool ended without reporting a result.
    #[error("a fixture worker stopped unexpectedly: {0}")]
    Worker(#[source] io::Error),
}

const EXTENSIONS: [&str; 12] = [
    "txt", "rs", "md", "png", "jpg", "pdf", "toml", "json", "bin", "log", "csv", "",
];

struct PlannedFile {
    path: PathBuf,
    size: u64,
    seed: u64,
}

/// Everything about the tree, decided before anything touches the disk.
struct Plan {
    /// Relative directory paths, parents before children.
    dirs: Vec<PathBuf>,
    files: Vec<PlannedFile>,
}

impl Plan {
    fn new(config: &Config) -> Self {
        let mut rng = SplitMix64::new(config.seed);
        // Index 0 is the root itself; `depth[i]` is the nesting of `parents[i]`.
        let mut parents: Vec<PathBuf> = vec![PathBuf::new()];
        let mut depth: Vec<u32> = vec![0];
        let mut open: Vec<usize> = vec![0]; // directories that may still get children
        let mut dirs = Vec::new();
        let mut files = Vec::with_capacity(config.entries);
        let entries_per_dir = config.entries_per_dir.max(1);

        for index in 0..config.entries {
            let make_dir = config.max_depth > 0 && index % entries_per_dir == entries_per_dir - 1;
            if make_dir {
                let parent = open[rng.index(open.len())];
                let path = parents[parent].join(format!("d{:05}", dirs.len() + 1));
                let nesting = depth[parent] + 1;
                dirs.push(path.clone());
                parents.push(path);
                depth.push(nesting);
                if nesting < config.max_depth {
                    open.push(parents.len() - 1);
                }
            } else {
                let parent = &parents[rng.index(parents.len())];
                let ext = EXTENSIONS[rng.index(EXTENSIONS.len())];
                let name = if ext.is_empty() {
                    format!("f{:06}", files.len() + 1)
                } else {
                    format!("f{:06}.{ext}", files.len() + 1)
                };
                files.push(PlannedFile {
                    path: parent.join(name),
                    size: config.sizes.draw(&mut rng),
                    seed: rng.next(),
                });
            }
        }
        Self { dirs, files }
    }
}

/// Applies `work` to every planned file below `root` on the runtime's
/// blocking pool with [`WORKERS`] workers. Files are claimed one at a time
/// from a shared counter, so the order of execution varies but the result
/// does not. Stops handing out files after the first error and returns it.
fn for_each_file<F>(files: Vec<PlannedFile>, root: &Path, work: F) -> Result<(), FixtureError>
where
    F: Fn(&Path, &PlannedFile) -> io::Result<()> + Send + Sync + 'static,
{
    let files = Arc::new(files);
    let root: Arc<Path> = Arc::from(root);
    let work = Arc::new(work);
    let next = Arc::new(AtomicUsize::new(0));
    let failed = Arc::new(AtomicBool::new(false));
    let handle = runtime::handle();

    runtime::block_on(async {
        let workers: Vec<_> = (0..WORKERS)
            .map(|_| {
                let (files, root, work) =
                    (Arc::clone(&files), Arc::clone(&root), Arc::clone(&work));
                let (next, failed) = (Arc::clone(&next), Arc::clone(&failed));
                handle.spawn_blocking(move || {
                    while !failed.load(Ordering::Relaxed) {
                        let Some(file) = files.get(next.fetch_add(1, Ordering::Relaxed)) else {
                            break;
                        };
                        let path = root.join(&file.path);
                        if let Err(source) = work(&path, file) {
                            failed.store(true, Ordering::Relaxed);
                            return Err(FixtureError::Create { path, source });
                        }
                    }
                    Ok(())
                })
            })
            .collect();

        let mut outcome = Ok(());
        for worker in workers {
            let result = match worker.await {
                Ok(result) => result,
                Err(join) => Err(FixtureError::Worker(io::Error::other(join))),
            };
            if outcome.is_ok() {
                outcome = result;
            }
        }
        outcome
    })
}

fn write_file(path: &Path, size: u64, seed: u64) -> io::Result<()> {
    let mut file = fs::File::create(path)?;
    if size == 0 {
        return Ok(());
    }
    let mut rng = SplitMix64::new(seed);
    let mut buffer = [0u8; 8192];
    let mut remaining = size;
    while remaining > 0 {
        let chunk = usize::try_from(remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
        for word in buffer[..chunk].chunks_mut(8) {
            let bytes = rng.next().to_le_bytes();
            word.copy_from_slice(&bytes[..word.len()]);
        }
        file.write_all(&buffer[..chunk])?;
        remaining -= chunk as u64;
    }
    Ok(())
}

/// `SplitMix64`: tiny, fast and identical everywhere, which an external `rand`
/// cannot promise across versions.
struct SplitMix64(u64);

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..bound`; `bound == 0` yields `0`.
    fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            return 0;
        }
        // Lemire's multiply-shift; the slight bias for huge bounds is
        // irrelevant for a fixture.
        let product = u128::from(self.next()) * u128::from(bound);
        u64::try_from(product >> 64).unwrap_or(0)
    }

    fn index(&mut self, len: usize) -> usize {
        usize::try_from(self.below(len as u64)).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix_matches_the_reference_sequence() {
        // First outputs of SplitMix64 for seed 0, as in the reference Java
        // implementation.
        let mut rng = SplitMix64::new(0);
        assert_eq!(rng.next(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(rng.next(), 0x6E78_9E6A_A1B9_65F4);
    }

    #[test]
    fn plan_has_exactly_the_requested_entries_and_respects_depth() {
        let config = Config {
            entries: 1_000,
            max_depth: 2,
            entries_per_dir: 10,
            ..Config::default()
        };
        let plan = Plan::new(&config);
        assert_eq!(plan.dirs.len() + plan.files.len(), 1_000);
        assert_eq!(plan.dirs.len(), 100);
        assert!(plan.dirs.iter().all(|d| d.components().count() <= 2));
        assert!(plan.dirs.iter().any(|d| d.components().count() == 2));
        assert!(plan.files.iter().all(|f| f.path.components().count() <= 3));
    }

    #[test]
    fn flat_plan_puts_everything_in_the_root() {
        let plan = Plan::new(&Config {
            entries: 500,
            max_depth: 0,
            ..Config::default()
        });
        assert_eq!(plan.dirs, [] as [PathBuf; 0]);
        assert_eq!(plan.files.len(), 500);
        assert!(plan.files.iter().all(|f| f.path.components().count() == 1));
    }

    #[test]
    fn size_distributions_stay_in_range() {
        let mut rng = SplitMix64::new(3);
        for _ in 0..10_000 {
            assert_eq!(SizeDistribution::Empty.draw(&mut rng), 0);
            assert_eq!(SizeDistribution::Fixed(17).draw(&mut rng), 17);
            let u = SizeDistribution::Uniform { min: 5, max: 9 }.draw(&mut rng);
            assert!((5..=9).contains(&u), "{u}");
            let l = SizeDistribution::LogUniform { min: 0, max: 4096 }.draw(&mut rng);
            assert!(l <= 4096, "{l}");
            let l = SizeDistribution::LogUniform { min: 100, max: 100 }.draw(&mut rng);
            assert_eq!(l, 100);
        }
        let mut rng = SplitMix64::new(4);
        let draws: Vec<u64> = (0..10_000)
            .map(|_| {
                SizeDistribution::LogUniform {
                    min: 0,
                    max: 1 << 20,
                }
                .draw(&mut rng)
            })
            .collect();
        let small = draws.iter().filter(|&&s| s < 1024).count();
        let large = draws.iter().filter(|&&s| s >= 1 << 19).count();
        assert!(
            small > large,
            "log-uniform favours small files: {small} vs {large}"
        );
        assert!(large > 0);
    }

    #[test]
    fn plan_is_a_pure_function_of_the_config() {
        let config = Config::new(300, 99);
        let a = Plan::new(&config);
        let b = Plan::new(&config);
        assert_eq!(a.dirs, b.dirs);
        assert!(
            a.files
                .iter()
                .zip(&b.files)
                .all(|(x, y)| x.path == y.path && x.size == y.size && x.seed == y.seed)
        );
        let c = Plan::new(&Config::new(300, 100));
        assert!(a.files.iter().zip(&c.files).any(|(x, y)| x.size != y.size));
    }
}
