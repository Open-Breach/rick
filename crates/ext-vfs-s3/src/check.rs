//! The connection test: what the dialogue's *Test* button and *Add* and
//! *Save* ask before a connection is saved (specs/s3-connection.md Part 3).
//!
//! Four checks, in order, on the values of the form:
//!
//! 1. The host is reachable: `ListBuckets` (`GET /`) got an answer.
//! 2. The credentials are accepted: that answer is not a refusal of the
//!    key or the signature. `AccessDenied` counts as accepted: the service
//!    knew the key and said what it may not do.
//! 3. The key may list buckets: that answer is the list.
//! 4. Each name in *Buckets* can be opened: `ListObjectsV2` with one key
//!    (`GET /<bucket>?list-type=2&max-keys=1`), per name, whether or not
//!    the key may list buckets.
//!
//! The test only reads: both requests are `GET`s that list, and nothing is
//! created, changed or deleted. Each request gives up after
//! [`ext::CHECK_TIMEOUT`]. The secret key lives in the client of the run
//! and is dropped with it; in *Edit…* a secret left empty is read from the
//! keychain and never written.

use std::future::Future;
use std::time::Duration;

use ext::{CheckOutcome, ConnectionCheck, ConnectionTest, ConnectionTestForm, t};
use types::{CancelToken, Error};

use crate::client::{S3Client, S3Error};
use crate::connection::{endpoint_and_buckets, field};

/// What the test checks, in order, as the dialogue lists them while it
/// runs.
pub(crate) fn labels() -> Vec<String> {
    vec![
        t!("s3-test-reachable"),
        t!("s3-test-credentials"),
        t!("s3-test-list-buckets"),
        t!("s3-test-buckets"),
    ]
}

fn check(label: String, outcome: CheckOutcome, detail: String) -> ConnectionCheck {
    ConnectionCheck {
        label,
        outcome,
        detail,
    }
}

/// What an error says, without the kind of error before it.
fn said(error: &Error) -> String {
    match error {
        Error::Extension { source, .. } => source.to_string(),
        Error::Io(inner) => inner.to_string(),
        other => other.to_string(),
    }
}

/// The service's own error inside `error`, when it answered with one.
fn service(error: &Error) -> Option<&S3Error> {
    match error {
        Error::Extension { source, .. } => source.downcast_ref(),
        _ => None,
    }
}

/// How one request of the test ended.
enum Asked<T> {
    /// The service answered.
    Answered(Result<T, Error>),
    /// No answer in time.
    TimedOut,
    /// The test was cancelled.
    Cancelled,
}

/// Runs one request under the check's time limit, ending early when the
/// test is cancelled.
async fn ask<T>(
    request: impl Future<Output = Result<T, Error>>,
    limit: Duration,
    cancel: &CancelToken,
) -> Asked<T> {
    let timed = std::pin::pin!(tokio::time::timeout(limit, request));
    match futures::future::select(cancel.cancelled(), timed).await {
        futures::future::Either::Left(_) => Asked::Cancelled,
        futures::future::Either::Right((Ok(answer), _)) => Asked::Answered(answer),
        futures::future::Either::Right((Err(_), _)) => Asked::TimedOut,
    }
}

/// Check 4: whether each of `names` can be opened, by listing one key of
/// it. Returns the checks, whether every bucket opened, and whether the
/// test was cancelled on the way; the names after a cancel are not run.
async fn open_buckets(
    client: &S3Client,
    names: &[String],
    limit: Duration,
    cancel: &CancelToken,
) -> (Vec<ConnectionCheck>, bool, bool) {
    let mut checks = Vec::with_capacity(names.len());
    let mut all_open = true;
    let mut cancelled = false;
    for name in names {
        let label = t!("s3-test-bucket", name = name.as_str());
        if cancelled {
            checks.push(check(label, CheckOutcome::NotRun, String::new()));
            continue;
        }
        let (outcome, detail) =
            match ask(client.list(name, "", None, Some("1")), limit, cancel).await {
                Asked::Answered(Ok(_)) => (CheckOutcome::Passed, String::new()),
                Asked::Answered(Err(error)) => (CheckOutcome::Failed, said(&error)),
                Asked::TimedOut => (CheckOutcome::Failed, timed_out(limit)),
                Asked::Cancelled => {
                    cancelled = true;
                    (CheckOutcome::NotRun, String::new())
                }
            };
        all_open &= outcome == CheckOutcome::Passed;
        checks.push(check(label, outcome, detail));
    }
    (checks, all_open, cancelled)
}

/// What a check that got no answer in time says.
fn timed_out(limit: Duration) -> String {
    t!(
        "s3-test-timeout",
        seconds = i64::try_from(limit.as_secs()).unwrap_or(0)
    )
}

/// The test on the values of `form`, with `secret` the secret key typed
/// or, in *Edit…*, the keychain's. `limit` is how long one check may take.
pub(crate) async fn run(
    form: &ConnectionTestForm<'_>,
    secret: Result<String, Error>,
    limit: Duration,
    cancel: &CancelToken,
) -> ConnectionTest {
    let (endpoint, names) =
        endpoint_and_buckets(form.get(field::ENDPOINT), form.get(field::BUCKETS));
    let [reachable, credentials, list_buckets, _] = <[String; 4]>::try_from(labels())
        .unwrap_or_else(|_| std::array::from_fn(|_| String::new()));
    let bucket_label = |name: &str| t!("s3-test-bucket", name = name);
    let not_run = |label: String| check(label, CheckOutcome::NotRun, String::new());
    let passed = |label: String| check(label, CheckOutcome::Passed, String::new());
    let failed = |label: String, detail: String| check(label, CheckOutcome::Failed, detail);
    // What is left when an early check ends the test.
    let rest = |from: usize| -> Vec<ConnectionCheck> {
        [credentials.clone(), list_buckets.clone()]
            .into_iter()
            .skip(from)
            .chain(names.iter().map(|name| bucket_label(name)))
            .map(not_run)
            .collect()
    };
    let ended = |mut checks: Vec<ConnectionCheck>, from: usize| {
        checks.extend(rest(from));
        ConnectionTest {
            checks,
            good: false,
        }
    };

    let client = secret.and_then(|secret| {
        S3Client::new(
            &endpoint,
            form.get(field::REGION),
            form.get(field::ACCESS_KEY),
            &secret,
            form.get(field::PATH_STYLE) != "false",
        )
    });
    let client = match client {
        Ok(client) => client,
        Err(error) => return ended(vec![failed(reachable, said(&error))], 0),
    };

    // Checks 1 to 3: one `ListBuckets`.
    let listed = match ask(client.list_buckets(), limit, cancel).await {
        Asked::Cancelled => return ended(vec![not_run(reachable)], 0),
        Asked::TimedOut => return ended(vec![failed(reachable, timed_out(limit))], 0),
        Asked::Answered(Err(error)) if error.is_unreachable() => {
            return ended(vec![failed(reachable, said(&error))], 0);
        }
        Asked::Answered(answer) => answer,
    };
    let mut checks = vec![passed(reachable)];
    let may_list = match &listed {
        Ok(_) => true,
        Err(error) if service(error).is_some_and(|said| said.code == "AccessDenied") => false,
        Err(error) => {
            checks.push(failed(credentials.clone(), said(error)));
            return ended(checks, 1);
        }
    };
    checks.push(passed(credentials.clone()));
    checks.push(match &listed {
        Ok(_) => passed(list_buckets.clone()),
        // With buckets named, a key that may not list is the expected
        // case: information, not a failure (3.3).
        Err(error) if !names.is_empty() => {
            let code = service(error).map_or("", |said| said.code.as_str());
            check(
                list_buckets.clone(),
                CheckOutcome::Info,
                t!("s3-test-list-denied-info", code = code),
            )
        }
        Err(error) => failed(list_buckets.clone(), said(error)),
    });

    // Check 4: each name, whether or not the key may list buckets.
    let (opened, all_open, cancelled) = open_buckets(&client, &names, limit, cancel).await;
    checks.extend(opened);
    let good = !cancelled && if names.is_empty() { may_list } else { all_open };
    ConnectionTest { checks, good }
}
