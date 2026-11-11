use std::sync::Arc;

use ext::platform::{ConfigDir as _, Fake};
use ext::{Capabilities, Registry};
use types::Location;

use super::*;

#[test]
fn locations_split_into_alias_and_remote_path() {
    assert_eq!(split(&Location::new(PROVIDER_ID, "/")), None);
    assert_eq!(split(&Location::new(PROVIDER_ID, "")), None);
    assert_eq!(
        split(&Location::new(PROVIDER_ID, "/nas")),
        Some(("nas".to_owned(), Vec::new()))
    );
    assert_eq!(
        split(&Location::new(PROVIDER_ID, "/nas/")),
        Some(("nas".to_owned(), Vec::new()))
    );
    assert_eq!(
        split(&Location::new(PROVIDER_ID, "/nas/srv/a b.txt")),
        Some(("nas".to_owned(), b"/srv/a b.txt".to_vec()))
    );
    assert_eq!(
        split(&Location::new(PROVIDER_ID, "/nas/srv/")),
        Some(("nas".to_owned(), b"/srv".to_vec()))
    );
    assert_eq!(
        split(&Location::new(PROVIDER_ID, "\\nas\\srv")),
        Some(("nas".to_owned(), b"/srv".to_vec()))
    );
    assert_eq!(location("nas", b"").to_string(), "ssh:/nas");
    assert_eq!(location("nas", b"/").to_string(), "ssh:/nas");
    assert_eq!(location("nas", b"/srv/a").to_string(), "ssh:/nas/srv/a");
    assert_eq!(location("nas", b"/srv/a/").to_string(), "ssh:/nas/srv/a");
    assert_eq!(location("nas", b"srv").to_string(), "ssh:/nas/srv");
    let raw = location("nas", &[b'/', 0xff, b'x']);
    assert_eq!(
        raw.path.as_bytes(),
        [b'/', b'n', b'a', b's', b'/', 0xff, b'x']
    );
    assert_eq!(
        split(&raw),
        Some(("nas".to_owned(), vec![b'/', 0xff, b'x']))
    );
}

#[test]
fn the_extension_registers_a_provider_a_connector_two_actions_and_settings() {
    let ext = SshExtension::new();
    assert_eq!(ext.manifest().id, ID);
    assert!(matches!(
        Registry::new().load(&ext),
        Err(types::Error::Unsupported(_))
    ));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut registry = Registry::with_platform(fake);
    registry.load(&ext).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(registry.providers().len(), 1);
    assert_eq!(registry.actions().len(), 2);
    assert_eq!(registry.settings_schemas().len(), 1);
    assert_eq!(registry.settings_schemas()[0].fields.len(), 3);
    let connector = registry
        .connector_of(PROVIDER_ID)
        .unwrap_or_else(|| panic!("connector"));
    let schema = connector.schema();
    assert_eq!(schema.title.key(), "ssh-ssh");
    assert_eq!(
        schema.secret_fields().collect::<Vec<_>>(),
        ["password", "passphrase"]
    );
    let provider = &registry.providers()[0];
    assert_eq!(provider.id(), PROVIDER_ID);
    assert_eq!(
        provider.capabilities(&Location::new(PROVIDER_ID, "/")),
        Capabilities::REMOTE
    );
    let below = provider.capabilities(&Location::new(PROVIDER_ID, "/nas/x"));
    assert!(
        below.contains(Capabilities::NATIVE_TRANSFER | Capabilities::REMOTE | Capabilities::SEEK)
    );
    assert!(!below.contains(Capabilities::WATCH | Capabilities::PERMISSIONS | Capabilities::TRASH));
    let ids: Vec<&str> = registry.actions().iter().map(|a| a.id().0).collect();
    assert_eq!(ids, ["ssh.console", "ssh.terminal"]);
}

/// Issue 149: what a host offers. A folder below a host takes folders and
/// files and is downloaded from; it is not searched beneath, for now, and
/// says so for the tooltip of the filter bar's disabled toggle.
#[test]
fn a_host_offers_folders_files_and_downloads_and_no_search_beneath() {
    ext::i18n::add_bundled("en", include_str!("../../locales/en.ftl"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let pool = Pool::new(Arc::new(Hosts::new(fake)), None);
    let provider = SshProvider::new(pool);
    let below = Location::new(PROVIDER_ID, "/nas/srv/photos");
    let caps = provider.capabilities(&below);
    assert!(
        caps.contains(
            Capabilities::CREATE_DIR | Capabilities::CREATE_FILE | Capabilities::DOWNLOAD
        )
    );
    assert!(!caps.contains(Capabilities::SEARCH_SUBTREE));
    assert_eq!(
        provider.unavailable_reason(&below, Capabilities::SEARCH_SUBTREE),
        Some("Subfolder search is not available over SSH yet".to_owned())
    );
    // The list of hosts takes nothing new: a host is added in the side
    // panel.
    let root = Location::new(PROVIDER_ID, "/");
    assert!(
        !provider
            .capabilities(&root)
            .intersects(Capabilities::CREATE_DIR | Capabilities::CREATE_FILE)
    );
    assert_eq!(
        provider.unavailable_reason(&root, Capabilities::CREATE_DIR),
        Some("Hosts are added in the side panel".to_owned())
    );
}

#[test]
fn the_root_lists_the_hosts_without_connecting() {
    use futures::StreamExt as _;

    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let ssh = fake.home_dir().join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(ssh.join("config"), "Host nas\n  HostName 192.0.2.1\n")
        .unwrap_or_else(|e| panic!("{e}"));
    let hosts = Arc::new(Hosts::new(fake));
    let pool = Pool::new(Arc::clone(&hosts), None);
    let provider = SshProvider::new(Arc::clone(&pool));
    let connector = SshConnector::new(Arc::clone(&pool));
    let mut rick = ext::Connection {
        name: "box".into(),
        ..ext::Connection::default()
    };
    rick.settings.insert("host".into(), "192.0.2.2".into());
    rick.settings
        .insert("preview_size_cap_mib".into(), "2".into());
    connector.update(vec![rick]);
    runtime::block_on(async {
        let batches: Vec<_> = provider
            .list(
                &Location::new(PROVIDER_ID, "/"),
                ext::ListOptions::default(),
                types::CancelToken::new(),
            )
            .collect()
            .await;
        let names: Vec<String> = batches
            .into_iter()
            .flat_map(Result::unwrap_or_default)
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["box", "nas"]);
        let row = provider
            .stat(&Location::new(PROVIDER_ID, "/nas"))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(row.kind, types::EntryKind::Dir);
        assert_eq!(row.path.to_string(), "/nas");
        assert!(matches!(
            provider.stat(&Location::new(PROVIDER_ID, "/gone")).await,
            Err(types::Error::NotFound)
        ));
        assert!(matches!(
            provider.stat(&Location::new(PROVIDER_ID, "/gone/x")).await,
            Err(types::Error::NotFound)
        ));
    });
    assert!(!connector.is_connected("nas"));
    assert_eq!(
        provider.remote_preview_cap(&Location::new(PROVIDER_ID, "/box/x")),
        Some(2 * 1024 * 1024)
    );
    assert_eq!(
        provider.remote_preview_cap(&Location::new(PROVIDER_ID, "/nas/x")),
        Some(4 * 1024 * 1024),
        "the setting's default"
    );
    assert_eq!(
        provider.remote_preview_cap(&Location::new(PROVIDER_ID, "/")),
        None
    );
    // The actions know an SSH folder from the rest.
    let actions = actions::all(&pool);
    let ctx = |loc: Location| ext::ActionCtx {
        pane: ext::PaneId(1),
        location: loc,
        selection: Vec::new(),
        invoked_from: ext::Invocation::ContextMenu,
        other: None,
    };
    assert_eq!(
        actions[0].availability(&ctx(Location::new(PROVIDER_ID, "/nas/srv"))),
        ext::Availability::Enabled
    );
    assert_eq!(
        actions[1].availability(&ctx(Location::new(types::ProviderId("local"), "/srv"))),
        ext::Availability::Hidden
    );
    assert_eq!(
        actions[1].availability(&ctx(Location::new(PROVIDER_ID, "/"))),
        ext::Availability::Hidden
    );
}

mod live;
