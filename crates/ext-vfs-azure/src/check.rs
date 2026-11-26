//! The connection test: what the dialogue's *Test* button and *Add* and
//! *Save* ask before a connection is saved (specs/s3-connection.md Part 3).
//!
//! Three checks, in order, answered by one request, `List Containers`
//! asking for a single name (`GET /?comp=list&maxresults=1`):
//!
//! 1. The host is reachable: the request got an answer.
//! 2. The credentials are accepted: the answer is not `401`, and not
//!    `AuthenticationFailed`, which is how the service refuses a wrong
//!    account key.
//! 3. The account's containers can be listed: the answer is the list.
//!
//! The test only reads, the request gives up after [`ext::CHECK_TIMEOUT`],
//! and the account key lives in the client of the run and is dropped with
//! it; in *Edit…* a key left empty is read from the keychain and never
//! written.

use std::time::Duration;

use ext::{CheckOutcome, ConnectionCheck, ConnectionTest, ConnectionTestForm, t};
use types::{CancelToken, Error};

use crate::client::AzureClient;
use crate::connection::field;

/// What the test checks, in order.
pub(crate) fn labels() -> [String; 3] {
    [
        t!("azure-test-reachable"),
        t!("azure-test-credentials"),
        t!("azure-test-list-containers"),
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

/// The test on the values of `form`, with `key` the account key typed or,
/// in *Edit…*, the keychain's. `limit` is how long the request may take.
pub(crate) async fn run(
    form: &ConnectionTestForm<'_>,
    key: Result<String, Error>,
    limit: Duration,
    cancel: &CancelToken,
) -> ConnectionTest {
    use CheckOutcome::{Failed, NotRun, Passed};
    let client = key.and_then(|key| {
        AzureClient::new(form.get(field::ACCOUNT), &key, form.get(field::ENDPOINT))
    });
    let client = match client {
        Ok(client) => client,
        Err(error) => return result([Failed, NotRun, NotRun], said(&error)),
    };
    let timed = std::pin::pin!(tokio::time::timeout(limit, client.probe()));
    let answer = match futures::future::select(cancel.cancelled(), timed).await {
        futures::future::Either::Left(_) => return result([NotRun; 3], String::new()),
        futures::future::Either::Right((Err(_), _)) => {
            let seconds = i64::try_from(limit.as_secs()).unwrap_or(0);
            return result(
                [Failed, NotRun, NotRun],
                t!("azure-test-timeout", seconds = seconds),
            );
        }
        futures::future::Either::Right((Ok(answer), _)) => answer,
    };
    match answer {
        Err(error) => result([Failed, NotRun, NotRun], said(&error)),
        Ok(Ok(())) => result([Passed; 3], String::new()),
        Ok(Err(refused)) if refused.status == 401 || refused.code == "AuthenticationFailed" => {
            result([Passed, Failed, NotRun], refused.to_string())
        }
        Ok(Err(refused)) => result([Passed, Passed, Failed], refused.to_string()),
    }
}
