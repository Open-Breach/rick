//! Acceptance (issue 120, the integration check of
//! specs/system-integration.md §6): what every platform shares. The items
//! the platform lists, checked without a change; the Linux items against
//! a temporary home are in `integration_linux.rs`.

use std::sync::Arc;

use platform::integration::{Actual, Call, Check, Desktop, Item, Request};
use platform::{Fake, FakeIntegration, Integration as _, Native};

/// Linux has the six items of §7.1; Windows and macOS have none yet
/// (milestones 4 and 5), so nothing is listed there and nothing can be
/// performed.
#[test]
fn the_native_platform_lists_the_items_of_its_system() {
    let listed = Native::new().integration_items();
    assert_ne!(listed.system, "", "the line naming the system");
    if ext::Platforms::current() == ext::Platforms::LINUX {
        assert_eq!(listed.items, Item::ALL);
    } else {
        assert!(listed.items.is_empty(), "{listed:?}");
        assert_eq!(listed.desktop, None);
        for item in Item::ALL {
            assert!(
                Native::new()
                    .perform_integration(item, &Request::default())
                    .is_err(),
                "{item:?}"
            );
        }
    }
}

/// The fake platform lists nothing by itself, and answers with whatever
/// integration a test puts in its place.
#[test]
fn the_fake_platform_lists_what_a_test_scripts() {
    let fake = Fake::new().unwrap_or_else(|e| panic!("{e}"));
    assert!(fake.integration_items().items.is_empty());

    let scripted = FakeIntegration::new("Fedora Linux 43 · GNOME", Some(Desktop::Gnome));
    scripted.set(Item::LauncherEntry, Check::missing());
    scripted.set(Item::RickCommand, Check::set_up());
    fake.set_integration(Arc::new(scripted.clone()));
    let listed = fake.integration_items();
    assert_eq!(listed.system, "Fedora Linux 43 · GNOME");
    assert_eq!(listed.desktop, Some(Desktop::Gnome));
    assert_eq!(listed.items, [Item::LauncherEntry, Item::RickCommand]);
    assert_eq!(
        fake.check_integration(Item::RickCommand, &Request::default())
            .actual,
        Actual::SetUp
    );
    // A check is a check: the scripted state is what it was.
    assert_eq!(scripted.actual(Item::LauncherEntry), Some(Actual::Missing));
    assert_eq!(scripted.calls(), [Call::Check(Item::RickCommand)]);
}
