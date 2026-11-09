//! The Third-party licences page of the About dialog
//! (specs/about-dialog.md): every component Rick ships with its licence,
//! in sections — the bundled runtimes of `about/bundled.toml` first, the
//! Rust crates of the generated `third-party.json` alphabetically, then one
//! section per plugin loaded from outside the release's `plugins/` folder
//! that brought a `licenses.json`. A search field filters by name and a
//! dropdown by licence id; a row opens in place with the copyright holder,
//! the repository and the full licence text, one at a time; *Export…*
//! writes the whole list with every text as one plain-text file through
//! the save dialog.

use std::path::PathBuf;
use std::sync::Arc;

use app::App;
use egui::{Align, Align2, FontId, Rect, Sense, pos2, vec2};
use ext::{Level, t};

use crate::about::{Component, PluginLicences};
use crate::shell::Request;
use crate::{icons, theme, widgets};

/// The side padding of the tools, the list and the footer.
const PAD: f32 = 24.0;
/// The gap under the title bar, and the tools row with the gap under it.
const HEAD_GAP: f32 = 10.0;
const TOOLS: f32 = 42.0;
const TOOL_HEIGHT: f32 = 30.0;
/// A section header, a row, the footer.
const SECTION: f32 = 29.0;
const ROW: f32 = 35.0;
const FOOTER: f32 = 58.0;
/// The licence text's size in the open row.
const TEXT_SIZE: f32 = 11.0;
/// The suggested name of the exported file.
const EXPORT_FILE: &str = "third-party-licences.txt";

/// A section of the list: its title and the components in their order.
#[derive(Debug)]
pub struct Section<'a> {
    /// The title, as the header shows it.
    pub title: String,
    /// The components.
    pub components: Vec<&'a Component>,
}

/// The sections: the bundled runtimes as listed, the crates sorted by name
/// (case aside), version and licence, then the plugins' entries under the
/// plugin's name. `crates` is `None` in a build without the generated
/// file, which leaves the section out.
#[must_use]
pub fn sections<'a>(
    bundled: &'a [Component],
    crates: Option<&'a [Component]>,
    plugins: &'a [PluginLicences],
) -> Vec<Section<'a>> {
    let mut out = vec![Section {
        title: t!("about-bundled-runtimes"),
        components: bundled.iter().collect(),
    }];
    if let Some(crates) = crates {
        let mut components: Vec<&Component> = crates.iter().collect();
        components.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.version.cmp(&b.version))
                .then_with(|| a.license.cmp(&b.license))
        });
        out.push(Section {
            title: t!("about-rust-crates"),
            components,
        });
    }
    for plugin in plugins {
        out.push(Section {
            title: plugin.name.clone(),
            components: plugin.components.iter().collect(),
        });
    }
    out
}

/// The number of components, each once by name and version, and of
/// licence ids over the sections.
#[must_use]
pub fn counts(sections: &[Section<'_>]) -> (usize, usize) {
    let mut components = std::collections::BTreeSet::new();
    let mut licences = std::collections::BTreeSet::new();
    for section in sections {
        for component in &section.components {
            components.insert((component.name.as_str(), component.version.as_str()));
            licences.insert(component.license.as_str());
        }
    }
    (components.len(), licences.len())
}

/// The licence ids over the sections, sorted, for the dropdown.
#[must_use]
pub fn licence_ids(sections: &[Section<'_>]) -> Vec<String> {
    let mut ids: Vec<String> = sections
        .iter()
        .flat_map(|s| s.components.iter().map(|c| c.license.clone()))
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// Whether a component passes the search (a case-insensitive part of the
/// name) and the licence filter.
#[must_use]
pub fn matches(component: &Component, query: &str, licence: Option<&str>) -> bool {
    let query = query.trim().to_lowercase();
    (query.is_empty() || component.name.to_lowercase().contains(&query))
        && licence.is_none_or(|id| component.license == id)
}

/// The exported text: a header, then every section with every component
/// — name, version and licence id, the copyright line, the URL and the
/// full licence text.
#[must_use]
pub fn export_text(version: &str, sections: &[Section<'_>], generated: bool) -> String {
    let mut out = t!("about-export-header", version = version);
    out.push('\n');
    if generated {
        out.push_str(&t!("about-export-generated"));
        out.push('\n');
    }
    for section in sections {
        out.push_str("\n\n== ");
        out.push_str(&section.title);
        out.push_str(" ==\n");
        for component in &section.components {
            out.push_str("\n--- ");
            out.push_str(&component.name);
            out.push(' ');
            out.push_str(&component.version);
            out.push_str(" (");
            out.push_str(&component.license);
            out.push_str(") ---\n");
            let copyright = component.copyright_line();
            if !copyright.is_empty() {
                out.push_str(&copyright);
                out.push('\n');
            }
            if !component.repository.is_empty() {
                out.push_str(&component.repository);
                out.push('\n');
            }
            out.push('\n');
            out.push_str(component.text.trim_end());
            out.push('\n');
        }
    }
    out
}

/// What the page is showing and doing.
#[derive(Debug, Default)]
pub struct LicencesPage {
    /// The search field's text.
    pub search: String,
    /// The licence id the dropdown filters to, `None` for all.
    pub licence: Option<String>,
    /// The open row: the section's index and the component's index in it.
    open: Option<(usize, usize)>,
    /// An export in progress: the path written, or why not.
    export: Option<runtime::oneshot::Receiver<Result<PathBuf, String>>>,
    /// Give the search field the keyboard on the next frame.
    focus: bool,
}

/// What a frame's clicks asked for.
#[derive(Default)]
struct Asked {
    toggle: Option<(usize, usize)>,
    open_url: Option<String>,
    export: bool,
}

impl LicencesPage {
    /// A fresh page, the search field about to take the keyboard.
    #[must_use]
    pub fn new() -> Self {
        Self {
            focus: true,
            ..Self::default()
        }
    }

    /// The open row, for the tests.
    #[must_use]
    pub fn open_row(&self) -> Option<(usize, usize)> {
        self.open
    }

    /// Whether an export is still being written.
    #[must_use]
    pub fn exporting(&self) -> bool {
        self.export.is_some()
    }

    /// Draws the page under the dialog's title bar: the tools row, the
    /// list in the room the page's height leaves, the footer under it.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        sections: &[Section<'_>],
        generated: bool,
        version: &str,
        app: &App,
        requests: &mut Vec<Request>,
    ) {
        self.poll_export(ui.ctx(), requests);
        let scheme = theme::scheme(ui);
        let mut asked = Asked::default();
        ui.add_space(HEAD_GAP);
        let (count, types) = counts(sections);
        self.tools(ui, sections, count);
        let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
        ui.painter()
            .hline(line.x_range(), line.top() + 0.5, scheme.hairline());
        let list_height =
            crate::about::PAGE_HEIGHT - crate::about::TITLE_BAR - HEAD_GAP - TOOLS - 1.0 - FOOTER;
        egui::ScrollArea::vertical()
            .id_salt("about-licences")
            .auto_shrink([false, false])
            .max_height(list_height)
            .min_scrolled_height(list_height)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                self.list(ui, sections, &mut asked);
            });
        self.footer(ui, generated, count, types, &mut asked);

        if let Some(key) = asked.toggle {
            self.open = if self.open == Some(key) {
                None
            } else {
                Some(key)
            };
        }
        if let Some(url) = asked.open_url {
            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
        }
        if asked.export && self.export.is_none() {
            self.start_export(version, sections, generated, app);
        }
    }

    /// The search field and the licence dropdown.
    fn tools(&mut self, ui: &mut egui::Ui, sections: &[Section<'_>], count: usize) {
        let scheme = theme::scheme(ui);
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), TOOLS), Sense::hover());
        let top = row.top();
        let ids = licence_ids(sections);
        let mut options = vec![t!("about-all-licences")];
        options.extend(ids.iter().cloned());
        let font = FontId::proportional(theme::TEXT_BODY);
        let widest = options
            .iter()
            .map(|o| {
                ui.painter()
                    .layout_no_wrap(o.clone(), font.clone(), scheme.text_primary)
                    .size()
                    .x
            })
            .fold(0.0_f32, f32::max);
        let dropdown_width = (widest + 44.0).clamp(120.0, 220.0);
        let search = Rect::from_min_size(
            pos2(row.left() + PAD, top),
            vec2(row.width() - 2.0 * PAD - dropdown_width - 10.0, TOOL_HEIGHT),
        );
        ui.painter().rect(
            search,
            8.0,
            scheme.bg_card,
            egui::Stroke::new(1.0, scheme.border_default),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            pos2(search.left() + 10.0 + theme::ICON / 2.0, search.center().y),
            Align2::CENTER_CENTER,
            icons::SEARCH,
            FontId::proportional(theme::ICON),
            scheme.icon_default,
        );
        let mut field = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    pos2(search.left() + 10.0 + theme::ICON + 6.0, search.top()),
                    pos2(search.right() - 8.0, search.bottom()),
                ))
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        let hint = t!("about-search-components", count = count as u64);
        let response = egui::TextEdit::singleline(&mut self.search)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .font(FontId::proportional(theme::TEXT_SECONDARY))
            .text_color(scheme.text_primary)
            .hint_text(
                egui::RichText::new(&hint)
                    .size(theme::TEXT_SECONDARY)
                    .color(scheme.text_placeholder),
            )
            .desired_width(f32::INFINITY)
            .vertical_align(Align::Center)
            .show(&mut field)
            .response;
        if std::mem::take(&mut self.focus) {
            response.request_focus();
        }
        let selected = self
            .licence
            .as_ref()
            .and_then(|id| ids.iter().position(|i| i == id))
            .map_or(0, |i| i + 1);
        let mut dropdown = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(
                    pos2(row.right() - PAD - dropdown_width, top - 3.0),
                    vec2(dropdown_width, widgets::FIELD_HEIGHT),
                ))
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        if let Some(picked) = widgets::dropdown(
            &mut dropdown,
            egui::Id::new("about-licence-filter"),
            dropdown_width,
            &options,
            selected,
            false,
        ) {
            self.licence = picked.checked_sub(1).and_then(|i| ids.get(i).cloned());
        }
    }

    /// The sections and their rows that pass the filters; the open row
    /// expanded under itself.
    fn list(&self, ui: &mut egui::Ui, sections: &[Section<'_>], asked: &mut Asked) {
        let scheme = theme::scheme(ui);
        let mut any = false;
        for (s, section) in sections.iter().enumerate() {
            let rows: Vec<(usize, &Component)> = section
                .components
                .iter()
                .enumerate()
                .filter(|(_, c)| matches(c, &self.search, self.licence.as_deref()))
                .map(|(i, c)| (i, *c))
                .collect();
            if rows.is_empty() {
                continue;
            }
            any = true;
            let (head, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), SECTION), Sense::hover());
            if ui.is_rect_visible(head) {
                ui.painter().text(
                    pos2(head.left() + PAD, head.top() + 12.0 + 6.5),
                    Align2::LEFT_CENTER,
                    section.title.to_uppercase(),
                    FontId::proportional(theme::TEXT_CAPS),
                    scheme.text_secondary,
                );
            }
            ui.interact(head, ui.id().with(("about-section", s)), Sense::hover())
                .widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Label,
                        true,
                        section.title.to_uppercase(),
                    )
                });
            for (i, component) in rows {
                let open = self.open == Some((s, i));
                if row(ui, component, open, (s, i)).clicked() {
                    asked.toggle = Some((s, i));
                }
                if open {
                    expanded(ui, component, asked);
                }
            }
        }
        if !any {
            let (rect, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), ROW * 2.0), Sense::hover());
            let text = t!("about-no-matches");
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                &text,
                FontId::proportional(theme::TEXT_BODY),
                scheme.text_secondary,
            );
            ui.interact(rect, ui.id().with("about-no-matches"), Sense::hover())
                .widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text.clone())
                });
        }
    }

    /// The footer: the generation line and the Export button.
    fn footer(
        &self,
        ui: &mut egui::Ui,
        generated: bool,
        count: usize,
        types: usize,
        asked: &mut Asked,
    ) {
        let scheme = theme::scheme(ui);
        let (band, _) = ui.allocate_exact_size(vec2(ui.available_width(), FOOTER), Sense::hover());
        ui.painter()
            .hline(band.x_range(), band.top() + 0.5, scheme.hairline());
        let label = t!("about-export");
        let button_width = ui
            .painter()
            .layout_no_wrap(
                label.clone(),
                FontId::proportional(theme::TEXT_BODY),
                scheme.text_primary,
            )
            .size()
            .x
            + 32.0;
        let mut button = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(
                    pos2(band.right() - PAD - button_width, band.center().y - 18.0),
                    vec2(button_width, 36.0),
                ))
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        if widgets::button(&mut button, &label, false, !self.exporting()).clicked() {
            asked.export = true;
        }
        let text = if generated {
            t!(
                "about-licences-footer",
                count = count as u64,
                types = types as u64
            )
        } else {
            t!(
                "about-licences-footer-partial",
                count = count as u64,
                types = types as u64
            )
        };
        let text_rect = Rect::from_min_max(
            pos2(band.left() + PAD, band.top() + 1.0),
            pos2(band.right() - PAD - button_width - 10.0, band.bottom()),
        );
        let mut note = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(text_rect)
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        note.add(
            egui::Label::new(
                egui::RichText::new(text)
                    .size(theme::TEXT_SECONDARY)
                    .color(scheme.text_secondary),
            )
            .wrap()
            .selectable(false),
        );
    }

    /// Asks the platform for a file and writes the list into it, off the
    /// frame; the outcome arrives through `export`.
    fn start_export(
        &mut self,
        version: &str,
        sections: &[Section<'_>],
        generated: bool,
        app: &App,
    ) {
        let text = export_text(version, sections, generated);
        let title = t!("about-export-title");
        let platform = Arc::clone(app.platform());
        let (tx, rx) = runtime::oneshot::channel();
        runtime::handle().spawn(async move {
            let Some(path) = platform.save_file(&title, EXPORT_FILE).await else {
                // Cancelled: nothing to say.
                drop(tx);
                return;
            };
            let written = runtime::handle()
                .spawn_blocking(move || {
                    std::fs::write(&path, text)
                        .map(|()| path.clone())
                        .map_err(|e| format!("{}: {e}", path.display()))
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
            let _ = tx.send(written);
        });
        self.export = Some(rx);
    }

    /// Reports an export that finished.
    fn poll_export(&mut self, ctx: &egui::Context, requests: &mut Vec<Request>) {
        let Some(rx) = self.export.as_mut() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(path)) => {
                requests.push(Request::Toast(
                    Level::Info,
                    t!("about-exported", path = path.display().to_string()),
                ));
                self.export = None;
            }
            Ok(Err(error)) => {
                requests.push(Request::Toast(
                    Level::Error,
                    t!("about-export-failed", error = error),
                ));
                self.export = None;
            }
            Err(runtime::oneshot::error::TryRecvError::Closed) => self.export = None,
            Err(runtime::oneshot::error::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
    }
}

/// A row: the name, the version in mono, the licence pill and the
/// chevron; `bg/selected-soft` while open. Named "name version".
fn row(
    ui: &mut egui::Ui,
    component: &Component,
    open: bool,
    key: (usize, usize),
) -> egui::Response {
    let scheme = theme::scheme(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    let spoken = format!("{} {}", component.name, component.version);
    if ui.is_rect_visible(rect) {
        if open {
            ui.painter().rect_filled(rect, 0.0, scheme.bg_selected_soft);
        } else if response.hovered() {
            ui.painter().rect_filled(rect, 0.0, scheme.bg_subtle);
        }
        let name = ui.painter().layout_no_wrap(
            component.name.clone(),
            FontId::proportional(theme::TEXT_BODY),
            scheme.text_primary,
        );
        let name_width = name.size().x;
        ui.painter().galley(
            pos2(rect.left() + PAD, rect.center().y - name.size().y / 2.0),
            name,
            scheme.text_primary,
        );
        ui.painter().text(
            pos2(rect.left() + PAD + name_width + 8.0, rect.center().y),
            Align2::LEFT_CENTER,
            &component.version,
            FontId::monospace(theme::TEXT_SECONDARY),
            scheme.text_secondary,
        );
        let caret_x = rect.right() - PAD - theme::ICON / 2.0;
        ui.painter().text(
            pos2(caret_x, rect.center().y),
            Align2::CENTER_CENTER,
            icons::SEGMENT,
            FontId::proportional(theme::ICON),
            scheme.icon_default,
        );
        let pill = ui.painter().layout_no_wrap(
            component.license.clone(),
            FontId::proportional(theme::TEXT_CAPS),
            scheme.trash_low_text,
        );
        let pill_size = pill.size() + vec2(16.0, 4.0);
        let pill_rect = Rect::from_min_size(
            pos2(
                caret_x - theme::ICON / 2.0 - 12.0 - pill_size.x,
                rect.center().y - pill_size.y / 2.0,
            ),
            pill_size,
        );
        ui.painter()
            .rect_filled(pill_rect, pill_size.y / 2.0, scheme.trash_low_bg);
        ui.painter().galley(
            pill_rect.center() - pill.size() / 2.0,
            pill,
            scheme.trash_low_text,
        );
    }
    let _ = key;
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, open, spoken.clone())
    });
    response
}

/// The open row's block: the copyright line and the repository link, then
/// the licence text in a mono box on `bg/card`, all on `bg/selected-soft`.
fn expanded(ui: &mut egui::Ui, component: &Component, asked: &mut Asked) {
    let scheme = theme::scheme(ui);
    let width = ui.available_width();
    let box_width = width - 2.0 * PAD;
    let text_width = box_width - 24.0 - 2.0;
    let galley = ui.fonts_mut(|f| {
        f.layout(
            component.text.trim_end().to_owned(),
            FontId::monospace(TEXT_SIZE),
            scheme.text_secondary,
            text_width,
        )
    });
    let box_height = galley.size().y + 20.0;
    let height = 8.0 + 14.0 + 8.0 + box_height + 14.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    ui.painter().rect_filled(rect, 0.0, scheme.bg_selected_soft);
    let meta_y = rect.top() + 8.0 + 7.0;
    let copyright = component.copyright_line();
    let copyright_width = if copyright.is_empty() {
        0.0
    } else {
        let galley = ui.painter().layout_no_wrap(
            copyright.clone(),
            FontId::proportional(theme::TEXT_SECONDARY),
            scheme.text_secondary,
        );
        let size = galley.size();
        let copyright_rect =
            Rect::from_min_size(pos2(rect.left() + PAD, meta_y - size.y / 2.0), size);
        ui.painter()
            .galley(copyright_rect.min, galley, scheme.text_secondary);
        ui.interact(
            copyright_rect,
            ui.id()
                .with(("about-copyright", &component.name, &component.version)),
            Sense::hover(),
        )
        .widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, copyright.clone())
        });
        size.x + 14.0
    };
    if !component.repository.is_empty() {
        let link = ui.painter().layout_no_wrap(
            crate::about::short_url(&component.repository),
            FontId::proportional(theme::TEXT_SECONDARY),
            scheme.text_on_selected,
        );
        let link_rect = Rect::from_min_size(
            pos2(
                rect.left() + PAD + copyright_width,
                meta_y - link.size().y / 2.0,
            ),
            link.size(),
        );
        ui.painter()
            .galley(link_rect.min, link, scheme.text_on_selected);
        let response = ui.interact(
            link_rect,
            ui.id()
                .with(("about-repo", &component.name, &component.version)),
            Sense::click(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Link, true, component.repository.clone())
        });
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() {
            asked.open_url = Some(component.repository.clone());
        }
    }
    let text_box = Rect::from_min_size(
        pos2(rect.left() + PAD, rect.top() + 8.0 + 14.0 + 8.0),
        vec2(box_width, box_height),
    );
    ui.painter().rect(
        text_box,
        6.0,
        scheme.bg_card,
        egui::Stroke::new(1.0, scheme.border_default),
        egui::StrokeKind::Inside,
    );
    let mut body = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(text_box.shrink2(vec2(12.0, 10.0)))
            .layout(egui::Layout::top_down(Align::Min)),
    );
    body.add(
        egui::Label::new(
            egui::RichText::new(component.text.trim_end())
                .font(FontId::monospace(TEXT_SIZE))
                .color(scheme.text_secondary),
        )
        .wrap()
        .selectable(true),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn component(name: &str, version: &str, license: &str, text: &str) -> Component {
        Component {
            name: name.to_owned(),
            version: version.to_owned(),
            license: license.to_owned(),
            copyright: vec![format!("{name} authors")],
            repository: format!("https://example.org/{name}"),
            text: text.to_owned(),
        }
    }

    #[test]
    fn the_sections_are_bundled_sorted_crates_then_plugins() {
        let bundled = vec![component("pdfium", "1", "BSD-3-Clause", "bsd")];
        let crates = vec![
            component("serde", "1.0", "MIT", "mit text"),
            component("Egui", "0.36", "Apache-2.0", "apache"),
            component("egui", "0.35", "MIT", "mit"),
        ];
        let plugins = vec![PluginLicences {
            name: "Probe".to_owned(),
            components: vec![component("probe-dep", "2", "ISC", "isc")],
        }];
        let list = sections(&bundled, Some(&crates), &plugins);
        // The unit tests run without the translations: `t!` gives the key.
        let titles: Vec<&str> = list.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                t!("about-bundled-runtimes"),
                t!("about-rust-crates"),
                "Probe".to_owned()
            ]
        );
        let names: Vec<String> = list[1]
            .components
            .iter()
            .map(|c| format!("{} {}", c.name, c.version))
            .collect();
        assert_eq!(names, ["egui 0.35", "Egui 0.36", "serde 1.0"]);
        assert_eq!(counts(&list), (5, 4));
        assert_eq!(
            licence_ids(&list),
            ["Apache-2.0", "BSD-3-Clause", "ISC", "MIT"]
        );
        assert_eq!(sections(&bundled, None, &[]).len(), 1);

        assert!(matches(&crates[0], "", None));
        assert!(matches(&crates[0], "SER", None));
        assert!(!matches(&crates[0], "egui", None));
        assert!(matches(&crates[0], "", Some("MIT")));
        assert!(!matches(&crates[0], "", Some("Apache-2.0")));

        let text = export_text("0.3.0-dev", &list, true);
        let header = t!("about-export-header", version = "0.3.0-dev");
        assert!(text.starts_with(&format!("{header}\n")), "{text}");
        assert!(text.contains(&format!("== {} ==\n", t!("about-rust-crates"))));
        assert!(text.contains(
            "\n--- serde 1.0 (MIT) ---\nserde authors\nhttps://example.org/serde\n\nmit text\n"
        ));
        assert!(text.contains("== Probe ==\n\n--- probe-dep 2 (ISC) ---"));
        assert!(text.contains("pdfium 1 (BSD-3-Clause)"));
        let partial = export_text("0.3.0-dev", &list, false);
        assert!(!partial.contains(&t!("about-export-generated")));
        assert!(text.contains(&t!("about-export-generated")));
    }
}
