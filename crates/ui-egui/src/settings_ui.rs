//! The Settings, Keyboard shortcuts, About and Properties windows.

use app::Command;
use ext::{Registry, Shortcut};
use model::{Column, Density, Settings, SettingsChange, SortDir, Theme};

use crate::keys::{Intent, KeyMap, UI_INTENTS, from_egui_key, from_egui_modifiers};
use crate::shell::Request;

/// The Settings screen; every change is a `Command::Settings`.
// One form, one row per setting in the spec's order.
#[allow(clippy::too_many_lines)]
pub fn settings_window(
    ctx: &egui::Context,
    open: &mut bool,
    settings: &Settings,
    requests: &mut Vec<Request>,
) {
    let mut change = |c: SettingsChange| requests.push(Request::Command(Command::Settings(c)));
    egui::Window::new("Settings")
        .open(open)
        .resizable(false)
        .show(ctx, |ui| {
            egui::Grid::new("settings-grid")
                .num_columns(2)
                .spacing([24.0, 8.0])
                .show(ui, |ui| {
                    let mut flag =
                        |ui: &mut egui::Ui,
                         label: &str,
                         value: bool,
                         make: fn(bool) -> SettingsChange| {
                            ui.label(label);
                            let mut v = value;
                            if ui.checkbox(&mut v, "").changed() {
                                change(make(v));
                            }
                            ui.end_row();
                        };
                    flag(
                        ui,
                        "Folders first",
                        settings.folders_first,
                        SettingsChange::FoldersFirst,
                    );
                    flag(
                        ui,
                        "Show hidden files",
                        settings.show_hidden,
                        SettingsChange::ShowHidden,
                    );
                    flag(
                        ui,
                        "Show system files",
                        settings.show_system,
                        SettingsChange::ShowSystem,
                    );
                    flag(
                        ui,
                        "Show file extensions",
                        settings.show_extensions,
                        SettingsChange::ShowExtensions,
                    );
                    flag(
                        ui,
                        "Zebra stripes",
                        settings.zebra_stripes,
                        SettingsChange::ZebraStripes,
                    );

                    ui.label("Default sort");
                    ui.horizontal(|ui| {
                        let mut by = settings.sort_by;
                        egui::ComboBox::from_id_salt("sort-by")
                            .selected_text(column_name(by))
                            .show_ui(ui, |ui| {
                                for column in Column::ALL {
                                    ui.selectable_value(&mut by, column, column_name(column));
                                }
                            });
                        let mut dir = settings.sort_dir;
                        egui::ComboBox::from_id_salt("sort-dir")
                            .selected_text(match dir {
                                SortDir::Ascending => "Ascending",
                                SortDir::Descending => "Descending",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut dir, SortDir::Ascending, "Ascending");
                                ui.selectable_value(&mut dir, SortDir::Descending, "Descending");
                            });
                        if by != settings.sort_by || dir != settings.sort_dir {
                            change(SettingsChange::Sort(by, dir));
                        }
                    });
                    ui.end_row();

                    ui.label("Theme");
                    let mut theme = settings.theme;
                    egui::ComboBox::from_id_salt("theme")
                        .selected_text(theme_name(theme))
                        .show_ui(ui, |ui| {
                            for t in [Theme::System, Theme::Light, Theme::Dark] {
                                ui.selectable_value(&mut theme, t, theme_name(t));
                            }
                        });
                    if theme != settings.theme {
                        change(SettingsChange::Theme(theme));
                    }
                    ui.end_row();

                    ui.label("Font size");
                    let mut size = settings.font_size;
                    if ui
                        .add(egui::Slider::new(&mut size, 9..=24).suffix(" pt"))
                        .changed()
                    {
                        change(SettingsChange::FontSize(size));
                    }
                    ui.end_row();

                    ui.label("Density");
                    let mut density = settings.density;
                    egui::ComboBox::from_id_salt("density")
                        .selected_text(density_name(density))
                        .show_ui(ui, |ui| {
                            for d in [Density::Compact, Density::Normal, Density::Comfortable] {
                                ui.selectable_value(&mut density, d, density_name(d));
                            }
                        });
                    if density != settings.density {
                        change(SettingsChange::Density(density));
                    }
                    ui.end_row();

                    ui.label("Accent colour");
                    ui.horizontal(|ui| {
                        let mut color = settings
                            .accent
                            .as_deref()
                            .and_then(parse_hex)
                            .unwrap_or(ui.visuals().selection.bg_fill);
                        if ui.color_edit_button_srgba(&mut color).changed() {
                            change(SettingsChange::Accent(Some(format!(
                                "#{:02x}{:02x}{:02x}",
                                color.r(),
                                color.g(),
                                color.b()
                            ))));
                        }
                        if settings.accent.is_some() && ui.small_button("Theme default").clicked() {
                            change(SettingsChange::Accent(None));
                        }
                    });
                    ui.end_row();
                });
        });
}

fn column_name(column: Column) -> &'static str {
    match column {
        Column::Name => "Name",
        Column::Type => "Type",
        Column::Modified => "Date modified",
        Column::Created => "Date created",
        Column::Accessed => "Date accessed",
        Column::Size => "Size",
        Column::Items => "Items",
    }
}

fn theme_name(theme: Theme) -> &'static str {
    match theme {
        Theme::System => "System",
        Theme::Light => "Light",
        Theme::Dark => "Dark",
    }
}

fn density_name(density: Density) -> &'static str {
    match density {
        Density::Compact => "Compact",
        Density::Normal => "Normal",
        Density::Comfortable => "Comfortable",
    }
}

/// `#rrggbb` to a colour.
#[must_use]
pub fn parse_hex(text: &str) -> Option<egui::Color32> {
    let hex = text.trim().strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some(egui::Color32::from_rgb(
        channel(0)?,
        channel(2)?,
        channel(4)?,
    ))
}

/// What the shortcuts screen is doing.
#[derive(Debug, Default)]
pub struct ShortcutsScreen {
    /// Shown.
    pub open: bool,
    /// Waiting for a key press to bind to this intent.
    pub rebinding: Option<Intent>,
    /// Filter text.
    pub filter: String,
}

impl ShortcutsScreen {
    /// The Keyboard shortcuts screen: the live table, with Change and Reset
    /// per row. A change is a `SettingsChange::Shortcut`.
    // Capture, header and table are one screen.
    #[allow(clippy::too_many_lines)]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        keys: &KeyMap,
        registry: &Registry,
        requests: &mut Vec<Request>,
    ) {
        if !self.open {
            return;
        }
        if let Some(intent) = self.rebinding {
            let captured = ctx.input_mut(|i| {
                let mut found = None;
                i.events.retain(|event| {
                    if found.is_some() {
                        return true;
                    }
                    if let egui::Event::Key {
                        key,
                        modifiers,
                        pressed: true,
                        ..
                    } = event
                    {
                        found = Some((*modifiers, *key));
                        return false;
                    }
                    true
                });
                found
            });
            if let Some((modifiers, key)) = captured {
                self.rebinding = None;
                if key != egui::Key::Escape
                    && let Some(key) = from_egui_key(key)
                {
                    let shortcut = Shortcut {
                        modifiers: from_egui_modifiers(modifiers),
                        key,
                    };
                    requests.push(Request::Command(Command::Settings(
                        SettingsChange::Shortcut {
                            action: intent.id().to_owned(),
                            binding: Some(shortcut.text()),
                        },
                    )));
                }
            }
        }

        let mut open = self.open;
        egui::Window::new("Keyboard shortcuts")
            .open(&mut open)
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Filter");
                    ui.text_edit_singleline(&mut self.filter);
                    if let Some(intent) = self.rebinding {
                        ui.strong(format!(
                            "Press the new shortcut for {}; Escape cancels",
                            label_of(intent, registry)
                        ));
                    }
                });
                ui.separator();
                let filter = self.filter.to_lowercase();
                egui::ScrollArea::vertical()
                    .max_height(480.0)
                    .show(ui, |ui| {
                        egui::Grid::new("shortcuts-grid")
                            .num_columns(4)
                            .striped(true)
                            .spacing([16.0, 4.0])
                            .show(ui, |ui| {
                                let mut rows: Vec<(String, Intent, String)> = registry
                                    .actions()
                                    .iter()
                                    .map(|a| {
                                        (
                                            "File operations".to_owned(),
                                            Intent::Action(a.id()),
                                            a.label().to_owned(),
                                        )
                                    })
                                    .chain(UI_INTENTS.iter().filter(|s| s.group != "List").map(
                                        |s| (s.group.to_owned(), s.intent, s.label.to_owned()),
                                    ))
                                    .collect();
                                rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.2.cmp(&b.2)));
                                for (group, intent, label) in rows {
                                    if !filter.is_empty()
                                        && !label.to_lowercase().contains(&filter)
                                        && !group.to_lowercase().contains(&filter)
                                    {
                                        continue;
                                    }
                                    ui.weak(group);
                                    ui.label(&label);
                                    let bound: Vec<String> =
                                        keys.shortcuts_for(intent).map(|s| s.text()).collect();
                                    if bound.is_empty() {
                                        ui.weak("—");
                                    } else {
                                        ui.monospace(bound.join(", "));
                                    }
                                    ui.horizontal(|ui| {
                                        if ui.small_button("Change").clicked() {
                                            self.rebinding = Some(intent);
                                        }
                                        if ui.small_button("Reset").clicked() {
                                            requests.push(Request::Command(Command::Settings(
                                                SettingsChange::Shortcut {
                                                    action: intent.id().to_owned(),
                                                    binding: None,
                                                },
                                            )));
                                        }
                                        if ui.small_button("Unbind").clicked() {
                                            requests.push(Request::Command(Command::Settings(
                                                SettingsChange::Shortcut {
                                                    action: intent.id().to_owned(),
                                                    binding: Some(String::new()),
                                                },
                                            )));
                                        }
                                    });
                                    ui.end_row();
                                }
                            });
                    });
            });
        self.open = open;
        if !self.open {
            self.rebinding = None;
        }
    }
}

/// The label of an intent: the action's, or the table row's.
#[must_use]
pub fn label_of(intent: Intent, registry: &Registry) -> String {
    match intent {
        Intent::Action(id) => registry
            .actions()
            .iter()
            .find(|a| a.id() == id)
            .map_or_else(|| id.0.to_owned(), |a| a.label().to_owned()),
        other => other
            .spec()
            .map_or_else(|| format!("{other:?}"), |s| s.label.to_owned()),
    }
}

/// The About window.
pub fn about_window(ctx: &egui::Context, open: &mut bool) {
    egui::Window::new("About Rick")
        .open(open)
        .resizable(false)
        .show(ctx, |ui| {
            ui.heading("Rick");
            ui.label("A god-like file manager.");
            ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
            ui.hyperlink("https://github.com/Open-Breach/rick");
        });
}

/// The properties rows Rick shows itself where the platform has no
/// dialogue.
pub fn properties_window(
    ctx: &egui::Context,
    properties: &mut Option<(String, Vec<(String, String)>)>,
) {
    let Some((title, rows)) = properties.as_ref() else {
        return;
    };
    let mut open = true;
    egui::Window::new(format!("Properties of {title}"))
        .open(&mut open)
        .show(ctx, |ui| {
            egui::Grid::new("properties-grid")
                .num_columns(2)
                .show(ui, |ui| {
                    for (key, value) in rows {
                        ui.weak(key);
                        ui.label(value);
                        ui.end_row();
                    }
                });
        });
    if !open {
        *properties = None;
    }
}
