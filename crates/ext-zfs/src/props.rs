//! The dataset properties the info section shows, read with one
//! `zfs get -H -p -o property,value,source <list> <dataset>`.
//!
//! A property whose value is `-` (one that does not apply, `keystatus`
//! without encryption) is left out; when an older ZFS rejects a property
//! of the list as unknown, the read is tried once more without it, so one
//! property never empties the section.

use crate::zfs::{Output, Zfs, ZfsError, classify, to_error};
use types::Error;

/// The properties shown, in this order.
pub const PROPERTIES: [&str; 19] = [
    "guid",
    "type",
    "mountpoint",
    "used",
    "available",
    "referenced",
    "usedbysnapshots",
    "quota",
    "refquota",
    "reservation",
    "compression",
    "compressratio",
    "recordsize",
    "dedup",
    "copies",
    "atime",
    "encryption",
    "keystatus",
    "readonly",
];

/// The properties whose value is a byte count.
const SIZES: [&str; 8] = [
    "used",
    "available",
    "referenced",
    "usedbysnapshots",
    "quota",
    "refquota",
    "reservation",
    "recordsize",
];

/// Where a property's value comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Set on the dataset itself.
    Local,
    /// Inherited from the named dataset.
    Inherited(String),
    /// ZFS's default.
    Default,
    /// Received with `zfs receive`.
    Received,
    /// A read-only property with no source (`-`), or one ZFS names
    /// otherwise.
    None,
}

impl Source {
    fn parse(text: &str) -> Self {
        match text {
            "local" => Self::Local,
            "default" => Self::Default,
            "received" => Self::Received,
            other => other
                .strip_prefix("inherited from ")
                .map_or(Self::None, |from| Self::Inherited(from.to_owned())),
        }
    }

    /// The source in the language in use; empty for [`Source::None`].
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Self::Local => ext::t!("zfs-source-local"),
            Self::Inherited(from) => ext::t!("zfs-source-inherited", dataset = from.clone()),
            Self::Default => ext::t!("zfs-source-default"),
            Self::Received => ext::t!("zfs-source-received"),
            Self::None => String::new(),
        }
    }
}

/// One property of a dataset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    /// Its name, `compression`.
    pub name: String,
    /// Its value as ZFS printed it with `-p`.
    pub value: String,
    /// Where the value comes from.
    pub source: Source,
}

impl Property {
    /// The value as the info section shows it: sizes in human units
    /// (`none` for a quota or reservation of 0), the compression ratio with
    /// its `x`, everything else as ZFS printed it.
    #[must_use]
    pub fn shown(&self) -> String {
        if SIZES.contains(&self.name.as_str())
            && let Ok(bytes) = self.value.parse::<u64>()
        {
            if bytes == 0 && self.name != "used" && self.name != "referenced" {
                return "none".to_owned();
            }
            return format_size(bytes);
        }
        if self.name.ends_with("compressratio") && !self.value.ends_with('x') {
            return format!("{}x", self.value);
        }
        self.value.clone()
    }
}

/// The arguments that read `properties` of `dataset`.
#[must_use]
pub fn get_args(properties: &[&str], dataset: &str) -> Vec<String> {
    vec![
        "get".to_owned(),
        "-H".to_owned(),
        "-p".to_owned(),
        "-o".to_owned(),
        "property,value,source".to_owned(),
        properties.join(","),
        dataset.to_owned(),
    ]
}

/// The properties in `zfs get -H -p -o property,value,source` output, in
/// the order of [`PROPERTIES`]; `-` values and unknown names are dropped.
#[must_use]
pub fn parse(output: &str) -> Vec<Property> {
    let mut read: Vec<Property> = output
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            let (Some(name), Some(value), Some(source)) =
                (fields.next(), fields.next(), fields.next())
            else {
                tracing::debug!(line, "zfs get: malformed line skipped");
                return None;
            };
            (value != "-" && PROPERTIES.contains(&name)).then(|| Property {
                name: name.to_owned(),
                value: value.to_owned(),
                source: Source::parse(source),
            })
        })
        .collect();
    read.sort_by_key(|p| PROPERTIES.iter().position(|n| *n == p.name));
    read
}

/// The property an older ZFS rejected as unknown, from its message
/// (`bad property list: invalid property 'keystatus'`).
fn rejected(message: &str) -> Option<&str> {
    let rest = message.split("invalid property '").nth(1)?;
    rest.split('\'').next()
}

/// Reads the properties of `dataset`, retrying once without a property
/// ZFS rejects as unknown.
///
/// # Errors
///
/// `Error::Unsupported` without `zfs`; ZFS's error otherwise.
pub fn read(zfs: &dyn Zfs, dataset: &str) -> Result<Vec<Property>, Error> {
    let mut list: Vec<&str> = PROPERTIES.to_vec();
    let first = zfs.run(&get_args(&list, dataset))?;
    let output = match classify(first) {
        Ok(out) => out,
        Err(ZfsError::Other(message)) => {
            let Some(unknown) = rejected(&message).filter(|u| list.contains(u)) else {
                return Err(to_error(ZfsError::Other(message)));
            };
            tracing::debug!(property = unknown, "zfs does not know a property; read without it");
            list.retain(|p| *p != unknown);
            let again: Output = zfs.run(&get_args(&list, dataset))?;
            classify(again).map_err(to_error)?
        }
        Err(error) => return Err(to_error(error)),
    };
    Ok(parse(&output))
}

/// `bytes` in binary units, one decimal from KiB on.
#[must_use]
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    // Precision loss is irrelevant for a one-decimal display.
    #[allow(clippy::cast_precision_loss)]
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zfs::ScriptedZfs;

    /// `zfs get` for `rpool/ROOT/ubuntu_62wyjh/var/lib` on the development
    /// machine (OpenZFS 2.4.1, an encrypted pool).
    const ENCRYPTED: &str = "guid\t2701085171283325975\t-
type\tfilesystem\t-
mountpoint\t/var/lib\tinherited from rpool/ROOT/ubuntu_62wyjh
used\t46816264192\t-
available\t422991925248\t-
referenced\t35075158016\t-
usedbysnapshots\t11363057664\t-
quota\t0\tdefault
refquota\t0\tdefault
reservation\t0\tdefault
compression\tlz4\tinherited from rpool
compressratio\t1.14\t-
recordsize\t131072\tdefault
dedup\toff\tdefault
copies\t1\tdefault
atime\ton\tdefault
encryption\taes-256-gcm\t-
keystatus\tavailable\t-
readonly\toff\tdefault
";

    fn find<'a>(props: &'a [Property], name: &str) -> Option<&'a Property> {
        props.iter().find(|p| p.name == name)
    }

    #[test]
    fn recorded_output_reads_in_order_with_sources() {
        let props = parse(ENCRYPTED);
        let names: Vec<&str> = props.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, PROPERTIES);
        let compression = find(&props, "compression");
        assert_eq!(compression.map(|p| p.value.as_str()), Some("lz4"));
        assert_eq!(
            compression.map(|p| p.source.clone()),
            Some(Source::Inherited("rpool".to_owned()))
        );
        assert_eq!(find(&props, "quota").map(|p| p.source.clone()), Some(Source::Default));
        assert_eq!(find(&props, "guid").map(|p| p.source.clone()), Some(Source::None));
        assert_eq!(
            find(&props, "guid").map(Property::shown).as_deref(),
            Some("2701085171283325975"),
            "a guid stays a plain number"
        );
        assert_eq!(find(&props, "used").map(Property::shown).as_deref(), Some("43.6 GiB"));
        assert_eq!(find(&props, "quota").map(Property::shown).as_deref(), Some("none"));
        assert_eq!(find(&props, "recordsize").map(Property::shown).as_deref(), Some("128.0 KiB"));
        assert_eq!(find(&props, "compressratio").map(Property::shown).as_deref(), Some("1.14x"));
    }

    #[test]
    fn an_unencrypted_dataset_drops_keystatus() {
        let plain = ENCRYPTED
            .replace("encryption\taes-256-gcm\t-", "encryption\toff\tdefault")
            .replace("keystatus\tavailable\t-", "keystatus\t-\t-");
        let props = parse(&plain);
        assert_eq!(find(&props, "encryption").map(|p| p.value.as_str()), Some("off"));
        assert!(find(&props, "keystatus").is_none());
        assert_eq!(props.len(), PROPERTIES.len() - 1);
    }

    #[test]
    fn local_and_received_sources_are_named() {
        let props = parse("compression\tzstd\tlocal\natime\toff\treceived\ncopies\t2\ttemporary\n");
        let source = |name| find(&props, name).map(|p| p.source.clone());
        assert_eq!(source("compression"), Some(Source::Local));
        assert_eq!(source("atime"), Some(Source::Received));
        assert_eq!(source("copies"), Some(Source::None));
        assert!(parse("nonsense\nfoo\tbar\tlocal\n").is_empty());
    }

    #[test]
    fn an_unknown_property_is_dropped_and_the_read_retried_once() {
        let zfs = ScriptedZfs::new();
        let all = get_args(&PROPERTIES, "tank");
        let all: Vec<&str> = all.iter().map(String::as_str).collect();
        zfs.on(
            &all,
            Output::failed(2, "bad property list: invalid property 'keystatus'\nusage:\n\tget ...\n"),
        );
        let without: Vec<&str> = PROPERTIES.iter().copied().filter(|p| *p != "keystatus").collect();
        let retry = get_args(&without, "tank");
        let retry: Vec<&str> = retry.iter().map(String::as_str).collect();
        zfs.on(&retry, Output::ok("guid\t42\t-\ncompression\tzstd\tlocal\n"));
        let props = read(&zfs, "tank").unwrap_or_default();
        assert_eq!(props.len(), 2);
        assert_eq!(zfs.calls().len(), 2);

        // A second unknown property is not chased: the read fails.
        let stubborn = ScriptedZfs::new();
        stubborn.on(&all, Output::failed(2, "bad property list: invalid property 'keystatus'\n"));
        stubborn.on(&retry, Output::failed(2, "bad property list: invalid property 'dedup'\n"));
        assert!(read(&stubborn, "tank").is_err());
        assert_eq!(stubborn.calls().len(), 2);
    }

    #[test]
    fn sizes_read_in_binary_units() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1536), "1.5 KiB");
        assert_eq!(format_size(3 * 1024 * 1024 * 1024), "3.0 GiB");
    }
}
