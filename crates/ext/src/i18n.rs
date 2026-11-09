//! The `t!` lookup: every user-facing string of Rick and its plugins goes
//! through [`t!`](crate::t) with a Fluent key, so a language is a translation file,
//! not a code change.
//!
//! The kernel's messages live in `locales/<lang>.ftl` of the repository
//! and are bundled into the binary by `app`; a plugin bundles its own with
//! `include_str!` and hands them to [`Registry::translations`] while it
//! registers. All of them land in one bundle per language here, in this
//! crate, the dylib the binary and the plugins share, so one lookup serves
//! them all. A file of the same name under `locales/` in the configuration
//! directory overrides the bundled messages key by key
//! ([`load_overrides`]), which is how a translation is fixed or added
//! without a build.
//!
//! The language is the setting ([`set_language`]): a tag such as `de`, or
//! none for the system's, negotiated against the languages that have
//! messages. English is the fallback for a key the language lacks, and a
//! key nobody wrote at all comes back as itself, so a missing translation
//! is a visible English text or key, never a panic. The lookup is live: a
//! label drawn every frame follows a language change at once.
//!
//! Fluent's isolation marks around placeables are off, since the text goes
//! to a GUI toolkit that does not render them.
//!
//! ```
//! use ext::{i18n, t};
//!
//! i18n::add_bundled("en", "greeting = Hello, { $name }!\nbye = Goodbye");
//! i18n::add_bundled("de", "greeting = Hallo, { $name }!");
//! i18n::set_language(Some("de"));
//! assert_eq!(t!("greeting", name = "Rick"), "Hallo, Rick!");
//! assert_eq!(t!("bye"), "Goodbye"); // English is the fallback
//! assert_eq!(t!("never-written"), "never-written");
//! i18n::set_language(Some("en"));
//! ```
//!
//! [`Registry::translations`]: crate::Registry::translations

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::sync::{LazyLock, RwLock, RwLockReadGuard, RwLockWriteGuard};

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource};
use unic_langid::LanguageIdentifier;

/// The language every key has a message in, and the fallback.
pub const FALLBACK: &str = "en";

/// One language's messages: the bundled sources in the order given, then
/// the override file, compiled into a bundle on demand.
struct Language {
    tag: LanguageIdentifier,
    sources: Vec<String>,
    override_source: Option<String>,
    bundle: Option<FluentBundle<FluentResource>>,
}

impl Language {
    fn new(tag: LanguageIdentifier) -> Self {
        Self {
            tag,
            sources: Vec::new(),
            override_source: None,
            bundle: None,
        }
    }

    /// The compiled bundle, built from the sources on first use after a
    /// change.
    fn bundle(&mut self) -> &FluentBundle<FluentResource> {
        if self.bundle.is_none() {
            let mut bundle = FluentBundle::new_concurrent(vec![self.tag.clone()]);
            bundle.set_use_isolating(false);
            for source in self.sources.iter().chain(self.override_source.iter()) {
                let resource = match FluentResource::try_new(source.clone()) {
                    Ok(resource) => resource,
                    Err((resource, errors)) => {
                        for error in errors {
                            tracing::warn!(language = %self.tag, %error, "a translation file has a syntax error");
                        }
                        resource
                    }
                };
                bundle.add_resource_overriding(resource);
            }
            self.bundle = Some(bundle);
        }
        self.bundle
            .as_ref()
            .unwrap_or_else(|| unreachable!("built above"))
    }
}

/// Every language with messages, and which one is active.
struct State {
    languages: BTreeMap<String, Language>,
    /// The setting: a tag, or `None` for the system's language.
    chosen: Option<String>,
    /// The tag in use, negotiated from the setting and the system.
    active: String,
    /// What the system reports, read once.
    system: Vec<LanguageIdentifier>,
}

static STATE: LazyLock<RwLock<State>> = LazyLock::new(|| {
    let system: Vec<LanguageIdentifier> = sys_locale::get_locales()
        .filter_map(|tag| tag.parse().ok())
        .collect();
    RwLock::new(State {
        languages: BTreeMap::new(),
        chosen: None,
        active: FALLBACK.to_owned(),
        system,
    })
});

fn read() -> RwLockReadGuard<'static, State> {
    STATE
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn write() -> RwLockWriteGuard<'static, State> {
    STATE
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Adds the messages of `source`, a Fluent file's text, to `language`
/// (`en`, `de`, `pt-BR`): what the kernel and every plugin do at start. A
/// key given twice takes the later message.
pub fn add_bundled(language: &str, source: &str) {
    let Ok(tag) = language.parse::<LanguageIdentifier>() else {
        tracing::warn!(
            language,
            "a translation for a language tag that does not parse"
        );
        return;
    };
    let mut state = write();
    let entry = state
        .languages
        .entry(tag.to_string())
        .or_insert_with(|| Language::new(tag));
    entry.sources.push(source.to_owned());
    entry.bundle = None;
    renegotiate(&mut state);
}

/// Reads `<config_dir>/locales/<lang>.ftl` for every language, each file
/// overriding the bundled messages of its language key by key; a file for a
/// language nobody bundled adds that language.
pub fn load_overrides(config_dir: &Path) {
    let dir = config_dir.join("locales");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let mut state = write();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "ftl") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(tag) = stem.parse::<LanguageIdentifier>() else {
            tracing::warn!(file = %path.display(), "a translation file whose name is no language tag");
            continue;
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                tracing::info!(file = %path.display(), "translation overrides");
                let language = state
                    .languages
                    .entry(tag.to_string())
                    .or_insert_with(|| Language::new(tag));
                language.override_source = Some(text);
                language.bundle = None;
            }
            Err(error) => {
                tracing::warn!(file = %path.display(), %error, "a translation file that does not read");
            }
        }
    }
    renegotiate(&mut state);
}

/// Chooses the language: a tag such as `de`, or `None` for the system's
/// language. Takes effect at the next lookup.
pub fn set_language(choice: Option<&str>) {
    let mut state = write();
    state.chosen = choice.map(str::to_owned);
    renegotiate(&mut state);
}

/// The language in use, as a tag.
#[must_use]
pub fn language() -> String {
    read().active.clone()
}

/// The languages that have messages, as tags, sorted.
#[must_use]
pub fn available() -> Vec<String> {
    read().languages.keys().cloned().collect()
}

/// The name a language gives itself (`language-name` in its own file), or
/// its tag.
#[must_use]
pub fn language_name(tag: &str) -> String {
    let mut state = write();
    let Some(language) = state.languages.get_mut(tag) else {
        return tag.to_owned();
    };
    message_of(language, "language-name", &FluentArgs::new()).unwrap_or_else(|| tag.to_owned())
}

/// Works out the active tag from the setting and the system.
fn renegotiate(state: &mut State) {
    let available: Vec<LanguageIdentifier> = state
        .languages
        .values()
        .map(|language| language.tag.clone())
        .collect();
    let requested: Vec<LanguageIdentifier> = match &state.chosen {
        Some(choice) => choice.parse().into_iter().collect(),
        None => state.system.clone(),
    };
    state.active = negotiate(&requested, &available)
        .map_or_else(|| FALLBACK.to_owned(), |tag| tag.to_string());
}

/// The first available language a requested one matches: the whole tag
/// (`pt-BR`), else its language alone (`pt`), in the order requested.
fn negotiate(
    requested: &[LanguageIdentifier],
    available: &[LanguageIdentifier],
) -> Option<LanguageIdentifier> {
    for wanted in requested {
        if let Some(exact) = available
            .iter()
            .find(|tag| tag.matches(wanted, false, false))
        {
            return Some(exact.clone());
        }
        if let Some(language) = available.iter().find(|tag| tag.language == wanted.language) {
            return Some(language.clone());
        }
    }
    None
}

/// The message `key` formats to in `language`, when it has one.
fn message_of(language: &mut Language, key: &str, args: &FluentArgs<'_>) -> Option<String> {
    let bundle = language.bundle();
    let message = bundle.get_message(key)?;
    let pattern = message.value()?;
    let mut errors = Vec::new();
    let text = bundle.format_pattern(pattern, Some(args), &mut errors);
    for error in errors {
        tracing::debug!(key, %error, "a translation formatted with an error");
    }
    Some(text.into_owned())
}

/// The text for `key` with `args`: the active language's message, else
/// English's, else the key itself.
#[must_use]
pub fn lookup(key: &str, args: &[(&str, FluentValue<'_>)]) -> String {
    let mut fluent_args = FluentArgs::new();
    for (name, value) in args {
        fluent_args.set(*name, value.clone());
    }
    let mut state = write();
    let active = state.active.clone();
    if let Some(language) = state.languages.get_mut(&active)
        && let Some(text) = message_of(language, key, &fluent_args)
    {
        return text;
    }
    if active != FALLBACK
        && let Some(language) = state.languages.get_mut(FALLBACK)
        && let Some(text) = message_of(language, key, &fluent_args)
    {
        return text;
    }
    tracing::debug!(key, "no translation; the key stands in");
    key.to_owned()
}

/// A Fluent key that reads as its message every time it is shown: for the
/// labels an extension hands the registry once (a setting's, a connection
/// field's, an info section's), so they follow a language change too.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Text(pub &'static str);

impl Text {
    /// The message, in the language in use.
    #[must_use]
    pub fn get(self) -> String {
        lookup(self.0, &[])
    }

    /// The key.
    #[must_use]
    pub const fn key(self) -> &'static str {
        self.0
    }
}

impl fmt::Debug for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Text").field(&self.0).finish()
    }
}

impl fmt::Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.get())
    }
}

impl From<&'static str> for Text {
    fn from(key: &'static str) -> Self {
        Self(key)
    }
}

/// The user-facing text for a Fluent key, in the language in use, with
/// English as the fallback: `t!("settings-theme")`, or with arguments,
/// `t!("job-copy-items", count = 3)`. Returns a `String`.
#[macro_export]
macro_rules! t {
    ($key:expr) => {
        $crate::i18n::lookup($key, &[])
    };
    ($key:expr, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::lookup(
            $key,
            &[$((stringify!($name), $crate::i18n::FluentValue::from($value))),+],
        )
    };
}

/// Re-exported for the [`t!`](crate::t) macro's arguments.
pub use fluent_bundle::FluentValue as Value;
#[doc(hidden)]
pub use fluent_bundle::FluentValue;

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard};

    use super::*;

    /// The language is process-wide; the tests take turns.
    static TURN: Mutex<()> = Mutex::new(());

    fn turn() -> MutexGuard<'static, ()> {
        TURN.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn keys_fall_back_to_english_and_then_to_themselves() {
        let _turn = turn();
        add_bundled(
            "en",
            "i18n-test-both = Both\ni18n-test-english = English only",
        );
        add_bundled("de", "i18n-test-both = Beide");
        set_language(Some("de"));
        assert_eq!(language(), "de");
        assert_eq!(lookup("i18n-test-both", &[]), "Beide");
        assert_eq!(lookup("i18n-test-english", &[]), "English only");
        assert_eq!(lookup("i18n-test-nowhere", &[]), "i18n-test-nowhere");
        set_language(Some("en"));
        assert_eq!(lookup("i18n-test-both", &[]), "Both");
        // A language nobody bundled negotiates to English.
        set_language(Some("xx"));
        assert_eq!(language(), "en");
        set_language(Some("en"));
    }

    #[test]
    fn a_regional_tag_negotiates_to_its_language() {
        let _turn = turn();
        add_bundled("de", "i18n-test-regional = da");
        add_bundled("en", "i18n-test-regional = there");
        set_language(Some("de-CH"));
        assert_eq!(language(), "de");
        assert_eq!(lookup("i18n-test-regional", &[]), "da");
        set_language(Some("en"));
    }

    #[test]
    fn arguments_format_without_isolation_marks() {
        let _turn = turn();
        add_bundled("en", "i18n-test-args = { $count } items for { $name }");
        set_language(Some("en"));
        assert_eq!(
            crate::t!("i18n-test-args", count = 3, name = "Rick"),
            "3 items for Rick"
        );
    }

    #[test]
    fn an_override_file_wins_over_the_bundled_message() {
        let _turn = turn();
        add_bundled("en", "i18n-test-override = bundled\ni18n-test-kept = kept");
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        std::fs::create_dir(dir.path().join("locales")).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(
            dir.path().join("locales").join("en.ftl"),
            "i18n-test-override = overridden\n",
        )
        .unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(dir.path().join("locales").join("notes.txt"), "ignored")
            .unwrap_or_else(|e| panic!("{e}"));
        load_overrides(dir.path());
        set_language(Some("en"));
        assert_eq!(lookup("i18n-test-override", &[]), "overridden");
        assert_eq!(lookup("i18n-test-kept", &[]), "kept");
        // Later bundled messages do not undo the override.
        add_bundled("en", "i18n-test-override = bundled again");
        assert_eq!(lookup("i18n-test-override", &[]), "overridden");
    }

    #[test]
    fn a_text_reads_as_its_message_each_time() {
        let _turn = turn();
        add_bundled("en", "i18n-test-text = Label\nlanguage-name = English");
        add_bundled(
            "de",
            "i18n-test-text = Beschriftung\nlanguage-name = Deutsch",
        );
        let label = Text("i18n-test-text");
        set_language(Some("en"));
        assert_eq!(label.to_string(), "Label");
        set_language(Some("de"));
        assert_eq!(label.get(), "Beschriftung");
        assert_eq!(language_name("de"), "Deutsch");
        assert_eq!(language_name("en"), "English");
        assert_eq!(language_name("fr"), "fr");
        assert!(available().contains(&"de".to_owned()));
        set_language(Some("en"));
    }
}
