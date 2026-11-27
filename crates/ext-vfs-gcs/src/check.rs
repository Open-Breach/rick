//! The connection test: what the dialogue's *Test* button and *Add* and
//! *Save* ask before a connection is saved (specs/s3-connection.md Part 3).
//!
//! Three checks, in order:
//!
//! 1. The host is reachable: `buckets.list` for a single name
//!    (`GET /storage/v1/b?project=…&maxResults=1`), sent without a token,
//!    got an answer, whatever it says.
//! 2. The credentials are accepted: the service account's key reads, the
//!    token endpoint gives a token for it, and the same request with the
//!    token is not answered `401`. Without a key, which an emulator takes,
//!    it is the answer of the first request that must not be `401`.
//! 3. The project's buckets can be listed: that answer is the list.
//!
//! The test only reads: the two listings are `GET`s, and the token is a
//! sign-in that creates nothing on the service. Each request gives up
//! after [`ext::CHECK_TIMEOUT`]. The key lives in the client of the run
//! and is dropped with it; in *Edit…* a key left empty is read from the
//! keychain and never written.

use std::future::Future;
use std::time::Duration;

use ext::{CheckOutcome, ConnectionCheck, ConnectionTest, ConnectionTestForm, t};
use types::{CancelToken, Error};

use crate::auth::ServiceAccount;
use crate::client::{GcsClient, GcsError};
use crate::connection::field;

/// What the test checks, in order.
pub(crate) fn labels() -> [String; 3] {
    [
        t!("gcs-test-reachable"),
        t!("gcs-test-credentials"),
        t!("gcs-test-list-buckets"),
    ]
}

/// What an error says, without the kind of error before it.
fn said(error: &Error) -> String {
    match error {
        Error::Extension { source, .. } => source.to_string(),
        Error::Io(inner) => inner.to_string(),
        other => other.to_string(),
    }
}

/// The checks with these outcomes, a detail by the one that did not pass.
fn result(outcomes: [CheckOutcome; 3], detail: String) -> ConnectionTest {
    let mut detail = Some(detail);
    let checks = labels()
        .into_iter()
        .zip(outcomes)
        .map(|(label, outcome)| ConnectionCheck {
            label,
            outcome,
            detail: if outcome == CheckOutcome::Failed {
                detail.take().unwrap_or_default()
            } else {
                String::new()
            },
        })
        .collect();
    ConnectionTest {
        checks,
        good: outcomes == [CheckOutcome::Passed; 3],
    }
}

/// How one request of the test ended.
enum Asked {
    /// An answer, or none: the service's refusal inside, as sent.
    Answered(Result<Result<(), GcsError>, Error>),
    /// No answer in time.
    TimedOut,
    /// The test was cancelled.
    Cancelled,
}

/// Runs one request under the check's time limit, ending early when the
/// test is cancelled.
async fn ask(
    request: impl Future<Output = Result<Result<(), GcsError>, Error>>,
    limit: Duration,
    cancel: &CancelToken,
) -> Asked {
    let timed = std::pin::pin!(tokio::time::timeout(limit, request));
    match futures::future::select(cancel.cancelled(), timed).await {
        futures::future::Either::Left(_) => Asked::Cancelled,
        futures::future::Either::Right((Ok(answer), _)) => Asked::Answered(answer),
        futures::future::Either::Right((Err(_), _)) => Asked::TimedOut,
    }
}

/// The test on the values of `form`, with `key` the service account's key
/// as typed or, in *Edit…*, the keychain's: the JSON, the path of the
/// file that holds it, or nothing. `limit` is how long a request may take.
pub(crate) async fn run(
    form: &ConnectionTestForm<'_>,
    key: Result<Option<String>, Error>,
    limit: Duration,
    cancel: &CancelToken,
) -> ConnectionTest {
    use CheckOutcome::{Failed, NotRun, Passed};
    let timed_out = || {
        let seconds = i64::try_from(limit.as_secs()).unwrap_or(0);
        t!("gcs-test-timeout", seconds = seconds)
    };
    let client =
        |account| GcsClient::new(form.get(field::PROJECT), form.get(field::ENDPOINT), account);

    // Check 1: any answer to a request without a token.
    let anonymous = match client(None) {
        Ok(client) => client,
        Err(error) => return result([Failed, NotRun, NotRun], said(&error)),
    };
    let unsigned = match ask(anonymous.probe(false), limit, cancel).await {
        Asked::Cancelled => return result([NotRun; 3], String::new()),
        Asked::TimedOut => return result([Failed, NotRun, NotRun], timed_out()),
        Asked::Answered(Err(error)) => return result([Failed, NotRun, NotRun], said(&error)),
        Asked::Answered(Ok(answer)) => answer,
    };

    // Check 2: the key reads and the service takes it.
    let account = key.and_then(|key| key.map(|key| account_of(&key)).transpose());
    let signed = match account.and_then(client) {
        Ok(signed) => signed,
        Err(error) => return result([Passed, Failed, NotRun], said(&error)),
    };
    let answer = if signed.signs() {
        match ask(signed.probe(true), limit, cancel).await {
            Asked::Cancelled => return result([Passed, NotRun, NotRun], String::new()),
            Asked::TimedOut => return result([Passed, Failed, NotRun], timed_out()),
            // The token endpoint did not answer or refused the key.
            Asked::Answered(Err(error)) => {
                return result([Passed, Failed, NotRun], said(&error));
            }
            Asked::Answered(Ok(answer)) => answer,
        }
    } else {
        unsigned
    };
    match answer {
        Ok(()) => result([Passed; 3], String::new()),
        Err(refused) if refused.status == 401 => {
            result([Passed, Failed, NotRun], refused.to_string())
        }
        // Check 3: the key was taken, the listing was not allowed.
        Err(refused) => result([Passed, Passed, Failed], refused.to_string()),
    }
}

/// The service account of a key as the field holds it: the JSON itself,
/// or the path of the file holding it.
pub(crate) fn account_of(key: &str) -> Result<ServiceAccount, Error> {
    let trimmed = key.trim();
    if trimmed.starts_with('{') {
        return ServiceAccount::parse(trimmed);
    }
    let json = std::fs::read_to_string(trimmed).map_err(|e| Error::Extension {
        ext: crate::ID,
        source: t!("gcs-test-key-file", path = trimmed, error = e.to_string()).into(),
    })?;
    ServiceAccount::parse(&json)
}
