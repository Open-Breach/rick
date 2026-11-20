//! The release's check of its build attestations
//! (specs/system-integration.md §9): `.github/scripts/verify-attestations.py`,
//! the script of the release workflow's `verify` job, run here against a
//! stand-in for `gh`.
//!
//! The job decides whether a release is published, so what it must not do
//! is take a failure for an answer. The stand-in plays what the real `gh`
//! prints in each case, word for word where the script matches on words:
//! a file that verifies, a file GitHub has no attestation for, an
//! attestation that does not verify, and the failures that are no answer
//! at all: no connection, a rate limit, a server error, a refused token.
//!
//! The script is Python, as the other release scripts are; where there is
//! no `python3` the checks are not run, as in `about.rs`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The stand-in for `gh`. `plan.json` beside it says, for the download and
/// for each file, what to answer on the first run, the second and so on;
/// the last answer is repeated. Every run is written to `calls.log`.
const STAND_IN: &str = r#"
import hashlib, json, os, sys

here = os.path.dirname(os.path.abspath(__file__))
plan = json.load(open(os.path.join(here, "plan.json")))
arguments = sys.argv[1:]
with open(os.path.join(here, "calls.log"), "a") as log:
    log.write(" ".join(arguments) + "\n")

def answer(key):
    answers = plan[key]
    counter = os.path.join(here, key + ".count")
    done = int(open(counter).read()) if os.path.exists(counter) else 0
    open(counter, "w").write(str(done + 1))
    return answers[min(done, len(answers) - 1)]

def say(text, status):
    sys.stderr.write(text + "\n")
    sys.exit(status)

LOOKUP = "https://api.github.com/orgs/Open-Breach/attestations/sha256:{digest}?per_page=30&predicate_type=https%3A%2F%2Fslsa.dev%2Fprovenance%2Fv1"

def fail(how, digest):
    lookup = LOOKUP.format(digest=digest)
    if how == "none":
        say("\nError: HTTP 404: Not Found (" + lookup + ")", 1)
    if how == "none-in-words":
        say("Error: no attestations found", 1)
    if how == "wrong-signer":
        say('\nError: verifying with issuer "sigstore.dev"', 1)
    if how == "wrong-repository":
        say("\nError: expected SourceRepositoryURI to be https://github.com/Open-Breach/rick, got https://github.com/Open-Breach/other", 1)
    if how == "no-verifier":
        say("error creating Sigstore verifier: no valid Sigstore verifiers could be initialized", 1)
    if how == "no-token":
        say("gh: To use GitHub CLI in a GitHub Actions workflow, set the GH_TOKEN environment variable. Example:", 4)
    if how == "network":
        say('Error: Get "' + lookup + '": dial tcp: lookup api.github.com: no such host', 1)
    if how == "server":
        say("Error: HTTP 502: Bad Gateway (" + lookup + ")", 1)
    if how == "rate-limit":
        say("Error: HTTP 403: API rate limit exceeded for installation. (" + lookup + ")", 1)
    if how == "token":
        say("Error: HTTP 401: Bad credentials (" + lookup + ")", 1)
    if how == "other-404":
        say("Error: HTTP 404: Not Found (https://api.github.com/orgs/Open-Breach)", 1)
    if how == "unknown-words":
        say("Error: something this script has never seen", 1)
    raise SystemExit("the plan names an answer the stand-in does not know: " + how)

if arguments[:2] == ["release", "download"]:
    how = answer("download")
    if how != "ok":
        say("Error: " + how, 1)
    for name, text in plan["files"].items():
        open(name, "w").write(text)
    sys.exit(0)

if arguments[:2] == ["attestation", "verify"]:
    name = arguments[2]
    digest = hashlib.sha256(open(name, "rb").read()).hexdigest()
    how = answer(name)
    if how == "ok":
        sys.exit(0)
    fail(how, digest)

raise SystemExit("the stand-in was asked something else: " + " ".join(arguments))
"#;

const PACKAGE: &str = "rick_9.9.9_debian-12_amd64.deb";
const MACOS: &str = "rick-9.9.9-macos-arm64.zip";
const WINDOWS: &str = "rick-9.9.9-windows-x64.zip";
const CHANGED: &str = "changed.deb";

/// What one run of the script left: its exit status, everything it
/// printed, the job summary it wrote, and the runs of `gh` it made.
struct Outcome {
    status: i32,
    printed: String,
    summary: String,
    calls: Vec<String>,
}

impl Outcome {
    /// How often `gh attestation verify` was run on this file.
    fn tries(&self, file: &str) -> usize {
        let wanted = format!("attestation verify {file} ");
        self.calls.iter().filter(|c| c.starts_with(&wanted)).count()
    }
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|e| panic!("{e}"))
}

/// Runs the script on a release whose three files and whose changed copy
/// get the answers given: one list per file, first run first. `None` where
/// there is no `python3`.
fn check(answers: &[(&str, &[&str])]) -> Option<Outcome> {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let stand_in = dir.path().join("gh.py");
    fs::write(&stand_in, STAND_IN).unwrap_or_else(|e| panic!("{e}"));
    let mut plan = serde_json::json!({
        "download": ["ok"],
        "files": { PACKAGE: "a package\n", MACOS: "the macOS archive\n", WINDOWS: "the Windows archive\n" },
        PACKAGE: ["ok"],
        MACOS: ["ok"],
        WINDOWS: ["ok"],
        CHANGED: ["none"],
    });
    for (key, list) in answers {
        plan[*key] = serde_json::json!(list);
    }
    fs::write(dir.path().join("plan.json"), plan.to_string()).unwrap_or_else(|e| panic!("{e}"));
    let summary = dir.path().join("summary.md");

    let output = Command::new("python3")
        .arg(repo().join(".github/scripts/verify-attestations.py"))
        .args([
            "v9.9.9",
            "--repo",
            "Open-Breach/rick",
            "--linux-signer",
            "Open-Breach/trebuchet",
        ])
        .args(["--gh", "python3"])
        .arg(&stand_in)
        .env("RICK_VERIFY_DELAYS", "0,0,0")
        .env("GITHUB_STEP_SUMMARY", &summary)
        .output();
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            eprintln!("python3 is not available ({error}); the check is not run here");
            return None;
        }
    };
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .replace("\r\n", "\n");
    let calls = fs::read_to_string(dir.path().join("calls.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect();
    Some(Outcome {
        status: output.status.code().unwrap_or(-1),
        printed,
        summary: fs::read_to_string(&summary).unwrap_or_default(),
        calls,
    })
}

/// The release passes: the three files verify with the line that fits
/// each, and the changed package is refused because GitHub has no
/// attestation for its checksum.
#[test]
fn the_three_files_verify_and_the_changed_one_has_no_attestation() {
    let Some(run) = check(&[]) else { return };
    assert_eq!(run.status, 0, "{}", run.printed);
    for file in [PACKAGE, MACOS, WINDOWS] {
        assert!(
            run.printed.contains(&format!("verified: {file}\n")),
            "{}",
            run.printed
        );
        assert_eq!(run.tries(file), 1, "{file} is asked about once");
    }
    assert!(
        run.printed
            .contains("refused, as it must be: no attestation for sha256:")
    );
    assert!(run.printed.ends_with(
        "the package and both archives verify, and the changed package has no attestation\n"
    ));
    // Each file with its documented line: the package and its changed copy
    // against the workflow that built them, the archives against this
    // repository.
    let linux = "--owner Open-Breach --signer-repo Open-Breach/trebuchet";
    let own = "--repo Open-Breach/rick";
    assert!(
        run.calls
            .contains(&format!("attestation verify {PACKAGE} {linux}"))
    );
    assert!(
        run.calls
            .contains(&format!("attestation verify {CHANGED} {linux}"))
    );
    assert!(
        run.calls
            .contains(&format!("attestation verify {MACOS} {own}"))
    );
    assert!(
        run.calls
            .contains(&format!("attestation verify {WINDOWS} {own}"))
    );
    assert!(
        run.summary
            .starts_with("### Verified, the draft of v9.9.9\n"),
        "{}",
        run.summary
    );
}

/// `gh`'s own words for an empty answer count as the refusal too, should
/// it stop passing GitHub's 404 on.
#[test]
fn gh_saying_it_found_no_attestations_is_the_expected_refusal() {
    let Some(run) = check(&[(CHANGED, &["none-in-words"])]) else {
        return;
    };
    assert_eq!(run.status, 0, "{}", run.printed);
}

/// The worst case, loudly: a changed file that verifies stops the release
/// at once, with a verdict, and is not asked about again.
#[test]
fn a_changed_file_that_verifies_fails_loudly() {
    let Some(run) = check(&[(CHANGED, &["ok"])]) else {
        return;
    };
    assert_eq!(run.status, 1, "{}", run.printed);
    assert!(
        run.printed
            .contains("::error::NOT VERIFIED: A CHANGED FILE VERIFIED"),
        "{}",
        run.printed
    );
    assert!(!run.printed.contains("COULD NOT CHECK"), "{}", run.printed);
    assert_eq!(run.tries(CHANGED), 1);
    assert!(
        run.summary.starts_with("### Not verified\n"),
        "{}",
        run.summary
    );
    assert!(
        run.summary.contains("A CHANGED FILE VERIFIED"),
        "{}",
        run.summary
    );
}

/// A failure of `gh` on the changed file is not a refusal. Before, any
/// failure passed for one. Now it is tried again, and if there is still
/// no answer the job fails, saying that the check could not be made.
#[test]
fn a_network_error_on_the_changed_file_is_not_a_refusal() {
    for failure in [
        "network",
        "server",
        "rate-limit",
        "token",
        "no-token",
        "no-verifier",
        "other-404",
        "unknown-words",
    ] {
        let Some(run) = check(&[(CHANGED, &[failure])]) else {
            return;
        };
        assert_eq!(run.status, 2, "{failure}: {}", run.printed);
        assert!(
            run.printed.contains("::error::COULD NOT CHECK"),
            "{failure}: {}",
            run.printed
        );
        assert!(
            !run.printed.contains("refused, as it must be"),
            "{failure}: {}",
            run.printed
        );
        assert!(
            !run.printed.contains("NOT VERIFIED"),
            "{failure}: {}",
            run.printed
        );
        assert_eq!(run.tries(CHANGED), 4, "{failure}: one run and three more");
        assert!(
            run.summary.starts_with("### Could not check\n"),
            "{failure}: {}",
            run.summary
        );
        assert!(
            run.summary.contains("This says nothing about the files"),
            "{failure}: {}",
            run.summary
        );
    }
}

/// A failure that passes is waited out: two runs without an answer, then
/// the refusal, and the release goes on.
#[test]
fn a_failure_that_passes_is_tried_again() {
    let Some(run) = check(&[
        (CHANGED, &["network", "server", "none"]),
        (MACOS, &["rate-limit", "ok"]),
    ]) else {
        return;
    };
    assert_eq!(run.status, 0, "{}", run.printed);
    assert_eq!(run.tries(CHANGED), 3);
    assert_eq!(run.tries(MACOS), 2);
    assert!(
        run.printed
            .contains("no answer about the changed package, try 1 of 4"),
        "{}",
        run.printed
    );
}

/// The same care for a file that should verify: no connection is "could
/// not check", not "does not verify".
#[test]
fn a_network_error_on_a_real_file_is_not_a_verdict() {
    for file in [PACKAGE, MACOS, WINDOWS] {
        let Some(run) = check(&[(file, &["network"])]) else {
            return;
        };
        assert_eq!(run.status, 2, "{file}: {}", run.printed);
        assert!(
            run.printed.contains("::error::COULD NOT CHECK"),
            "{file}: {}",
            run.printed
        );
        assert!(
            !run.printed.contains("NOT VERIFIED"),
            "{file}: {}",
            run.printed
        );
        assert_eq!(run.tries(file), 4);
        assert_eq!(
            run.tries(CHANGED),
            0,
            "the run ends at the file it has no answer about"
        );
    }
}

/// A real file that does not verify is a verdict, at once: GitHub has no
/// attestation for it, or it has one that another workflow signed.
#[test]
fn a_real_file_that_does_not_verify_is_a_verdict() {
    for (answer, words) in [
        ("none", "has no attestation"),
        ("none-in-words", "has no attestation"),
        ("wrong-signer", "has an attestation that does not verify"),
        (
            "wrong-repository",
            "has an attestation that does not verify",
        ),
    ] {
        let Some(run) = check(&[(WINDOWS, &[answer])]) else {
            return;
        };
        assert_eq!(run.status, 1, "{answer}: {}", run.printed);
        assert!(
            run.printed.contains("::error::NOT VERIFIED"),
            "{answer}: {}",
            run.printed
        );
        assert!(run.printed.contains(words), "{answer}: {}", run.printed);
        assert!(
            !run.printed.contains("COULD NOT CHECK"),
            "{answer}: {}",
            run.printed
        );
        assert_eq!(
            run.tries(WINDOWS),
            1,
            "{answer}: a verdict is not asked for twice"
        );
        assert!(
            run.summary.starts_with("### Not verified\n"),
            "{answer}: {}",
            run.summary
        );
    }
}

/// The changed package refused for another reason than a missing
/// attestation is not the proof the check is after.
#[test]
fn a_changed_file_refused_for_another_reason_is_not_the_expected_refusal() {
    let Some(run) = check(&[(CHANGED, &["wrong-signer"])]) else {
        return;
    };
    assert_eq!(run.status, 2, "{}", run.printed);
    assert!(
        run.printed.contains("not because it has no attestation"),
        "{}",
        run.printed
    );
}

/// Files that cannot be downloaded: the check could not be made.
#[test]
fn a_download_that_fails_is_tried_again_and_then_could_not_check() {
    let Some(run) = check(&[("download", &["HTTP 503: Service Unavailable"])]) else {
        return;
    };
    assert_eq!(run.status, 2, "{}", run.printed);
    assert!(
        run.printed
            .contains("::error::COULD NOT CHECK: the files of v9.9.9 could not be downloaded")
    );
    let downloads = run
        .calls
        .iter()
        .filter(|c| c.starts_with("release download "))
        .count();
    assert_eq!(downloads, 4);

    let Some(run) = check(&[("download", &["connection reset", "ok"])]) else {
        return;
    };
    assert_eq!(run.status, 0, "{}", run.printed);
}
