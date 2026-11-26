//! Provider recognition: which S3 service an endpoint's host belongs to,
//! and what that settles in the connection dialogue
//! (specs/s3-connection.md 1.3).
//!
//! There is no provider dropdown. The dialogue hands the endpoint to
//! [`recognise`] as it is typed or pasted, shows the provider's name by the
//! field and fills the region and the addressing style where the user has
//! not typed. Recognition is the table [`TABLE`]: a host pattern, the
//! provider's name, how the region is found and the addressing style the
//! provider's own documentation gives. Adding a provider means adding a
//! row; each row names the page it was checked against.

use ext::Text;

/// How the signing region of a recognised provider is found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionRule {
    /// Always this value, whatever the host: `auto`.
    Fixed(&'static str),
    /// The label of the host the pattern marks with `*`.
    FromHost,
}

/// One row of the recognition table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// The provider's name by the endpoint: a Fluent key.
    pub provider: Text,
    /// The host pattern: literal labels, with one `*` that stands for one
    /// label (the region) or, as the first label of a
    /// [`RegionRule::Fixed`] row, for one or more labels (an account id,
    /// with or without a jurisdiction).
    pub host: &'static str,
    /// How the region is found.
    pub region: RegionRule,
    /// The provider's addressing style: path-style when `true`.
    pub path_style: bool,
    /// The page of the provider's own documentation the row was checked
    /// against.
    pub source: &'static str,
}

/// What an endpoint's host was recognised as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recognised {
    /// The provider's name: a Fluent key.
    pub provider: Text,
    /// The region the provider signs with for this host; `None` when the
    /// user enters it (AWS with an empty endpoint).
    pub region: Option<String>,
    /// The provider's addressing style: path-style when `true`.
    pub path_style: bool,
}

/// The host of an endpoint as typed: without the scheme, the port and
/// anything after them, in lower case.
fn host_of(endpoint: &str) -> String {
    let rest = endpoint.trim();
    let rest = rest.split_once("://").map_or(rest, |(_, rest)| rest);
    let rest = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let rest = rest.rsplit_once('@').map_or(rest, |(_, host)| host);
    let host = rest.split(':').next().unwrap_or_default();
    host.trim_end_matches('.').to_ascii_lowercase()
}

/// The text `*` stands for when `host` matches `pattern`.
fn matched<'a>(pattern: &str, host: &'a str, many: bool) -> Option<&'a str> {
    let (before, after) = pattern.split_once('*')?;
    let middle = host.strip_prefix(before)?.strip_suffix(after)?;
    let labels_ok = middle.split('.').all(|label| {
        !label.is_empty() && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    });
    (labels_ok && (many || !middle.contains('.'))).then_some(middle)
}

/// The provider `endpoint` belongs to, by its host. An empty endpoint is
/// AWS, where only the region matters and the user enters it; a host no
/// row knows is `None`, the generic S3-compatible form.
#[must_use]
pub fn recognise(endpoint: &str) -> Option<Recognised> {
    if endpoint.trim().is_empty() {
        return Some(Recognised {
            provider: AWS,
            region: None,
            path_style: false,
        });
    }
    let host = host_of(endpoint);
    TABLE.iter().find_map(|row| {
        let region = match row.region {
            RegionRule::Fixed(region) => {
                if row.host.contains('*') {
                    matched(row.host, &host, true)?;
                } else if row.host != host {
                    return None;
                }
                region.to_owned()
            }
            RegionRule::FromHost => matched(row.host, &host, false)?.to_owned(),
        };
        Some(Recognised {
            provider: row.provider,
            region: Some(region),
            path_style: row.path_style,
        })
    })
}

/// Amazon's name by the endpoint.
const AWS: Text = Text("s3-provider-aws");

const fn row(
    provider: &'static str,
    host: &'static str,
    region: RegionRule,
    path_style: bool,
    source: &'static str,
) -> Row {
    Row {
        provider: Text(provider),
        host,
        region,
        path_style,
        source,
    }
}

const AWS_DOCS: &str = "https://docs.aws.amazon.com/general/latest/gr/s3.html";
const WASABI_DOCS: &str =
    "https://docs.wasabi.com/docs/what-are-the-service-urls-for-wasabi-s-different-storage-regions";

/// The recognition table, first match wins. Each row was checked against
/// the provider's own documentation on 2026-11-26; the page is in
/// [`Row::source`].
pub const TABLE: &[Row] = &[
    // Amazon S3: virtual-hosted addressing is the documented standard; the
    // region is the host's label, and the legacy global host is us-east-1.
    row(
        "s3-provider-aws",
        "s3.*.amazonaws.com",
        RegionRule::FromHost,
        false,
        AWS_DOCS,
    ),
    row(
        "s3-provider-aws",
        "s3.dualstack.*.amazonaws.com",
        RegionRule::FromHost,
        false,
        AWS_DOCS,
    ),
    row(
        "s3-provider-aws",
        "s3.amazonaws.com",
        RegionRule::Fixed("us-east-1"),
        false,
        "https://docs.aws.amazon.com/AmazonS3/latest/userguide/VirtualHosting.html",
    ),
    // Cloudflare R2: the account's endpoint, with or without a
    // jurisdiction (eu, us, fedramp); the region is always `auto`.
    row(
        "s3-provider-r2",
        "*.r2.cloudflarestorage.com",
        RegionRule::Fixed("auto"),
        true,
        "https://developers.cloudflare.com/r2/api/tokens/",
    ),
    row(
        "s3-provider-b2",
        "s3.*.backblazeb2.com",
        RegionRule::FromHost,
        true,
        "https://www.backblaze.com/apidocs/introduction-to-the-s3-compatible-api",
    ),
    // Wasabi: the bare host is us-east-1, and six alias hosts carry a
    // label that is not their region's name.
    row(
        "s3-provider-wasabi",
        "s3.wasabisys.com",
        RegionRule::Fixed("us-east-1"),
        true,
        WASABI_DOCS,
    ),
    row(
        "s3-provider-wasabi",
        "s3.nl-1.wasabisys.com",
        RegionRule::Fixed("eu-central-1"),
        true,
        WASABI_DOCS,
    ),
    row(
        "s3-provider-wasabi",
        "s3.de-1.wasabisys.com",
        RegionRule::Fixed("eu-central-2"),
        true,
        WASABI_DOCS,
    ),
    row(
        "s3-provider-wasabi",
        "s3.uk-1.wasabisys.com",
        RegionRule::Fixed("eu-west-1"),
        true,
        WASABI_DOCS,
    ),
    row(
        "s3-provider-wasabi",
        "s3.fr-1.wasabisys.com",
        RegionRule::Fixed("eu-west-2"),
        true,
        WASABI_DOCS,
    ),
    row(
        "s3-provider-wasabi",
        "s3.uk-2.wasabisys.com",
        RegionRule::Fixed("eu-west-3"),
        true,
        WASABI_DOCS,
    ),
    row(
        "s3-provider-wasabi",
        "s3.it-1.wasabisys.com",
        RegionRule::Fixed("eu-south-1"),
        true,
        WASABI_DOCS,
    ),
    row(
        "s3-provider-wasabi",
        "s3.*.wasabisys.com",
        RegionRule::FromHost,
        true,
        WASABI_DOCS,
    ),
    row(
        "s3-provider-spaces",
        "*.digitaloceanspaces.com",
        RegionRule::FromHost,
        true,
        "https://docs.digitalocean.com/reference/api/spaces/",
    ),
    row(
        "s3-provider-scaleway",
        "s3.*.scw.cloud",
        RegionRule::FromHost,
        true,
        "https://www.scaleway.com/en/docs/object-storage/concepts/",
    ),
    row(
        "s3-provider-ovh",
        "s3.*.io.cloud.ovh.net",
        RegionRule::FromHost,
        true,
        "https://docs.ovhcloud.com/en/guides/storage-and-backup/object-storage/s3-location",
    ),
    // Google Cloud Storage through its S3-compatible XML API; the native
    // provider is `ext-vfs-gcs`.
    row(
        "s3-provider-gcs",
        "storage.googleapis.com",
        RegionRule::Fixed("auto"),
        true,
        "https://docs.cloud.google.com/storage/docs/aws-simple-migration",
    ),
];
