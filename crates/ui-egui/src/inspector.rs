//! The inspector pane: what is selected in the focused pane, and a preview
//! placeholder.

use std::time::SystemTime;

use app::App;
use model::PaneId;
use types::EntryKind;

use crate::format;

/// Draws the inspector for `pane`'s selection.
pub fn show(
    ui: &mut egui::Ui,
    app: &App,
    pane: PaneId,
    now: SystemTime,
    previews: &mut crate::preview::Previews,
) {
    ui.heading("Inspector");
    ui.separator();
    let Some((_, tab)) = app.model().active_tab(pane) else {
        ui.weak("No tab");
        return;
    };
    let selected: Vec<&types::Entry> = tab.selected().collect();
    match selected.as_slice() {
        [] => {
            ui.strong(crate::paths::title(&tab.location.path));
            ui.weak(tab.location.path.to_string_lossy());
            ui.add_space(8.0);
            ui.label(format!("{} entries", tab.listing.len()));
        }
        [entry] => {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(crate::views::icon(entry)).size(28.0));
                ui.strong(&entry.name);
            });
            egui::Grid::new("inspector-grid")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.weak("Type");
                    ui.label(model::type_of(entry));
                    ui.end_row();
                    if entry.kind != EntryKind::Dir {
                        ui.weak("Size");
                        ui.label(format::size(entry.size));
                        ui.end_row();
                    }
                    for (label, time) in [
                        ("Modified", entry.modified),
                        ("Created", entry.created),
                        ("Accessed", entry.accessed),
                    ] {
                        let (_, full) = format::date(time, now);
                        if !full.is_empty() {
                            ui.weak(label);
                            ui.label(full);
                            ui.end_row();
                        }
                    }
                    if let Some(mime) = &entry.mime {
                        ui.weak("MIME");
                        ui.label(mime.as_str());
                        ui.end_row();
                    }
                    ui.weak("Path");
                    ui.label(entry.path.to_string_lossy());
                    ui.end_row();
                });
            ui.add_space(12.0);
            let target = app
                .preview_target()
                .filter(|(p, _)| *p == pane)
                .map(|(_, id)| id);
            previews.show(ui, pane, target);
        }
        many => {
            let bytes: u64 = many.iter().map(|e| e.size.unwrap_or(0)).sum();
            ui.strong(format!("{} items", many.len()));
            ui.label(format::size(Some(bytes)));
        }
    }
}
