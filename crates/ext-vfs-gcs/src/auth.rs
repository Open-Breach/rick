//! Service-account authentication: the key file's JSON, the JWT it signs
//! and the bearer token the token endpoint exchanges it for.

use std::time::{Duration, SystemTime};

use base64::Engine as _;
use serde::Deserialize;
use types::Error;

use crate::ID;

/// The scope every request asks for.
const SCOPE: &str = "https://www.googleapis.com/auth/devstorage.read_write";

/// How long a token is asked for, and how long before its end it is
/// renewed.
const LIFETIME: Duration = Duration::from_hours(1);
const MARGIN: Duration = Duration::from_mins(1);

fn message(text: impl Into<String>) -> Error {
    Error::Extension {
        ext: ID,
        source: text.into().into(),
    }
}

/// What a service account's key file holds, the parts that matter.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ServiceAccount {
    /// The account's e-mail, the JWT's issuer.
    pub client_email: String,
    /// The RSA private key, PEM, PKCS#8.
    pub private_key: String,
    /// Where the JWT is exchanged for a token.
    #[serde(default = "default_token_uri")]
    pub token_uri: String,
}

fn default_token_uri() -> String {
    "https://oauth2.googleapis.com/token".to_owned()
}

impl ServiceAccount {
    /// Parses a key file's JSON.
    ///
    /// # Errors
    ///
    /// The plugin's error when the text is not a service account's JSON.
    pub fn parse(json: &str) -> Result<Self, Error> {
        serde_json::from_str(json).map_err(|e| {
            message(format!(
                "the service account key is not the JSON of one: {e}"
            ))
        })
    }

    /// The DER of the private key, out of its PEM.
    fn key_der(&self) -> Result<Vec<u8>, Error> {
        let body: String = self
            .private_key
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .map(str::trim)
            .collect();
        base64::engine::general_purpose::STANDARD
            .decode(body)
            .map_err(|e| message(format!("the private key is not PEM: {e}")))
    }

    /// A signed JWT asking for a token valid from `now` for an hour.
    ///
    /// # Errors
    ///
    /// The plugin's error when the key cannot be read or does not sign.
    pub fn jwt(&self, now: SystemTime) -> Result<String, Error> {
        let der = self.key_der()?;
        let key = ring::signature::RsaKeyPair::from_pkcs8(&der)
            .map_err(|e| message(format!("the private key is not an RSA PKCS#8 key: {e}")))?;
        let iat = now
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|_| message("the clock is before 1970"))?
            .as_secs();
        let exp = iat + LIFETIME.as_secs();
        let header = base64url(br#"{"alg":"RS256","typ":"JWT"}"#);
        let claims = serde_json::json!({
            "iss": self.client_email,
            "scope": SCOPE,
            "aud": self.token_uri,
            "iat": iat,
            "exp": exp,
        });
        let claims = base64url(claims.to_string().as_bytes());
        let signing_input = format!("{header}.{claims}");
        let mut signature = vec![0u8; key.public().modulus_len()];
        key.sign(
            &ring::signature::RSA_PKCS1_SHA256,
            &ring::rand::SystemRandom::new(),
            signing_input.as_bytes(),
            &mut signature,
        )
        .map_err(|e| message(format!("signing the token request failed: {e}")))?;
        Ok(format!("{signing_input}.{}", base64url(&signature)))
    }
}

fn base64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// A bearer token and when to renew it.
#[derive(Debug, Clone)]
pub struct Token {
    /// The token itself.
    pub access_token: String,
    /// When it should be renewed: its end less a margin.
    pub renew_at: SystemTime,
}

impl Token {
    /// Whether the token is still good to use.
    #[must_use]
    pub fn is_fresh(&self, now: SystemTime) -> bool {
        now < self.renew_at
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: Option<u64>,
}

/// A token's fields out of the token endpoint's answer, valid from `now`.
pub(crate) fn parse_token(json: &str, now: SystemTime) -> Result<Token, Error> {
    let response: TokenResponse = serde_json::from_str(json)
        .map_err(|e| message(format!("the token endpoint answered no token: {e}")))?;
    let lifetime = response.expires_in.map_or(LIFETIME, Duration::from_secs);
    Ok(Token {
        access_token: response.access_token,
        renew_at: now + lifetime.saturating_sub(MARGIN),
    })
}

/// Exchanges a service account's JWT for a bearer token.
pub(crate) async fn fetch_token(
    http: &reqwest::Client,
    account: &ServiceAccount,
) -> Result<Token, Error> {
    let now = SystemTime::now();
    let jwt = account.jwt(now)?;
    let body =
        format!("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer&assertion={jwt}");
    let response = http
        .post(&account.token_uri)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|e| message(format!("the token endpoint did not answer: {e}")))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(message(format!(
            "the token endpoint refused the service account ({status}): {}",
            text.chars().take(200).collect::<String>()
        )));
    }
    parse_token(&text, now)
}
