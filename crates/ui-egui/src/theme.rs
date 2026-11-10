//! The scheme and density tokens of SPEC.md's *Look and feel*, and the egui
//! style derived from them.
//!
//! A [`Scheme`] is the colour token set: one field per variable of the Figma
//! collection *Rick*, with the variable's name and value, and the
//! collection's two modes as the two schemes Rick ships, [`Scheme::LIGHT`]
//! and [`Scheme::DARK`]. [`Metrics`] is the *Density* collection the same
//! way, [`Metrics::COMFORTABLE`] and [`Metrics::COMPACT`]. [`apply`]
//! installs a density on a context: egui's `Visuals` and `Spacing` are
//! derived from the tokens for both of egui's themes, never the other way
//! round, and the active sets are kept in the context so a widget that
//! needs a token egui has no field for reads it through [`scheme`] and
//! [`metrics`]. No widget hard-codes a colour or a size: what is not a
//! token here is one of the fixed constants below.
//!
//! The scheme is a value, not a palette baked into the widgets, so a
//! further scheme is one more constant and a setting, not a change to a
//! widget.

use egui::style::{HandleShape, Selection, TextCursorStyle, WidgetVisuals, Widgets};
use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle, Vec2, Visuals};
use model::Density;

/// A `#rrggbb` literal as a colour, at compile time.
const fn hex(text: &str) -> Color32 {
    const fn nibble(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => panic!("not a hex digit"),
        }
    }
    const fn byte(high: u8, low: u8) -> u8 {
        nibble(high) * 16 + nibble(low)
    }
    let b = text.as_bytes();
    assert!(b.len() == 7 && b[0] == b'#', "a colour is #rrggbb");
    Color32::from_rgb(byte(b[1], b[2]), byte(b[3], b[4]), byte(b[5], b[6]))
}

macro_rules! scheme {
    ($( $(#[$doc:meta])* $field:ident => $name:literal, $light:literal, $dark:literal; )*) => {
        /// The colour tokens of one scheme: the Figma collection *Rick*,
        /// one field per variable, named as the variable is.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct Scheme {
            /// The scheme's name, as Settings shows it.
            pub name: &'static str,
            /// Dark backgrounds with light text.
            pub dark: bool,
            $( $(#[$doc])* pub $field: Color32, )*
        }

        impl Scheme {
            /// The *Light* mode of the collection.
            pub const LIGHT: Self = Self {
                name: "Light",
                dark: false,
                $( $field: hex($light), )*
            };

            /// The *Dark* mode of the collection.
            pub const DARK: Self = Self {
                name: "Dark",
                dark: true,
                $( $field: hex($dark), )*
            };

            /// The variables' names, in the collection's order.
            pub const NAMES: &'static [&'static str] = &[$($name,)*];

            /// Every token with its Figma name, in the collection's order.
            #[must_use]
            pub fn tokens(&self) -> Vec<(&'static str, Color32)> {
                vec![$(($name, self.$field),)*]
            }

            /// The token with a Figma name, `bg/window` and so on.
            #[must_use]
            pub fn token(&self, name: &str) -> Option<Color32> {
                match name {
                    $( $name => Some(self.$field), )*
                    _ => None,
                }
            }
        }
    };
}

scheme! {
    /// The title row and the window background behind the panes.
    bg_window => "bg/window", "#ededed", "#2b2b2b";
    /// The side panel.
    bg_panel => "bg/panel", "#f7f7f7", "#262626";
    /// Panes, the active workspace tab, text fields.
    bg_pane => "bg/pane", "#fafafa", "#1e1e1e";
    /// Resting controls: segmented backgrounds, the round cancel button,
    /// secondary buttons.
    bg_control => "bg/control", "#e4e4e4", "#3a3a3a";
    /// Strong selection, reserved; the lists use the soft one.
    bg_selected => "bg/selected", "#90d1ff", "#2e5e8f";
    /// A tool button while pressed.
    bg_active => "bg/active", "#90d1ff", "#2e5e8f";
    /// Body text.
    text_primary => "text/primary", "#2b2b2b", "#ededed";
    /// Column headers, section headers, footer statistics, the secondary
    /// columns, inactive tabs, detail lines, shortcuts in menus.
    text_secondary => "text/secondary", "#6b6b6b", "#a0a0a0";
    /// Icons at rest.
    icon_default => "icon/default", "#4a4a4a", "#cfcfcf";
    /// Hairlines, dividers, field and card borders, the window border.
    border_default => "border/default", "#d6d6d6", "#3d3d3d";
    /// The fill of a drive's usage bar.
    accent_usage => "accent/usage", "#3c84c2", "#5ab0ff";
    /// Selected rows in the focused pane, the active menu entry, the active
    /// settings category, active toggles, the selection pill.
    bg_selected_soft => "bg/selected-soft", "#cfe6ff", "#24456a";
    /// Selected rows in a pane without focus.
    bg_selected_inactive => "bg/selected-inactive", "#e4e4e4", "#3a3a3a";
    /// Blue text, focus rings, the icon of an active toggle, a switch that
    /// is on, progress bars, the portal animations, the filter-active dot.
    accent_focus => "accent/focus", "#2873c0", "#4ea1ff";
    /// The track of a drive's usage bar.
    bg_track => "bg/track", "#e0e0e0", "#3d3d3d";
    /// The Trash pill under 1 %.
    trash_low_bg => "trash/low-bg", "#e2e2e2", "#3a3a3a";
    /// The text of the Trash pill under 1 %.
    trash_low_text => "trash/low-text", "#5c5c5c", "#bdbdbd";
    /// The Trash pill from 1 % to 5 %.
    trash_mid_bg => "trash/mid-bg", "#fce8b2", "#4a3a12";
    /// The text of the Trash pill from 1 % to 5 %.
    trash_mid_text => "trash/mid-text", "#8a5a00", "#f2c46b";
    /// The Trash pill over 5 %.
    trash_high_bg => "trash/high-bg", "#f9d3d0", "#4d2220";
    /// The text of the Trash pill over 5 %.
    trash_high_text => "trash/high-text", "#a61b12", "#ff9c93";
    /// Cards, menus, the operations drawer, the tables in dialogues.
    bg_card => "bg/card", "#ffffff", "#2e2e2e";
    /// The column header band, drive cards, the parent chip, hovered rows
    /// and buttons, the tracks of the drawer's bars.
    bg_subtle => "bg/subtle", "#ececec", "#333333";
    /// Text on `bg/selected-soft`.
    text_on_selected => "text/on-selected", "#1f528c", "#a9d2ff";
    /// Placeholder text in fields.
    text_placeholder => "text/placeholder", "#737373", "#979797";
    /// A switch that is off.
    bg_switch_off => "bg/switch-off", "#8e8e8e", "#7a7a7a";
    /// Checkbox borders and other small controls that need more contrast.
    border_strong => "border/strong", "#939393", "#777777";
    /// A hash that matches: field border, pill text, row label.
    status_success => "status/success", "#1d7943", "#6fd49a";
    /// The *Matches* pill.
    status_success_bg => "status/success-bg", "#d9f2e0", "#1e3b2a";
    /// The matching hash row.
    status_success_row => "status/success-row", "#edfaf0", "#1b2e22";
    /// Warning text, such as the missing version in the installer.
    status_warning => "status/warning", "#8c5c00", "#f2c46b";
    /// The border of a field with a warning.
    status_warning_border => "status/warning-border", "#d98c0d", "#d98c0d";
    /// Destructive menu entries, a hash that does not match, errors.
    status_danger => "status/danger", "#b3261a", "#ff8a80";
    /// The background of dialogues and of the settings window.
    dialog_bg => "dialog/bg", "#fafafa", "#2a2a2a";
    /// The background of a primary button, with white text on it.
    accent_button => "accent/button", "#2a77c6", "#2975c7";
    /// The band of the title row. The values are the *Dusty rose* preset;
    /// the *Title bar colour* setting puts the chosen pair in their place
    /// ([`apply`]), so read the band through [`band`], not from here.
    bg_titlebar => "bg/titlebar", "#ece0db", "#332c2c";
}

impl Scheme {
    /// The text on a primary button.
    pub const ON_BUTTON: Color32 = Color32::WHITE;

    /// A hairline in `border/default`.
    #[must_use]
    pub fn hairline(&self) -> Stroke {
        Stroke::new(1.0, self.border_default)
    }

    /// The 2 px keyboard focus ring.
    #[must_use]
    pub fn focus_ring(&self) -> Stroke {
        Stroke::new(2.0, self.accent_focus)
    }

    /// The shadow colour under cards and dialogues: a translucent black,
    /// deeper on dark backgrounds.
    #[must_use]
    pub fn shadow(&self) -> Color32 {
        Color32::from_black_alpha(if self.dark { 120 } else { 40 })
    }
}

/// The size tokens of one density: the Figma collection *Density*, one
/// field per variable, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    /// The density's name, as Settings shows it.
    pub name: &'static str,
    /// `titlebar/height`: the title row.
    pub titlebar_height: f32,
    /// `tab/height`: a workspace tab.
    pub tab_height: f32,
    /// `button/size`: a square tool button.
    pub button_size: f32,
    /// `search/height`: the search box in the title row.
    pub search_height: f32,
    /// `row/height`: a row of the file list.
    pub row_height: f32,
    /// `sidebar/item-height`: a row of the side panel.
    pub sidebar_item_height: f32,
    /// `space/row-x`: horizontal padding of list and side-panel rows.
    pub space_row_x: f32,
    /// `space/toolbar-x`: horizontal padding of toolbars and footers.
    pub space_toolbar_x: f32,
    /// `space/toolbar-y`: vertical padding of toolbars and footers; the
    /// gap between items on the 6 px grid.
    pub space_toolbar_y: f32,
    /// `radius/control`: the corner radius of buttons, fields, chips and
    /// side-panel rows.
    pub radius_control: f32,
}

impl Metrics {
    /// The *Comfortable* mode, the default.
    pub const COMFORTABLE: Self = Self {
        name: "Comfortable",
        titlebar_height: 40.0,
        tab_height: 34.0,
        button_size: 34.0,
        search_height: 28.0,
        row_height: 32.0,
        sidebar_item_height: 34.0,
        space_row_x: 12.0,
        space_toolbar_x: 12.0,
        space_toolbar_y: 6.0,
        radius_control: 6.0,
    };

    /// The *Compact* mode.
    pub const COMPACT: Self = Self {
        name: "Compact",
        titlebar_height: 34.0,
        tab_height: 28.0,
        button_size: 28.0,
        search_height: 22.0,
        row_height: 24.0,
        sidebar_item_height: 22.0,
        space_row_x: 6.0,
        space_toolbar_x: 8.0,
        space_toolbar_y: 4.0,
        radius_control: 4.0,
    };

    /// The metrics of a density setting.
    #[must_use]
    pub const fn of(density: Density) -> &'static Self {
        match density {
            Density::Compact => &Self::COMPACT,
            Density::Comfortable => &Self::COMFORTABLE,
        }
    }

    /// Every token with its Figma name.
    #[must_use]
    pub fn tokens(&self) -> Vec<(&'static str, f32)> {
        vec![
            ("row/height", self.row_height),
            ("sidebar/item-height", self.sidebar_item_height),
            ("button/size", self.button_size),
            ("titlebar/height", self.titlebar_height),
            ("tab/height", self.tab_height),
            ("space/row-x", self.space_row_x),
            ("space/toolbar-y", self.space_toolbar_y),
            ("space/toolbar-x", self.space_toolbar_x),
            ("radius/control", self.radius_control),
            ("search/height", self.search_height),
        ]
    }

    /// A toolbar's height: a tool button with the vertical padding above
    /// and below it.
    #[must_use]
    pub fn toolbar_height(&self) -> f32 {
        self.button_size + 2.0 * self.space_toolbar_y
    }

    /// A footer's height, the Figma status bar's: the hairline over it,
    /// then the 26 px filter field with the toolbar padding above and
    /// below it — 39 px Comfortable, 35 px Compact (QUESTIONS.md 254).
    #[must_use]
    pub fn footer_height(&self) -> f32 {
        HAIRLINE + FOOTER_CONTROL + 2.0 * self.space_toolbar_y
    }

    /// `radius/control` as egui spells a corner radius.
    #[must_use]
    pub fn corner_radius(&self) -> CornerRadius {
        // The radius is a few pixels; the conversion cannot lose anything.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        CornerRadius::same(self.radius_control.round() as u8)
    }
}

/// An icon, whatever the density.
pub const ICON: f32 = 16.0;
/// The column header band.
pub const COLUMN_HEADER: f32 = 34.0;
/// A workspace tab's width.
pub const TAB_WIDTH: f32 = 220.0;
/// The search box's width.
pub const SEARCH_WIDTH: f32 = 320.0;
/// A progress or usage bar.
pub const BAR: f32 = 4.0;
/// A hairline.
pub const HAIRLINE: f32 = 1.0;
/// The footer's filter field and its hidden-files toggle, whatever the
/// density (SPEC.md *Footer*).
pub const FOOTER_CONTROL: f32 = 26.0;
/// A menu entry.
pub const MENU_ENTRY: f32 = 30.0;
/// The top corners of a workspace tab.
pub const TAB_CORNER: u8 = 10;
/// The corners of cards, drive cards and menus.
pub const CARD_RADIUS: u8 = 8;
/// The corners of dialogues and of the operations drawer.
pub const DIALOG_RADIUS: u8 = 12;
/// A fully round pill: egui clamps the radius to half the height.
pub const PILL_RADIUS: u8 = u8::MAX;
/// Body text.
pub const TEXT_BODY: f32 = 13.0;
/// Secondary text: column headers, footer statistics, detail lines,
/// shortcuts in menus.
pub const TEXT_SECONDARY: f32 = 12.0;
/// Letter-spaced capitals: section headers and pills.
pub const TEXT_CAPS: f32 = 11.0;
/// A page title.
pub const TEXT_HEADING: f32 = 18.0;

/// What a context shows: a scheme for each of egui's themes, and the
/// metrics.
#[derive(Debug, Clone, Copy)]
struct Active {
    light: &'static Scheme,
    dark: &'static Scheme,
    metrics: &'static Metrics,
    /// What the *Title bar colour* setting wrote into `bg/titlebar`: the
    /// band in the light scheme and in the dark one.
    titlebar: (Color32, Color32),
}

impl Default for Active {
    fn default() -> Self {
        Self {
            light: &Scheme::LIGHT,
            dark: &Scheme::DARK,
            metrics: &Metrics::COMFORTABLE,
            titlebar: (Scheme::LIGHT.bg_titlebar, Scheme::DARK.bg_titlebar),
        }
    }
}

fn active_id() -> egui::Id {
    egui::Id::new("rick-theme")
}

fn active(ctx: &egui::Context) -> Active {
    ctx.data(|d| d.get_temp::<Active>(active_id()))
        .unwrap_or_default()
}

/// Installs the tokens on a context: the light scheme on egui's light
/// theme, the dark one on its dark theme, and the metrics of `density` on
/// both, so following the system keeps working; `titlebar` is the pair
/// the *Title bar colour* setting chose, written into `bg/titlebar` for
/// the light and for the dark scheme. Call it when the settings change;
/// it is cheap enough to call every frame.
pub fn apply(ctx: &egui::Context, density: Density, titlebar: (Color32, Color32)) {
    let active = Active {
        metrics: Metrics::of(density),
        titlebar,
        ..Active::default()
    };
    ctx.data_mut(|d| d.insert_temp(active_id(), active));
    for (theme, scheme) in [
        (egui::Theme::Light, active.light),
        (egui::Theme::Dark, active.dark),
    ] {
        ctx.style_mut_of(theme, |style| {
            style.visuals = visuals(scheme, active.metrics);
            spacing(&mut style.spacing, active.metrics);
            style.text_styles = text_styles();
        });
    }
}

/// The scheme of the theme a context shows now.
#[must_use]
pub fn scheme_of(ctx: &egui::Context) -> &'static Scheme {
    let active = active(ctx);
    match ctx.theme() {
        egui::Theme::Dark => active.dark,
        egui::Theme::Light => active.light,
    }
}

/// The scheme the `ui` draws with.
#[must_use]
pub fn scheme(ui: &egui::Ui) -> &'static Scheme {
    let active = active(ui.ctx());
    if ui.visuals().dark_mode {
        active.dark
    } else {
        active.light
    }
}

/// The metrics a context draws with.
#[must_use]
pub fn metrics(ctx: &egui::Context) -> &'static Metrics {
    active(ctx).metrics
}

/// The title row's band as the `ui` draws it: `bg/titlebar` with what the
/// *Title bar colour* setting wrote into it, and everything that sits on
/// the band.
#[must_use]
pub fn band(ui: &egui::Ui) -> Band {
    let active = active(ui.ctx());
    if ui.visuals().dark_mode {
        Band::on(active.dark, active.titlebar.1)
    } else {
        Band::on(active.light, active.titlebar.0)
    }
}

/// The title row's band in the theme a context shows now.
#[must_use]
pub fn band_of(ctx: &egui::Context) -> Band {
    let active = active(ctx);
    match ctx.theme() {
        egui::Theme::Dark => Band::on(active.dark, active.titlebar.1),
        egui::Theme::Light => Band::on(active.light, active.titlebar.0),
    }
}

/// A colour of the settings as egui's.
#[must_use]
pub fn rgb(colour: model::Rgb) -> Color32 {
    let [r, g, b] = colour.0;
    Color32::from_rgb(r, g, b)
}

/// `top` at `share` of its strength over `under`, as one opaque colour:
/// the mix a translucent fill gives, so its contrast can be measured.
#[must_use]
pub fn over(top: Color32, share: f32, under: Color32) -> Color32 {
    let mix = |t: u8, u: u8| {
        let value = f32::from(t) * share + f32::from(u) * (1.0 - share);
        // The channels are bytes: the mix is one too.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let byte = value.round().clamp(0.0, 255.0) as u8;
        byte
    };
    Color32::from_rgb(
        mix(top.r(), under.r()),
        mix(top.g(), under.g()),
        mix(top.b(), under.b()),
    )
}

/// What is drawn on the title row's band, chosen by contrast with it
/// (SPEC.md *Title bar colour*, *On the band*). The active tab is not
/// here: it is `bg/pane` with the scheme's own colours whatever the band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Band {
    /// The band itself.
    pub fill: Color32,
    /// The band does not agree with the scheme — dark in Light, light in
    /// Dark — so the row is drawn in the band's own text colour.
    pub contrary: bool,
    /// The titles of the inactive tabs.
    pub title: Color32,
    /// The icons: the menu button, the tab icons and close buttons, the
    /// new-tab button, the layout toggles and the window controls.
    pub icon: Color32,
    /// The divider of the window group.
    pub divider: Color32,
    /// A tool button under the pointer.
    pub hover: Color32,
    /// An inactive tab under the pointer.
    pub tab_hover: Color32,
    /// The search box at rest.
    pub search_fill: Color32,
    /// The text typed into the search box.
    pub search_text: Color32,
    /// The search box's icon and placeholder.
    pub search_placeholder: Color32,
}

impl Band {
    /// The text colour of a light band: `text/primary` of the light
    /// scheme.
    pub const INK: Color32 = Scheme::LIGHT.text_primary;

    /// Whether a band is dark: white contrasts with it more than
    /// [`INK`](Self::INK) does.
    #[must_use]
    pub fn is_dark(fill: Color32) -> bool {
        contrast(Color32::WHITE, fill) > contrast(Self::INK, fill)
    }

    /// The colours on a band of `fill` under `scheme`.
    ///
    /// A band that agrees with the scheme is drawn from the scheme: icons
    /// in `icon/default`, inactive titles in `text/secondary` — or, where
    /// that does not reach 4.5:1 on the band, in the band's own text
    /// colour at 78 %, and at full strength should that fall short too —
    /// a hovered tab `bg/pane` at 50 % over the band, the search box a
    /// `bg/pane` field. A band that does not agree switches the row to its
    /// own text colour, white on a dark band and [`INK`](Self::INK) on a
    /// light one: titles at 78 %, icons at 90 %, the divider at 25 %, a
    /// hovered button or tab at 12 %, the search box at 14 % with its
    /// placeholder at 70 % and its text at 90 %.
    #[must_use]
    pub fn on(scheme: &Scheme, fill: Color32) -> Self {
        let dark = Self::is_dark(fill);
        let own = if dark { Color32::WHITE } else { Self::INK };
        let readable = |candidates: &[Color32]| {
            candidates
                .iter()
                .copied()
                .find(|colour| contrast(*colour, fill) >= 4.5)
                .unwrap_or(own)
        };
        if dark == scheme.dark {
            Self {
                fill,
                contrary: false,
                title: readable(&[scheme.text_secondary, over(own, 0.78, fill)]),
                icon: scheme.icon_default,
                divider: scheme.border_default,
                hover: scheme.bg_subtle,
                tab_hover: over(scheme.bg_pane, 0.5, fill),
                search_fill: scheme.bg_pane,
                search_text: scheme.text_primary,
                search_placeholder: scheme.text_placeholder,
            }
        } else {
            let search_fill = over(own, 0.14, fill);
            Self {
                fill,
                contrary: true,
                title: readable(&[over(own, 0.78, fill)]),
                icon: over(own, 0.90, fill),
                divider: over(own, 0.25, fill),
                hover: over(own, 0.12, fill),
                tab_hover: over(own, 0.12, fill),
                search_fill,
                search_text: over(own, 0.90, search_fill),
                search_placeholder: over(own, 0.70, search_fill),
            }
        }
    }
}

/// egui's visuals derived from a scheme.
#[must_use]
pub fn visuals(s: &Scheme, m: &Metrics) -> Visuals {
    let base = if s.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    let text = Stroke::new(1.0, s.text_primary);
    let radius = m.corner_radius();
    let widgets = Widgets {
        // Labels, separators, frames: the hairline is the separator.
        noninteractive: WidgetVisuals {
            weak_bg_fill: s.bg_pane,
            bg_fill: s.bg_pane,
            bg_stroke: s.hairline(),
            fg_stroke: text,
            corner_radius: radius,
            expansion: 0.0,
        },
        // A button at rest: a secondary button's fill.
        inactive: WidgetVisuals {
            weak_bg_fill: s.bg_control,
            bg_fill: s.bg_control,
            bg_stroke: Stroke::NONE,
            fg_stroke: text,
            corner_radius: radius,
            expansion: 0.0,
        },
        hovered: WidgetVisuals {
            weak_bg_fill: s.bg_subtle,
            bg_fill: s.bg_subtle,
            bg_stroke: s.hairline(),
            fg_stroke: text,
            corner_radius: radius,
            expansion: 0.0,
        },
        active: WidgetVisuals {
            weak_bg_fill: s.bg_active,
            bg_fill: s.bg_active,
            bg_stroke: Stroke::new(1.0, s.accent_focus),
            fg_stroke: text,
            corner_radius: radius,
            expansion: 0.0,
        },
        open: WidgetVisuals {
            weak_bg_fill: s.bg_subtle,
            bg_fill: s.bg_subtle,
            bg_stroke: s.hairline(),
            fg_stroke: text,
            corner_radius: radius,
            expansion: 0.0,
        },
    };
    Visuals {
        dark_mode: s.dark,
        override_text_color: None,
        weak_text_alpha: 1.0,
        weak_text_color: Some(s.text_secondary),
        widgets,
        selection: Selection {
            bg_fill: s.bg_selected_soft,
            stroke: Stroke::new(1.0, s.text_on_selected),
        },
        hyperlink_color: s.accent_focus,
        faint_bg_color: s.bg_subtle,
        extreme_bg_color: s.bg_pane,
        text_edit_bg_color: Some(s.bg_pane),
        code_bg_color: s.bg_subtle,
        warn_fg_color: s.status_warning,
        error_fg_color: s.status_danger,
        window_corner_radius: CornerRadius::same(DIALOG_RADIUS),
        window_shadow: egui::epaint::Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: s.shadow(),
        },
        window_fill: s.dialog_bg,
        window_stroke: s.hairline(),
        menu_corner_radius: CornerRadius::same(CARD_RADIUS),
        panel_fill: s.bg_window,
        popup_shadow: egui::epaint::Shadow {
            offset: [0, 4],
            blur: 16,
            spread: 0,
            color: s.shadow(),
        },
        text_cursor: TextCursorStyle {
            stroke: Stroke::new(2.0, s.accent_focus),
            ..TextCursorStyle::default()
        },
        indent_has_left_vline: false,
        striped: false,
        slider_trailing_fill: true,
        handle_shape: HandleShape::Circle,
        ..base
    }
}

/// egui's spacing derived from the metrics; what the metrics do not name
/// keeps egui's value.
pub fn spacing(spacing: &mut egui::style::Spacing, m: &Metrics) {
    spacing.item_spacing = Vec2::splat(m.space_toolbar_y);
    // A text button is its text plus this padding: 30 px menu entries in
    // Comfortable. Tool buttons size themselves to `button/size`; the
    // minimum interactive height stays egui's, so menus and rows do not
    // grow with it.
    spacing.button_padding = Vec2::new(m.space_toolbar_x, m.space_toolbar_y);
    spacing.menu_margin = Margin::same(6);
    spacing.window_margin = Margin::same(12);
    spacing.icon_width = ICON;
    spacing.icon_width_inner = 10.0;
    spacing.icon_spacing = 6.0;
    spacing.slider_rail_height = BAR;
}

/// The text styles: body and buttons at 13 px, secondary text at 12 px,
/// headings at 18 px, the monospace at the body size.
#[must_use]
pub fn text_styles() -> std::collections::BTreeMap<TextStyle, FontId> {
    [
        (
            TextStyle::Small,
            FontId::new(TEXT_SECONDARY, FontFamily::Proportional),
        ),
        (
            TextStyle::Body,
            FontId::new(TEXT_BODY, FontFamily::Proportional),
        ),
        (
            TextStyle::Button,
            FontId::new(TEXT_BODY, FontFamily::Proportional),
        ),
        (
            TextStyle::Heading,
            FontId::new(TEXT_HEADING, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(TEXT_BODY, FontFamily::Monospace),
        ),
    ]
    .into()
}

/// The WCAG 2 relative luminance of a colour, from its sRGB channels.
#[must_use]
pub fn luminance(color: Color32) -> f64 {
    fn channel(byte: u8) -> f64 {
        let c = f64::from(byte) / 255.0;
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
}

/// The WCAG 2 contrast ratio of two colours, 1 to 21.
#[must_use]
pub fn contrast(a: Color32, b: Color32) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

#[cfg(test)]
// The tokens are exact literals, so the tests compare them exactly.
#[allow(clippy::float_cmp, clippy::assertions_on_constants)]
mod tests {
    use super::*;

    /// The collection *Rick* as the Figma file holds it: name, Light,
    /// Dark. Read through the connector, not retyped from a screenshot.
    const FIGMA_RICK: &[(&str, &str, &str)] = &[
        ("bg/window", "#ededed", "#2b2b2b"),
        ("bg/panel", "#f7f7f7", "#262626"),
        ("bg/pane", "#fafafa", "#1e1e1e"),
        ("bg/control", "#e4e4e4", "#3a3a3a"),
        ("bg/selected", "#90d1ff", "#2e5e8f"),
        ("bg/active", "#90d1ff", "#2e5e8f"),
        ("text/primary", "#2b2b2b", "#ededed"),
        ("text/secondary", "#6b6b6b", "#a0a0a0"),
        ("icon/default", "#4a4a4a", "#cfcfcf"),
        ("border/default", "#d6d6d6", "#3d3d3d"),
        ("accent/usage", "#3c84c2", "#5ab0ff"),
        ("bg/selected-soft", "#cfe6ff", "#24456a"),
        ("bg/selected-inactive", "#e4e4e4", "#3a3a3a"),
        ("accent/focus", "#2873c0", "#4ea1ff"),
        ("bg/track", "#e0e0e0", "#3d3d3d"),
        ("trash/low-bg", "#e2e2e2", "#3a3a3a"),
        ("trash/low-text", "#5c5c5c", "#bdbdbd"),
        ("trash/mid-bg", "#fce8b2", "#4a3a12"),
        ("trash/mid-text", "#8a5a00", "#f2c46b"),
        ("trash/high-bg", "#f9d3d0", "#4d2220"),
        ("trash/high-text", "#a61b12", "#ff9c93"),
        ("bg/card", "#ffffff", "#2e2e2e"),
        ("bg/subtle", "#ececec", "#333333"),
        ("text/on-selected", "#1f528c", "#a9d2ff"),
        ("text/placeholder", "#737373", "#979797"),
        ("bg/switch-off", "#8e8e8e", "#7a7a7a"),
        ("border/strong", "#939393", "#777777"),
        ("status/success", "#1d7943", "#6fd49a"),
        ("status/success-bg", "#d9f2e0", "#1e3b2a"),
        ("status/success-row", "#edfaf0", "#1b2e22"),
        ("status/warning", "#8c5c00", "#f2c46b"),
        ("status/warning-border", "#d98c0d", "#d98c0d"),
        ("status/danger", "#b3261a", "#ff8a80"),
        ("dialog/bg", "#fafafa", "#2a2a2a"),
        ("accent/button", "#2a77c6", "#2975c7"),
        ("bg/titlebar", "#ece0db", "#332c2c"),
    ];

    /// The collection *Density*: name, Comfortable, Compact.
    const FIGMA_DENSITY: &[(&str, f32, f32)] = &[
        ("row/height", 32.0, 24.0),
        ("sidebar/item-height", 34.0, 22.0),
        ("button/size", 34.0, 28.0),
        ("titlebar/height", 40.0, 34.0),
        ("tab/height", 34.0, 28.0),
        ("space/row-x", 12.0, 6.0),
        ("space/toolbar-y", 6.0, 4.0),
        ("space/toolbar-x", 12.0, 8.0),
        ("radius/control", 6.0, 4.0),
        ("search/height", 28.0, 22.0),
    ];

    /// The default band: the token's own pair.
    const ROSE: (Color32, Color32) = (Scheme::LIGHT.bg_titlebar, Scheme::DARK.bg_titlebar);

    fn hex_text(color: Color32) -> String {
        format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b())
    }

    #[test]
    fn the_schemes_are_the_figma_collection_one_to_one() {
        let light = Scheme::LIGHT.tokens();
        let dark = Scheme::DARK.tokens();
        assert_eq!(light.len(), FIGMA_RICK.len());
        assert_eq!(Scheme::NAMES.len(), FIGMA_RICK.len());
        for (i, (name, l, d)) in FIGMA_RICK.iter().enumerate() {
            assert_eq!(light[i].0, *name);
            assert_eq!(Scheme::NAMES[i], *name);
            assert_eq!(hex_text(light[i].1), *l, "{name} light");
            assert_eq!(hex_text(dark[i].1), *d, "{name} dark");
            assert_eq!(Scheme::LIGHT.token(name), Some(light[i].1));
            assert_eq!(Scheme::DARK.token(name), Some(dark[i].1));
        }
        assert_eq!(Scheme::LIGHT.token("bg/nothing"), None);
        assert!(!Scheme::LIGHT.dark);
        assert!(Scheme::DARK.dark);
    }

    #[test]
    fn the_metrics_are_the_figma_collection_one_to_one() {
        let comfortable = Metrics::COMFORTABLE.tokens();
        let compact = Metrics::COMPACT.tokens();
        assert_eq!(comfortable.len(), FIGMA_DENSITY.len());
        for (i, (name, a, b)) in FIGMA_DENSITY.iter().enumerate() {
            assert_eq!(comfortable[i].0, *name);
            assert_eq!(comfortable[i].1, *a, "{name} comfortable");
            assert_eq!(compact[i].1, *b, "{name} compact");
        }
        assert_eq!(Metrics::COMFORTABLE.toolbar_height(), 46.0);
        assert_eq!(Metrics::COMPACT.toolbar_height(), 36.0);
        // The Figma status bar: 39 and 35, the hairline included.
        assert_eq!(Metrics::COMFORTABLE.footer_height(), 39.0);
        assert_eq!(Metrics::COMPACT.footer_height(), 35.0);
        assert_eq!(Metrics::of(Density::Comfortable), &Metrics::COMFORTABLE);
        assert_eq!(Metrics::of(Density::Compact), &Metrics::COMPACT);
        assert_eq!(Metrics::of(Density::default()), &Metrics::COMFORTABLE);
    }

    /// The text pairs the design puts together, at 4.5:1, and the control
    /// and icon pairs, at 3:1, in both schemes. The pairs the Figma file
    /// holds that fall short are listed in QUESTIONS.md 242 and are not
    /// here: `text/secondary` on `bg/control` and `bg/selected-inactive`,
    /// `border/strong` on `bg/pane` in Light, and the warning border on
    /// light backgrounds.
    #[test]
    fn the_pairs_meet_wcag_contrast() {
        for scheme in [&Scheme::LIGHT, &Scheme::DARK] {
            let s = scheme;
            let text_pairs: [(&str, Color32, Color32); 27] = [
                ("primary on window", s.text_primary, s.bg_window),
                ("primary on panel", s.text_primary, s.bg_panel),
                ("primary on pane", s.text_primary, s.bg_pane),
                ("primary on card", s.text_primary, s.bg_card),
                ("primary on subtle", s.text_primary, s.bg_subtle),
                ("primary on control", s.text_primary, s.bg_control),
                (
                    "primary on inactive",
                    s.text_primary,
                    s.bg_selected_inactive,
                ),
                ("primary on dialog", s.text_primary, s.dialog_bg),
                (
                    "primary on success row",
                    s.text_primary,
                    s.status_success_row,
                ),
                ("secondary on window", s.text_secondary, s.bg_window),
                ("secondary on panel", s.text_secondary, s.bg_panel),
                ("secondary on pane", s.text_secondary, s.bg_pane),
                ("secondary on card", s.text_secondary, s.bg_card),
                ("secondary on subtle", s.text_secondary, s.bg_subtle),
                ("secondary on dialog", s.text_secondary, s.dialog_bg),
                ("placeholder on pane", s.text_placeholder, s.bg_pane),
                ("placeholder on card", s.text_placeholder, s.bg_card),
                (
                    "on-selected on soft",
                    s.text_on_selected,
                    s.bg_selected_soft,
                ),
                ("focus text on pane", s.accent_focus, s.bg_pane),
                ("focus text on card", s.accent_focus, s.bg_card),
                ("success on pane", s.status_success, s.bg_pane),
                ("success on its pill", s.status_success, s.status_success_bg),
                ("success on its row", s.status_success, s.status_success_row),
                ("warning on dialog", s.status_warning, s.dialog_bg),
                ("danger on card", s.status_danger, s.bg_card),
                ("trash pills", s.trash_low_text, s.trash_low_bg),
                ("button text", Scheme::ON_BUTTON, s.accent_button),
            ];
            for (what, fg, bg) in text_pairs {
                let ratio = contrast(fg, bg);
                assert!(ratio >= 4.5, "{}: {what} {ratio:.2}", s.name);
            }
            assert!(contrast(s.trash_mid_text, s.trash_mid_bg) >= 4.5);
            assert!(contrast(s.trash_high_text, s.trash_high_bg) >= 4.5);
            let control_pairs: [(&str, Color32, Color32); 14] = [
                ("icon on window", s.icon_default, s.bg_window),
                ("icon on panel", s.icon_default, s.bg_panel),
                ("icon on pane", s.icon_default, s.bg_pane),
                ("icon on card", s.icon_default, s.bg_card),
                ("icon on subtle", s.icon_default, s.bg_subtle),
                ("icon on control", s.icon_default, s.bg_control),
                ("icon on soft", s.icon_default, s.bg_selected_soft),
                ("focus on pane", s.accent_focus, s.bg_pane),
                ("focus on subtle", s.accent_focus, s.bg_subtle),
                ("focus on soft", s.accent_focus, s.bg_selected_soft),
                ("strong border on card", s.border_strong, s.bg_card),
                ("switch off on pane", s.bg_switch_off, s.bg_pane),
                ("usage on track", s.accent_usage, s.bg_track),
                ("button on dialog", s.accent_button, s.dialog_bg),
            ];
            for (what, fg, bg) in control_pairs {
                let ratio = contrast(fg, bg);
                assert!(ratio >= 3.0, "{}: {what} {ratio:.2}", s.name);
            }
        }
    }

    /// Every preset of *Title bar colour*, in both schemes: the titles of
    /// the inactive tabs and the search box's text at 4.5:1, the icons at
    /// 3:1, at rest and on hover (SPEC.md *On the band*).
    #[test]
    fn every_title_bar_preset_holds_its_contrast_in_both_schemes() {
        use model::TitlebarColour;
        let mut presets = 0;
        for preset in TitlebarColour::ALL {
            let Some((light, dark)) = preset.preset() else {
                assert_eq!(preset, TitlebarColour::Custom);
                continue;
            };
            presets += 1;
            for (scheme, fill) in [(&Scheme::LIGHT, rgb(light)), (&Scheme::DARK, rgb(dark))] {
                let band = Band::on(scheme, fill);
                let at = format!("{preset:?} in {}", scheme.name);
                assert_eq!(band.fill, fill);
                // Light in Light and dark in Dark, but for the Dark band.
                let dark_band = Band::is_dark(fill);
                assert_eq!(
                    dark_band,
                    scheme.dark || preset == TitlebarColour::DarkBand,
                    "{at}"
                );
                assert_eq!(band.contrary, dark_band != scheme.dark, "{at}");
                let text = [
                    ("title", band.title, band.fill),
                    ("title on a hovered tab", band.title, band.tab_hover),
                    ("search text", band.search_text, band.search_fill),
                    (
                        "search placeholder",
                        band.search_placeholder,
                        band.search_fill,
                    ),
                ];
                for (what, fg, bg) in text {
                    let ratio = contrast(fg, bg);
                    assert!(ratio >= 4.5, "{at}: {what} {ratio:.2}");
                }
                let icons = [
                    ("icon", band.icon, band.fill),
                    ("icon on a hovered tab", band.icon, band.tab_hover),
                    ("icon on a hovered button", band.icon, band.hover),
                    ("search icon", band.search_placeholder, band.search_fill),
                ];
                for (what, fg, bg) in icons {
                    let ratio = contrast(fg, bg);
                    assert!(ratio >= 3.0, "{at}: {what} {ratio:.2}");
                }
            }
        }
        assert_eq!(presets, 6);

        // The default preset is the token.
        assert_eq!(
            TitlebarColour::default()
                .preset()
                .map(|(l, d)| (rgb(l), rgb(d))),
            Some((Scheme::LIGHT.bg_titlebar, Scheme::DARK.bg_titlebar))
        );
        // On the five light presets in Light `text/secondary` falls short,
        // and the titles are the band's text colour at 78 %; in Dark it
        // holds and stays.
        let rose = Band::on(&Scheme::LIGHT, Scheme::LIGHT.bg_titlebar);
        assert!(contrast(Scheme::LIGHT.text_secondary, rose.fill) < 4.5);
        assert_eq!(rose.title, over(Band::INK, 0.78, rose.fill));
        assert_eq!(rose.icon, Scheme::LIGHT.icon_default);
        assert_eq!(rose.search_fill, Scheme::LIGHT.bg_pane);
        let charcoal = Band::on(&Scheme::DARK, Scheme::DARK.bg_titlebar);
        assert_eq!(charcoal.title, Scheme::DARK.text_secondary);
        // The Dark band in Light: the row in white, as frame 167:5261
        // draws it.
        let fill = hex("#2f3237");
        let dark = Band::on(&Scheme::LIGHT, fill);
        assert!(dark.contrary);
        assert_eq!(dark.title, over(Color32::WHITE, 0.78, fill));
        assert_eq!(dark.icon, over(Color32::WHITE, 0.90, fill));
        assert_eq!(dark.divider, over(Color32::WHITE, 0.25, fill));
        assert_eq!(dark.hover, over(Color32::WHITE, 0.12, fill));
        assert_eq!(dark.tab_hover, dark.hover);
        assert_eq!(dark.search_fill, over(Color32::WHITE, 0.14, fill));
        // A light custom colour in Dark is the mirror image.
        let light = Band::on(&Scheme::DARK, hex("#e6e2dc"));
        assert!(light.contrary);
        assert_eq!(light.icon, over(Band::INK, 0.90, light.fill));
        // A mid grey no 78 % reads on takes the text colour whole.
        let grey = hex("#808080");
        let mid = Band::on(&Scheme::LIGHT, grey);
        assert!(contrast(mid.title, grey) >= contrast(over(Color32::WHITE, 0.78, grey), grey));
    }

    #[test]
    fn contrast_is_wcag() {
        assert!((contrast(Color32::WHITE, Color32::BLACK) - 21.0).abs() < 1e-9);
        assert!((contrast(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 1e-9);
        assert!((contrast(Color32::WHITE, Color32::WHITE) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn apply_derives_both_themes_and_keeps_the_sets() {
        let ctx = egui::Context::default();
        apply(&ctx, Density::Compact, ROSE);
        let light = ctx.style_of(egui::Theme::Light);
        let dark = ctx.style_of(egui::Theme::Dark);
        assert_eq!(light.visuals.panel_fill, Scheme::LIGHT.bg_window);
        assert_eq!(dark.visuals.panel_fill, Scheme::DARK.bg_window);
        assert!(!light.visuals.dark_mode);
        assert!(dark.visuals.dark_mode);
        assert_eq!(
            light.visuals.selection.bg_fill,
            Scheme::LIGHT.bg_selected_soft
        );
        assert_eq!(dark.visuals.error_fg_color, Scheme::DARK.status_danger);
        assert_eq!(
            light.visuals.weak_text_color,
            Some(Scheme::LIGHT.text_secondary)
        );
        assert_eq!(
            light.spacing.button_padding.y,
            Metrics::COMPACT.space_toolbar_y
        );
        assert_eq!(
            dark.spacing.item_spacing.y,
            Metrics::COMPACT.space_toolbar_y
        );
        assert_eq!(metrics(&ctx), &Metrics::COMPACT);
        ctx.set_theme(egui::Theme::Dark);
        assert_eq!(scheme_of(&ctx), &Scheme::DARK);
        ctx.set_theme(egui::Theme::Light);
        assert_eq!(scheme_of(&ctx), &Scheme::LIGHT);
        apply(&ctx, Density::Comfortable, ROSE);
        assert_eq!(metrics(&ctx), &Metrics::COMFORTABLE);
        assert_eq!(
            ctx.style_of(egui::Theme::Light).spacing.button_padding.y,
            Metrics::COMFORTABLE.space_toolbar_y
        );
    }

    #[test]
    fn a_context_without_apply_reads_the_defaults() {
        let ctx = egui::Context::default();
        assert_eq!(metrics(&ctx), &Metrics::COMFORTABLE);
        assert_eq!(scheme_of(&ctx), &Scheme::DARK);
        let text = text_styles();
        assert_eq!(text[&TextStyle::Body].size, TEXT_BODY);
        assert_eq!(text[&TextStyle::Small].size, TEXT_SECONDARY);
    }
}
