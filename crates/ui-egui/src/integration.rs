//! The integration dialog (specs/system-integration.md §6.1 and §6.2, the
//! Figma frame *Integration dialog — first run*): *Set up Rick on this
//! system*, 560 px wide in the dialogue frame. The header has the app
//! icon, the title and a line naming the system and the desktop. The body
//! says what the dialog is for, has *Install for* (*Just me*; *All users*
//! is there and cannot be chosen yet), the card of the items the check
//! found missing or reset — an icon, the title, a line that says what is
//! written where, a *Best effort* pill where the item is one, and a
//! switch, on at first — and the line about passwords. The footer has
//! *Don't ask again*, *Not now* and *Apply N items*.
//!
//! A row whose place holds a file Rick did not write says so and asks:
//! it is set up only with *Replace* ticked, and left for the next start
//! otherwise. A row switched off is stored as declined. While the rows
//! are set up the footer shows which, and the dialog closes when the
//! last is done; what failed is a toast of the app's.
//!
//! The shell asks for the check once, after the first frame, and opens
//! the dialog when the answer says to.
//!
//! the settings' Integration page (§6.1, the Figma frame *Settings —
//! Integration*) words its rows here too: the line naming the system and
//! the last check, and for every item its line, its pill — *Set up*,
//! *Reset by system*, *Skipped*, *Not set up*, *Unsupported* — and what
//! its button asks for.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use app::{App, Command, IntegrationCommand};
use chrono::{DateTime, Local, NaiveDate};
use egui::{Align, Align2, FontId, Rect, Sense, pos2, vec2};
use ext::{Header, HeaderIcon, t};
use model::{Integration, IntegrationItem, ItemRecord, ItemStatus, SettingsChange};
use platform::integration::{Desktop, Item, Reason, Scope};

use crate::shell::Request;
use crate::theme::{self, CARD_RADIUS, ICON, Scheme, TEXT_BODY, TEXT_SECONDARY};
use crate::{dialog, icons, widgets};

/// A row with a one-line description.
const ROW: f32 = 57.0;
/// The padding above and below a row's text.
const ROW_PAD: f32 = 12.0;
/// The inset of a row's icon and of its switch from the card's edges.
const ROW_INSET: f32 = 16.0;
/// Where a row's text starts.
const TEXT_INSET: f32 = 46.0;
/// The gap between a row's text, its pill and its switch.
const ROW_GAP: f32 = 14.0;
/// The switch's width.
const SWITCH: f32 = 32.0;
/// The gap between a row's title and its description.
const LINE_GAP: f32 = 3.0;
/// The height of *Install for* and its segmented control.
const SCOPE_HEIGHT: f32 = 34.0;
/// The gap between a row's notice and its *Replace* box.
const REPLACE_GAP: f32 = 6.0;

/// The app icon of the header, at twice the 48 px it is shown at.
fn app_icon() -> Arc<Vec<u8>> {
    static ICON_PNG: OnceLock<Arc<Vec<u8>>> = OnceLock::new();
    Arc::clone(
        ICON_PNG.get_or_init(|| {
            Arc::new(include_bytes!("../../../assets/icons/png/rick-96.png").to_vec())
        }),
    )
}

/// The dialog's state between frames.
#[derive(Debug, Default)]
pub struct IntegrationDialog {
    open: Option<Open>,
    startup: Startup,
}

/// Where the check after the first frame stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Startup {
    /// No frame was drawn yet.
    #[default]
    Before,
    /// The first frame was drawn.
    Drawn,
    /// The check was asked for.
    Asked,
}

/// What the open dialog shows.
#[derive(Debug)]
struct Open {
    rows: Vec<Row>,
    dont_ask: bool,
    /// The rows are being set up: the controls rest and the footer shows
    /// the progress.
    applying: bool,
}

/// One listed item and what the user chose for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Row {
    item: Item,
    /// The switch.
    on: bool,
    /// *Replace*, for a row with a file of another origin in its way.
    replace: bool,
}

/// What a frame of the dialog decided.
#[derive(Debug, Default)]
struct Asked {
    apply: bool,
    not_now: bool,
}

/// `path` with `~` for the home directory, as the rows show paths.
pub(crate) fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// The icon of an item's row.
pub(crate) fn item_icon(item: Item) -> &'static str {
    match item {
        Item::InstallLocation => icons::FOLDER,
        Item::LauncherEntry => icons::DESKTOP,
        Item::DefaultFileManager => icons::HOME,
        Item::ShowInFolder => icons::LINK,
        Item::RickCommand => icons::CODE,
        Item::DockFavourite => icons::STAR,
    }
}

/// The title of an item's row in the dialog, which says what happens:
/// *Install to the standard location*, not *Install location*.
fn row_title(item: Item) -> String {
    match item {
        Item::InstallLocation => t!("integration-row-install-location"),
        Item::LauncherEntry => t!("integration-item-launcher-entry"),
        Item::DefaultFileManager => t!("integration-item-default-file-manager"),
        Item::ShowInFolder => t!("integration-item-show-in-folder"),
        Item::RickCommand => t!("integration-row-rick-command"),
        Item::DockFavourite => t!("integration-item-dock-favourite"),
    }
}

/// The line under a row's title: what is written where, worded from what
/// the check found.
fn row_detail(row: &IntegrationItem, desktop: Option<Desktop>, home: &Path) -> String {
    let check = &row.check;
    let shown = |path: Option<&Path>| path.map_or_default(|p| tilde(p, home));
    match row.item {
        Item::InstallLocation => t!(
            "integration-row-install-location-detail",
            from = shown(check.target.as_deref()),
            to = shown(check.place.as_deref())
        ),
        Item::LauncherEntry => match desktop {
            Some(Desktop::Gnome) => t!("integration-row-launcher-entry-detail-gnome"),
            _ => t!("integration-row-launcher-entry-detail"),
        },
        Item::DefaultFileManager => match &check.name {
            Some(name) => t!(
                "integration-row-default-file-manager-detail",
                name = name.as_str()
            ),
            None => t!("integration-row-default-file-manager-detail-none"),
        },
        Item::ShowInFolder => t!("integration-row-show-in-folder-detail"),
        Item::RickCommand => t!(
            "integration-row-rick-command-detail",
            dir = shown(check.place.as_deref().and_then(Path::parent))
        ),
        Item::DockFavourite => match desktop {
            Some(Desktop::Gnome) => t!("integration-row-dock-favourite-detail-gnome"),
            Some(Desktop::Plasma) => t!("integration-row-dock-favourite-detail-plasma"),
            Some(Desktop::Cosmic) => t!("integration-row-dock-favourite-detail-cosmic"),
            Some(Desktop::Cinnamon) => t!("integration-row-dock-favourite-detail-cinnamon"),
            None => t!("integration-row-dock-favourite-detail"),
        },
    }
}

impl IntegrationDialog {
    /// Counts the frames to the check at startup: asked for once, at the
    /// start of the second frame, so the first is on screen before
    /// anything is read.
    pub fn frame(&mut self, requests: &mut Vec<Request>) {
        match self.startup {
            Startup::Before => self.startup = Startup::Drawn,
            Startup::Drawn => {
                self.startup = Startup::Asked;
                requests.push(Request::Command(Command::Integration(
                    IntegrationCommand::Check { startup: true },
                )));
            }
            Startup::Asked => {}
        }
    }

    /// Opens the dialog on the rows the check lists, each switched on.
    /// Nothing opens without a row, or over a dialog that is open.
    pub fn open(&mut self, integration: &Integration) {
        if self.open.is_some() {
            return;
        }
        let rows: Vec<Row> = integration
            .asked()
            .map(|row| Row {
                item: row.item,
                on: true,
                replace: false,
            })
            .collect();
        if rows.is_empty() {
            return;
        }
        self.open = Some(Open {
            rows,
            dont_ask: false,
            applying: false,
        });
    }

    /// The rows were set up: the dialog that asked for them closes.
    pub fn applied(&mut self) {
        if self.open.as_ref().is_some_and(|open| open.applying) {
            self.open = None;
        }
    }

    /// Whether the dialog is on screen.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Draws the dialog, when open.
    pub fn show(&mut self, ctx: &egui::Context, app: &App, requests: &mut Vec<Request>) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        let integration = &app.model().integration;
        let home = app.platform().home_dir();
        let mut asked = Asked::default();
        let modal = dialog::modal(
            ctx,
            egui::Id::new("integration"),
            dialog::FORM_WIDTH,
            |ui| {
                let header = Header::new(HeaderIcon::Image {
                    bytes: app_icon(),
                    svg: false,
                })
                .subtitle(t!(
                    "integration-dialog-subtitle",
                    system = integration.system.as_str()
                ));
                dialog::header(ui, &t!("integration-dialog-title"), Some(&header), false);
                dialog::body(ui, |ui| {
                    intro(ui);
                    ui.add_space(4.0);
                    scope_row(ui);
                    ui.add_space(4.0);
                    rows_card(ui, integration, open, &home);
                    ui.add_space(4.0);
                    note(ui);
                });
                footer(ui, integration, open, &mut asked);
                if !open.applying && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    asked.not_now = true;
                }
            },
        );
        if modal.should_close() && !open.applying {
            asked.not_now = true;
        }
        if open.applying {
            ctx.request_repaint();
        }
        if !(asked.apply || asked.not_now) {
            return;
        }
        if open.dont_ask {
            requests.push(Request::Command(Command::Settings(
                SettingsChange::IntegrationAsk(false),
            )));
        }
        if asked.apply {
            let (perform, replace) = chosen(open, integration);
            let decline: Vec<Item> = open
                .rows
                .iter()
                .filter(|row| !row.on)
                .map(|row| row.item)
                .collect();
            let wait = !perform.is_empty();
            requests.push(Request::Command(Command::Integration(
                IntegrationCommand::Apply {
                    scope: Scope::User,
                    perform,
                    decline,
                    replace,
                },
            )));
            if wait {
                open.applying = true;
                return;
            }
        }
        self.open = None;
    }
}

/// Whether a file Rick did not write is in the way of `item`.
fn in_the_way(integration: &Integration, item: Item) -> bool {
    integration
        .get(item)
        .is_some_and(|row| !row.check.foreign.is_empty())
}

/// The rows *Apply* sets up, and those among them whose foreign files
/// the user said to replace. A row with such a file and no *Replace* is
/// neither set up nor declined: it is asked about again.
fn chosen(open: &Open, integration: &Integration) -> (Vec<Item>, Vec<Item>) {
    let mut perform = Vec::new();
    let mut replace = Vec::new();
    for row in open.rows.iter().filter(|row| row.on) {
        let foreign = in_the_way(integration, row.item);
        if foreign && !row.replace {
            continue;
        }
        perform.push(row.item);
        if foreign {
            replace.push(row.item);
        }
    }
    (perform, replace)
}

/// The line height of text that may wrap: two more than its size, as
/// the frame's two-line descriptions are set.
fn line_height(size: f32) -> f32 {
    size + 2.0
}

/// A line of text that wraps, read aloud as a label.
fn line(ui: &mut egui::Ui, text: &str, size: f32, colour: egui::Color32) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .size(size)
                .color(colour)
                .line_height(Some(line_height(size))),
        )
        .wrap()
        .selectable(false),
    );
}

/// The size [`line`] takes for `text` wrapped at `wrap`.
fn measure(ui: &egui::Ui, text: &str, size: f32, wrap: f32) -> egui::Vec2 {
    let mut job = egui::text::LayoutJob::simple(
        text.to_owned(),
        FontId::proportional(size),
        egui::Color32::BLACK,
        wrap,
    );
    for section in &mut job.sections {
        section.format.line_height = Some(line_height(size));
    }
    ui.painter().layout_job(job).size()
}

/// The size of `text` on one line.
fn one_line(ui: &egui::Ui, text: &str, size: f32) -> egui::Vec2 {
    ui.painter()
        .layout_no_wrap(
            text.to_owned(),
            FontId::proportional(size),
            egui::Color32::BLACK,
        )
        .size()
}

/// What the dialog is for.
fn intro(ui: &mut egui::Ui) {
    let colour = theme::scheme(ui).text_secondary;
    line(ui, &t!("integration-dialog-intro"), TEXT_BODY, colour);
}

/// *Install for*: the label at the left, *Just me* and *All users* at
/// the right. Setting Rick up for all users needs the installer's helper
/// for the system scope, which is not there yet: the option is shown and
/// cannot be chosen.
fn scope_row(ui: &mut egui::Ui) {
    let scheme = theme::scheme(ui);
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), SCOPE_HEIGHT), Sense::hover());
    let label = t!("integration-install-for");
    let galley = ui.painter().layout_no_wrap(
        label.clone(),
        FontId::proportional(TEXT_BODY),
        scheme.text_primary,
    );
    let at = pos2(rect.left(), rect.center().y - galley.size().y / 2.0);
    let label_rect = Rect::from_min_size(at, galley.size());
    ui.painter().galley(at, galley, scheme.text_primary);
    ui.interact(label_rect, ui.id().with("install-for"), Sense::hover())
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label.clone()));
    let options = [t!("integration-scope-user"), t!("integration-scope-system")];
    let widest = options
        .iter()
        .map(|option| {
            ui.painter()
                .layout_no_wrap(
                    option.clone(),
                    FontId::proportional(TEXT_BODY),
                    scheme.text_primary,
                )
                .size()
                .x
        })
        .fold(0.0_f32, f32::max);
    let width = (widest + 36.0) * 2.0;
    let control = Rect::from_min_max(pos2(rect.right() - width, rect.top()), rect.max);
    // *Just me* is the one scope there is: nothing to choose yet.
    let _ = widgets::segmented_with(
        ui,
        control,
        ui.id().with("install-for-scope"),
        &options,
        0,
        Some((1, &t!("integration-not-available-yet"))),
    );
}

/// The card of the listed items, a hairline between rows.
fn rows_card(ui: &mut egui::Ui, integration: &Integration, open: &mut Open, home: &Path) {
    let scheme = theme::scheme(ui);
    let enabled = !open.applying;
    egui::Frame::new()
        .fill(scheme.bg_card)
        .stroke(egui::Stroke::new(1.0, scheme.border_default))
        .corner_radius(CARD_RADIUS)
        .inner_margin(0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            let mut first = true;
            for row in &mut open.rows {
                let Some(found) = integration.get(row.item) else {
                    continue;
                };
                row_ui(ui, row, found, integration.desktop, home, first, enabled);
                first = false;
            }
        });
}

/// One item: the icon, the title over its line, the *Best effort* pill
/// and the switch; under the line, when a file of another origin is in
/// the item's way, which one and *Replace*.
fn row_ui(
    ui: &mut egui::Ui,
    row: &mut Row,
    found: &IntegrationItem,
    desktop: Option<Desktop>,
    home: &Path,
    first: bool,
    enabled: bool,
) {
    let scheme = theme::scheme(ui);
    let title = row_title(row.item);
    let detail = row_detail(found, desktop, home);
    let foreign = found
        .check
        .foreign
        .first()
        .map(|path| t!("integration-foreign", path = tilde(path, home)));
    let pill = row
        .item
        .best_effort()
        .then(|| t!("integration-best-effort"));
    let width = ui.available_width();
    let pill_room = pill
        .as_deref()
        .map_or(0.0, |text| one_line(ui, text, 11.0).x + 14.0 + ROW_GAP);
    let text_width = width - TEXT_INSET - ROW_GAP - pill_room - SWITCH - ROW_INSET;
    let mut content = one_line(ui, &title, TEXT_BODY).y
        + LINE_GAP
        + measure(ui, &detail, TEXT_SECONDARY, text_width).y;
    if let Some(foreign) = &foreign {
        content +=
            LINE_GAP + measure(ui, foreign, TEXT_SECONDARY, text_width).y + REPLACE_GAP + ICON;
    }
    let height = (content + 2.0 * ROW_PAD).max(ROW);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    if !first {
        ui.painter().hline(
            (rect.left() + 1.0)..=(rect.right() - 1.0),
            rect.top() + 0.5,
            scheme.hairline(),
        );
    }
    ui.painter().text(
        pos2(rect.left() + ROW_INSET + ICON / 2.0, rect.center().y),
        Align2::CENTER_CENTER,
        item_icon(row.item),
        FontId::proportional(ICON),
        scheme.icon_default,
    );

    let top = rect.center().y - content / 2.0;
    let text_rect = Rect::from_min_size(
        pos2(rect.left() + TEXT_INSET, top),
        vec2(text_width, content),
    );
    row_text(
        ui,
        text_rect,
        row,
        (&title, &detail, foreign.as_deref()),
        enabled,
    );
    let side = Rect::from_min_max(
        pos2(text_rect.right(), rect.top()),
        pos2(rect.right() - ROW_INSET, rect.bottom()),
    );
    row_controls(ui, side, row, &title, pill.as_deref(), enabled);
}

/// A row's text in `rect`: the title, its line and, for a file of
/// another origin in the item's way, which one and *Replace*.
fn row_text(
    ui: &mut egui::Ui,
    rect: Rect,
    row: &mut Row,
    (title, detail, foreign): (&str, &str, Option<&str>),
    enabled: bool,
) {
    let scheme = theme::scheme(ui);
    let mut text = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("integration-row", row.item))
            .max_rect(rect)
            .layout(egui::Layout::top_down(Align::Min)),
    );
    if !enabled {
        text.disable();
    }
    text.spacing_mut().item_spacing = vec2(0.0, LINE_GAP);
    text.add(
        egui::Label::new(
            egui::RichText::new(title)
                .size(TEXT_BODY)
                .color(scheme.text_primary),
        )
        .truncate()
        .selectable(false),
    );
    line(&mut text, detail, TEXT_SECONDARY, scheme.text_secondary);
    if let Some(foreign) = foreign {
        line(&mut text, foreign, TEXT_SECONDARY, scheme.status_warning);
        text.add_space(REPLACE_GAP - LINE_GAP);
        if widgets::checkbox(&mut text, row.replace, &t!("integration-replace")).clicked() {
            row.replace = !row.replace;
        }
    }
}

/// A row's controls in `rect`, from the right: the switch, named after
/// the row, and the pill before it.
fn row_controls(
    ui: &mut egui::Ui,
    rect: Rect,
    row: &mut Row,
    title: &str,
    pill: Option<&str>,
    enabled: bool,
) {
    let scheme = theme::scheme(ui);
    let mut side = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("integration-row-side", row.item))
            .max_rect(rect)
            .layout(egui::Layout::right_to_left(Align::Center)),
    );
    if !enabled {
        side.disable();
    }
    side.spacing_mut().item_spacing.x = ROW_GAP;
    if widgets::toggle(&mut side, row.on, title).clicked() {
        row.on = !row.on;
    }
    if let Some(pill) = pill {
        widgets::pill(&mut side, pill, scheme.trash_low_bg, scheme.trash_low_text);
    }
}

/// The line about passwords under the card.
fn note(ui: &mut egui::Ui) {
    let scheme = theme::scheme(ui);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (rect, _) = ui.allocate_exact_size(vec2(ICON, ICON), Sense::hover());
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            icons::INFO,
            FontId::proportional(ICON),
            scheme.icon_default,
        );
        line(
            ui,
            &t!("integration-password-note"),
            TEXT_SECONDARY,
            scheme.text_secondary,
        );
    });
}

/// The footer: *Don't ask again* at the left, *Not now* and *Apply N
/// items* at the right; while the rows are set up, which one and how far
/// along instead of the checkbox, and the buttons rest.
fn footer(ui: &mut egui::Ui, integration: &Integration, open: &mut Open, asked: &mut Asked) {
    let scheme = theme::scheme(ui);
    let applying = open.applying;
    let (perform, _) = chosen(open, integration);
    let declined = open.rows.iter().any(|row| !row.on);
    let can_apply = !applying && (!perform.is_empty() || declined);
    dialog::footer(
        ui,
        |left| {
            if applying {
                let text = match &integration.applying {
                    Some(progress) => t!(
                        "integration-applying",
                        item = progress.current.map_or_default(row_title),
                        done = (progress.done + 1).min(progress.total),
                        total = progress.total
                    ),
                    None => t!("integration-finishing"),
                };
                left.add(
                    egui::Label::new(
                        egui::RichText::new(text)
                            .size(TEXT_SECONDARY)
                            .color(scheme.text_secondary),
                    )
                    .truncate()
                    .selectable(false),
                );
            } else if widgets::checkbox(left, open.dont_ask, &t!("integration-dont-ask")).clicked()
            {
                open.dont_ask = !open.dont_ask;
            }
        },
        |right| {
            let apply = t!("integration-apply", count = perform.len());
            if widgets::button(right, &apply, true, can_apply).clicked() && can_apply {
                asked.apply = true;
            }
            if widgets::button(right, &t!("integration-not-now"), false, !applying).clicked()
                && !applying
            {
                asked.not_now = true;
            }
        },
    );
}

/// The line under the title of the settings' Integration page: the system and
/// the desktop, then when the check last ran.
pub(crate) fn checked_line(integration: &Integration) -> String {
    let checked = if integration.checking {
        t!("integration-checking")
    } else {
        match integration.checked_at {
            Some(at) => {
                let time = DateTime::<Local>::from(at).format("%H:%M").to_string();
                if integration.at_startup {
                    t!("integration-checked-at-start", time = time)
                } else {
                    t!("integration-checked", time = time)
                }
            }
            None => t!("integration-not-checked"),
        }
    };
    if integration.system.is_empty() {
        checked
    } else {
        format!("{} · {checked}", integration.system)
    }
}

/// The items of one group of the settings' Integration page, in the listed
/// order: the `rick` command under *Terminal and links*, the others
/// under *This system*.
pub(crate) fn settings_rows(integration: &Integration, terminal: bool) -> Vec<&IntegrationItem> {
    integration
        .items
        .iter()
        .filter(|row| (row.item == Item::RickCommand) == terminal)
        .collect()
}

/// An item's title on the settings' Integration page.
pub(crate) fn item_name(item: Item) -> String {
    match item {
        Item::InstallLocation => t!("integration-item-install-location"),
        Item::LauncherEntry => t!("integration-item-launcher-entry"),
        Item::DefaultFileManager => t!("integration-item-default-file-manager"),
        Item::ShowInFolder => t!("integration-item-show-in-folder"),
        Item::RickCommand => t!("integration-item-rick-command"),
        Item::DockFavourite => t!("integration-item-dock-favourite"),
    }
}

/// The day a record was last touched, as `8 Nov`.
fn record_day(record: Option<&ItemRecord>) -> Option<String> {
    let date = record?.checked?.date?;
    let date = NaiveDate::from_ymd_opt(date.year.into(), date.month.into(), date.day.into())?;
    Some(date.format("%-d %b").to_string())
}

/// The line under an item's title on the settings' Integration page: where a
/// set-up item is, what is in the way of one that is not, why one cannot
/// be set up, or what setting it up would do.
pub(crate) fn settings_line(
    row: &IntegrationItem,
    desktop: Option<Desktop>,
    record: Option<&ItemRecord>,
    home: &Path,
) -> String {
    let check = &row.check;
    let shown = |path: Option<&Path>| path.map_or_default(|p| tilde(p, home));
    if let Some(path) = check.foreign.first() {
        return t!("integration-foreign", path = tilde(path, home));
    }
    match &row.status {
        ItemStatus::SetUp => match row.item {
            Item::InstallLocation => t!(
                "integration-line-install-location",
                path = shown(check.place.as_deref())
            ),
            Item::LauncherEntry => t!(
                "integration-line-launcher-entry",
                dir = shown(check.place.as_deref())
            ),
            Item::DefaultFileManager => t!("integration-line-default-file-manager"),
            Item::ShowInFolder => t!("integration-line-show-in-folder"),
            Item::RickCommand => t!(
                "integration-line-rick-command",
                link = shown(check.place.as_deref()),
                target = shown(check.target.as_deref())
            ),
            Item::DockFavourite => match desktop {
                Some(Desktop::Gnome) => t!("integration-line-dock-favourite-gnome"),
                Some(Desktop::Plasma) => t!("integration-line-dock-favourite-plasma"),
                Some(Desktop::Cosmic) => t!("integration-line-dock-favourite-cosmic"),
                Some(Desktop::Cinnamon) => t!("integration-line-dock-favourite-cinnamon"),
                None => t!("integration-line-dock-favourite"),
            },
        },
        ItemStatus::Drifted => t!("integration-line-drifted"),
        ItemStatus::Declined => match record_day(record) {
            Some(day) => t!("integration-line-skipped-on", date = day),
            None => t!("integration-line-skipped"),
        },
        ItemStatus::Missing => row_detail(row, desktop, home),
        ItemStatus::Unsupported(reason) => match reason {
            Reason::Desktop => t!("integration-unsupported-desktop"),
            Reason::Program(name) => t!("integration-unsupported-program", name = name.as_str()),
            Reason::Config(path) => t!("integration-unsupported-config", path = tilde(path, home)),
            Reason::AllUsers => t!("integration-not-available-yet"),
            Reason::Plugin(name) => t!("integration-unsupported-plugin", name = name.as_str()),
        },
    }
}

/// An item's pill on the settings' Integration page: its text, its tint and its
/// text colour.
pub(crate) fn status_pill(
    status: &ItemStatus,
    scheme: &Scheme,
) -> (String, egui::Color32, egui::Color32) {
    let grey = (scheme.trash_low_bg, scheme.trash_low_text);
    let (text, (bg, fg)) = match status {
        ItemStatus::SetUp => (
            t!("integration-pill-set-up"),
            (scheme.status_success_bg, scheme.status_success),
        ),
        ItemStatus::Drifted => (
            t!("integration-pill-reset"),
            (scheme.trash_mid_bg, scheme.status_warning),
        ),
        ItemStatus::Declined => (t!("integration-pill-skipped"), grey),
        ItemStatus::Missing => (t!("integration-pill-not-set-up"), grey),
        ItemStatus::Unsupported(_) => (t!("integration-pill-unsupported"), grey),
    };
    (text, bg, fg)
}

/// What an item's button on the settings' Integration page reads and asks for,
/// and whether it is the primary one: *Restore* for an item the system
/// reset, *Add* for one skipped or not set up; *Replace* instead of
/// either while a file Rick did not write is in the way, which is the
/// user's word to replace it. No button for an item that is set up or
/// cannot be.
pub(crate) fn status_action(row: &IntegrationItem) -> Option<(String, bool, IntegrationCommand)> {
    let item = row.item;
    let replace = !row.check.foreign.is_empty();
    match &row.status {
        ItemStatus::SetUp | ItemStatus::Unsupported(_) => None,
        ItemStatus::Drifted if replace => Some((
            t!("integration-replace"),
            false,
            IntegrationCommand::Restore { item, replace },
        )),
        ItemStatus::Drifted => Some((
            t!("integration-restore"),
            true,
            IntegrationCommand::Restore { item, replace },
        )),
        ItemStatus::Declined | ItemStatus::Missing => Some((
            if replace {
                t!("integration-replace")
            } else {
                t!("integration-add")
            },
            false,
            IntegrationCommand::Add { item, replace },
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use model::ItemStatus;
    use platform::integration::Check;

    use super::*;

    fn listed(item: Item, check: Check) -> IntegrationItem {
        IntegrationItem {
            item,
            status: ItemStatus::Missing,
            check,
        }
    }

    #[test]
    fn paths_under_the_home_are_shown_with_a_tilde() {
        let home = Path::new("/home/u");
        assert_eq!(tilde(Path::new("/home/u/.local/bin"), home), "~/.local/bin");
        assert_eq!(tilde(Path::new("/home/u"), home), "~");
        assert_eq!(tilde(Path::new("/opt/rick"), home), "/opt/rick");
        assert_eq!(tilde(Path::new("/home/user2/x"), home), "/home/user2/x");
    }

    #[test]
    fn a_row_with_a_foreign_file_is_set_up_only_with_replace() {
        let integration = Integration {
            items: vec![
                listed(Item::LauncherEntry, Check::default()),
                listed(
                    Item::RickCommand,
                    Check {
                        foreign: vec![PathBuf::from("/home/u/.local/bin/rick")],
                        ..Check::default()
                    },
                ),
                listed(Item::DockFavourite, Check::default()),
            ],
            ..Integration::default()
        };
        let mut dialog = IntegrationDialog::default();
        dialog.open(&integration);
        let open = dialog.open.as_mut().unwrap_or_else(|| panic!("open"));
        assert_eq!(open.rows.len(), 3);
        assert!(open.rows.iter().all(|row| row.on && !row.replace));
        // The command's row waits for the answer.
        assert_eq!(
            chosen(open, &integration),
            (vec![Item::LauncherEntry, Item::DockFavourite], vec![])
        );
        open.rows[1].replace = true;
        assert_eq!(
            chosen(open, &integration),
            (
                vec![Item::LauncherEntry, Item::RickCommand, Item::DockFavourite],
                vec![Item::RickCommand]
            )
        );
        // Switched off, a row is neither.
        open.rows[1].on = false;
        open.rows[2].on = false;
        assert_eq!(
            chosen(open, &integration),
            (vec![Item::LauncherEntry], vec![])
        );
    }

    #[test]
    fn nothing_opens_without_a_row_and_the_check_is_asked_for_once() {
        let mut dialog = IntegrationDialog::default();
        dialog.open(&Integration::default());
        assert!(!dialog.is_open());
        let mut requests = Vec::new();
        dialog.frame(&mut requests);
        assert_eq!(requests.len(), 0, "the first frame is drawn first");
        dialog.frame(&mut requests);
        dialog.frame(&mut requests);
        assert_eq!(requests.len(), 1);
        assert!(matches!(
            requests.first(),
            Some(Request::Command(Command::Integration(
                IntegrationCommand::Check { startup: true }
            )))
        ));
    }
}
