//! The Extensions dialogue (SPEC.md *Extensions dialogue*): master and
//! detail on the dialogue shell.
//!
//! On the left, on `bg/panel`, a search field and the plugins the host
//! found, grouped by what they contribute, each row with the plugin's
//! icon, its name and its enable switch; a disabled plugin's row is
//! dimmed and an amber dot marks one that needs attention. On the right
//! the chosen plugin: its head with the id, the version, the kind and a
//! *Built-in plugin* or *User plugin* chip, the description from its
//! manifest, an attention card when there is one, an info card whose rows
//! each have a copy button, and its actions — *Settings* when it has a
//! page, *Open folder*, and *Remove* for a user's plugin. The footer has
//! *Install from file…*, which unpacks a plugin package, and *Close*.
//! Switching, installing and removing take effect at the next start; until
//! then the row and the details carry a *restart* chip, and the footer
//! offers *Restart now*.
//!
//! A segmented control above the search field switches the list between
//! *Installed* and *Available* (specs/plugin-distribution.md §7): the
//! plugins the registry has for this Rick that are in neither plugins
//! folder, under the same headings, each row with its download size. The
//! details of one have *Install* in place of the switch, and an info card
//! with what it contributes, its size and its source. Where there is no
//! list, the view says why in one line — not set up in this build, a
//! pre-release, too old a Rick, offline, switched off, not verified — and
//! offers *Check now* where checking can help.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use app::{
    App, Available, Command, Offering, Plugin, PluginRegistry, PluginStatus, RegistryCommand,
    RegistryProblem, Update,
};
use egui::{Align, Align2, FontId, Rect, Sense, pos2, vec2};
use ext::{Level, Points, t};
use model::SettingsChange;

use crate::copy_button::CopyButton;
use crate::shell::Request;
use crate::theme::{self, ICON, TEXT_BODY, TEXT_CAPS, TEXT_SECONDARY};
use crate::widgets::{self, ButtonKind};
use crate::{dialog, format, icons};

/// The dialogue's width.
const WIDTH: f32 = 840.0;
/// The body's height where the window has the room.
const BODY: f32 = 560.0;
/// The list's width.
const LIST: f32 = 360.0;
/// A plugin row's height.
const ROW: f32 = 34.0;
/// A group heading's height.
const HEADING: f32 = 27.0;
/// The details' padding at the sides.
const PAD: f32 = 22.0;
/// The gap between the details' blocks.
const GAP: f32 = 16.0;
/// The icon tile of the details' head.
const TILE: f32 = 48.0;
/// The corner radius of the details' cards.
const CARD_RADIUS: f32 = 10.0;
/// The label column of the info card.
const LABEL: f32 = 96.0;
/// A row of the info card.
const INFO_ROW: f32 = 39.0;
/// The attention dot.
const DOT: f32 = widgets::ATTENTION_DOT;
/// The strip at the foot of the *Available* list: the list's date, or why
/// the last check gave none, and *Check now*.
const STRIP: f32 = 52.0;
/// The least width the footer's sentence is drawn in.
const FOOTER_TEXT: f32 = 150.0;
/// The width of the prompt of *Install from URL…*.
const PROMPT: f32 = 520.0;

/// Which of the dialogue's two lists is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    /// The plugins in the two plugins folders.
    #[default]
    Installed,
    /// The plugins the registry offers.
    Available,
}

/// What the dialogue is doing.
#[derive(Debug, Default)]
pub struct ExtensionsDialog {
    open: bool,
    /// The search field's text.
    filter: String,
    /// The chosen plugin, by its folder; the list's first when it is none
    /// of those listed.
    chosen: Option<PathBuf>,
    /// The file dialogue of *Install from file…*, while it is open.
    picking: Option<runtime::oneshot::Receiver<Vec<PathBuf>>>,
    /// The list scrolls to the chosen row in the next frame: a plugin was
    /// just installed, and its row may be out of view.
    reveal: bool,
    /// The list that is shown.
    tab: Tab,
    /// The chosen plugin of the *Available* list, by its id; the list's
    /// first when it is none of those listed.
    chosen_available: Option<String>,
    /// The prompt of *Install from URL…*, while it is open.
    address: Option<AddressPrompt>,
}

/// The prompt of *Install from URL…*: the address as typed so far.
#[derive(Debug, Default)]
struct AddressPrompt {
    /// The field's text.
    input: String,
    /// The field takes the focus in the next frame.
    focus: bool,
}

/// What a plugin mainly is: the heading it is listed under and the kind
/// in its details, in the order of the headings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Provider,
    Previewer,
    Action,
    Search,
    Decorator,
    /// Its manifest names no point Rick knows: one that could not be read.
    Unknown,
}

impl Kind {
    /// The main point of a plugin: the first it fills of provider,
    /// previewer, search index, action — so SSH is a provider and Git an
    /// action — and decorator for one that fills nothing else.
    fn of(points: Points) -> Self {
        if points.contains(Points::PROVIDER) {
            Self::Provider
        } else if points.contains(Points::PREVIEWER) {
            Self::Previewer
        } else if points.contains(Points::SEARCH_INDEX) {
            Self::Search
        } else if points.contains(Points::ACTION) {
            Self::Action
        } else if points.contains(Points::DECORATOR) {
            Self::Decorator
        } else {
            Self::Unknown
        }
    }

    /// The heading of the kind's group in the list.
    fn heading(self) -> String {
        match self {
            Self::Provider => t!("extensions-providers"),
            Self::Previewer => t!("extensions-previewers"),
            Self::Action => t!("extensions-actions"),
            Self::Search => t!("extensions-search"),
            Self::Decorator => t!("extensions-decorators"),
            Self::Unknown => t!("extensions-unknown"),
        }
    }

    /// The kind as the details name it: *Provider*.
    fn name(self) -> String {
        match self {
            Self::Provider => t!("extensions-provider"),
            Self::Previewer => t!("extensions-previewer"),
            Self::Action => t!("extensions-action"),
            Self::Search => t!("extensions-search"),
            Self::Decorator => t!("extensions-decorator"),
            Self::Unknown => t!("extensions-plugin"),
        }
    }
}

/// A plugin's name as its row shows it: the manifest's, or its id when
/// the manifest gave none.
fn name_of(plugin: &Plugin) -> &str {
    if plugin.name.is_empty() {
        &plugin.id
    } else {
        &plugin.name
    }
}

/// Why a plugin needs attention, as its card says it: the host refused
/// or failed to load it, its previewer reported a third-party library
/// missing in this session, or it has disabled itself because what it
/// connects to is not on the machine and said so (`ext::attention`), which
/// gives the plugin's own reason. `None` for a plugin that is in order.
fn attention(app: &App, plugin: &Plugin) -> Option<String> {
    if let PluginStatus::Refused(reason) = &plugin.status {
        return Some(t!("extensions-refused", reason = reason.as_str()));
    }
    app.missing_library(&plugin.id)
        .map(|library| t!("extensions-missing-library", name = library))
        .or_else(|| ext::attention::of(&plugin.id))
}

/// Whether the plugin is switched on: not on the settings' disabled list.
fn enabled(app: &App, plugin: &Plugin) -> bool {
    !app.model().settings.plugins.disabled.contains(&plugin.id)
}

/// Whether something about the plugin waits for the next start: it was
/// installed or removed in this session, or its switch no longer says what
/// the host did at start.
fn waits_for_restart(app: &App, plugin: &Plugin) -> bool {
    plugin.removed
        || plugin.status == PluginStatus::Installed
        || enabled(app, plugin) == (plugin.status == PluginStatus::Disabled)
}

/// What the plugin fills its points with: the points in the order of the
/// kinds, a provider with the schemes of its locations and a previewer
/// with its file types — *Provider · azure://*.
fn contributes(app: &App, plugin: &Plugin) -> String {
    let mut parts = Vec::new();
    let detail = |name: String, of: String| {
        if of.is_empty() {
            name
        } else {
            t!("extensions-point-of", point = name, of = of)
        }
    };
    if plugin.points.contains(Points::PROVIDER) {
        parts.push(detail(
            t!("extensions-provider"),
            app.plugin_schemes(&plugin.id).join(", "),
        ));
    }
    if plugin.points.contains(Points::PREVIEWER) {
        parts.push(detail(
            t!("extensions-previewer"),
            plugin.previews.join(", "),
        ));
    }
    if plugin.points.contains(Points::SEARCH_INDEX) {
        parts.push(t!("extensions-search"));
    }
    if plugin.points.contains(Points::ACTION) {
        parts.push(t!("extensions-action"));
    }
    if plugin.points.contains(Points::DECORATOR) {
        parts.push(t!("extensions-decorator"));
    }
    parts.join("; ")
}

/// The plugin's state as the info card says it: *Loaded*, *Disabled*, what
/// stopped it, or what the next start will find.
fn status(plugin: &Plugin) -> String {
    if plugin.removed {
        return t!("extensions-status-removed");
    }
    match &plugin.status {
        PluginStatus::Loaded => t!("extensions-status-loaded"),
        PluginStatus::Disabled => t!("extensions-status-disabled"),
        PluginStatus::Refused(reason) => reason.clone(),
        PluginStatus::Installed => t!("extensions-status-installed"),
    }
}

/// What a frame of the dialogue asked for.
enum Ask {
    Close,
    Show(Tab),
    ChooseAvailable(String),
    InstallAvailable(String),
    UpdateAll,
    CheckNow,
    Restart,
    Choose(PathBuf),
    Toggle(String, bool),
    Copy(String),
    Settings(String),
    OpenFolder(PathBuf),
    Remove(PathBuf),
    Install,
    /// *Install from URL…* was pressed: open its prompt.
    PromptAddress,
    /// The prompt's *Install*: the address to install from.
    InstallFrom(String),
    /// The prompt was dismissed.
    DropAddress,
}

impl ExtensionsDialog {
    /// Opens the dialogue with an empty search, on the installed plugins
    /// and the one it showed last.
    pub fn open(&mut self) {
        self.open = true;
        self.filter.clear();
        self.tab = Tab::Installed;
    }

    /// Whether the dialogue is open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// A plugin package was unpacked into `path`: its row, under
    /// *Installed*, is the chosen one.
    pub fn installed(&mut self, path: PathBuf) {
        self.filter.clear();
        self.tab = Tab::Installed;
        self.chosen = Some(path);
        self.reveal = true;
    }

    /// The dialogue, when it is open.
    pub fn show(&mut self, ctx: &egui::Context, app: &App, requests: &mut Vec<Request>) {
        self.poll_package(requests);
        if !self.open {
            return;
        }
        // The list: what the search leaves, by kind and then by name.
        let needle = self.filter.trim().to_lowercase();
        let found = |name: &str, id: &str| {
            needle.is_empty()
                || name.to_lowercase().contains(&needle)
                || id.to_lowercase().contains(&needle)
        };
        let mut listed: Vec<&Plugin> = app
            .plugins()
            .iter()
            .filter(|plugin| found(name_of(plugin), &plugin.id))
            .collect();
        listed.sort_by_key(|plugin| (Kind::of(plugin.points), name_of(plugin).to_lowercase()));
        let chosen = listed
            .iter()
            .find(|plugin| Some(&plugin.path) == self.chosen.as_ref())
            .or(listed.first())
            .copied();
        // The other list: what the registry offers, in the same order.
        let offering = app.offering();
        let on_offer: &[Available] = match &offering {
            Offering::Plugins(plugins) => plugins,
            _ => &[],
        };
        let mut available: Vec<&Available> = on_offer
            .iter()
            .filter(|plugin| found(&plugin.name, &plugin.id))
            .collect();
        available.sort_by_key(|plugin| (Kind::of(plugin.points), plugin.name.to_lowercase()));
        let chosen_available = available
            .iter()
            .find(|plugin| Some(&plugin.id) == self.chosen_available.as_ref())
            .or(available.first())
            .copied();
        let lists = Lists {
            listed: &listed,
            chosen,
            available: &available,
            chosen_available,
            on_offer: on_offer.len(),
            // The strip stands under a list that is there to be dated.
            strip: matches!(offering, Offering::Plugins(_)),
        };
        let restart = app
            .plugins()
            .iter()
            .any(|plugin| waits_for_restart(app, plugin));
        // The plugins the registry has this Rick's build of.
        let updates = app.updates();

        let mut asked = None;
        let modal = dialog::modal(ctx, egui::Id::new("extensions"), WIDTH, |ui| {
            let scheme = theme::scheme(ui);
            if dialog::band(ui, None, &t!("extensions"), &counts(app, updates.len())) {
                asked = Some(Ask::Close);
            }
            // Header and footer stay in view in a low window.
            let room = ui.ctx().content_rect().height() - 64.0 - 68.0 - 48.0;
            let height = BODY.min(room).max(ROW * 6.0);
            let (body, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
            let split = body.left() + LIST;
            let left = Rect::from_min_max(body.min, pos2(split, body.bottom()));
            let right = Rect::from_min_max(pos2(split, body.top()), body.max);
            ui.painter().rect_filled(left, 0.0, scheme.bg_panel);
            ui.painter()
                .vline(split - 0.5, body.y_range(), scheme.hairline());
            self.list(ui, left, app, &lists, &mut asked);
            match (self.tab, chosen, chosen_available) {
                (Tab::Installed, Some(plugin), _) => {
                    let update = updates.iter().find(|update| update.path == plugin.path);
                    details(ui, right, app, plugin, update, &mut asked);
                }
                (Tab::Installed, None, _) => {
                    let text = if app.plugins().is_empty() {
                        t!("settings-no-plugins-found")
                    } else {
                        t!("extensions-no-match")
                    };
                    nothing(ui, right, &text, false, &mut asked);
                }
                (Tab::Available, _, Some(plugin)) => {
                    available_details(ui, right, app, plugin, &mut asked);
                }
                (Tab::Available, _, None) => {
                    let (text, check) = no_offer(app, &offering, on_offer.is_empty());
                    nothing(ui, right, &text, check, &mut asked);
                }
            }
            if let Some(ask) = footer(ui, app, restart, !updates.is_empty()) {
                asked = Some(ask);
            }
        });
        // The prompt of *Install from URL…* stands over the dialogue, and
        // is what Escape and a click beside it close.
        if let Some(prompt) = &mut self.address {
            asked = address_prompt(ctx, prompt);
        } else if modal.should_close() {
            asked = Some(Ask::Close);
        }
        self.act(asked, app, requests);
    }

    /// Does what the frame asked for.
    fn act(&mut self, asked: Option<Ask>, app: &App, requests: &mut Vec<Request>) {
        let toast = |requests: &mut Vec<Request>, error: &dyn std::fmt::Display| {
            requests.push(Request::Toast(Level::Error, error.to_string()));
        };
        match asked {
            None => {}
            Some(Ask::Close) => self.open = false,
            Some(Ask::Show(tab)) => {
                if tab == Tab::Available && self.tab != tab {
                    // Opening the view is an occasion to read the list.
                    requests.push(Request::Command(Command::Registry(RegistryCommand::Opened)));
                }
                self.tab = tab;
            }
            Some(Ask::ChooseAvailable(id)) => self.chosen_available = Some(id),
            Some(Ask::InstallAvailable(id)) => {
                requests.push(Request::Command(Command::InstallAvailable(id)));
            }
            Some(Ask::UpdateAll) => requests.push(Request::Command(Command::UpdateAll)),
            Some(Ask::CheckNow) => {
                requests.push(Request::Command(Command::Registry(
                    RegistryCommand::CheckNow,
                )));
            }
            Some(Ask::Restart) => requests.push(Request::Restart),
            Some(Ask::Choose(path)) => self.chosen = Some(path),
            Some(Ask::Toggle(id, enabled)) => {
                requests.push(Request::Command(Command::Settings(
                    SettingsChange::Plugin { id, enabled },
                )));
            }
            Some(Ask::Copy(text)) => {
                if let Err(error) = app.platform().set_text(&text) {
                    toast(requests, &error);
                }
            }
            Some(Ask::Settings(id)) => {
                self.open = false;
                requests.push(Request::PluginSettings(id));
            }
            Some(Ask::OpenFolder(folder)) => {
                if let Err(error) = app.platform().open(&folder) {
                    toast(requests, &error);
                }
            }
            Some(Ask::Remove(path)) => {
                requests.push(Request::Command(Command::RemovePlugin(path)));
            }
            Some(Ask::Install) => self.pick_package(app),
            Some(Ask::PromptAddress) => {
                self.address = Some(AddressPrompt {
                    input: String::new(),
                    focus: true,
                });
            }
            Some(Ask::InstallFrom(url)) => {
                self.address = None;
                requests.push(Request::Command(Command::InstallPluginFrom(url)));
            }
            Some(Ask::DropAddress) => self.address = None,
        }
    }

    /// Opens the platform's file dialogue for a plugin package; the answer
    /// is polled each frame.
    fn pick_package(&mut self, app: &App) {
        if self.picking.is_some() {
            return;
        }
        let platform = Arc::clone(app.platform());
        let title = t!("extensions-install-title");
        let filters = [app::plugins::PACKAGE_FILTER];
        let (tx, rx) = runtime::oneshot::channel();
        runtime::handle().spawn(async move {
            let _ = tx.send(platform.pick_files(&title, &filters).await);
        });
        self.picking = Some(rx);
    }

    /// The package the file dialogue picked is installed; closing the
    /// dialogue without one installs nothing.
    fn poll_package(&mut self, requests: &mut Vec<Request>) {
        let Some(picking) = self.picking.as_mut() else {
            return;
        };
        match picking.try_recv() {
            Ok(files) => {
                self.picking = None;
                if let Some(package) = files.into_iter().next() {
                    requests.push(Request::Command(Command::InstallPlugin(package)));
                }
            }
            Err(runtime::oneshot::error::TryRecvError::Empty) => {}
            Err(runtime::oneshot::error::TryRecvError::Closed) => self.picking = None,
        }
    }

    /// The list: the *Installed · Available* control, the search field,
    /// then the plugins of the chosen list under the heading of their
    /// kind, and under the *Available* list its strip.
    fn list(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        app: &App,
        lists: &Lists<'_>,
        asked: &mut Option<Ask>,
    ) {
        let tabs = Rect::from_min_max(
            pos2(rect.left() + 12.0, rect.top() + 12.0),
            pos2(rect.right() - 13.0, rect.top() + 42.0),
        );
        // The control counts what is on offer: *Available 7*.
        let available = if lists.on_offer > 0 {
            t!("extensions-tab-available-count", count = lists.on_offer)
        } else {
            t!("extensions-tab-available")
        };
        let options = [t!("extensions-tab-installed"), available];
        let picked = widgets::segmented(
            ui,
            tabs,
            ui.id().with("extension-tabs"),
            &options,
            usize::from(self.tab == Tab::Available),
        );
        match picked {
            Some(0) => *asked = Some(Ask::Show(Tab::Installed)),
            Some(_) => *asked = Some(Ask::Show(Tab::Available)),
            None => {}
        }
        let field = Rect::from_min_max(
            pos2(rect.left() + 12.0, tabs.bottom() + 8.0),
            pos2(rect.right() - 13.0, tabs.bottom() + 40.0),
        );
        search_field(ui, field, &mut self.filter);
        let strip = self.tab == Tab::Available && lists.strip;
        let bottom = if strip {
            rect.bottom() - STRIP
        } else {
            rect.bottom()
        };
        if strip {
            let foot = Rect::from_min_max(pos2(rect.left(), bottom), rect.max);
            list_strip(ui, foot, app, asked);
        }
        let below = Rect::from_min_max(
            pos2(rect.left(), field.bottom() + 8.0),
            pos2(rect.right(), bottom),
        );
        let mut area = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(below)
                .layout(egui::Layout::top_down(Align::Min)),
        );
        area.set_clip_rect(below.intersect(ui.clip_rect()));
        egui::ScrollArea::vertical()
            .id_salt(("extension-rows", self.tab == Tab::Available))
            .auto_shrink([false, false])
            .show(&mut area, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                let mut heading = None;
                let mut under = |ui: &mut egui::Ui, kind: Kind| {
                    if heading != Some(kind) {
                        heading = Some(kind);
                        let (line, _) = ui.allocate_exact_size(
                            vec2(ui.available_width(), HEADING),
                            Sense::hover(),
                        );
                        let mut at =
                            ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(
                                pos2(line.left() + 18.0, line.top() + 10.0),
                                line.max,
                            )));
                        widgets::caps_heading(&mut at, &kind.heading());
                    }
                };
                match self.tab {
                    Tab::Installed => {
                        for plugin in lists.listed {
                            under(ui, Kind::of(plugin.points));
                            let is_chosen = lists.chosen.is_some_and(|c| c.path == plugin.path);
                            let line = row(ui, app, plugin, is_chosen, asked);
                            if is_chosen && std::mem::take(&mut self.reveal) {
                                ui.scroll_to_rect(line, Some(Align::Center));
                            }
                        }
                    }
                    Tab::Available => {
                        // Nothing is downloaded in a packaged Rick, so no
                        // row has a download size.
                        let sized = app.packaging().is_none();
                        for plugin in lists.available {
                            under(ui, Kind::of(plugin.points));
                            let is_chosen =
                                lists.chosen_available.is_some_and(|c| c.id == plugin.id);
                            available_row(ui, plugin, is_chosen, sized, asked);
                        }
                    }
                }
                ui.add_space(8.0);
            });
    }
}

/// The dialogue's footer: where the plugins live at the left — or, while
/// `rick.toml` names another index or key than this build's own, where
/// the plugin list is read from — and at the right **Update all** while
/// any plugin has an update (`updates`), *Restart now* while anything
/// waits for the next start (`restart`), *Install from file…* and
/// *Close*.
fn footer(ui: &mut egui::Ui, app: &App, restart: bool, updates: bool) -> Option<Ask> {
    let scheme = theme::scheme(ui);
    let asked = std::cell::Cell::new(None);
    dialog::footer(
        ui,
        |left| {
            // The buttons come first: the sentence stands where they
            // leave it the room of a few words a line.
            if left.available_width() < FOOTER_TEXT {
                return;
            }
            let text = match &app.plugin_registry().custom {
                Some(address) => t!("extensions-registry-address", address = address.as_str()),
                None => t!("extensions-where"),
            };
            left.add(
                egui::Label::new(
                    egui::RichText::new(text)
                        .size(TEXT_SECONDARY)
                        .color(scheme.text_secondary),
                )
                .wrap()
                .selectable(false),
            );
        },
        |right| {
            if widgets::button(right, &t!("close"), true, true).clicked() {
                asked.set(Some(Ask::Close));
            }
            let install = t!("extensions-install");
            if widgets::icon_button(right, Some(icons::INSTALL), &install, false, true).clicked() {
                asked.set(Some(Ask::Install));
            }
            let from = t!("extensions-install-url");
            if widgets::icon_button(right, Some(icons::LINK), &from, false, true).clicked() {
                asked.set(Some(Ask::PromptAddress));
            }
            if restart
                && widgets::icon_button(
                    right,
                    Some(icons::REFRESH),
                    &t!("extensions-restart-now"),
                    false,
                    true,
                )
                .clicked()
            {
                asked.set(Some(Ask::Restart));
            }
            if updates
                && widgets::icon_button(
                    right,
                    Some(icons::INSTALL),
                    &t!("extensions-update-all"),
                    false,
                    true,
                )
                .clicked()
            {
                asked.set(Some(Ask::UpdateAll));
            }
        },
    );
    asked.into_inner()
}

/// The prompt of *Install from URL…*: a field for the `https://` address
/// of a plugin package, focused when it opens, and under it what is
/// wrong with a text that is none. *Install* — and Enter — take an
/// address; Escape, *Cancel* and a click beside the prompt dismiss it.
/// The footer says that nothing is checked, as for a file.
fn address_prompt(ctx: &egui::Context, prompt: &mut AddressPrompt) -> Option<Ask> {
    let mut asked = None;
    let typed = prompt.input.trim().to_owned();
    let valid = app::plugins::is_package_address(&typed);
    let modal = dialog::modal(ctx, egui::Id::new("extensions-address"), PROMPT, |ui| {
        let scheme = theme::scheme(ui);
        if dialog::band(
            ui,
            Some(icons::LINK),
            &t!("extensions-url-title"),
            &t!("extensions-url-subtitle"),
        ) {
            asked = Some(Ask::DropAddress);
        }
        dialog::body(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 8.0);
            widgets::caps_heading(ui, &t!("extensions-url-heading"));
            // A text that is no address is said once something is typed.
            let wrong = !typed.is_empty() && !valid;
            let field = dialog::text_field(
                ui,
                &mut prompt.input,
                "https://",
                true,
                wrong.then_some(scheme.status_warning_border),
                std::mem::take(&mut prompt.focus),
            );
            if wrong {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(t!("extensions-url-invalid"))
                            .size(TEXT_SECONDARY)
                            .color(scheme.status_warning),
                    )
                    .selectable(false),
                );
            }
            if valid && field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                asked = Some(Ask::InstallFrom(typed.clone()));
            }
        });
        let footer = std::cell::Cell::new(None);
        dialog::footer(
            ui,
            |left| {
                left.add(
                    egui::Label::new(
                        egui::RichText::new(t!("extensions-url-note"))
                            .size(TEXT_SECONDARY)
                            .color(scheme.text_secondary),
                    )
                    .wrap()
                    .selectable(false),
                );
            },
            |right| {
                let install = t!("extensions-install-available");
                if widgets::icon_button(right, Some(icons::INSTALL), &install, true, valid)
                    .clicked()
                    && valid
                {
                    footer.set(Some(Ask::InstallFrom(typed.clone())));
                }
                if widgets::button(right, &t!("cancel"), false, true).clicked() {
                    footer.set(Some(Ask::DropAddress));
                }
            },
        );
        if let Some(ask) = footer.into_inner() {
            asked = Some(ask);
        }
    });
    if modal.should_close() && asked.is_none() {
        asked = Some(Ask::DropAddress);
    }
    asked
}

/// What stands in place of **Install** in a Rick a package manager
/// installed: the line *Installed through your package manager* and,
/// under it, the command that installs the plugin, monospace with a copy
/// button.
fn packaged_card(ui: &mut egui::Ui, command: &str, asked: &mut Option<Ask>) {
    let scheme = theme::scheme(ui);
    egui::Frame::new()
        .fill(scheme.bg_card)
        .stroke(egui::Stroke::new(1.0, scheme.border_default))
        .corner_radius(CARD_RADIUS)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            ui.set_width(ui.available_width());
            let (head, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), INFO_ROW), Sense::hover());
            widgets::line(
                ui,
                pos2(head.left() + 14.0, head.center().y),
                &t!("extensions-packaged"),
                FontId::proportional(TEXT_SECONDARY),
                scheme.text_secondary,
                head.width() - 28.0,
            );
            let (line, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), INFO_ROW), Sense::hover());
            ui.painter()
                .hline(line.x_range(), line.top() + 0.5, scheme.hairline());
            let button = Rect::from_center_size(
                pos2(line.right() - 6.0 - 11.0, line.center().y),
                vec2(22.0, 22.0),
            );
            let left = line.left() + 14.0;
            widgets::line(
                ui,
                pos2(left, line.center().y),
                command,
                FontId::monospace(TEXT_SECONDARY),
                scheme.text_primary,
                button.left() - 12.0 - left,
            );
            let name = t!("extensions-copy", what = t!("extensions-command"));
            if CopyButton::new(&name).show_at(ui, button, &name).clicked() {
                *asked = Some(Ask::Copy(command.to_owned()));
            }
        });
}

/// The two lists of a frame, as the search left them.
struct Lists<'a> {
    /// The installed plugins, in the list's order.
    listed: &'a [&'a Plugin],
    /// The chosen one of them.
    chosen: Option<&'a Plugin>,
    /// The plugins on offer, in the list's order.
    available: &'a [&'a Available],
    /// The chosen one of them.
    chosen_available: Option<&'a Available>,
    /// How many plugins are on offer, whatever the search says.
    on_offer: usize,
    /// The *Available* list has its strip: there is a plugin list.
    strip: bool,
}

/// Why the last check gave no plugin list, in the view's words.
fn problem_line(problem: RegistryProblem) -> String {
    match problem {
        RegistryProblem::Offline => t!("extensions-offline"),
        RegistryProblem::Unverified => t!("extensions-unverified"),
        RegistryProblem::NewerRick => t!("extensions-newer-rick"),
        RegistryProblem::Unreadable => t!("extensions-unreadable"),
    }
}

/// What the *Available* view says in place of a plugin's details, and
/// whether *Check now* stands under it: why there is no list, or that
/// nothing is left to offer. `none_at_all` is the list being empty before
/// the search.
fn no_offer(app: &App, offering: &Offering, none_at_all: bool) -> (String, bool) {
    let registry = app.plugin_registry();
    let version = registry.rick.as_str();
    match offering {
        Offering::NotSetUp => (t!("extensions-not-set-up"), false),
        Offering::PreRelease => (t!("extensions-pre-release", version = version), false),
        Offering::TooOld => (t!("extensions-too-old", version = version), false),
        Offering::Nothing if registry.checking => (t!("extensions-checking"), false),
        Offering::Nothing => {
            let text = match registry.problem {
                Some(problem) => problem_line(problem),
                // Nothing cached, and nothing went wrong: nothing was
                // asked, because the switch is off.
                None => t!("extensions-check-off"),
            };
            (text, true)
        }
        Offering::Plugins(_) if none_at_all => {
            (t!("extensions-none-available", version = version), false)
        }
        Offering::Plugins(_) => (t!("extensions-no-match"), false),
    }
}

/// The strip under the *Available* list: why the last check gave no list
/// — the one shown is then the last good one — or the date of the list,
/// and *Check now*.
fn list_strip(ui: &mut egui::Ui, rect: Rect, app: &App, asked: &mut Option<Ask>) {
    let scheme = theme::scheme(ui);
    let registry: &PluginRegistry = app.plugin_registry();
    ui.painter()
        .hline(rect.x_range(), rect.top() + 0.5, scheme.hairline());
    let text = if registry.checking {
        t!("extensions-checking")
    } else if let Some(problem) = registry.problem {
        problem_line(problem)
    } else {
        let date = format::date(
            registry.fetched,
            std::time::SystemTime::now(),
            app.model().settings.date_format,
        )
        .0;
        t!("extensions-list-of", date = date)
    };
    let mut right = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("extension-strip")
            .max_rect(rect.shrink2(vec2(13.0, 0.0)))
            .layout(egui::Layout::right_to_left(Align::Center)),
    );
    let check = widgets::card_button(
        &mut right,
        None,
        &t!("extensions-check-now"),
        ButtonKind::Secondary,
    );
    if check.clicked() && !registry.checking {
        *asked = Some(Ask::CheckNow);
    }
    let left = rect.left() + 18.0;
    widgets::line(
        ui,
        pos2(left, rect.center().y),
        &text,
        FontId::proportional(TEXT_SECONDARY),
        scheme.text_secondary,
        check.rect.left() - 10.0 - left,
    );
}

/// The header's secondary line: how many plugins there are, how many are
/// switched on, how many need attention and, while there are any, how
/// many have an update — *2 updates*.
fn counts(app: &App, updates: usize) -> String {
    let plugins: Vec<&Plugin> = app.plugins().iter().filter(|p| !p.removed).collect();
    let on = plugins.iter().filter(|p| enabled(app, p)).count();
    let attention = plugins
        .iter()
        .filter(|p| attention(app, p).is_some())
        .count();
    let mut parts = vec![
        t!("extensions-installed", count = plugins.len()),
        t!("extensions-enabled", count = on),
        t!("extensions-attention", count = attention),
    ];
    if updates > 0 {
        parts.push(t!("extensions-updates", count = updates));
    }
    parts.join("  ·  ")
}

/// The search field of the list: a magnifier and the text, on `bg/card`.
fn search_field(ui: &mut egui::Ui, field: Rect, text: &mut String) {
    let scheme = theme::scheme(ui);
    ui.painter().rect(
        field,
        theme::CARD_RADIUS,
        scheme.bg_card,
        egui::Stroke::new(1.0, scheme.border_strong),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        pos2(field.left() + 11.0 + ICON / 2.0, field.center().y),
        Align2::CENTER_CENTER,
        icons::SEARCH,
        FontId::proportional(ICON),
        scheme.icon_default,
    );
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(
                pos2(field.left() + 35.0, field.top()),
                pos2(field.right() - 10.0, field.bottom()),
            ))
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    let font = FontId::proportional(TEXT_BODY);
    inner.add(
        egui::TextEdit::singleline(text)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .font(font.clone())
            .text_color(scheme.text_primary)
            .hint_text(
                egui::RichText::new(t!("extensions-search-hint"))
                    .font(font)
                    .color(scheme.text_placeholder),
            )
            .desired_width(f32::INFINITY)
            .vertical_align(Align::Center),
    );
}

/// The width a [`chip`] of `text` takes.
fn chip_width(ui: &egui::Ui, text: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(
            text.to_owned(),
            FontId::proportional(TEXT_CAPS),
            egui::Color32::PLACEHOLDER,
        )
        .size()
        .x
        + 14.0
}

/// A small chip: `text` in 11 px `text/secondary` on `bg/subtle`, at
/// `left`, centred on `y`. Returns its right edge.
fn chip(ui: &mut egui::Ui, left: f32, y: f32, text: &str) -> f32 {
    let scheme = theme::scheme(ui);
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        FontId::proportional(TEXT_CAPS),
        scheme.text_secondary,
    );
    let rect = Rect::from_min_size(pos2(left, y - 8.5), vec2(galley.size().x + 14.0, 17.0));
    ui.painter().rect_filled(rect, 4.0, scheme.bg_subtle);
    ui.painter().galley(
        pos2(rect.left() + 7.0, rect.center().y - galley.size().y / 2.0),
        galley,
        scheme.text_secondary,
    );
    let response = ui.interact(
        rect,
        ui.id().with(("chip", text, rect.top().to_bits())),
        Sense::hover(),
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text.to_owned()));
    rect.right()
}

/// The enable switch of a plugin at `center`, named after the plugin; a
/// click asks for the other state. A plugin whose folder is gone has
/// none.
fn switch(
    ui: &mut egui::Ui,
    center: egui::Pos2,
    app: &App,
    plugin: &Plugin,
    place: &str,
    asked: &mut Option<Ask>,
) {
    if plugin.removed {
        return;
    }
    let on = enabled(app, plugin);
    let rect = Rect::from_center_size(center, vec2(32.0, 18.0));
    let mut at = ui.new_child(
        egui::UiBuilder::new()
            .id_salt((place, &plugin.path))
            .max_rect(rect),
    );
    let name = t!("extensions-enable", name = name_of(plugin));
    if widgets::toggle(&mut at, on, &name).clicked() {
        *asked = Some(Ask::Toggle(plugin.id.clone(), !on));
    }
}

/// One row of the list, 34 px: the plugin's icon, its name, the *restart*
/// chip and the attention dot when they apply, and its switch. Returns
/// where the row is.
fn row(
    ui: &mut egui::Ui,
    app: &App,
    plugin: &Plugin,
    chosen: bool,
    asked: &mut Option<Ask>,
) -> Rect {
    let scheme = theme::scheme(ui);
    let radius = theme::metrics(ui.ctx()).corner_radius();
    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
    let inner = Rect::from_min_max(
        pos2(line.left() + 8.0, line.top()),
        pos2(line.right() - 9.0, line.bottom()),
    );
    let name = name_of(plugin);
    let response = ui.interact(inner, ui.id().with(("row", &plugin.path)), Sense::click());
    if chosen {
        ui.painter()
            .rect_filled(inner, radius, scheme.bg_selected_soft);
    } else if response.hovered() {
        ui.painter().rect_filled(inner, radius, scheme.bg_subtle);
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, chosen, name.to_owned())
    });
    if response.clicked() {
        *asked = Some(Ask::Choose(plugin.path.clone()));
    }
    // A plugin that is switched off, or gone, is dimmed.
    let active = enabled(app, plugin) && !plugin.removed;
    let (icon, text) = if active {
        (scheme.icon_default, scheme.text_primary)
    } else {
        (
            scheme.icon_default.gamma_multiply(0.4),
            scheme.text_secondary,
        )
    };
    ui.painter().text(
        pos2(inner.left() + 10.0 + ICON / 2.0, inner.center().y),
        Align2::CENTER_CENTER,
        icons::for_plugin(&plugin.id),
        FontId::proportional(ICON),
        icon,
    );
    // From the right: the switch, the dot, the chip; the name has the rest.
    let mut right = inner.right() - 10.0;
    switch(
        ui,
        pos2(right - 16.0, inner.center().y),
        app,
        plugin,
        "row",
        asked,
    );
    right -= 32.0 + 10.0;
    if attention(app, plugin).is_some() {
        widgets::attention_dot(
            ui,
            pos2(right - DOT / 2.0, inner.center().y),
            ui.id().with(("dot", &plugin.path)),
            &t!("extensions-needs-attention"),
        );
        right -= DOT + 10.0;
    }
    if waits_for_restart(app, plugin) {
        let label = t!("extensions-restart");
        let width = chip_width(ui, &label);
        chip(ui, right - width, inner.center().y, &label);
        right -= width + 10.0;
    }
    let left = inner.left() + 36.0;
    let galley = widgets::elided(
        ui,
        name,
        FontId::proportional(TEXT_BODY),
        text,
        right - left,
    );
    ui.painter().galley(
        pos2(left, inner.center().y - galley.size().y / 2.0),
        galley,
        text,
    );
    line
}

/// The details when no plugin is listed: `text`, one line or a few,
/// centred, and *Check now* under it when `check` says checking can help.
fn nothing(ui: &mut egui::Ui, rect: Rect, text: &str, check: bool, asked: &mut Option<Ask>) {
    let scheme = theme::scheme(ui);
    let width = rect.width() - 2.0 * PAD;
    let galley = ui.painter().layout(
        text.to_owned(),
        FontId::proportional(TEXT_BODY),
        scheme.text_secondary,
        width,
    );
    let button = if check { 32.0 + 14.0 } else { 0.0 };
    let top = rect.center().y - f32::midpoint(galley.size().y, button);
    let at = Rect::from_min_size(
        pos2(rect.center().x - galley.size().x / 2.0, top),
        galley.size(),
    );
    ui.painter().galley(at.min, galley, scheme.text_secondary);
    ui.interact(at, ui.id().with("extensions-nothing"), Sense::hover())
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text.to_owned()));
    if !check {
        return;
    }
    let label = t!("extensions-check-now");
    let mut below = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("extensions-nothing-check")
            .max_rect(Rect::from_min_max(
                pos2(rect.left(), at.bottom() + 14.0),
                pos2(rect.right(), at.bottom() + 14.0 + 32.0),
            ))
            .layout(egui::Layout::top_down(Align::Center)),
    );
    if widgets::card_button(&mut below, None, &label, ButtonKind::Secondary).clicked() {
        *asked = Some(Ask::CheckNow);
    }
}

/// One row of the *Available* list, 34 px: the icon of the plugin's kind,
/// its name and, where an installed plugin has its switch, the size of
/// the download when `sized` says there is one.
fn available_row(
    ui: &mut egui::Ui,
    plugin: &Available,
    chosen: bool,
    sized: bool,
    asked: &mut Option<Ask>,
) {
    let scheme = theme::scheme(ui);
    let radius = theme::metrics(ui.ctx()).corner_radius();
    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
    let inner = Rect::from_min_max(
        pos2(line.left() + 8.0, line.top()),
        pos2(line.right() - 9.0, line.bottom()),
    );
    let response = ui.interact(
        inner,
        ui.id().with(("available", &plugin.id)),
        Sense::click(),
    );
    if chosen {
        ui.painter()
            .rect_filled(inner, radius, scheme.bg_selected_soft);
    } else if response.hovered() {
        ui.painter().rect_filled(inner, radius, scheme.bg_subtle);
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::RadioButton,
            true,
            chosen,
            plugin.name.clone(),
        )
    });
    if response.clicked() {
        *asked = Some(Ask::ChooseAvailable(plugin.id.clone()));
    }
    ui.painter().text(
        pos2(inner.left() + 10.0 + ICON / 2.0, inner.center().y),
        Align2::CENTER_CENTER,
        icons::for_plugin(&plugin.id),
        FontId::proportional(ICON),
        scheme.icon_default,
    );
    // The size stands where the switch does; the name has the rest.
    let mut right = inner.right();
    if sized {
        let galley = ui.painter().layout_no_wrap(
            format::size(Some(plugin.size)),
            FontId::proportional(TEXT_SECONDARY),
            scheme.text_secondary,
        );
        right -= 10.0 + galley.size().x;
        let at = Rect::from_min_size(
            pos2(right, inner.center().y - galley.size().y / 2.0),
            galley.size(),
        );
        let spoken = galley.text().to_owned();
        ui.painter().galley(at.min, galley, scheme.text_secondary);
        ui.interact(at, ui.id().with(("size", &plugin.id)), Sense::hover())
            .widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Label, true, spoken.clone())
            });
    }
    let left = inner.left() + 36.0;
    let name = widgets::elided(
        ui,
        &plugin.name,
        FontId::proportional(TEXT_BODY),
        scheme.text_primary,
        right - 10.0 - left,
    );
    ui.painter().galley(
        pos2(left, inner.center().y - name.size().y / 2.0),
        name,
        scheme.text_primary,
    );
}

/// What an available plugin will fill its points with, from what the
/// plugin list says it is for: a provider with its kinds of connection, a
/// previewer with its file types, and the entries that make a folder its
/// business — *Action; Decorator; folders with .git*.
fn offers(plugin: &Available) -> String {
    let mut parts = Vec::new();
    let detail = |name: String, of: String| {
        if of.is_empty() {
            name
        } else {
            t!("extensions-point-of", point = name, of = of)
        }
    };
    if plugin.points.contains(Points::PROVIDER) {
        parts.push(detail(
            t!("extensions-provider"),
            plugin.connections.join(", "),
        ));
    }
    if plugin.points.contains(Points::PREVIEWER) {
        parts.push(detail(
            t!("extensions-previewer"),
            plugin.previews.join(", "),
        ));
    }
    if plugin.points.contains(Points::SEARCH_INDEX) {
        parts.push(t!("extensions-search"));
    }
    if plugin.points.contains(Points::ACTION) {
        parts.push(t!("extensions-action"));
    }
    if plugin.points.contains(Points::DECORATOR) {
        parts.push(t!("extensions-decorator"));
    }
    if !plugin.markers.is_empty() {
        parts.push(t!(
            "extensions-folders-with",
            names = plugin.markers.join(", ")
        ));
    }
    parts.join("; ")
}

/// The details of the chosen plugin of the *Available* list: the head as
/// an installed plugin has it, with the chip *Available* and **Install**
/// in place of the switch, the description, and an info card with
/// *Contributes*, *Size* and *Source*, the registry's host.
fn available_details(
    ui: &mut egui::Ui,
    rect: Rect,
    app: &App,
    plugin: &Available,
    asked: &mut Option<Ask>,
) {
    let scheme = theme::scheme(ui);
    let inner = rect.shrink2(vec2(PAD, 20.0));
    let mut column = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("available-details", &plugin.id))
            .max_rect(inner)
            .layout(egui::Layout::top_down(Align::Min)),
    );
    column.set_clip_rect(inner.intersect(ui.clip_rect()));
    egui::ScrollArea::vertical()
        .id_salt("available-details")
        .auto_shrink([false, false])
        .show(&mut column, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, GAP);
            ui.set_width(inner.width());
            let packaging = app.packaging();
            available_head(ui, plugin, packaging.is_none(), asked);
            if !plugin.description.is_empty() {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&plugin.description)
                            .size(TEXT_BODY)
                            .line_height(Some(19.0))
                            .color(scheme.text_primary),
                    )
                    .wrap()
                    .selectable(false),
                );
            }
            let mut rows = vec![InfoRow::text(t!("extensions-contributes"), offers(plugin))];
            match packaging {
                // The package manager installs it: the command, and
                // nothing about a download that does not happen.
                Some(packaging) => packaged_card(ui, &packaging.command(&plugin.id), asked),
                None => rows.extend([
                    InfoRow::text(t!("extensions-size"), format::size(Some(plugin.size))),
                    InfoRow::text(t!("extensions-source"), app.plugin_registry().host.clone()),
                ]),
            }
            info_rows(ui, &rows, asked);
        });
}

/// The head of an available plugin's details: the icon tile, the name,
/// under it the id, the version and the kind with the chip *Available*,
/// and **Install** at the right, which says *Installing…* and takes no
/// click while the install runs. Without `install` — a Rick a package
/// manager installed — the head has no button.
fn available_head(ui: &mut egui::Ui, plugin: &Available, install: bool, asked: &mut Option<Ask>) {
    let scheme = theme::scheme(ui);
    let (band, _) = ui.allocate_exact_size(vec2(ui.available_width(), TILE), Sense::hover());
    let tile = Rect::from_min_size(band.min, vec2(TILE, TILE));
    ui.painter().rect_filled(tile, 12.0, scheme.bg_subtle);
    ui.painter().text(
        tile.center(),
        Align2::CENTER_CENTER,
        icons::for_plugin(&plugin.id),
        FontId::proportional(26.0),
        scheme.icon_default,
    );
    // The button first: the name and the line under it have what it leaves.
    let mut right = band.right();
    if install {
        let mut at = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("install", &plugin.id))
                .max_rect(band)
                .layout(egui::Layout::right_to_left(Align::Center)),
        );
        let label = if plugin.installing {
            t!("extensions-installing")
        } else {
            t!("extensions-install-available")
        };
        let button = widgets::button_of(
            &mut at,
            Some(icons::INSTALL),
            &label,
            ButtonKind::Primary,
            !plugin.installing,
        );
        if button.clicked() && !plugin.installing {
            *asked = Some(Ask::InstallAvailable(plugin.id.clone()));
        }
        right = button.rect.left() - 14.0;
    }
    let left = tile.right() + 14.0;
    widgets::line(
        ui,
        pos2(left, band.top() + 14.0),
        &plugin.name,
        FontId::proportional(15.0),
        scheme.text_primary,
        right - left,
    );
    let kind = Kind::of(plugin.points).name();
    let meta: Vec<&str> = [plugin.id.as_str(), plugin.version.as_str(), kind.as_str()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect();
    let y = band.top() + 34.5;
    let chip_text = t!("extensions-available");
    let line = widgets::line(
        ui,
        pos2(left, y),
        &meta.join("  ·  "),
        FontId::proportional(TEXT_SECONDARY),
        scheme.text_secondary,
        right - left - chip_width(ui, &chip_text) - 6.0,
    );
    chip(ui, line.right() + 6.0, y, &chip_text);
}

/// The details of the chosen plugin; `update` is the build the registry
/// has for it, when it has one.
fn details(
    ui: &mut egui::Ui,
    rect: Rect,
    app: &App,
    plugin: &Plugin,
    update: Option<&Update>,
    asked: &mut Option<Ask>,
) {
    let scheme = theme::scheme(ui);
    let inner = rect.shrink2(vec2(PAD, 20.0));
    // The actions keep the foot; the rest scrolls above them when a low
    // window leaves it too little room.
    let foot = Rect::from_min_max(pos2(inner.left(), inner.bottom() - 32.0), inner.max);
    actions(ui, foot, app, plugin, asked);
    let above = Rect::from_min_max(inner.min, pos2(inner.right(), foot.top() - GAP));
    let mut column = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("details", &plugin.path))
            .max_rect(above)
            .layout(egui::Layout::top_down(Align::Min)),
    );
    column.set_clip_rect(above.intersect(ui.clip_rect()));
    egui::ScrollArea::vertical()
        .id_salt("extension-details")
        .auto_shrink([false, false])
        .show(&mut column, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, GAP);
            ui.set_width(above.width());
            head(ui, app, plugin, asked);
            if !plugin.description.is_empty() {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&plugin.description)
                            .size(TEXT_BODY)
                            .line_height(Some(19.0))
                            .color(scheme.text_primary),
                    )
                    .wrap()
                    .selectable(false),
                );
            }
            update_or_attention(ui, app, plugin, update, asked);
            info_card(ui, app, plugin, asked);
        });
}

/// The head of the details: the icon tile, the name, under it the id, the
/// version and the kind with the chips, and the switch at the right.
fn head(ui: &mut egui::Ui, app: &App, plugin: &Plugin, asked: &mut Option<Ask>) {
    let scheme = theme::scheme(ui);
    let (band, _) = ui.allocate_exact_size(vec2(ui.available_width(), TILE), Sense::hover());
    let tile = Rect::from_min_size(band.min, vec2(TILE, TILE));
    ui.painter().rect_filled(tile, 12.0, scheme.bg_subtle);
    ui.painter().text(
        tile.center(),
        Align2::CENTER_CENTER,
        icons::for_plugin(&plugin.id),
        FontId::proportional(26.0),
        scheme.icon_default,
    );
    let left = tile.right() + 14.0;
    let right = band.right() - 32.0 - 14.0;
    widgets::line(
        ui,
        pos2(left, band.top() + 14.0),
        name_of(plugin),
        FontId::proportional(15.0),
        scheme.text_primary,
        right - left,
    );
    let kind = Kind::of(plugin.points).name();
    let meta: Vec<&str> = [plugin.id.as_str(), plugin.version.as_str(), kind.as_str()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect();
    let y = band.top() + 34.5;
    let source = if plugin.built_in {
        t!("extensions-built-in")
    } else {
        t!("extensions-user")
    };
    let mut chips = vec![source];
    if waits_for_restart(app, plugin) {
        chips.push(t!("extensions-restart"));
    }
    // The chips take what they need; the line is cut before them.
    let taken: f32 = chips.iter().map(|text| chip_width(ui, text) + 6.0).sum();
    let line = widgets::line(
        ui,
        pos2(left, y),
        &meta.join("  ·  "),
        FontId::proportional(TEXT_SECONDARY),
        scheme.text_secondary,
        right - left - taken,
    );
    let mut x = line.right();
    for text in &chips {
        x = chip(ui, x + 6.0, y, text);
    }
    switch(
        ui,
        pos2(band.right() - 16.0, band.center().y),
        app,
        plugin,
        "head",
        asked,
    );
}

/// The card over the info card. A plugin with an update gets the update's:
/// the host's reason as it stands, *Built for Rick 0.5.0*, with
/// **Update** — or, for a plugin that is loaded, the version that is
/// there to have. Any other plugin that needs attention gets the
/// attention card.
fn update_or_attention(
    ui: &mut egui::Ui,
    app: &App,
    plugin: &Plugin,
    update: Option<&Update>,
    asked: &mut Option<Ask>,
) {
    let Some(update) = update else {
        if let Some(text) = attention(app, plugin) {
            attention_card(ui, &t!("extensions-needs-attention"), &text, None);
        }
        return;
    };
    let (title, text) = match &plugin.status {
        PluginStatus::Refused(reason) => (t!("extensions-needs-attention"), reason.clone()),
        _ => (
            t!("extensions-update-available"),
            t!(
                "extensions-update-version",
                version = update.version.as_str()
            ),
        ),
    };
    let label = if update.updating {
        t!("extensions-updating")
    } else {
        t!("extensions-update")
    };
    if attention_card(ui, &title, &text, Some((&label, !update.updating))) {
        *asked = Some(Ask::InstallAvailable(update.id.clone()));
    }
}

/// The attention card: `bg/card` with an amber border, never an amber
/// fill, the dot, `title` and the reason, and under them the card's
/// button when it has one, with whether it takes a click. Returns
/// whether the button was clicked.
fn attention_card(
    ui: &mut egui::Ui,
    title: &str,
    text: &str,
    button: Option<(&str, bool)>,
) -> bool {
    let scheme = theme::scheme(ui);
    let mut clicked = false;
    egui::Frame::new()
        .fill(scheme.bg_card)
        .stroke(egui::Stroke::new(1.5, scheme.status_warning))
        .corner_radius(CARD_RADIUS)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            ui.horizontal(|ui| {
                let (dot, _) = ui.allocate_exact_size(vec2(DOT, DOT), Sense::hover());
                ui.painter()
                    .circle_filled(dot.center(), DOT / 2.0, scheme.status_warning);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(title)
                            .size(TEXT_BODY)
                            .color(scheme.text_primary),
                    )
                    .selectable(false),
                );
            });
            ui.add(
                egui::Label::new(
                    egui::RichText::new(text)
                        .size(TEXT_SECONDARY)
                        .line_height(Some(17.0))
                        .color(scheme.text_secondary),
                )
                .wrap()
                .selectable(false),
            );
            if let Some((label, enabled)) = button {
                ui.add_space(2.0);
                let update = widgets::button_of(
                    ui,
                    Some(icons::INSTALL),
                    label,
                    ButtonKind::Primary,
                    enabled,
                );
                clicked = update.clicked() && enabled;
            }
        });
    clicked
}

/// The path as the card shows it: under the home directory, with `~`.
fn shown(app: &App, path: &Path) -> String {
    let home = app.platform().home_dir();
    match path.strip_prefix(&home) {
        Ok(rest) if !rest.as_os_str().is_empty() => {
            format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display())
        }
        _ => path.display().to_string(),
    }
}

/// One row of an info card.
struct InfoRow {
    /// The row's label.
    label: String,
    /// What is shown.
    value: String,
    /// What the copy button copies, when it is not what is shown.
    copied: Option<String>,
    /// The value is a path or the like, in monospace.
    mono: bool,
}

impl InfoRow {
    /// A row of plain text, copied as it is shown.
    fn text(label: String, value: String) -> Self {
        Self {
            label,
            value,
            copied: None,
            mono: false,
        }
    }

    /// A row of a path: shown short, in monospace, and copied whole.
    fn path(label: String, shown: String, whole: String) -> Self {
        Self {
            label,
            value: shown,
            copied: Some(whole),
            mono: true,
        }
    }
}

/// The info card: *Contributes*, *Status*, *Location* and, for a plugin
/// with settings, *Config* — the two paths monospace, every row with a
/// copy button.
fn info_card(ui: &mut egui::Ui, app: &App, plugin: &Plugin, asked: &mut Option<Ask>) {
    let mut rows = vec![
        InfoRow::text(t!("extensions-contributes"), contributes(app, plugin)),
        InfoRow::text(t!("extensions-status"), status(plugin)),
        InfoRow::path(
            t!("extensions-location"),
            shown(app, &plugin.path),
            plugin.path.display().to_string(),
        ),
    ];
    if let Some(config) = app.plugin_config(&plugin.id) {
        rows.push(InfoRow::path(
            t!("extensions-config"),
            shown(app, &config),
            config.display().to_string(),
        ));
    }
    info_rows(ui, &rows, asked);
}

/// An info card of `rows`: the label, the value and a copy button each,
/// with hairlines between them.
fn info_rows(ui: &mut egui::Ui, rows: &[InfoRow], asked: &mut Option<Ask>) {
    let scheme = theme::scheme(ui);
    egui::Frame::new()
        .fill(scheme.bg_card)
        .stroke(egui::Stroke::new(1.0, scheme.border_default))
        .corner_radius(CARD_RADIUS)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            ui.set_width(ui.available_width());
            for (index, row) in rows.iter().enumerate() {
                let (line, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), INFO_ROW), Sense::hover());
                if index > 0 {
                    ui.painter()
                        .hline(line.x_range(), line.top() + 0.5, scheme.hairline());
                }
                widgets::line(
                    ui,
                    pos2(line.left() + 14.0, line.center().y),
                    &row.label,
                    FontId::proportional(TEXT_SECONDARY),
                    scheme.text_secondary,
                    LABEL,
                );
                let button = Rect::from_center_size(
                    pos2(line.right() - 6.0 - 11.0, line.center().y),
                    vec2(22.0, 22.0),
                );
                let left = line.left() + 14.0 + LABEL + 12.0;
                let font = if row.mono {
                    FontId::monospace(TEXT_SECONDARY)
                } else {
                    FontId::proportional(TEXT_SECONDARY)
                };
                widgets::line(
                    ui,
                    pos2(left, line.center().y),
                    &row.value,
                    font,
                    scheme.text_primary,
                    button.left() - 12.0 - left,
                );
                let name = t!("extensions-copy", what = row.label.as_str());
                if CopyButton::new(&name).show_at(ui, button, &name).clicked() {
                    *asked = Some(Ask::Copy(
                        row.copied.clone().unwrap_or_else(|| row.value.clone()),
                    ));
                }
            }
        });
}

/// The actions at the foot of the details: *Settings* for a plugin with a
/// settings page and *Open folder* at the left, *Remove* at the right for
/// a user's plugin — a built-in one can be switched off, not removed.
fn actions(ui: &mut egui::Ui, rect: Rect, app: &App, plugin: &Plugin, asked: &mut Option<Ask>) {
    let mut left = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("actions", &plugin.path))
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    left.spacing_mut().item_spacing.x = 8.0;
    let loaded = plugin.status == PluginStatus::Loaded;
    if loaded
        && app.plugin_config(&plugin.id).is_some()
        && widgets::card_button(
            &mut left,
            None,
            &t!("extensions-settings"),
            ButtonKind::Secondary,
        )
        .clicked()
    {
        *asked = Some(Ask::Settings(plugin.id.clone()));
    }
    if !plugin.removed
        && widgets::card_button(
            &mut left,
            Some(icons::FOLDER_OPEN),
            &t!("extensions-open-folder"),
            ButtonKind::Secondary,
        )
        .clicked()
    {
        *asked = Some(Ask::OpenFolder(plugin.path.clone()));
    }
    if plugin.built_in || plugin.removed {
        return;
    }
    let mut right = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("remove", &plugin.path))
            .max_rect(rect)
            .layout(egui::Layout::right_to_left(Align::Center)),
    );
    if widgets::card_button(
        &mut right,
        None,
        &t!("extensions-remove"),
        ButtonKind::Danger,
    )
    .clicked()
    {
        *asked = Some(Ask::Remove(plugin.path.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plugin_is_listed_under_the_first_point_it_fills() {
        // Provider, previewer, search index, action, in that order.
        assert_eq!(
            Kind::of(Points::PROVIDER | Points::ACTION),
            Kind::Provider,
            "SSH is a provider"
        );
        assert_eq!(
            Kind::of(Points::DECORATOR | Points::ACTION),
            Kind::Action,
            "Git is an action"
        );
        assert_eq!(
            Kind::of(Points::PREVIEWER | Points::ACTION),
            Kind::Previewer
        );
        assert_eq!(
            Kind::of(Points::SEARCH_INDEX | Points::ACTION),
            Kind::Search
        );
        assert_eq!(Kind::of(Points::DECORATOR), Kind::Decorator);
        assert_eq!(Kind::of(Points::empty()), Kind::Unknown);
        // The headings stand in the order of the list.
        assert!(Kind::Provider < Kind::Previewer);
        assert!(Kind::Previewer < Kind::Action);
        assert!(Kind::Action < Kind::Search);
        assert!(Kind::Search < Kind::Decorator);
    }
}
