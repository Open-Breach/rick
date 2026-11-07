//! Inline editing of a name in the list: Rename edits the row itself with
//! the base name pre-selected, New file and New folder insert a row at the
//! top in edit mode. Enter commits, Escape cancels, a name that is taken is
//! refused with the row shaking red.

use egui::{Color32, Rect, Stroke, vec2};
use types::EntryId;

/// What is being edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    /// The name of this entry.
    Rename(EntryId),
    /// The name of a file to create in the tab's location.
    NewFile,
    /// The name of a folder to create in the tab's location.
    NewFolder,
}

/// An edit in progress.
#[derive(Debug, Clone)]
pub struct RowEdit {
    /// What the name is for.
    pub kind: EditKind,
    /// The text so far.
    pub text: String,
    /// The name before editing; an unchanged Rename is a cancel.
    pub original: String,
    /// Take the keyboard and select the base name on the next frame.
    pub focus: bool,
    /// Until this frame time the row shakes red: the name was refused.
    pub shake_until: Option<f64>,
}

impl RowEdit {
    /// Starts editing `text`.
    #[must_use]
    pub fn new(kind: EditKind, text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            kind,
            original: text.clone(),
            text,
            focus: true,
            shake_until: None,
        }
    }

    /// Whether the edit is being refused right now.
    #[must_use]
    pub fn is_shaking(&self, now: f64) -> bool {
        self.shake_until.is_some_and(|until| until > now)
    }

    fn refuse(&mut self, now: f64) {
        self.shake_until = Some(now + SHAKE_FOR);
        self.focus = true;
    }
}

/// How long a refused name shakes, in seconds.
const SHAKE_FOR: f64 = 0.45;

/// What a frame of editing produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOutcome {
    /// Still editing.
    Editing,
    /// Enter with a name that is free.
    Commit(String),
    /// Escape, an empty or unchanged name, or the focus went elsewhere.
    Cancel,
}

/// The length of the base name: everything before the extension, the whole
/// name when there is none or it is a dot-file.
#[must_use]
pub fn base_len(name: &str) -> usize {
    match name.rfind('.') {
        Some(dot) if dot > 0 => dot,
        _ => name.len(),
    }
}

/// Whether a name can be a file name at all.
fn well_formed(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\']) && name != "." && name != ".."
}

/// Draws the editor in `rect`; `taken` says whether a name exists in the
/// folder already. `select_base` selects the base name on the first frame
/// (Rename), otherwise the whole text (New file, New folder).
pub fn editor(
    ui: &mut egui::Ui,
    edit: &mut RowEdit,
    rect: Rect,
    select_base: bool,
    taken: &dyn Fn(&str) -> bool,
) -> EditOutcome {
    let now = ui.input(|i| i.time);
    let shaking = edit.is_shaking(now);
    let dx = if shaking {
        ((now * 60.0).sin() * 4.0) as f32
    } else {
        0.0
    };
    let rect = rect.translate(vec2(dx, 0.0));
    let id = ui.id().with(("inline-edit", edit.kind_key()));
    let mut text_edit = egui::TextEdit::singleline(&mut edit.text)
        .id(id)
        .desired_width(rect.width())
        .vertical_align(egui::Align::Center)
        .margin(egui::Margin::symmetric(4, 2));
    if shaking {
        text_edit = text_edit.text_color(ui.visuals().error_fg_color);
    }
    let output = text_edit.show(&mut ui.new_child(egui::UiBuilder::new().max_rect(rect)));
    if shaking {
        ui.painter().rect_stroke(
            output.response.rect,
            3.0,
            Stroke::new(1.5, ui.visuals().error_fg_color),
            egui::StrokeKind::Outside,
        );
        ui.ctx().request_repaint();
    }
    let response = output.response;
    if edit.focus {
        response.request_focus();
        let end = if select_base {
            edit.text
                .char_indices()
                .take_while(|(i, _)| *i < base_len(&edit.text))
                .count()
        } else {
            edit.text.chars().count()
        };
        let mut state = output.state;
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(end),
            )));
        state.store(ui.ctx(), response.id);
        edit.focus = false;
        return EditOutcome::Editing;
    }
    let (enter, escape) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    if escape {
        return EditOutcome::Cancel;
    }
    if !response.lost_focus() {
        return EditOutcome::Editing;
    }
    if !enter {
        return EditOutcome::Cancel;
    }
    let name = edit.text.trim().to_owned();
    if name.is_empty() || (matches!(edit.kind, EditKind::Rename(_)) && name == edit.original) {
        return EditOutcome::Cancel;
    }
    if !well_formed(&name) || taken(&name) {
        edit.refuse(now);
        return EditOutcome::Editing;
    }
    EditOutcome::Commit(name)
}

impl RowEdit {
    /// A stable key for the text edit's id.
    fn kind_key(&self) -> u128 {
        match self.kind {
            EditKind::Rename(id) => id.key,
            EditKind::NewFile => 1,
            EditKind::NewFolder => 2,
        }
    }
}

/// The colour of a refused row's outline.
#[must_use]
pub fn refused_color(ui: &egui::Ui) -> Color32 {
    ui.visuals().error_fg_color
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_base_name_excludes_the_extension() {
        assert_eq!(base_len("report.pdf"), 6);
        assert_eq!(base_len("Makefile"), 8);
        assert_eq!(base_len(".bashrc"), 7);
        assert_eq!(base_len("x.tar.gz"), 5);
        assert!(well_formed("a.txt"));
        assert!(!well_formed("a/b"));
        assert!(!well_formed(""));
        assert!(!well_formed(".."));
    }
}
