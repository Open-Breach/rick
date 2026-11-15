//! The desktop's background on Linux (SPEC.md *Set as desktop
//! background*): GNOME through `gsettings`, KDE Plasma through
//! `plasma-apply-wallpaperimage`. Both point the desktop at the file where
//! it lies. No other desktop is set, and none is offered.

use std::ffi::OsString;
use std::path::Path;

use types::Error;

use crate::integration::Desktop;

/// What GNOME and KDE Plasma both show as a background without a loader
/// that may be missing.
const FORMATS: &[&str] = &["jpg", "jpeg", "png", "bmp"];

/// The schema GNOME keeps its background in.
const GNOME_SCHEMA: &str = "org.gnome.desktop.background";

/// The desktop of this session, by `XDG_CURRENT_DESKTOP`.
fn current() -> Option<Desktop> {
    Desktop::from_current_desktop(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default())
}

/// The formats `desktop` takes: none for a desktop Rick does not set.
pub(super) fn formats_of(desktop: Option<Desktop>) -> &'static [&'static str] {
    match desktop {
        Some(Desktop::Gnome | Desktop::Plasma) => FORMATS,
        Some(Desktop::Cosmic | Desktop::Cinnamon) | None => &[],
    }
}

/// The formats this session's desktop takes.
pub(super) fn formats() -> &'static [&'static str] {
    formats_of(current())
}

/// The commands that point `desktop` at `image`, each a program and its
/// arguments, to run in order; none for a desktop Rick does not set.
/// GNOME has one key for the light style and one for the dark, and both
/// get the picture; Plasma's tool sets it on every screen.
pub(super) fn commands(desktop: Desktop, image: &Path) -> Vec<(&'static str, Vec<OsString>)> {
    match desktop {
        Desktop::Gnome => {
            let uri = crate::paths::file_uri(image);
            ["picture-uri", "picture-uri-dark"]
                .into_iter()
                .map(|key| {
                    (
                        "gsettings",
                        vec![
                            OsString::from("set"),
                            OsString::from(GNOME_SCHEMA),
                            OsString::from(key),
                            OsString::from(&uri),
                        ],
                    )
                })
                .collect()
        }
        Desktop::Plasma => vec![(
            "plasma-apply-wallpaperimage",
            vec![image.as_os_str().to_owned()],
        )],
        Desktop::Cosmic | Desktop::Cinnamon => Vec::new(),
    }
}

/// Points this session's desktop at `image`.
pub(super) fn set(image: &Path) -> Result<(), Error> {
    let commands = current().map(|desktop| commands(desktop, image));
    let Some(commands) = commands.filter(|commands| !commands.is_empty()) else {
        return Err(Error::Unsupported(
            "the background of this desktop cannot be set",
        ));
    };
    for (program, args) in commands {
        let args: Vec<&std::ffi::OsStr> = args.iter().map(OsString::as_os_str).collect();
        super::volumes::run(program, &args)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lossy(commands: Vec<(&'static str, Vec<OsString>)>) -> Vec<String> {
        commands
            .into_iter()
            .map(|(program, args)| {
                let args: Vec<String> = args
                    .iter()
                    .map(|a| a.to_string_lossy().into_owned())
                    .collect();
                format!("{program} {}", args.join(" "))
            })
            .collect()
    }

    /// GNOME gets the file's URI in both of its keys, the light and the
    /// dark; Plasma's tool gets the path; no other desktop gets anything.
    #[test]
    fn each_desktop_is_pointed_at_the_file_where_it_lies() {
        let image = Path::new("/home/me/My pictures/lake.jpg");
        assert_eq!(
            lossy(commands(Desktop::Gnome, image)),
            [
                "gsettings set org.gnome.desktop.background picture-uri \
                 file:///home/me/My%20pictures/lake.jpg",
                "gsettings set org.gnome.desktop.background picture-uri-dark \
                 file:///home/me/My%20pictures/lake.jpg",
            ]
        );
        assert_eq!(
            lossy(commands(Desktop::Plasma, image)),
            ["plasma-apply-wallpaperimage /home/me/My pictures/lake.jpg"]
        );
        for other in [Desktop::Cosmic, Desktop::Cinnamon] {
            assert_eq!(lossy(commands(other, image)), Vec::<String>::new());
        }
    }

    /// JPEG and PNG at least on the two desktops Rick sets, and nothing
    /// on any other, which is what keeps the action from being offered.
    #[test]
    fn only_gnome_and_plasma_take_a_background() {
        for desktop in [Desktop::Gnome, Desktop::Plasma] {
            let formats = formats_of(Some(desktop));
            assert!(formats.contains(&"jpg") && formats.contains(&"jpeg"));
            assert!(formats.contains(&"png"));
        }
        let none: &[&str] = &[];
        for other in [Some(Desktop::Cosmic), Some(Desktop::Cinnamon), None] {
            assert_eq!(formats_of(other), none, "{other:?}");
        }
    }
}
