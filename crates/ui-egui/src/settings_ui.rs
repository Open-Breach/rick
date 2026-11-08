//! The Settings, Keyboard shortcuts, Extensions, About and Properties windows.

use app::{App, Command, PluginStatus};
use ext::{Registry, Shortcut};
use model::{
    AfterDrag, Column, Density, PanelSide, SettingsChange, SidePanelChange, SortDir, Theme,
    WindowId,
};

use crate::keys::{Intent, KeyMap, UI_INTENTS, from_egui_key, from_egui_modifiers};
use crate::shell::Request;

/// The Settings screen; every change is a `Command::Settings`, except the
/// side panel row, which changes `window`'s panel (session state).
// One form, one row per setting in the spec's order.
#[allow(clippy::too_many_lines)]
pub fn settings_window(
    ctx: &egui::Context,
    open: &mut bool,
    app: &App,
    window: WindowId,
    requests: &mut Vec<Request>,
) {
    let settings = &app.model().settings;
    let side_panel = app
        .model()
        .window(window)
        .map(|w| w.side_panel.clone())
        .unwrap_or_default();
    let layouts = app.layouts();
    let mut panel_changes: Vec<SidePanelChange> = Vec::new();
    let mut panel = |c: SidePanelChange| panel_changes.push(c);
    let mut ext_changes: Vec<(String, String, ext::SettingValue)> = Vec::new();
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

                    ui.label("Side panel");
                    ui.horizontal(|ui| {
                        let mut visible = side_panel.visible;
                        if ui.checkbox(&mut visible, "Show").changed() {
                            panel(SidePanelChange::Visible(visible));
                        }
                        let mut side = side_panel.side;
                        egui::ComboBox::from_id_salt("side-panel-side")
                            .selected_text(match side {
                                PanelSide::Left => "Left",
                                PanelSide::Right => "Right",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut side, PanelSide::Left, "Left");
                                ui.selectable_value(&mut side, PanelSide::Right, "Right");
                            });
                        if side != side_panel.side {
                            panel(SidePanelChange::Side(side));
                        }
                    });
                    ui.end_row();

                    ui.label("Start with");
                    let start = settings.session.start_layout().map(str::to_owned);
                    let mut picked = start.clone();
                    egui::ComboBox::from_id_salt("session-start")
                        .selected_text(
                            start
                                .clone()
                                .unwrap_or_else(|| "Continue where I left off".to_owned()),
                        )
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut picked, None, "Continue where I left off");
                            for name in layouts {
                                ui.selectable_value(&mut picked, Some(name.clone()), name);
                            }
                        });
                    if picked != start {
                        change(SettingsChange::StartLayout(picked));
                    }
                    ui.end_row();

                    ui.label("Open outside the viewer");
                    ui.horizontal(|ui| {
                        // The types that go to the system application; typed
                        // as a list and saved when the field is left.
                        let id = ui.make_persistent_id("viewer-external");
                        let saved = settings.viewer.external.join(", ");
                        let mut text: String = ui
                            .data(|d| d.get_temp::<String>(id))
                            .unwrap_or_else(|| saved.clone());
                        let output = egui::TextEdit::singleline(&mut text)
                            .hint_text("Extensions, e.g. docx, xlsx")
                            .desired_width(220.0)
                            .show(ui);
                        if output.response.has_focus() {
                            ui.data_mut(|d| d.insert_temp(id, text.clone()));
                        } else if output.response.lost_focus() {
                            ui.data_mut(|d| d.remove::<String>(id));
                            let types: Vec<String> =
                                text.split([',', ' ']).map(str::to_owned).collect();
                            if types
                                .iter()
                                .map(|t| t.trim().trim_start_matches('.').to_ascii_lowercase())
                                .filter(|t| !t.is_empty())
                                .ne(settings.viewer.external.iter().cloned())
                            {
                                change(SettingsChange::ViewerExternal(types));
                            }
                        }
                    })
                    .response
                    .on_hover_text(
                        "Files of these types open with the system's default application instead of the viewer",
                    );
                    ui.end_row();

                    ui.label("Spring-loading delay");
                    let mut delay = settings.drag.spring_delay_ms;
                    if ui
                        .add(egui::Slider::new(&mut delay, 200..=3000).suffix(" ms"))
                        .changed()
                    {
                        change(SettingsChange::SpringDelay(delay));
                    }
                    ui.end_row();

                    ui.label("After a drag");
                    let mut after = settings.drag.after;
                    egui::ComboBox::from_id_salt("after-drag")
                        .selected_text(after_drag_name(after))
                        .show_ui(ui, |ui| {
                            for a in [AfterDrag::Stay, AfterDrag::Return] {
                                ui.selectable_value(&mut after, a, after_drag_name(a));
                            }
                        });
                    if after != settings.drag.after {
                        change(SettingsChange::AfterDrag(after));
                    }
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
            extension_settings(ui, app, &mut ext_changes);
        });
    for change in panel_changes {
        requests.push(Request::Command(Command::SidePanel { window, change }));
    }
    for (ext, key, value) in ext_changes {
        requests.push(Request::Command(Command::ExtSetting { ext, key, value }));
    }
}

/// The settings extensions declared ([`ext::SettingsSchema`]), one section
/// per extension; a change is written to the extension's own TOML file.
fn extension_settings(
    ui: &mut egui::Ui,
    app: &App,
    changes: &mut Vec<(String, String, ext::SettingValue)>,
) {
    let schemas = app.registry().settings_schemas();
    if schemas.iter().all(|s| s.fields.is_empty()) {
        return;
    }
    ui.separator();
    for schema in schemas {
        if schema.fields.is_empty() {
            continue;
        }
        let name = app
            .registry()
            .manifests()
            .iter()
            .find(|m| m.id == schema.ext)
            .map_or(schema.ext.0, |m| m.name);
        ui.strong(name);
        egui::Grid::new(("ext-settings", schema.ext.0))
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                for field in &schema.fields {
                    let current = app
                        .ext_settings()
                        .get(schema.ext.0, field.key)
                        .cloned()
                        .unwrap_or_else(|| field.kind.default_value());
                    ui.label(field.label);
                    let new_value = match (&field.kind, &current) {
                        (ext::SettingKind::Bool { .. }, ext::SettingValue::Bool(on)) => {
                            let mut on = *on;
                            ui.checkbox(&mut on, "")
                                .changed()
                                .then_some(ext::SettingValue::Bool(on))
                        }
                        (ext::SettingKind::Text { .. }, ext::SettingValue::Text(text)) => {
                            let mut text = text.clone();
                            // Every change is written, as the main settings are.
                            ui.add(egui::TextEdit::singleline(&mut text).desired_width(200.0))
                                .changed()
                                .then_some(ext::SettingValue::Text(text))
                        }
                        (
                            ext::SettingKind::Integer { min, max, .. },
                            ext::SettingValue::Integer(value),
                        ) => {
                            let mut value = *value;
                            ui.add(egui::Slider::new(&mut value, *min..=*max))
                                .changed()
                                .then_some(ext::SettingValue::Integer(value))
                        }
                        (
                            ext::SettingKind::Choice { options, .. },
                            ext::SettingValue::Choice(chosen),
                        ) => {
                            let mut selected = chosen.clone();
                            egui::ComboBox::from_id_salt(("ext-choice", schema.ext.0, field.key))
                                .selected_text(selected.clone())
                                .show_ui(ui, |ui| {
                                    for option in options {
                                        ui.selectable_value(
                                            &mut selected,
                                            (*option).to_owned(),
                                            *option,
                                        );
                                    }
                                });
                            (selected != *chosen).then_some(ext::SettingValue::Choice(selected))
                        }
                        _ => None,
                    };
                    if let Some(value) = new_value {
                        changes.push((schema.ext.0.to_owned(), field.key.to_owned(), value));
                    }
                    ui.end_row();
                }
            });
    }
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

fn after_drag_name(after: AfterDrag) -> &'static str {
    match after {
        AfterDrag::Stay => "Stay in the folder",
        AfterDrag::Return => "Return to where the drag started",
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

/// The About window; `version` is the binary's version and git hash.
pub fn about_window(ctx: &egui::Context, open: &mut bool, version: &str) {
    egui::Window::new("About Rick")
        .open(open)
        .resizable(false)
        .show(ctx, |ui| {
            ui.heading("Rick");
            ui.label("A god-like file manager.");
            ui.label(format!("Version {version}"));
            ui.hyperlink("https://github.com/Open-Breach/rick");
        });
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
    egui::Window::new("Extensions")
        .open(open)
        .default_width(640.0)
        .show(ctx, |ui| {
            if plugins.is_empty() {
                ui.label("No plugins found.");
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
                            if ui.checkbox(&mut enabled, "Enabled").changed() {
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
