//! The compare tab: two files side by side with their differences
//! highlighted and one scroll for both sides, or two folders entry by
//! entry with added, removed and changed rows marked and a copy button
//! each way.
//!
//! The comparison ([`ext::Compare`]) is computed by the job the action
//! started; until it is there the tab says so and asks for a repaint. The
//! copy buttons of a folder diff run the kernel's copy
//! ([`Command::Receive`]), with the conflict dialogue and the operations
//! area as for any copy.

use ext::t;
use std::sync::Arc;
use std::time::Duration;

use app::{Command, TransferMode};
use egui::{Color32, RichText};
use ext::{Compare, Diff, FolderDiff, FolderRow, HunkKind, RowStatus, TextDiff};

use crate::shell::Request;
use crate::{format, icons};

/// One aligned row of a side-by-side text diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Row {
    kind: HunkKind,
    left: Option<usize>,
    right: Option<usize>,
}

/// The rows of a text diff: equal and replaced lines side by side, a
/// deletion against a blank right, an insertion against a blank left.
fn rows_of(diff: &TextDiff) -> Vec<Row> {
    let mut rows = Vec::new();
    for hunk in &diff.hunks {
        let len = hunk.left.len().max(hunk.right.len());
        for i in 0..len {
            rows.push(Row {
                kind: hunk.kind,
                left: (i < hunk.left.len()).then_some(hunk.left.start + i),
                right: (i < hunk.right.len()).then_some(hunk.right.start + i),
            });
        }
    }
    rows
}

/// The background of a differing row, on a dark or a light theme.
fn tint(kind: HunkKind, dark: bool) -> Option<Color32> {
    let (r, g, b) = match kind {
        HunkKind::Equal => return None,
        HunkKind::Delete => (0xc0, 0x30, 0x30),
        HunkKind::Insert => (0x30, 0xa0, 0x40),
        HunkKind::Replace => (0xc0, 0x90, 0x20),
    };
    let alpha = if dark { 70 } else { 50 };
    Some(Color32::from_rgba_unmultiplied(r, g, b, alpha))
}

/// Draws the tab.
pub fn show(ui: &mut egui::Ui, compare: &Arc<Compare>, requests: &mut Vec<Request>) {
    let Some(result) = compare.result() else {
        ui.vertical_centered(|ui| {
            ui.add_space(24.0);
            ui.spinner();
            ui.label(format!(
                "Comparing {} with {}…",
                compare.left, compare.right
            ));
        });
        ui.ctx().request_repaint_after(Duration::from_millis(150));
        return;
    };
    match result {
        Err(error) => {
            ui.vertical_centered(|ui| {
                ui.add_space(24.0);
                ui.label(
                    RichText::new(t!("compare-failed", error = error.as_str()))
                        .color(ui.visuals().error_fg_color),
                );
            });
        }
        Ok(diff) => match &*diff {
            Diff::Text(text) => show_text(ui, compare, text),
            Diff::Folders(folders) => show_folders(ui, folders, requests),
            Diff::Binary { identical } => {
                ui.vertical_centered(|ui| {
                    ui.add_space(24.0);
                    ui.label(if *identical {
                        format!("{} and {} are identical", compare.left, compare.right)
                    } else {
                        format!(
                            "{} and {} are binary files and differ",
                            compare.left, compare.right
                        )
                    });
                });
            }
        },
    }
}

/// Two texts side by side: a header with the names and the count, then
/// the rows in one virtualised scroll, so both sides move together.
fn show_text(ui: &mut egui::Ui, compare: &Compare, diff: &TextDiff) {
    let rows = rows_of(diff);
    let changed = diff.changed_lines();
    ui.horizontal(|ui| {
        ui.columns(2, |columns| {
            columns[0].strong(&compare.left);
            columns[1].strong(&compare.right);
        });
    });
    ui.weak(if changed == 0 {
        "The files are identical".to_owned()
    } else {
        format!("{changed} changed lines")
    });
    ui.separator();
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let row_height = ui.fonts_mut(|f| f.row_height(&font)) + 2.0;
    let dark = ui.visuals().dark_mode;
    let gutter = 48.0;
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show_rows(ui, row_height, rows.len(), |ui, range| {
            let width = ui.available_width().max(400.0);
            let half = (width - 8.0) / 2.0;
            for row in &rows[range] {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(width, row_height), egui::Sense::hover());
                let left = egui::Rect::from_min_size(rect.min, egui::vec2(half, row_height));
                let right = egui::Rect::from_min_size(
                    rect.min + egui::vec2(half + 8.0, 0.0),
                    egui::vec2(half, row_height),
                );
                let painter = ui.painter();
                if let Some(color) = tint(row.kind, dark) {
                    if row.left.is_some() || row.kind == HunkKind::Replace {
                        painter.rect_filled(left, 0.0, color);
                    }
                    if row.right.is_some() || row.kind == HunkKind::Replace {
                        painter.rect_filled(right, 0.0, color);
                    }
                }
                let side = |rect: egui::Rect, index: Option<usize>, lines: &[String]| {
                    let Some(index) = index else {
                        return;
                    };
                    painter.text(
                        rect.left_top() + egui::vec2(gutter - 6.0, 1.0),
                        egui::Align2::RIGHT_TOP,
                        (index + 1).to_string(),
                        font.clone(),
                        ui.visuals().weak_text_color(),
                    );
                    let text = lines.get(index).map_or("", String::as_str);
                    let clip = egui::Rect::from_min_max(
                        rect.left_top() + egui::vec2(gutter, 0.0),
                        rect.right_bottom(),
                    );
                    painter.with_clip_rect(clip).text(
                        clip.left_top() + egui::vec2(0.0, 1.0),
                        egui::Align2::LEFT_TOP,
                        text.replace('\t', "    "),
                        font.clone(),
                        ui.visuals().text_color(),
                    );
                };
                side(left, row.left, &diff.left);
                side(right, row.right, &diff.right);
            }
        });
}

/// The word for a status, and whether the row differs.
fn status_text(status: RowStatus) -> (&'static str, bool) {
    match status {
        RowStatus::Same => ("same", false),
        RowStatus::Added => ("only on the right", true),
        RowStatus::Removed => ("only on the left", true),
        RowStatus::Changed => ("differs", true),
    }
}

fn side_text(side: Option<&ext::Side>) -> String {
    let Some(side) = side else {
        return "—".to_owned();
    };
    let mut parts = Vec::new();
    if side.is_dir {
        parts.push("folder".to_owned());
    } else {
        parts.push(format::size(side.size));
    }
    if let Some(modified) = side.modified {
        parts.push(format::exact(modified));
    }
    parts.join(" · ")
}

/// Two folders entry by entry, with the differences highlighted and a
/// copy button towards each side.
fn show_folders(ui: &mut egui::Ui, diff: &FolderDiff, requests: &mut Vec<Request>) {
    let differing = diff
        .rows
        .iter()
        .filter(|r| r.status != RowStatus::Same)
        .count();
    ui.horizontal(|ui| {
        ui.strong(crate::paths::text_of(&diff.left));
        ui.label(icons::COMPARE);
        ui.strong(crate::paths::text_of(&diff.right));
    });
    ui.weak(if differing == 0 {
        "The folders hold the same names, sizes and dates".to_owned()
    } else {
        format!("{differing} of {} entries differ", diff.rows.len())
    });
    ui.separator();
    let dark = ui.visuals().dark_mode;
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("folder-diff")
                .num_columns(6)
                .striped(true)
                .show(ui, |ui| {
                    ui.strong(t!("compare-name"));
                    ui.strong(t!("compare-left"));
                    ui.label("");
                    ui.strong(t!("compare-right"));
                    ui.label("");
                    ui.strong("");
                    ui.end_row();
                    for row in &diff.rows {
                        folder_row(ui, diff, row, dark, requests);
                    }
                });
        });
}

fn folder_row(
    ui: &mut egui::Ui,
    diff: &FolderDiff,
    row: &FolderRow,
    dark: bool,
    requests: &mut Vec<Request>,
) {
    let (word, differs) = status_text(row.status);
    let kind = match row.status {
        RowStatus::Same => HunkKind::Equal,
        RowStatus::Added => HunkKind::Insert,
        RowStatus::Removed => HunkKind::Delete,
        RowStatus::Changed => HunkKind::Replace,
    };
    let color = tint(kind, dark);
    let name = |ui: &mut egui::Ui, text: &str| {
        let label = RichText::new(text);
        let label = if differs { label.strong() } else { label };
        if let Some(color) = color {
            ui.label(label.background_color(color));
        } else {
            ui.label(label);
        }
    };
    name(ui, &row.name);
    ui.label(side_text(row.left.as_ref()));
    // Copy the left entry over to the right.
    if let Some(left) = &row.left {
        if ui
            .small_button(icons::FORWARD)
            .on_hover_text(t!("compare-copy-to-the-right-folder"))
            .clicked()
        {
            requests.push(Request::Command(Command::Receive {
                items: vec![left.location.clone()],
                to: diff.right.clone(),
                mode: TransferMode::Copy,
            }));
        }
    } else {
        ui.label("");
    }
    ui.label(side_text(row.right.as_ref()));
    if let Some(right) = &row.right {
        if ui
            .small_button(icons::BACK)
            .on_hover_text(t!("compare-copy-to-the-left-folder"))
            .clicked()
        {
            requests.push(Request::Command(Command::Receive {
                items: vec![right.location.clone()],
                to: diff.left.clone(),
                mode: TransferMode::Copy,
            }));
        }
    } else {
        ui.label("");
    }
    ui.weak(word);
    ui.end_row();
}

#[cfg(test)]
mod tests {
    use super::*;
    use ext::Hunk;

    #[test]
    fn rows_align_hunks_side_by_side() {
        let diff = TextDiff {
            left: vec!["a".into(), "b".into(), "c".into()],
            right: vec!["a".into(), "x".into(), "y".into(), "c".into()],
            hunks: vec![
                Hunk {
                    kind: HunkKind::Equal,
                    left: 0..1,
                    right: 0..1,
                },
                Hunk {
                    kind: HunkKind::Replace,
                    left: 1..2,
                    right: 1..3,
                },
                Hunk {
                    kind: HunkKind::Equal,
                    left: 2..3,
                    right: 3..4,
                },
            ],
        };
        let rows = rows_of(&diff);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[1].left, Some(1));
        assert_eq!(rows[1].right, Some(1));
        assert_eq!(rows[2].left, None);
        assert_eq!(rows[2].right, Some(2));
        assert_eq!(
            rows[3],
            Row {
                kind: HunkKind::Equal,
                left: Some(2),
                right: Some(3)
            }
        );
        assert!(tint(HunkKind::Equal, true).is_none());
        assert!(tint(HunkKind::Insert, false).is_some());
    }
}
