//! The Settings window, the Keyboard shortcuts and Extensions screens; the
//! About dialog is [`crate::about`].

use app::{App, Command, IntegrationCommand, PluginStatus};
use egui::{Align, Align2, FontId, Rect, Sense, pos2, vec2};
use ext::t;
use model::{
    AfterDrag, Column, Density, OnOpen, PanelSide, SettingsChange, SidePanelChange, SortDir, Theme,
    WindowId,
};
use platform::integration::Item;

use crate::shell::Request;
use crate::{dialog, icons, integration, theme, widgets};

/// The window's size, as the spec gives it.
const SIZE: egui::Vec2 = vec2(880.0, 640.0);
/// The smallest the window goes, as a viewport of its own.
const MIN_SIZE: [f32; 2] = [720.0, 480.0];
/// The left column's width.
const COLUMN: f32 = 220.0;
/// The inset of the column's title, search field and section header.
const COLUMN_INSET: f32 = 20.0;
/// The inset of the column's rows from its edges, as the side panel's.
const ROW_INSET: f32 = 6.0;
/// The search field's height.
const SEARCH_HEIGHT: f32 = 30.0;
/// The page's padding.
const PAGE_PAD: f32 = 32.0;
/// A setting row without a description, and with one.
const ROW: f32 = 44.0;
const ROW_DESCRIBED: f32 = 58.0;
/// The width of a dropdown and of a text field in a row.
const DROPDOWN: f32 = 180.0;
const FIELD: f32 = 260.0;
/// The height of a segmented control in a row.
const SEGMENT_HEIGHT: f32 = 28.0;
/// The round close button.
const CLOSE_BUTTON: f32 = 28.0;
/// The spring-loading delays the General page offers, in milliseconds.
const DELAYS: [u64; 7] = [200, 500, 750, 1000, 1500, 2000, 3000];
/// The font sizes the Appearance page offers, in points.
const FONT_SIZES: std::ops::RangeInclusive<u8> = 9..=24;

/// The Settings window (SPEC.md *Settings window*): 880 × 640,
/// GNOME-style. The left column, `bg/panel`, holds the *Settings* title,
/// a search field that filters the categories and rows to those whose
/// label or description match, and the category list — *General*,
/// *View*, *Appearance*, then *PLUGINS* with a row per loaded plugin that
/// declares settings. The right column shows the chosen page: its title,
/// a round close button, and grouped cards, one row per setting with the
/// label and its description at the left and the control at the right.
/// A window of its own where the backend has viewports; embedded over
/// the main window otherwise.
#[derive(Debug, Default)]
pub struct SettingsWindow {
    open: bool,
    /// The window whose side panel the View page changes.
    window: Option<WindowId>,
    page: Page,
    search: String,
}

/// A page of the settings.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum Page {
    #[default]
    General,
    View,
    Appearance,
    /// The integration items and their preferences
    /// (specs/system-integration.md §6.1).
    Integration,
    /// A plugin's page, by the extension's id.
    Plugin(String),
}

/// What a page's rows need: the app and the window the View page's side
/// panel row changes.
struct PageCx<'a> {
    app: &'a App,
    window: Option<WindowId>,
    requests: &'a mut Vec<Request>,
}

impl PageCx<'_> {
    fn change(&mut self, change: SettingsChange) {
        self.requests
            .push(Request::Command(Command::Settings(change)));
    }

    fn panel(&mut self, change: SidePanelChange) {
        if let Some(window) = self.window {
            self.requests
                .push(Request::Command(Command::SidePanel { window, change }));
        }
    }
}

/// A setting's label and description, for the search and the row.
#[derive(Debug, Clone)]
struct Spec {
    label: String,
    description: Option<String>,
}

impl Spec {
    fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            description: None,
        }
    }

    fn described(label: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            description: Some(description.into()),
        }
    }

    /// Whether `query` is in the label or the description, case aside.
    fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty()
            || self.label.to_lowercase().contains(&query)
            || self
                .description
                .as_deref()
                .is_some_and(|d| d.to_lowercase().contains(&query))
    }
}

/// A group of settings: its title and the rows' specs, in order.
struct Group {
    title: String,
    rows: Vec<Spec>,
}

impl SettingsWindow {
    /// Opens the window on the View page's `window`, keeping the page and
    /// the search of the last time.
    pub fn open(&mut self, window: WindowId) {
        self.open = true;
        self.window = Some(window);
    }

    /// Whether the window is open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Draws the window, when open.
    pub fn show(&mut self, ctx: &egui::Context, app: &App, requests: &mut Vec<Request>) {
        if !self.open {
            return;
        }
        let scheme = theme::scheme_of(ctx);
        if ctx.embed_viewports() {
            // No viewports (the tests, a web backend): the window over the
            // main one, in the dialogue frame.
            let frame = egui::Frame::new()
                .fill(scheme.dialog_bg)
                .corner_radius(theme::DIALOG_RADIUS)
                .shadow(egui::epaint::Shadow {
                    offset: [0, 8],
                    blur: 32,
                    spread: 0,
                    color: scheme.shadow(),
                })
                .inner_margin(0);
            egui::Area::new(egui::Id::new("settings-window"))
                .order(egui::Order::Foreground)
                .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    frame.show(ui, |ui| {
                        ui.set_min_size(SIZE);
                        ui.set_max_size(SIZE);
                        let rect = Rect::from_min_size(ui.max_rect().min, SIZE);
                        self.content(ui, rect, true, app, requests);
                    });
                });
        } else {
            let id = egui::ViewportId::from_hash_of("rick-settings");
            let builder = egui::ViewportBuilder::default()
                .with_title(t!("settings"))
                .with_inner_size(SIZE)
                .with_min_inner_size(MIN_SIZE);
            let mut close = false;
            ctx.show_viewport_immediate(id, builder, |ctx, _class| {
                if ctx.input(|i| i.viewport().close_requested()) {
                    close = true;
                }
                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(scheme.dialog_bg))
                    .show(ctx, |ui| {
                        let rect = ui.max_rect();
                        self.content(ui, rect, false, app, requests);
                    });
            });
            if close {
                self.open = false;
            }
        }
    }

    /// The two columns in `rect`; `rounded` rounds the column's corners,
    /// as the embedded frame does.
    fn content(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        rounded: bool,
        app: &App,
        requests: &mut Vec<Request>,
    ) {
        let scheme = theme::scheme(ui);
        let column = Rect::from_min_size(rect.min, vec2(COLUMN, rect.height()));
        let page = Rect::from_min_max(pos2(rect.left() + COLUMN, rect.top()), rect.max);
        let radius = if rounded {
            egui::CornerRadius {
                nw: theme::DIALOG_RADIUS,
                sw: theme::DIALOG_RADIUS,
                ne: 0,
                se: 0,
            }
        } else {
            egui::CornerRadius::ZERO
        };
        ui.painter().rect_filled(column, radius, scheme.bg_panel);
        ui.painter()
            .vline(column.right() - 0.5, column.y_range(), scheme.hairline());
        // The pages a search leaves: the current one when it still has a
        // row, else the first that has.
        let pages = Self::pages(app);
        let shown: Vec<&Page> = pages
            .iter()
            .filter(|page| self.page_matches(page, app))
            .collect();
        if !shown.is_empty() && !shown.contains(&&self.page) {
            self.page = shown[0].clone();
        }
        let mut column_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(column)
                .layout(egui::Layout::top_down(Align::Min)),
        );
        self.column(&mut column_ui, app, &pages);
        let mut page_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(page)
                .layout(egui::Layout::top_down(Align::Min)),
        );
        if self.page_ui(&mut page_ui, app, requests) {
            self.open = false;
        }
        ui.allocate_rect(rect, Sense::hover());
    }

    /// The pages, in the list's order: the four of Rick's own, then one
    /// per plugin that declares settings.
    fn pages(app: &App) -> Vec<Page> {
        let mut pages = vec![
            Page::General,
            Page::View,
            Page::Appearance,
            Page::Integration,
        ];
        for schema in app.registry().settings_schemas() {
            if !schema.fields.is_empty() {
                pages.push(Page::Plugin(schema.ext.0.to_owned()));
            }
        }
        pages
    }

    /// Whether a page has a row the search leaves.
    fn page_matches(&self, page: &Page, app: &App) -> bool {
        groups_of(page, app)
            .iter()
            .any(|group| group.rows.iter().any(|row| row.matches(&self.search)))
    }

    /// The left column: the title, the search field and the category list.
    fn column(&mut self, ui: &mut egui::Ui, app: &App, pages: &[Page]) {
        let scheme = theme::scheme(ui);
        let metrics = theme::metrics(ui.ctx());
        ui.add_space(COLUMN_INSET);
        let mut title = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(
                    pos2(ui.max_rect().left() + COLUMN_INSET, ui.cursor().top()),
                    vec2(COLUMN - 2.0 * COLUMN_INSET, 24.0),
                ))
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        title.add(
            egui::Label::new(
                egui::RichText::new(t!("settings"))
                    .size(theme::TEXT_HEADING)
                    .strong()
                    .color(scheme.text_primary),
            )
            .selectable(false),
        );
        ui.add_space(24.0 + 10.0);
        self.search_field(ui);
        ui.add_space(12.0);
        let mut plugins_header = false;
        for page in pages {
            if !self.page_matches(page, app) {
                continue;
            }
            if matches!(page, Page::Plugin(_)) && !plugins_header {
                plugins_header = true;
                ui.add_space(14.0);
                let mut header = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(Rect::from_min_size(
                            pos2(ui.max_rect().left() + COLUMN_INSET, ui.cursor().top()),
                            vec2(COLUMN - 2.0 * COLUMN_INSET, 18.0),
                        ))
                        .layout(egui::Layout::left_to_right(Align::Center)),
                );
                widgets::caps_heading(&mut header, &t!("settings-plugins"));
                ui.add_space(18.0 + 6.0);
            }
            let (row, response) =
                ui.allocate_exact_size(vec2(COLUMN, metrics.sidebar_item_height), Sense::click());
            let inner = Rect::from_min_max(
                pos2(row.left() + ROW_INSET, row.top()),
                pos2(row.right() - ROW_INSET, row.bottom()),
            );
            let active = *page == self.page;
            if active {
                ui.painter()
                    .rect_filled(inner, metrics.corner_radius(), scheme.bg_selected_soft);
            } else if response.hovered() {
                ui.painter()
                    .rect_filled(inner, metrics.corner_radius(), scheme.bg_subtle);
            }
            let fg = if active {
                scheme.text_on_selected
            } else {
                scheme.text_primary
            };
            ui.painter().text(
                pos2(
                    inner.left() + metrics.space_row_x + theme::ICON / 2.0,
                    inner.center().y,
                ),
                Align2::CENTER_CENTER,
                page_icon(page),
                FontId::proportional(theme::ICON),
                if active {
                    scheme.text_on_selected
                } else {
                    scheme.icon_default
                },
            );
            let name = page_name(page, app);
            ui.painter().text(
                pos2(
                    inner.left() + metrics.space_row_x + theme::ICON + 12.0,
                    inner.center().y,
                ),
                Align2::LEFT_CENTER,
                &name,
                FontId::proportional(theme::TEXT_BODY),
                fg,
            );
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::RadioButton,
                    true,
                    active,
                    name.clone(),
                )
            });
            if response.clicked() {
                self.page = page.clone();
            }
        }
    }

    /// The search field: a `bg/pane` field with the `SEARCH` glyph and the
    /// *Search settings* placeholder.
    fn search_field(&mut self, ui: &mut egui::Ui) {
        let scheme = theme::scheme(ui);
        let radius = theme::metrics(ui.ctx()).corner_radius();
        let rect = Rect::from_min_size(
            pos2(ui.max_rect().left() + COLUMN_INSET, ui.cursor().top()),
            vec2(COLUMN - 2.0 * COLUMN_INSET, SEARCH_HEIGHT),
        );
        ui.painter().rect(
            rect,
            radius,
            scheme.bg_pane,
            egui::Stroke::new(1.0, scheme.border_default),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            pos2(rect.left() + 10.0 + 7.0, rect.center().y),
            Align2::CENTER_CENTER,
            icons::SEARCH,
            FontId::proportional(14.0),
            scheme.icon_default,
        );
        let mut inner = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    pos2(rect.left() + 10.0 + 14.0 + 8.0, rect.top()),
                    pos2(rect.right() - 8.0, rect.bottom()),
                ))
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        let font = FontId::proportional(theme::TEXT_BODY);
        egui::TextEdit::singleline(&mut self.search)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .font(font.clone())
            .text_color(scheme.text_primary)
            .hint_text(
                egui::RichText::new(t!("settings-search-placeholder"))
                    .font(font)
                    .color(scheme.text_placeholder),
            )
            .desired_width(f32::INFINITY)
            .vertical_align(Align::Center)
            .show(&mut inner);
        ui.allocate_rect(rect, Sense::hover());
    }

    /// The right column: the page's title with the close button, then its
    /// groups as cards in a scroll area. Returns whether the close button
    /// was clicked.
    fn page_ui(&mut self, ui: &mut egui::Ui, app: &App, requests: &mut Vec<Request>) -> bool {
        let scheme = theme::scheme(ui);
        let (head, _) = ui.allocate_exact_size(
            vec2(ui.available_width(), PAGE_PAD + CLOSE_BUTTON + 12.0),
            Sense::hover(),
        );
        let title_y = head.top() + PAGE_PAD + CLOSE_BUTTON / 2.0;
        let name = page_name(&self.page, app);
        ui.painter().text(
            pos2(head.left() + PAGE_PAD, title_y),
            Align2::LEFT_CENTER,
            &name,
            FontId::proportional(theme::TEXT_HEADING),
            scheme.text_primary,
        );
        let heading = Rect::from_min_max(
            pos2(head.left() + PAGE_PAD, title_y - 12.0),
            pos2(
                head.right() - PAGE_PAD - CLOSE_BUTTON - 12.0,
                title_y + 12.0,
            ),
        );
        ui.interact(heading, ui.id().with("settings-page-title"), Sense::hover())
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, name.clone()));
        let close = close_button(
            ui,
            pos2(head.right() - PAGE_PAD - CLOSE_BUTTON / 2.0, title_y),
        );
        let groups = groups_of(&self.page, app);
        let search = self.search.clone();
        let page = self.page.clone();
        let mut cx = PageCx {
            app,
            window: self.window,
            requests,
        };
        egui::ScrollArea::vertical()
            .id_salt("settings-page")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin {
                        left: PAGE_PAD as i8,
                        right: PAGE_PAD as i8,
                        top: 4,
                        bottom: PAGE_PAD as i8,
                    })
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                        if page == Page::Integration {
                            integration_status(ui, &mut cx);
                            ui.add_space(20.0);
                        }
                        for (index, group) in groups.iter().enumerate() {
                            let visible: Vec<bool> =
                                group.rows.iter().map(|row| row.matches(&search)).collect();
                            if !visible.iter().any(|v| *v) {
                                continue;
                            }
                            group_ui(ui, group, &visible, |card| {
                                rows_of(&page, index, card, &mut cx);
                            });
                            ui.add_space(20.0);
                        }
                    });
            });
        close
    }
}

/// The round close button at `center`: `bg/control`, `bg/active` under the
/// pointer, the `CLOSE` glyph; named *Close*.
fn close_button(ui: &mut egui::Ui, center: egui::Pos2) -> bool {
    let scheme = theme::scheme(ui);
    let rect = Rect::from_center_size(center, vec2(CLOSE_BUTTON, CLOSE_BUTTON));
    let response = ui.interact(rect, ui.id().with("settings-close"), Sense::click());
    ui.painter().circle_filled(
        center,
        CLOSE_BUTTON / 2.0,
        if response.hovered() {
            scheme.bg_active
        } else {
            scheme.bg_control
        },
    );
    ui.painter().text(
        center,
        Align2::CENTER_CENTER,
        icons::CLOSE,
        FontId::proportional(14.0),
        scheme.icon_default,
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, t!("close")));
    response.clicked()
}

/// The category list's glyph for a page.
fn page_icon(page: &Page) -> &'static str {
    match page {
        Page::General => icons::GEAR,
        Page::View => icons::VIEW_LIST,
        Page::Appearance => icons::HIDDEN_SHOWN,
        Page::Integration => icons::LINK,
        Page::Plugin(id) => icons::for_plugin(id),
    }
}

/// A page's title: Rick's own pages' names, a plugin's manifest name.
fn page_name(page: &Page, app: &App) -> String {
    match page {
        Page::General => t!("settings-general"),
        Page::View => t!("settings-view"),
        Page::Appearance => t!("settings-appearance"),
        Page::Integration => t!("settings-integration"),
        Page::Plugin(id) => app
            .registry()
            .manifests()
            .iter()
            .find(|m| m.id.0 == id)
            .map_or_else(|| id.clone(), |m| m.name.to_owned()),
    }
}

/// The groups of a page with their rows' labels and descriptions, in the
/// order the page draws them (QUESTIONS.md 234).
#[allow(clippy::too_many_lines)]
fn groups_of(page: &Page, app: &App) -> Vec<Group> {
    match page {
        Page::General => vec![
            Group {
                title: t!("settings-group-interface"),
                rows: vec![Spec::described(
                    t!("settings-language"),
                    t!("settings-language-desc"),
                )],
            },
            Group {
                title: t!("settings-group-session"),
                rows: vec![Spec::described(
                    t!("settings-start-with"),
                    t!("settings-start-with-desc"),
                )],
            },
            Group {
                title: t!("settings-group-archives"),
                rows: vec![Spec::new(t!("settings-opening-an-archive"))],
            },
            Group {
                title: t!("settings-group-drag"),
                rows: vec![
                    Spec::new(t!("settings-after-a-drag")),
                    Spec::described(
                        t!("settings-spring-loading-delay"),
                        t!("settings-spring-loading-delay-desc"),
                    ),
                ],
            },
            Group {
                title: t!("settings-group-search"),
                rows: vec![Spec::described(
                    t!("settings-search-ignore"),
                    t!("settings-search-ignore-desc"),
                )],
            },
        ],
        Page::View => vec![
            Group {
                title: t!("settings-group-files"),
                rows: vec![
                    Spec::described(
                        t!("settings-folders-first"),
                        t!("settings-folders-first-desc"),
                    ),
                    Spec::new(t!("settings-show-hidden-files")),
                    Spec::new(t!("settings-show-system-files")),
                    Spec::new(t!("settings-show-file-extensions")),
                    Spec::described(
                        t!("settings-zebra-stripes"),
                        t!("settings-zebra-stripes-desc"),
                    ),
                ],
            },
            Group {
                title: t!("settings-group-sorting"),
                rows: vec![
                    Spec::new(t!("settings-sort-by")),
                    Spec::new(t!("settings-order")),
                ],
            },
            Group {
                title: t!("settings-side-panel"),
                rows: vec![
                    Spec::new(t!("settings-show-side-panel")),
                    Spec::new(t!("settings-position")),
                ],
            },
        ],
        Page::Appearance => vec![
            Group {
                title: t!("settings-group-colours"),
                rows: vec![Spec::described(
                    t!("settings-theme"),
                    t!("settings-theme-desc"),
                )],
            },
            Group {
                title: t!("settings-group-type"),
                rows: vec![
                    Spec::described(t!("settings-density"), t!("settings-density-desc")),
                    Spec::new(t!("settings-font-size")),
                ],
            },
        ],
        Page::Integration => integration_groups(app),
        Page::Plugin(id) => app
            .registry()
            .settings_schemas()
            .iter()
            .find(|schema| schema.ext.0 == id)
            .map(|schema| {
                vec![Group {
                    title: page_name(page, app),
                    rows: schema
                        .fields
                        .iter()
                        .map(|field| Spec::new(field.label.get()))
                        .collect(),
                }]
            })
            .unwrap_or_default(),
    }
}

/// A group: its title in `text/secondary` over a `bg/card` card whose rows
/// are separated by hairlines; `draw` adds the rows through the card.
fn group_ui(ui: &mut egui::Ui, group: &Group, visible: &[bool], draw: impl FnOnce(&mut Card<'_>)) {
    let scheme = theme::scheme(ui);
    let (title, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
    ui.painter().text(
        pos2(title.left(), title.center().y),
        Align2::LEFT_CENTER,
        &group.title,
        FontId::proportional(theme::TEXT_BODY),
        scheme.text_secondary,
    );
    ui.interact(
        title,
        ui.id().with(("settings-group", &group.title)),
        Sense::hover(),
    )
    .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, group.title.clone()));
    ui.add_space(8.0);
    egui::Frame::new()
        .fill(scheme.bg_card)
        .stroke(egui::Stroke::new(1.0, scheme.border_default))
        .corner_radius(theme::CARD_RADIUS)
        .inner_margin(0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            let mut card = Card {
                ui,
                rows: &group.rows,
                visible,
                drawn: 0,
            };
            draw(&mut card);
        });
}

/// A card being filled: `row` draws a setting's row when the search left
/// it, with the label, the description and the control at the right.
struct Card<'a> {
    ui: &'a mut egui::Ui,
    rows: &'a [Spec],
    visible: &'a [bool],
    drawn: usize,
}

impl Card<'_> {
    /// Row `index` of the group, its control drawn by `control` in a
    /// right-to-left child: the first widget sits at the right edge.
    fn row(&mut self, index: usize, control: impl FnOnce(&mut egui::Ui)) {
        if !self.visible.get(index).copied().unwrap_or(false) {
            return;
        }
        let Some(spec) = self.rows.get(index) else {
            return;
        };
        let scheme = theme::scheme(self.ui);
        let height = if spec.description.is_some() {
            ROW_DESCRIBED
        } else {
            ROW
        };
        let (rect, _) = self
            .ui
            .allocate_exact_size(vec2(self.ui.available_width(), height), Sense::hover());
        if self.drawn > 0 {
            self.ui.painter().hline(
                (rect.left() + 1.0)..=(rect.right() - 1.0),
                rect.top() + 0.5,
                scheme.hairline(),
            );
        }
        self.drawn += 1;
        let text_left = rect.left() + 16.0;
        let mut control_ui = self.ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    pos2(text_left + 120.0, rect.top()),
                    pos2(rect.right() - 14.0, rect.bottom()),
                ))
                .layout(egui::Layout::right_to_left(Align::Center)),
        );
        control_ui.spacing_mut().item_spacing.x = 8.0;
        control(&mut control_ui);
        let text_right = control_ui.min_rect().left().min(rect.right() - 14.0) - 16.0;
        let text_rect =
            Rect::from_min_max(pos2(text_left, rect.top()), pos2(text_right, rect.bottom()));
        let mut text = self.ui.new_child(
            egui::UiBuilder::new()
                .max_rect(text_rect)
                .layout(egui::Layout::top_down(Align::Min)),
        );
        text.spacing_mut().item_spacing = vec2(0.0, 2.0);
        let pad = if spec.description.is_some() {
            11.0
        } else {
            (height - 18.0) / 2.0
        };
        text.add_space(pad);
        text.add(
            egui::Label::new(
                egui::RichText::new(&spec.label)
                    .size(theme::TEXT_BODY)
                    .color(scheme.text_primary),
            )
            .truncate()
            .selectable(false),
        );
        if let Some(description) = &spec.description {
            text.add(
                egui::Label::new(
                    egui::RichText::new(description)
                        .size(theme::TEXT_SECONDARY)
                        .color(scheme.text_secondary),
                )
                .truncate()
                .selectable(false),
            );
        }
    }
}

/// A switch in a row: toggled on a click, named after the row.
fn switch_row(card: &mut Card<'_>, index: usize, on: bool, set: impl FnOnce(bool)) {
    let label = card
        .rows
        .get(index)
        .map(|s| s.label.clone())
        .unwrap_or_default();
    card.row(index, |ui| {
        if widgets::toggle(ui, on, &label).clicked() {
            set(!on);
        }
    });
}

/// A dropdown in a row.
fn dropdown_row(
    card: &mut Card<'_>,
    index: usize,
    id: egui::Id,
    options: &[String],
    selected: usize,
    set: impl FnOnce(usize),
) {
    card.row(index, |ui| {
        if let Some(picked) = widgets::dropdown(ui, id, DROPDOWN, options, selected, false) {
            set(picked);
        }
    });
}

/// A segmented control in a row, as wide as its options need.
fn segmented_row(
    card: &mut Card<'_>,
    index: usize,
    id: egui::Id,
    options: &[String],
    selected: usize,
    set: impl FnOnce(usize),
) {
    card.row(index, |ui| {
        let font = FontId::proportional(theme::TEXT_BODY);
        let widest = options
            .iter()
            .map(|o| {
                ui.painter()
                    .layout_no_wrap(o.clone(), font.clone(), egui::Color32::BLACK)
                    .size()
                    .x
            })
            .fold(0.0_f32, f32::max);
        #[allow(clippy::cast_precision_loss)]
        let width = (widest + 28.0) * options.len() as f32;
        let (rect, _) = ui.allocate_exact_size(vec2(width, SEGMENT_HEIGHT), Sense::hover());
        if let Some(picked) = widgets::segmented(ui, rect, id, options, selected) {
            set(picked);
        }
    });
}

/// A text field in a row; `set` gets the text when it changed.
fn field_row(
    card: &mut Card<'_>,
    index: usize,
    id: egui::Id,
    text: &str,
    set: impl FnOnce(String),
) {
    card.row(index, |ui| {
        let (slot, _) = ui.allocate_exact_size(vec2(FIELD, widgets::FIELD_HEIGHT), Sense::hover());
        let mut field_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(slot)
                .layout(egui::Layout::top_down(Align::Min))
                .id_salt(id),
        );
        let mut edited = text.to_owned();
        let response = dialog::text_field(&mut field_ui, &mut edited, "", false, None, false);
        if response.changed() {
            set(edited);
        }
    });
}

/// The rows of group `index` of `page`, through `card`.
#[allow(clippy::too_many_lines)]
fn rows_of(page: &Page, index: usize, card: &mut Card<'_>, cx: &mut PageCx<'_>) {
    let settings = &cx.app.model().settings;
    match (page, index) {
        (Page::General, 0) => {
            let mut options = vec![t!("settings-language-system")];
            let tags = ext::i18n::available();
            options.extend(tags.iter().map(|tag| ext::i18n::language_name(tag)));
            let selected = settings
                .language
                .as_ref()
                .and_then(|tag| tags.iter().position(|t| t == tag))
                .map_or(0, |i| i + 1);
            dropdown_row(
                card,
                0,
                egui::Id::new("settings-language"),
                &options,
                selected,
                |i| {
                    cx.change(SettingsChange::Language(
                        i.checked_sub(1).and_then(|i| tags.get(i).cloned()),
                    ));
                },
            );
        }
        (Page::General, 1) => {
            let layouts = cx.app.layouts();
            let mut options = vec![t!("continue-where-i-left-off")];
            options.extend(layouts.iter().cloned());
            let start = settings.session.start_layout();
            let selected = start
                .and_then(|name| layouts.iter().position(|l| l == name))
                .map_or(0, |i| i + 1);
            dropdown_row(
                card,
                0,
                egui::Id::new("settings-start"),
                &options,
                selected,
                |i| {
                    cx.change(SettingsChange::StartLayout(
                        i.checked_sub(1).and_then(|i| layouts.get(i).cloned()),
                    ));
                },
            );
        }
        (Page::General, 2) => {
            let options = vec![
                t!("settings-archives-enter"),
                t!("settings-archives-system"),
            ];
            let selected = usize::from(settings.archives.on_open == OnOpen::System);
            dropdown_row(
                card,
                0,
                egui::Id::new("settings-archives"),
                &options,
                selected,
                |i| {
                    cx.change(SettingsChange::ArchivesOnOpen(if i == 1 {
                        OnOpen::System
                    } else {
                        OnOpen::Enter
                    }));
                },
            );
        }
        (Page::General, 3) => {
            let options = vec![
                after_drag_name(AfterDrag::Stay),
                after_drag_name(AfterDrag::Return),
            ];
            let selected = usize::from(settings.drag.after == AfterDrag::Return);
            dropdown_row(
                card,
                0,
                egui::Id::new("settings-after-drag"),
                &options,
                selected,
                |i| {
                    cx.change(SettingsChange::AfterDrag(if i == 1 {
                        AfterDrag::Return
                    } else {
                        AfterDrag::Stay
                    }));
                },
            );
            let current = settings.drag.spring_delay_ms;
            let mut delays: Vec<u64> = DELAYS.to_vec();
            if !delays.contains(&current) {
                delays.push(current);
                delays.sort_unstable();
            }
            let options: Vec<String> = delays
                .iter()
                .map(|ms| t!("settings-milliseconds", value = *ms))
                .collect();
            let selected = delays.iter().position(|d| *d == current).unwrap_or(0);
            dropdown_row(
                card,
                1,
                egui::Id::new("settings-spring"),
                &options,
                selected,
                |i| {
                    if let Some(ms) = delays.get(i) {
                        cx.change(SettingsChange::SpringDelay(*ms));
                    }
                },
            );
        }
        (Page::General, 4) => {
            let text = settings.search.ignore.join(", ");
            field_row(
                card,
                0,
                egui::Id::new("settings-search-ignore"),
                &text,
                |edited| {
                    let names = edited
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .collect();
                    cx.change(SettingsChange::SearchIgnore(names));
                },
            );
        }
        (Page::View, 0) => {
            let flags: [(bool, Flag); 5] = [
                (settings.folders_first, SettingsChange::FoldersFirst),
                (settings.show_hidden, SettingsChange::ShowHidden),
                (settings.show_system, SettingsChange::ShowSystem),
                (settings.show_extensions, SettingsChange::ShowExtensions),
                (settings.zebra_stripes, SettingsChange::ZebraStripes),
            ];
            for (i, (on, make)) in flags.into_iter().enumerate() {
                switch_row(card, i, on, |v| cx.change(make(v)));
            }
        }
        (Page::View, 1) => {
            let options: Vec<String> = Column::ALL.into_iter().map(column_name).collect();
            let selected = Column::ALL
                .iter()
                .position(|c| *c == settings.sort_by)
                .unwrap_or(0);
            let dir = settings.sort_dir;
            dropdown_row(
                card,
                0,
                egui::Id::new("settings-sort-by"),
                &options,
                selected,
                |i| {
                    if let Some(column) = Column::ALL.get(i) {
                        cx.change(SettingsChange::Sort(*column, dir));
                    }
                },
            );
            let by = settings.sort_by;
            let options = vec![t!("settings-ascending"), t!("settings-descending")];
            segmented_row(
                card,
                1,
                egui::Id::new("settings-order"),
                &options,
                usize::from(dir == SortDir::Descending),
                |i| {
                    cx.change(SettingsChange::Sort(
                        by,
                        if i == 1 {
                            SortDir::Descending
                        } else {
                            SortDir::Ascending
                        },
                    ));
                },
            );
        }
        (Page::View, 2) => {
            let side_panel = cx
                .window
                .and_then(|w| cx.app.model().window(w).map(|w| w.side_panel.clone()))
                .unwrap_or_default();
            switch_row(card, 0, side_panel.visible, |v| {
                cx.panel(SidePanelChange::Visible(v));
            });
            let options = vec![t!("compare-left"), t!("compare-right")];
            segmented_row(
                card,
                1,
                egui::Id::new("settings-side"),
                &options,
                usize::from(side_panel.side == PanelSide::Right),
                |i| {
                    cx.panel(SidePanelChange::Side(if i == 1 {
                        PanelSide::Right
                    } else {
                        PanelSide::Left
                    }));
                },
            );
        }
        (Page::Appearance, 0) => {
            let themes = [Theme::System, Theme::Light, Theme::Dark];
            let options: Vec<String> = themes.into_iter().map(theme_name).collect();
            let selected = themes
                .iter()
                .position(|t| *t == settings.theme)
                .unwrap_or(0);
            segmented_row(
                card,
                0,
                egui::Id::new("settings-theme"),
                &options,
                selected,
                |i| {
                    if let Some(theme) = themes.get(i) {
                        cx.change(SettingsChange::Theme(*theme));
                    }
                },
            );
        }
        (Page::Appearance, 1) => {
            let densities = [Density::Comfortable, Density::Compact];
            let options: Vec<String> = densities.into_iter().map(density_name).collect();
            let selected = densities
                .iter()
                .position(|d| *d == settings.density)
                .unwrap_or(0);
            segmented_row(
                card,
                0,
                egui::Id::new("settings-density"),
                &options,
                selected,
                |i| {
                    if let Some(density) = densities.get(i) {
                        cx.change(SettingsChange::Density(*density));
                    }
                },
            );
            let sizes: Vec<u8> = FONT_SIZES.collect();
            let options: Vec<String> = sizes
                .iter()
                .map(|pt| t!("settings-points", value = *pt))
                .collect();
            let selected = sizes
                .iter()
                .position(|s| *s == settings.font_size)
                .unwrap_or(0);
            dropdown_row(
                card,
                1,
                egui::Id::new("settings-font-size"),
                &options,
                selected,
                |i| {
                    if let Some(size) = sizes.get(i) {
                        cx.change(SettingsChange::FontSize(*size));
                    }
                },
            );
        }
        (Page::Integration, group) => integration_rows(group, card, cx),
        (Page::Plugin(id), 0) => plugin_rows(id, card, cx),
        _ => {}
    }
}

/// The groups of the settings' Integration page (specs/system-integration.md
/// §6.1): the items of *This system* and the `rick` command under
/// *Terminal and links*, each with the line its state words, then the
/// preferences. A platform that lists no item has no row of the first
/// kind, and a group without rows is not drawn.
fn integration_groups(app: &App) -> Vec<Group> {
    let integration = &app.model().integration;
    let records = &app.model().settings.integration;
    let home = app.platform().home_dir();
    let specs = |terminal: bool| -> Vec<Spec> {
        integration::settings_rows(integration, terminal)
            .into_iter()
            .map(|row| {
                Spec::described(
                    integration::item_name(row.item),
                    integration::settings_line(
                        row,
                        integration.desktop,
                        records.record(row.item),
                        &home,
                    ),
                )
            })
            .collect()
    };
    let mut terminal = specs(true);
    terminal.push(Spec::described(
        t!("integration-shell-integration"),
        t!("integration-shell-integration-desc"),
    ));
    vec![
        Group {
            title: t!("integration-group-system"),
            rows: specs(false),
        },
        Group {
            title: t!("integration-group-terminal"),
            rows: terminal,
        },
        Group {
            title: t!("integration-group-startup"),
            rows: vec![
                Spec::described(
                    t!("integration-check-at-startup"),
                    t!("integration-check-at-startup-desc"),
                ),
                Spec::described(
                    t!("integration-skipped-items"),
                    t!("integration-skipped-items-desc"),
                ),
            ],
        },
    ]
}

/// The line over the groups of the settings' Integration page: the system, the
/// desktop and the last check at the left, *Check again* at the right. A
/// page opened before any check asks for one.
fn integration_status(ui: &mut egui::Ui, cx: &mut PageCx<'_>) {
    let scheme = theme::scheme(ui);
    let found = &cx.app.model().integration;
    let busy = found.checking || found.applying.is_some();
    if found.checked_at.is_none() && !busy {
        cx.requests.push(Request::Command(Command::Integration(
            IntegrationCommand::Check { startup: false },
        )));
    }
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
    let mut side = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::right_to_left(Align::Center)),
    );
    let again = widgets::icon_button(
        &mut side,
        Some(icons::REFRESH),
        &t!("integration-check-again"),
        false,
        !busy,
    );
    if again.clicked() && !busy {
        cx.requests.push(Request::Command(Command::Integration(
            IntegrationCommand::Check { startup: false },
        )));
    }
    let mut text = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(
                rect.min,
                pos2(side.min_rect().left() - 12.0, rect.bottom()),
            ))
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    text.add(
        egui::Label::new(
            egui::RichText::new(integration::checked_line(found))
                .size(theme::TEXT_SECONDARY)
                .color(scheme.text_secondary),
        )
        .truncate()
        .selectable(false),
    );
}

/// The rows of group `group` of the settings' Integration page: an item's pill
/// and, where its state has one, its button; the switches for shell
/// integration and the check at startup; *Reset choices*.
fn integration_rows(group: usize, card: &mut Card<'_>, cx: &mut PageCx<'_>) {
    let app = cx.app;
    let found = &app.model().integration;
    let settings = &app.model().settings.integration;
    let busy = found.checking || found.applying.is_some();
    if group == 2 {
        switch_row(card, 0, settings.check_at_startup, |on| {
            cx.change(SettingsChange::IntegrationCheckAtStartup(on));
        });
        // Something to ask again about: a row switched off, or the whole
        // dialog silenced.
        let choices = !settings.ask || Item::ALL.iter().any(|item| settings.declined(*item));
        card.row(1, |ui| {
            let reset = widgets::button(ui, &t!("integration-reset-choices"), false, choices);
            if reset.clicked() && choices && !busy {
                cx.requests.push(Request::Command(Command::Integration(
                    IntegrationCommand::Reset,
                )));
            }
        });
        return;
    }
    let rows = integration::settings_rows(found, group == 1);
    for (index, row) in rows.iter().enumerate() {
        card.row(index, |ui| {
            let scheme = theme::scheme(ui);
            if let Some((label, primary, command)) = integration::status_action(row) {
                let button = widgets::button(ui, &label, primary, !busy);
                // Read aloud with its row's name: several rows have an
                // *Add*.
                let spoken = t!(
                    "integration-action-of",
                    action = label.as_str(),
                    item = integration::item_name(row.item)
                );
                button.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, !busy, spoken.clone())
                });
                if button.clicked() && !busy {
                    cx.requests
                        .push(Request::Command(Command::Integration(command)));
                }
            }
            let (text, bg, fg) = integration::status_pill(&row.status, scheme);
            widgets::pill(ui, &text, bg, fg);
        });
    }
    if group == 1 {
        switch_row(card, rows.len(), settings.shell_integration, |on| {
            cx.change(SettingsChange::ShellIntegration(on));
        });
    }
}

/// A switch's setting change, by its new value.
type Flag = fn(bool) -> SettingsChange;

/// A plugin's rows: its schema's fields, each control by its kind, every
/// change written to the plugin's own file.
fn plugin_rows(id: &str, card: &mut Card<'_>, cx: &mut PageCx<'_>) {
    let Some(schema) = cx
        .app
        .registry()
        .settings_schemas()
        .iter()
        .find(|schema| schema.ext.0 == id)
        .cloned()
    else {
        return;
    };
    for (i, field) in schema.fields.iter().enumerate() {
        let current = cx
            .app
            .ext_settings()
            .get(schema.ext.0, field.key)
            .unwrap_or_else(|| field.kind.default_value());
        let mut set = |value: ext::SettingValue| {
            cx.requests.push(Request::Command(Command::ExtSetting {
                ext: schema.ext.0.to_owned(),
                key: field.key.to_owned(),
                value,
            }));
        };
        let salt = egui::Id::new(("settings-plugin", schema.ext.0, field.key));
        match (&field.kind, &current) {
            (ext::SettingKind::Bool { .. }, ext::SettingValue::Bool(on)) => {
                switch_row(card, i, *on, |v| set(ext::SettingValue::Bool(v)));
            }
            (ext::SettingKind::Text { .. }, ext::SettingValue::Text(text)) => {
                field_row(card, i, salt, text, |edited| {
                    set(ext::SettingValue::Text(edited));
                });
            }
            (ext::SettingKind::Integer { min, max, .. }, ext::SettingValue::Integer(value)) => {
                let (min, max) = (*min, *max);
                field_row(card, i, salt, &value.to_string(), |edited| {
                    if let Ok(number) = edited.trim().parse::<i64>()
                        && (min..=max).contains(&number)
                    {
                        set(ext::SettingValue::Integer(number));
                    }
                });
            }
            (ext::SettingKind::Choice { options, .. }, ext::SettingValue::Choice(chosen)) => {
                let texts: Vec<String> = options.iter().map(|o| (*o).to_owned()).collect();
                let selected = options.iter().position(|o| *o == chosen).unwrap_or(0);
                dropdown_row(card, i, salt, &texts, selected, |picked| {
                    if let Some(option) = options.get(picked) {
                        set(ext::SettingValue::Choice((*option).to_owned()));
                    }
                });
            }
            _ => {}
        }
    }
}

fn column_name(column: Column) -> String {
    match column {
        Column::Name => t!("column-name"),
        Column::Type => t!("column-type"),
        Column::Modified => t!("column-date-modified"),
        Column::Created => t!("column-date-created"),
        Column::Accessed => t!("column-date-accessed"),
        Column::Size => t!("column-size"),
        Column::Items => t!("column-items"),
    }
}

fn theme_name(theme: Theme) -> String {
    match theme {
        Theme::System => t!("settings-theme-system"),
        Theme::Light => t!("settings-theme-light"),
        Theme::Dark => t!("settings-theme-dark"),
    }
}

fn after_drag_name(after: AfterDrag) -> String {
    match after {
        AfterDrag::Stay => t!("settings-after-drag-stay"),
        AfterDrag::Return => t!("settings-after-drag-return"),
    }
}

fn density_name(density: Density) -> String {
    match density {
        Density::Compact => t!("settings-density-compact"),
        Density::Comfortable => t!("settings-density-comfortable"),
    }
}

/// The Extensions screen: the plugins the host found, each with its name,
/// version, the points it fills and its status (loaded, disabled, or
/// refused with the reason), and an Enabled toggle per row. The toggle is a
/// `SettingsChange::Plugin`, which writes `rick.toml`; the row says that it
/// takes effect on the next start.
pub fn extensions_window(
    ctx: &egui::Context,
    open: &mut bool,
    app: &App,
    requests: &mut Vec<Request>,
) {
    let plugins = app.plugins();
    let disabled = &app.model().settings.plugins.disabled;
    egui::Window::new(t!("extensions"))
        .open(open)
        .default_width(640.0)
        .show(ctx, |ui| {
            if plugins.is_empty() {
                ui.label(t!("settings-no-plugins-found"));
            }
            egui::Grid::new("extensions-grid")
                .num_columns(5)
                .striped(true)
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    for heading in ["Name", "Version", "Points", "Status", ""] {
                        ui.strong(heading);
                    }
                    ui.end_row();
                    for plugin in plugins {
                        let name = if plugin.name.is_empty() {
                            plugin.id.as_str()
                        } else {
                            plugin.name.as_str()
                        };
                        ui.label(name).on_hover_text(format!(
                            "{}\n{}",
                            plugin.id,
                            plugin.path.display()
                        ));
                        ui.label(&plugin.version);
                        ui.label(plugin.points.to_string());
                        let status = match &plugin.status {
                            PluginStatus::Loaded => "loaded".to_owned(),
                            PluginStatus::Disabled => "disabled".to_owned(),
                            PluginStatus::Refused(reason) => format!("refused: {reason}"),
                        };
                        ui.label(status);
                        let mut enabled = !disabled.contains(&plugin.id);
                        ui.horizontal(|ui| {
                            if ui.checkbox(&mut enabled, t!("settings-enabled")).changed() {
                                requests.push(Request::Command(Command::Settings(
                                    SettingsChange::Plugin {
                                        id: plugin.id.clone(),
                                        enabled,
                                    },
                                )));
                            }
                            let disabled_at_start = plugin.status == PluginStatus::Disabled;
                            if enabled == disabled_at_start {
                                ui.weak(if enabled {
                                    "loads on the next start"
                                } else {
                                    "disabled on the next start"
                                });
                            }
                        });
                        ui.end_row();
                    }
                });
            ui.separator();
            ui.weak(
                "Plugins live in the plugins folder beside the binary and in the config \
                 directory; a per-user plugin wins on an id clash. Changes take effect \
                 on the next start.",
            );
        });
}
