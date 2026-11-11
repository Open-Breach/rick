//! The Permissions tab of the Properties dialogue: **Ownership** (the
//! owner and the group, each a dropdown over the machine's users or
//! groups), **Access** (the grid of mode bits and the octal field) and
//! **Advanced** (the access control list, opened in place, and *Apply to
//! enclosed items* for a folder), with *Revert* and *Apply* in the footer.

use app::PropertiesInfo;
use egui::{Align, Align2, FontId, Rect, Sense, pos2, vec2};
use ext::t;
use platform::Principal;
use types::EntryKind;

use super::{Ask, Card, Edit, INSET, INSET_CONTROL, ROW, ROW_DESCRIBED, acl, group};
use crate::theme::{self, ICON, TEXT_BODY, TEXT_CAPS, TEXT_SECONDARY};
use crate::{dialog, icons, widgets};

/// A row that holds a dropdown.
const ROW_DROPDOWN: f32 = 46.0;
/// The size of a dropdown.
const DROPDOWN: egui::Vec2 = vec2(150.0, 32.0);
/// A row of a permission grid, and its head.
pub(super) const GRID_ROW: f32 = 30.0;
/// A column of a permission grid.
pub(super) const GRID_COLUMN: f32 = 90.0;
/// The three permissions of a grid, as the bit each is among `rwx`.
pub(super) const BITS: [u8; 3] = [4, 2, 1];

/// The tab's groups for `info`, editing `edit`.
pub(super) fn page(ui: &mut egui::Ui, info: &PropertiesInfo, edit: &mut Edit, acl_open: &mut bool) {
    let Some(current) = &info.permissions else {
        return;
    };
    group(ui, &t!("properties-ownership"), |card| {
        principal_row(
            card,
            &t!("owner"),
            &info.principals.users,
            &mut edit.owner,
            &mut edit.owner_other,
        );
        principal_row(
            card,
            &t!("group"),
            &info.principals.groups,
            &mut edit.group,
            &mut edit.group_other,
        );
    });
    if current.mode.is_some() {
        group(ui, &t!("properties-access"), |card| {
            mode_grid(card, edit);
            octal_row(card, edit);
        });
    }
    let folder = info.entry.kind == EntryKind::Dir;
    if edit.acl.is_some() || folder {
        group(ui, &t!("properties-advanced"), |card| {
            if edit.acl.is_some() {
                acl::rows(card, current, edit, acl_open);
            }
            if folder {
                enclosed_row(card, &mut edit.recursive);
            }
        });
    }
}

/// The tab's footer: *Apply* and *Revert*, both off until something
/// differs from the entry and while a change is being applied.
pub(super) fn footer(ui: &mut egui::Ui, dirty: bool, applying: bool, asked: &mut Option<Ask>) {
    let scheme = theme::scheme(ui);
    let enabled = dirty && !applying;
    dialog::footer(
        ui,
        |ui| {
            if applying {
                ui.label(
                    egui::RichText::new(t!("properties-applying"))
                        .size(TEXT_SECONDARY)
                        .color(scheme.text_secondary),
                );
            }
        },
        |ui| {
            // Laid out from the right edge inwards.
            if widgets::button(ui, &t!("apply"), true, enabled).clicked() && enabled {
                *asked = Some(Ask::Apply);
            }
            if widgets::button(ui, &t!("revert"), false, enabled).clicked() && enabled {
                *asked = Some(Ask::Revert);
            }
        },
    );
}

/// The principal a typed name stands for: the known one of that name or
/// id, else the name as it is, which the platform resolves when the
/// change is applied.
fn resolve(known: &[Principal], text: &str) -> Principal {
    known
        .iter()
        .find(|p| p.display() == text || p.id == text)
        .cloned()
        .unwrap_or_else(|| Principal::named(text, text))
}

/// A row of **Ownership**: the label and, at the right, the dropdown.
fn principal_row(
    card: &mut Card<'_>,
    label: &str,
    known: &[Principal],
    chosen: &mut Option<Principal>,
    other: &mut Option<String>,
) {
    let rect = card.take(ROW_DROPDOWN);
    card.label(rect, label);
    let button = Rect::from_min_size(
        pos2(
            rect.right() - INSET_CONTROL - DROPDOWN.x,
            rect.center().y - DROPDOWN.y / 2.0,
        ),
        DROPDOWN,
    );
    principal_dropdown(card.ui, button, label, known, chosen, other);
}

/// A dropdown over the machine's users or groups, named `label`: a
/// `bg/subtle` button showing the chosen one, which opens the list. The
/// list's last entry, *Other…*, puts a search field over it — `other`
/// holds its text — for a name that is not listed: typing narrows the
/// list, Enter takes the name as typed.
fn principal_dropdown(
    ui: &mut egui::Ui,
    rect: Rect,
    label: &str,
    known: &[Principal],
    chosen: &mut Option<Principal>,
    other: &mut Option<String>,
) {
    let scheme = theme::scheme(ui);
    let radius = theme::metrics(ui.ctx()).corner_radius();
    let response = ui.interact(rect, ui.id().with(("principal", label)), Sense::click());
    let popup = egui::Popup::default_response_id(&response);
    let open = egui::Popup::is_id_open(ui.ctx(), popup);
    let fill = if response.hovered() || open {
        scheme.bg_active
    } else {
        scheme.bg_subtle
    };
    ui.painter().rect_filled(rect, radius, fill);
    let current = chosen
        .as_ref()
        .map_or_else(String::new, |p| p.display().to_owned());
    let galley = widgets::elided(
        ui,
        &current,
        FontId::proportional(TEXT_BODY),
        scheme.text_primary,
        rect.width() - 12.0 - 8.0 - ICON - 8.0,
    );
    ui.painter().galley(
        pos2(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0),
        galley,
        scheme.text_primary,
    );
    ui.painter().text(
        pos2(rect.right() - 8.0 - ICON / 2.0, rect.center().y),
        Align2::CENTER_CENTER,
        icons::DROPDOWN,
        FontId::proportional(ICON),
        scheme.icon_default,
    );
    response.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, label);
        info.current_text_value = Some(current.clone());
        info
    });
    if !open {
        *other = None;
    }
    let focus = popup.with("focus");
    egui::Popup::menu(&response)
        .width(200.0)
        .gap(4.0)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let mut close = false;
            if let Some(text) = other {
                let wanted = ui.data_mut(|d| d.remove_temp::<bool>(focus)).is_some();
                let hint = t!("properties-user-or-group-name");
                let field = dialog::text_field(ui, text, &hint, false, None, wanted);
                let typed = text.trim();
                if field.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && !typed.is_empty()
                {
                    *chosen = Some(resolve(known, typed));
                    close = true;
                }
                ui.add_space(4.0);
            }
            let filter = other.as_deref().map(|text| text.trim().to_lowercase());
            egui::ScrollArea::vertical()
                .max_height(220.0)
                .show(ui, |ui| {
                    ui.with_layout(egui::Layout::top_down_justified(Align::LEFT), |ui| {
                        for principal in known {
                            let name = principal.display();
                            if filter
                                .as_ref()
                                .is_some_and(|f| !name.to_lowercase().contains(f.as_str()))
                            {
                                continue;
                            }
                            let is = chosen.as_ref().is_some_and(|c| c.id == principal.id);
                            if ui.selectable_label(is, name).clicked() {
                                *chosen = Some(principal.clone());
                                close = true;
                            }
                        }
                    });
                });
            if other.is_none() {
                ui.with_layout(egui::Layout::top_down_justified(Align::LEFT), |ui| {
                    if ui.selectable_label(false, t!("properties-other")).clicked() {
                        *other = Some(String::new());
                        ui.data_mut(|d| d.insert_temp(focus, true));
                    }
                });
            }
            if close {
                egui::Popup::close_id(ui.ctx(), popup);
            }
        });
}

/// Where the three columns of a permission grid stand in `rect`: their
/// centres, the last column ending `trailing` before the right edge.
pub(super) fn columns(rect: Rect, trailing: f32) -> [f32; 3] {
    let right = rect.right() - trailing;
    [
        right - GRID_COLUMN * 2.5,
        right - GRID_COLUMN * 1.5,
        right - GRID_COLUMN * 0.5,
    ]
}

/// The head of a permission grid: *Read*, *Write* and *Execute* over the
/// columns, in 11 px `text/secondary`.
pub(super) fn grid_head(ui: &mut egui::Ui, rect: Rect, trailing: f32) {
    let scheme = theme::scheme(ui);
    let titles = [t!("read"), t!("write"), t!("execute")];
    for (x, title) in columns(rect, trailing).into_iter().zip(titles) {
        ui.painter().text(
            pos2(x, rect.center().y),
            Align2::CENTER_CENTER,
            title,
            FontId::proportional(TEXT_CAPS),
            scheme.text_secondary,
        );
    }
}

/// The three checkboxes of one row of a permission grid, for the `rwx`
/// bits `perms`; `who` names the row for a screen reader. Returns the
/// bits after a click, if one changed.
pub(super) fn grid_checks(
    ui: &mut egui::Ui,
    rect: Rect,
    trailing: f32,
    who: &str,
    perms: u8,
) -> Option<u8> {
    let titles = [t!("read"), t!("write"), t!("execute")];
    let mut changed = None;
    for ((x, bit), what) in columns(rect, trailing).into_iter().zip(BITS).zip(titles) {
        let name = t!("properties-may", who = who, what = what);
        let on = perms & bit != 0;
        if widgets::check_at(ui, pos2(x, rect.center().y), on, &name).clicked() {
            changed = Some(perms ^ bit);
        }
    }
    changed
}

/// The **Access** grid: the rows *Owner*, *Group* and *Others*, the
/// columns *Read*, *Write* and *Execute*, one checkbox per mode bit.
fn mode_grid(card: &mut Card<'_>, edit: &mut Edit) {
    let scheme = theme::scheme(card.ui);
    let block = card.take(6.0 + GRID_ROW * 4.0 + 6.0);
    let row = |index: usize| {
        #[allow(clippy::cast_precision_loss)]
        let top = block.top() + 6.0 + GRID_ROW * index as f32;
        Rect::from_min_max(pos2(block.left(), top), pos2(block.right(), top + GRID_ROW))
    };
    grid_head(card.ui, row(0), INSET);
    let mut mode = edit.mode;
    let rows = [(t!("owner"), 6u32), (t!("group"), 3), (t!("others"), 0)];
    for (index, (who, shift)) in rows.into_iter().enumerate() {
        let rect = row(index + 1);
        widgets::line(
            card.ui,
            pos2(rect.left() + INSET, rect.center().y),
            &who,
            FontId::proportional(TEXT_BODY),
            scheme.text_primary,
            columns(rect, INSET)[0] - GRID_COLUMN / 2.0 - rect.left() - INSET,
        );
        // Three bits of the mode, so the cast loses nothing.
        #[allow(clippy::cast_possible_truncation)]
        let perms = ((mode >> shift) & 7) as u8;
        if let Some(perms) = grid_checks(card.ui, rect, INSET, &who, perms) {
            mode = (mode & !(7 << shift)) | (u32::from(perms) << shift);
        }
    }
    if mode != edit.mode {
        edit.set_mode(mode);
    }
}

/// The special bits that are on, *setgid · sticky*, or all three named as
/// off.
fn special_bits(mode: u32) -> String {
    let on: Vec<&str> = [
        ("setuid", 0o4000u32),
        ("setgid", 0o2000),
        ("sticky", 0o1000),
    ]
    .into_iter()
    .filter(|(_, bit)| mode & bit != 0)
    .map(|(name, _)| name)
    .collect();
    if on.is_empty() {
        t!("properties-special-bits-off")
    } else {
        on.join(" · ")
    }
}

/// The *Octal* row: the mode as four digits in a monospace field, whose
/// first digit sets setuid, setgid and sticky, and the special bits that
/// are on at the right end.
fn octal_row(card: &mut Card<'_>, edit: &mut Edit) {
    let scheme = theme::scheme(card.ui);
    let rect = card.take(ROW);
    let name = t!("octal");
    let left = card.label(rect, &name);
    let special = special_bits(edit.mode);
    let font = FontId::proportional(TEXT_SECONDARY);
    let width = card
        .ui
        .painter()
        .layout_no_wrap(special.clone(), font.clone(), scheme.text_secondary)
        .size()
        .x;
    widgets::line(
        card.ui,
        pos2(rect.right() - INSET - width, rect.center().y),
        &special,
        font,
        scheme.text_secondary,
        width + 1.0,
    );
    let field = Rect::from_min_max(pos2(left, rect.top()), pos2(left + 64.0, rect.bottom()));
    let mut child = card.ui.new_child(
        egui::UiBuilder::new()
            .max_rect(field)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    let output = egui::TextEdit::singleline(&mut edit.octal)
        .frame(egui::Frame::NONE)
        .margin(egui::Margin::ZERO)
        .font(FontId::monospace(TEXT_BODY))
        .text_color(scheme.text_primary)
        .char_limit(4)
        .desired_width(field.width())
        .vertical_align(Align::Center)
        .show(&mut child);
    let text = edit.octal.clone();
    output.response.widget_info(|| {
        let mut info = egui::WidgetInfo::text_edit(true, &text, &text, "");
        info.label = Some(name.clone());
        info
    });
    if output.response.changed()
        && let Ok(typed) = u32::from_str_radix(edit.octal.trim(), 8)
        && typed <= 0o7777
        && typed != edit.mode
    {
        // The digits stay as typed while the field is being edited.
        let digits = std::mem::take(&mut edit.octal);
        edit.set_mode(typed);
        edit.octal = digits;
    }
    if output.response.lost_focus() {
        edit.octal = format!("{:04o}", edit.mode);
    }
}

/// A row with a title and a line under it, `room` left free at its right
/// end for the row's control; returns the rect of the row. The title is
/// painted only: the control is what carries it as its name.
pub(super) fn described(card: &mut Card<'_>, title: &str, detail: &str, room: f32) -> Rect {
    let scheme = theme::scheme(card.ui);
    let rect = card.take(ROW_DESCRIBED);
    let width = rect.width() - INSET - room;
    let galley = widgets::elided(
        card.ui,
        title,
        FontId::proportional(TEXT_BODY),
        scheme.text_primary,
        width,
    );
    card.ui.painter().galley(
        pos2(
            rect.left() + INSET,
            rect.center().y - 9.0 - galley.size().y / 2.0,
        ),
        galley,
        scheme.text_primary,
    );
    widgets::line(
        card.ui,
        pos2(rect.left() + INSET, rect.center().y + 9.0),
        detail,
        FontId::proportional(TEXT_SECONDARY),
        scheme.text_secondary,
        width,
    );
    rect
}

/// *Apply to enclosed items*, with what it does under it and a switch.
fn enclosed_row(card: &mut Card<'_>, recursive: &mut bool) {
    let title = t!("properties-apply-to-enclosed-items");
    let rect = described(
        card,
        &title,
        &t!("properties-apply-to-enclosed-detail"),
        INSET_CONTROL + 32.0 + 12.0,
    );
    let switch = Rect::from_center_size(
        pos2(rect.right() - INSET_CONTROL - 16.0, rect.center().y),
        vec2(32.0, 18.0),
    );
    let mut child = card.ui.new_child(egui::UiBuilder::new().max_rect(switch));
    if widgets::toggle(&mut child, *recursive, &title).clicked() {
        *recursive = !*recursive;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_special_bits_that_are_on_are_named_or_all_three_as_off() {
        crate::english();
        assert_eq!(special_bits(0o0710), "setuid · setgid · sticky: off");
        assert_eq!(special_bits(0o3755), "setgid · sticky");
        assert_eq!(special_bits(0o4755), "setuid");
    }

    #[test]
    fn a_typed_name_is_the_known_principal_or_the_name_itself() {
        let known = [Principal::named("1000", "serge")];
        assert_eq!(resolve(&known, "serge").id, "1000");
        assert_eq!(resolve(&known, "1000").id, "1000");
        assert_eq!(resolve(&known, "qemu"), Principal::named("qemu", "qemu"));
    }
}
