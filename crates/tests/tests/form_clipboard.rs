//! Acceptance (issue 221, item 5; SPEC.md *A form field can take its value
//! from the clipboard* under Core types, and *Git clone dialogue*): a field
//! of a form an action shows may carry a clipboard rule, a regular
//! expression. When the dialogue opens, the window reads the clipboard's
//! text once and a field whose rule matches opens with the match — the
//! first capture group where the rule has one. The clipboard is not looked
//! at again while the dialogue shows. The Git clone dialogue's URL field
//! uses such a rule, which takes the shapes of a Git URL.

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use ext::{
    API_VERSION, Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability,
    Capabilities, Extension, Field, Form, Manifest, Platforms, Points, Registry, Shortcut, UiHost,
    Version,
};
use platform::{Clipboard as _, ConfigDir, Fake};
use runtime::JobRunner;
use tests::local;
use types::{Error, ExtId};
use ui_egui::keys::Intent;
use ui_egui::{EguiHost, RickUi};

const LINK: ActionId = ActionId("test.link-issue");

/// What the action's form was confirmed with.
type Taken = Arc<Mutex<Vec<(String, String)>>>;

/// An action that asks for an issue's number and a note in a form: the
/// number's field takes it from the clipboard, out of `#4821` or a link
/// that ends in `/issues/4821`; the note has no rule.
struct LinkIssue(Taken);

fn link_form() -> Form {
    Form::new("Link an issue", "Link")
        .field(
            Field::text("issue", "Issue", "", "A number").from_clipboard(r"(?:#|/issues/)(\d+)\b"),
        )
        .field(Field::text("note", "Note", "as written", ""))
}

impl Action for LinkIssue {
    fn id(&self) -> ActionId {
        LINK
    }
    fn label(&self) -> String {
        "Link an issue…".to_owned()
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, _: &ActionCtx) -> Availability {
        Availability::Enabled
    }
    fn run(
        &self,
        _ctx: ActionCtx,
        _jobs: &JobRunner,
        ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        if let Some(values) = ui.form(link_form()) {
            self.0.lock().unwrap_or_else(PoisonError::into_inner).push((
                values.text("issue").to_owned(),
                values.text("note").to_owned(),
            ));
        }
        Ok(ActionOutcome::Done)
    }
}

struct LinkExtension(Manifest, Taken);

impl Extension for LinkExtension {
    fn manifest(&self) -> &Manifest {
        &self.0
    }
    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.action(Arc::new(LinkIssue(Arc::clone(&self.1))));
        Ok(())
    }
}

fn window(start: &Path) -> (Harness<'static, RickUi>, Arc<Fake>, Taken) {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let taken = Taken::default();
    let extension = LinkExtension(
        Manifest {
            id: ExtId("test-link"),
            name: "Link",
            description: "",
            version: Version::new(1, 0, 0),
            api_version: API_VERSION,
            requires: Capabilities::empty(),
            points: Points::ACTION,
            platforms: Platforms::ALL,
            previews: &[],
            commands: &[],
        },
        Arc::clone(&taken),
    );
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![Arc::new(extension)],
        config_dir: Some(fake.config_dir()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    (harness, fake, taken)
}

fn settle(harness: &mut Harness<'_, RickUi>) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        for _ in 0..3 {
            harness.step();
        }
        if !harness.state().app().is_busy() {
            std::thread::sleep(Duration::from_millis(30));
            harness.step();
            if !harness.state().app().is_busy() {
                break;
            }
        }
        assert!(Instant::now() < deadline, "did not settle");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Opens the action's dialogue with `clipboard` on the clipboard.
fn open(harness: &mut Harness<'_, RickUi>, fake: &Fake, clipboard: &str) {
    fake.set_text(clipboard).unwrap_or_else(|e| panic!("{e}"));
    let pane = harness.state().app().model().focused;
    harness.state_mut().queue(Intent::Action(LINK), pane);
    for _ in 0..4 {
        harness.step();
    }
    harness.get_by_label("Link an issue");
}

/// The texts of the dialogue's fields, top to bottom: the text inputs
/// between its title and its confirming button.
fn fields(harness: &Harness<'_, RickUi>) -> Vec<String> {
    let top = harness.get_by_label("Link an issue").rect().top();
    let bottom = harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Link")
        .rect()
        .bottom();
    let mut found: Vec<(f32, String)> = harness
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .filter(|node| (top..bottom).contains(&node.rect().center().y))
        .map(|node| {
            (
                node.rect().top(),
                node.accesskit_node().value().unwrap_or_default(),
            )
        })
        .collect();
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found.into_iter().map(|(_, text)| text).collect()
}

fn close(harness: &mut Harness<'_, RickUi>) {
    harness.key_press(egui::Key::Escape);
    harness.step();
    harness.step();
    assert!(harness.query_by_label("Link an issue").is_none());
}

#[test]
fn a_form_field_opens_with_what_its_rule_takes_from_the_clipboard() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, fake, taken) = window(dir.path());

    // A rule with a capture group: the field opens with the group's
    // text, and the field without a rule as it is written.
    open(&mut harness, &fake, "  see #4821 before the release\n");
    assert_eq!(fields(&harness), ["4821", "as written"]);

    // Once, on opening: the clipboard is not watched while it shows.
    fake.set_text("#77").unwrap_or_else(|e| panic!("{e}"));
    for _ in 0..6 {
        harness.step();
    }
    assert_eq!(fields(&harness), ["4821", "as written"]);

    // Confirmed, the action gets what the field held.
    harness.key_press(egui::Key::Enter);
    harness.step();
    settle(&mut harness);
    assert_eq!(
        *taken.lock().unwrap_or_else(PoisonError::into_inner),
        [("4821".to_owned(), "as written".to_owned())]
    );

    // The same rule out of a link; without a match, and without text, the
    // field opens as written, which is empty.
    for (clipboard, issue) in [
        ("https://github.com/Open-Breach/rick/issues/221", "221"),
        ("a sentence of text, and nothing else", ""),
        ("", ""),
    ] {
        open(&mut harness, &fake, clipboard);
        assert_eq!(fields(&harness), [issue, "as written"], "{clipboard:?}");
        close(&mut harness);
    }
}

/// The rule of the Git clone dialogue's URL field takes what the plugin's
/// own check of a URL takes, shape for shape, and nothing with white space
/// around or inside it; an absolute path is the plugin's to look at.
#[test]
fn the_git_clone_rule_takes_the_shapes_of_a_git_url() {
    use ext_git::clone::{URL_RULE, is_git_url};
    let form = Form::new("Clone", "Clone")
        .field(Field::text("url", "URL", "", "").from_clipboard(URL_RULE));
    let taken = |clipboard: &str| {
        let mut values = form.defaults();
        ui_egui::dialog::take_from_clipboard(&form, &mut values, Some(clipboard));
        values.text("url").to_owned()
    };
    for url in [
        "https://github.com/Open-Breach/rick.git",
        "HTTPS://github.com/Open-Breach/rick",
        "https://user:secret@example.com:8443/team/rick.git",
        "ssh://git@example.com:22/team/rick.git",
        "git://example.com/rick.git",
        "file:///srv/git/rick.git",
        "file://server/share/rick.git",
        "git@github.com:Open-Breach/rick.git",
        "git@example.com:/srv/git/rick.git",
        "deploy@10.0.0.7:rick",
    ] {
        assert!(is_git_url(url), "{url} is a Git URL to the plugin");
        assert_eq!(taken(url), url, "{url}");
        // Copied with the line's end and a space before it.
        assert_eq!(taken(&format!("  {url}\n")), url, "{url}, trimmed");
    }
    for other in [
        "http://example.com/rick.git",
        "ftp://example.com/rick.git",
        "https://github.com",
        "https://github.com/",
        "https:///rick.git",
        "https://-oProxyCommand=x/rick.git",
        "git@github.com:",
        "git@github.com:/",
        "@github.com:rick.git",
        "-oProxyCommand=x@host:rick",
        "user/name@host:rick",
        "git@host://rick",
        "just some text",
        "git clone https://github.com/Open-Breach/rick.git",
        "https://github.com/Open-Breach/rick.git and more",
        "see\thttps://example.com/rick.git",
        "rick.git",
        "C:\\Users\\me\\rick",
    ] {
        assert!(!is_git_url(other), "{other} is no Git URL to the plugin");
        assert_eq!(taken(other), "", "{other}");
    }
    // An absolute path is no shape: the rule leaves it to the plugin.
    assert_eq!(taken("/srv/git/rick.git"), "");
}
