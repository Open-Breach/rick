//! The access control list on the Permissions tab: a row with a one-line
//! summary that opens in place into the list's table on `bg/subtle`.
//!
//! A POSIX list is a grid like **Access** — *Owner*, *Owning group*, the
//! named entries, *Mask*, *Others*, each with *Read*, *Write* and
//! *Execute* — and an *Add entry* line. The two grids show one set of
//! permissions and move together ([`mode_into_list`], [`list_into_mode`]):
//! the owner's and the others' bits are the same in both, and once the
//! list has named entries the mode's group bits are the mask, as `chmod`
//! and `ls -l` have it. The other kinds of list — `NFSv4`, macOS, NTFS —
//! open in the same place with the entries and rights they showed in 0.3.

use egui::{Align, Align2, FontId, Rect, Sense, pos2, vec2};
use ext::t;
use platform::{Acl, DarwinAce, FilePermissions, NtAce, PosixAce, PosixTag, Principal};

use super::permissions::{GRID_ROW, columns, described, grid_checks, grid_head};
use super::{Card, Edit, INSET_CONTROL, ROW_ICON_BUTTON, icon_button};
use crate::theme::{self, CARD_RADIUS, ICON, TEXT_BODY, TEXT_CAPS};
use crate::{dialog, icons, widgets};

/// The rights an NTFS entry can be given, as the dialogue's checkboxes.
const NT_RIGHTS: &[(&str, u32)] = &[
    ("Full control", 0x001F_01FF),
    ("Modify", 0x0013_01BF),
    ("Read and execute", 0x0012_00A9),
    ("Read", 0x0012_0089),
    ("Write", 0x0000_0116),
];

/// The permission words of a macOS entry the dialogue offers.
const DARWIN_PERMS: &[&str] = &[
    "read",
    "write",
    "execute",
    "delete",
    "append",
    "readattr",
    "writeattr",
    "readextattr",
    "writeextattr",
    "readsecurity",
    "writesecurity",
    "chown",
    "file_inherit",
    "directory_inherit",
];

impl Edit {
    /// Sets the mode, as the Access grid or the octal field has it now; a
    /// POSIX list follows.
    pub(super) fn set_mode(&mut self, mode: u32) {
        self.mode = mode;
        self.octal = format!("{mode:04o}");
        if let Some(Acl::Posix { access, .. }) = &mut self.acl {
            mode_into_list(mode, access);
        }
    }

    /// A POSIX list changed in its table: the mode follows.
    fn list_changed(&mut self) {
        if let Some(Acl::Posix { access, .. }) = &self.acl {
            let mode = list_into_mode(self.mode, access);
            if mode != self.mode {
                self.mode = mode;
                self.octal = format!("{mode:04o}");
            }
        }
    }
}

/// Whether an entry names a user or a group, beyond the owner, the owning
/// group and the others.
fn named(entry: &PosixAce) -> bool {
    matches!(entry.tag, PosixTag::User | PosixTag::Group)
}

/// Puts the mode's permission bits into a POSIX list: the owner's and the
/// others' into their entries, the group's into the mask when the list
/// has one, else into the owning group's entry.
pub(super) fn mode_into_list(mode: u32, access: &mut [PosixAce]) {
    // Three bits each, so the casts lose nothing.
    #[allow(clippy::cast_possible_truncation)]
    let bits = |shift: u32| ((mode >> shift) & 7) as u8;
    let masked = access.iter().any(|e| e.tag == PosixTag::Mask);
    for entry in access {
        match entry.tag {
            PosixTag::UserObj => entry.perms = bits(6),
            PosixTag::Mask => entry.perms = bits(3),
            PosixTag::GroupObj if !masked => entry.perms = bits(3),
            PosixTag::Other => entry.perms = bits(0),
            _ => {}
        }
    }
}

/// The mode a POSIX list stands for: `mode` with its nine permission bits
/// taken from the list, the group's from the mask when there is one.
pub(super) fn list_into_mode(mode: u32, access: &[PosixAce]) -> u32 {
    let of = |tag: PosixTag| {
        access
            .iter()
            .find(|e| e.tag == tag)
            .map(|e| u32::from(e.perms & 7))
    };
    let mut out = mode;
    let mut put = |shift: u32, perms: Option<u32>| {
        if let Some(perms) = perms {
            out = (out & !(7 << shift)) | (perms << shift);
        }
    };
    put(6, of(PosixTag::UserObj));
    put(3, of(PosixTag::Mask).or_else(|| of(PosixTag::GroupObj)));
    put(0, of(PosixTag::Other));
    out
}

/// Adds a named entry with read access to a POSIX list. The mask comes
/// with the first one and always covers the new entry, as `setfacl` has
/// it: the union of the owning group's and the named entries' rights.
fn add_named(access: &mut Vec<PosixAce>, name: &str, group: bool) {
    const NEW: u8 = 4;
    access.push(PosixAce {
        tag: if group {
            PosixTag::Group
        } else {
            PosixTag::User
        },
        qualifier: Some(Principal::named(name, name)),
        perms: NEW,
    });
    if let Some(mask) = access.iter_mut().find(|e| e.tag == PosixTag::Mask) {
        mask.perms |= NEW;
    } else {
        let owning = access
            .iter()
            .find(|e| e.tag == PosixTag::GroupObj)
            .map_or(0, |e| e.perms);
        access.push(PosixAce {
            tag: PosixTag::Mask,
            qualifier: None,
            perms: owning | NEW,
        });
    }
}

/// Removes entry `index` from a POSIX list; the mask goes with the last
/// named entry.
fn remove_entry(access: &mut Vec<PosixAce>, index: usize) {
    if index < access.len() {
        access.remove(index);
    }
    if !access.iter().any(named) {
        access.retain(|e| e.tag != PosixTag::Mask);
    }
}

/// `r-x` for the three bits of `perms`.
fn rwx(perms: u8) -> String {
    [(4, 'r'), (2, 'w'), (1, 'x')]
        .into_iter()
        .map(|(bit, letter)| if perms & bit == 0 { '-' } else { letter })
        .collect()
}

/// The name an entry carries, or nothing.
fn who(entry: &PosixAce) -> String {
    entry
        .qualifier
        .as_ref()
        .map_or_else(String::new, |p| p.display().to_owned())
}

/// The word for what a named entry is: *user* or *group*.
fn tag_word(entry: &PosixAce) -> String {
    if entry.tag == PosixTag::Group {
        t!("properties-acl-group")
    } else {
        t!("properties-acl-user")
    }
}

/// The list in one line: *1 extra entry: user qemu  --x* for a POSIX
/// list, the number of entries for the other kinds.
pub(super) fn summary(acl: &Acl) -> String {
    match acl {
        Acl::Posix { access, .. } => {
            let extra: Vec<String> = access
                .iter()
                .filter(|e| named(e))
                .map(|e| format!("{} {}  {}", tag_word(e), who(e), rwx(e.perms)))
                .collect();
            if extra.is_empty() {
                t!("properties-acl-no-extra")
            } else {
                t!(
                    "properties-acl-extra",
                    count = extra.len(),
                    entries = extra.join(", ")
                )
            }
        }
        Acl::Nfs4(entries) => t!("properties-acl-entries", count = entries.len()),
        Acl::Darwin(entries) => t!("properties-acl-entries", count = entries.len()),
        Acl::Nt { entries, .. } => t!("properties-acl-entries", count = entries.len()),
    }
}

/// The list's rows in the **Advanced** card: the summary row, which opens
/// and closes the list, and under it the table while open. For a file no
/// row follows, so the table takes the card's bottom corners.
pub(super) fn rows(
    card: &mut Card<'_>,
    current: &FilePermissions,
    edit: &mut Edit,
    open: &mut bool,
) {
    let Some(acl) = &edit.acl else {
        return;
    };
    let scheme = theme::scheme(card.ui);
    let title = t!("properties-acl");
    let rect = described(card, &title, &summary(acl), INSET_CONTROL + ICON + 12.0);
    let response = card
        .ui
        .interact(rect, card.ui.id().with("acl-row"), Sense::click());
    card.ui.painter().text(
        pos2(rect.right() - INSET_CONTROL - ICON / 2.0, rect.center().y),
        Align2::CENTER_CENTER,
        if *open {
            icons::TREE_OPEN
        } else {
            icons::TREE_CLOSED
        },
        FontId::proportional(ICON),
        scheme.icon_default,
    );
    let is_open = *open;
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::CollapsingHeader,
            true,
            is_open,
            title.clone(),
        )
    });
    if response.clicked() {
        *open = !*open;
    }
    if *open {
        table(card, current, edit);
    }
}

/// The open list: its table on `bg/subtle`, as the next row of the card.
fn table(card: &mut Card<'_>, current: &FilePermissions, edit: &mut Edit) {
    let scheme = theme::scheme(card.ui);
    // Under a file's list nothing follows: the card's corners are its own.
    let corner = if current.is_dir {
        0
    } else {
        CARD_RADIUS.saturating_sub(1)
    };
    let owner = edit
        .owner
        .as_ref()
        .unwrap_or(&current.owner)
        .display()
        .to_owned();
    let owning = edit
        .group
        .as_ref()
        .unwrap_or(&current.group)
        .display()
        .to_owned();
    let mut changed = false;
    egui::Frame::new()
        .fill(scheme.bg_subtle)
        .corner_radius(egui::CornerRadius {
            nw: 0,
            ne: 0,
            sw: corner,
            se: corner,
        })
        .inner_margin(egui::Margin {
            left: 14,
            right: 10,
            top: 4,
            bottom: 10,
        })
        .show(card.ui, |ui| {
            ui.set_width(ui.available_width());
            let Edit {
                acl,
                new_name,
                new_group,
                new_allow,
                ..
            } = edit;
            let mut new = NewEntry {
                name: new_name,
                group: new_group,
                allow: new_allow,
            };
            match acl {
                Some(Acl::Posix { access, .. }) => {
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    changed = posix_table(ui, access, &owner, &owning, &mut new);
                }
                Some(Acl::Nfs4(entries)) => {
                    ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
                    nfs4_section(ui, entries);
                }
                Some(Acl::Darwin(entries)) => {
                    ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
                    darwin_section(ui, entries, &mut new);
                }
                Some(Acl::Nt { entries, protected }) => {
                    ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
                    nt_section(ui, entries, protected, &mut new);
                }
                None => {}
            }
        });
    card.drawn += 1;
    if changed {
        edit.list_changed();
    }
}

/// The three fields a new ACL entry is typed into.
struct NewEntry<'a> {
    name: &'a mut String,
    group: &'a mut bool,
    allow: &'a mut bool,
}

/// The order the table shows a POSIX list in: the owner, the owning
/// group, the named entries as the list has them, the mask, the others.
fn shown_order(access: &[PosixAce]) -> Vec<usize> {
    let rank = |entry: &PosixAce| match entry.tag {
        PosixTag::UserObj => 0,
        PosixTag::GroupObj => 1,
        PosixTag::User | PosixTag::Group => 2,
        PosixTag::Mask => 3,
        PosixTag::Other => 4,
    };
    let mut order: Vec<usize> = (0..access.len()).collect();
    order.sort_by_key(|&index| rank(&access[index]));
    order
}

/// The table of a POSIX list and its *Add entry* line. Returns whether
/// the list changed.
fn posix_table(
    ui: &mut egui::Ui,
    access: &mut Vec<PosixAce>,
    owner: &str,
    owning: &str,
    new: &mut NewEntry<'_>,
) -> bool {
    let scheme = theme::scheme(ui);
    let mut changed = false;
    let mut remove: Option<usize> = None;
    let row = |ui: &mut egui::Ui| {
        ui.allocate_exact_size(vec2(ui.available_width(), GRID_ROW), Sense::hover())
            .0
    };
    let head = row(ui);
    grid_head(ui, head, ROW_ICON_BUTTON);
    for index in shown_order(access) {
        let entry = &access[index];
        let rect = row(ui);
        let cells = columns(rect, ROW_ICON_BUTTON)[0] - super::permissions::GRID_COLUMN / 2.0;
        let label = match entry.tag {
            PosixTag::UserObj => t!("properties-acl-owner", name = owner),
            PosixTag::GroupObj => t!("properties-acl-owning-group", name = owning),
            PosixTag::User | PosixTag::Group => who(entry),
            PosixTag::Mask => t!("properties-acl-mask"),
            PosixTag::Other => t!("others"),
        };
        let mut room = cells - rect.left() - 8.0;
        if named(entry) {
            // The small tag, ending where the cells begin.
            let word = tag_word(entry);
            let galley = ui.painter().layout_no_wrap(
                word.clone(),
                FontId::proportional(TEXT_CAPS),
                scheme.text_secondary,
            );
            let tag = Rect::from_min_max(
                pos2(cells - galley.size().x - 12.0, rect.center().y - 9.0),
                pos2(cells, rect.center().y + 9.0),
            );
            ui.painter().rect(
                tag,
                4.0,
                scheme.bg_card,
                egui::Stroke::new(1.0, scheme.border_default),
                egui::StrokeKind::Inside,
            );
            ui.painter().galley(
                pos2(tag.left() + 6.0, tag.center().y - galley.size().y / 2.0),
                galley,
                scheme.text_secondary,
            );
            ui.interact(tag, ui.id().with(("acl-tag", index)), Sense::hover())
                .widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Label, true, word.clone())
                });
            room = tag.left() - rect.left() - 8.0;
        }
        widgets::line(
            ui,
            pos2(rect.left(), rect.center().y),
            &label,
            FontId::proportional(TEXT_BODY),
            scheme.text_primary,
            room,
        );
        if let Some(perms) = grid_checks(ui, rect, ROW_ICON_BUTTON, &label, entry.perms) {
            access[index].perms = perms;
            changed = true;
        }
        if named(&access[index]) {
            let button = Rect::from_center_size(
                pos2(rect.right() - ROW_ICON_BUTTON / 2.0, rect.center().y),
                vec2(ROW_ICON_BUTTON, ROW_ICON_BUTTON),
            );
            let name = t!("properties-acl-remove", name = label.as_str());
            if icon_button(ui, button, icons::CLOSE, &name).clicked() {
                remove = Some(index);
            }
        }
    }
    if let Some(index) = remove {
        remove_entry(access, index);
        changed = true;
    }
    ui.add_space(6.0);
    if add_line(ui, new) {
        add_named(access, new.name.trim(), *new.group);
        new.name.clear();
        changed = true;
    }
    changed
}

/// The *Add entry* line: the name field, *User* / *Group* and the *Add*
/// button, which is off until a name is typed. Returns whether an entry is
/// to be added: the button, or Enter in the field.
fn add_line(ui: &mut egui::Ui, new: &mut NewEntry<'_>) -> bool {
    let (rect, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), dialog::FIELD_HEIGHT),
        Sense::hover(),
    );
    let mut line = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::right_to_left(Align::Center)),
    );
    line.spacing_mut().item_spacing.x = 8.0;
    let ready = !new.name.trim().is_empty();
    // Laid out from the right edge inwards.
    let mut add = widgets::icon_button(&mut line, Some(icons::PLUS), &t!("add"), false, ready)
        .clicked()
        && ready;
    let options = [t!("properties-acl-user-option"), t!("group")];
    let (segments, _) = line.allocate_exact_size(vec2(124.0, dialog::FIELD_HEIGHT), Sense::hover());
    let id = line.id().with("acl-new-kind");
    if let Some(picked) =
        widgets::segmented(&mut line, segments, id, &options, usize::from(*new.group))
    {
        *new.group = picked == 1;
    }
    let hint = t!("properties-user-or-group-name");
    let field = dialog::text_field(&mut line, new.name, &hint, false, None, false);
    if field.lost_focus() && line.input(|i| i.key_pressed(egui::Key::Enter)) && ready {
        add = true;
    }
    add
}

/// An `NFSv4` list: its entries, which can be removed here.
fn nfs4_section(ui: &mut egui::Ui, entries: &mut Vec<platform::Nfs4Ace>) {
    egui::Grid::new("nfs4-grid")
        .num_columns(4)
        .spacing([12.0, 2.0])
        .show(ui, |ui| {
            for (index, entry) in entries.iter().enumerate() {
                ui.label(match entry.kind {
                    platform::Nfs4Kind::Allow => "Allow",
                    platform::Nfs4Kind::Deny => "Deny",
                    platform::Nfs4Kind::Audit => "Audit",
                    platform::Nfs4Kind::Alarm => "Alarm",
                });
                ui.label(&entry.who);
                ui.monospace(format!("{:#x}", entry.mask));
                if ui.small_button(t!("remove")).clicked() {
                    // Marked for removal below; the index is kept.
                    ui.data_mut(|d| d.insert_temp(egui::Id::new("nfs4-remove"), index));
                }
                ui.end_row();
            }
        });
    if let Some(index) = ui.data_mut(|d| d.remove_temp::<usize>(egui::Id::new("nfs4-remove")))
        && index < entries.len()
    {
        entries.remove(index);
    }
    ui.weak(t!("properties-nfsv4-entries-are-removed-here"));
}

/// A macOS list: allow and deny entries with their permission words.
fn darwin_section(ui: &mut egui::Ui, entries: &mut Vec<DarwinAce>, new: &mut NewEntry<'_>) {
    let mut remove: Option<usize> = None;
    for (index, entry) in entries.iter_mut().enumerate() {
        ui.horizontal_wrapped(|ui| {
            ui.label(if entry.allow { t!("allow") } else { t!("deny") });
            ui.strong(entry.principal.display());
            ui.weak(if entry.group {
                t!("properties-group")
            } else {
                t!("properties-user")
            });
            for perm in DARWIN_PERMS {
                let mut set = entry.perms.iter().any(|p| p == perm);
                if ui.checkbox(&mut set, *perm).changed() {
                    if set {
                        entry.perms.push((*perm).to_owned());
                    } else {
                        entry.perms.retain(|p| p != perm);
                    }
                }
            }
            if ui.small_button(t!("remove")).clicked() {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        entries.remove(index);
    }
    ui.horizontal(|ui| {
        egui::TextEdit::singleline(new.name)
            .hint_text(t!("properties-user-or-group-name"))
            .desired_width(160.0)
            .show(ui);
        ui.checkbox(new.group, t!("group"));
        ui.checkbox(new.allow, t!("allow"));
        if ui
            .add_enabled(
                !new.name.trim().is_empty(),
                egui::Button::new(t!("add-entry")),
            )
            .clicked()
        {
            let name = new.name.trim().to_owned();
            entries.push(DarwinAce {
                allow: *new.allow,
                group: *new.group,
                principal: Principal::named(name.clone(), name),
                perms: vec!["read".to_owned()],
            });
            new.name.clear();
        }
    });
}

/// An NTFS list: allow and deny entries with their rights, and whether
/// inheritance from the parent is blocked.
fn nt_section(
    ui: &mut egui::Ui,
    entries: &mut Vec<NtAce>,
    protected: &mut bool,
    new: &mut NewEntry<'_>,
) {
    let mut remove: Option<usize> = None;
    for (index, entry) in entries.iter_mut().enumerate() {
        let inherited = entry.inherited();
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled_ui(!inherited, |ui| {
                ui.label(if entry.allow { t!("allow") } else { t!("deny") });
                ui.strong(entry.trustee.display());
                if inherited {
                    ui.weak(t!("properties-inherited"));
                }
                for (label, mask) in NT_RIGHTS {
                    let mut set = entry.mask & mask == *mask;
                    if ui.checkbox(&mut set, *label).changed() {
                        if set {
                            entry.mask |= mask;
                        } else {
                            entry.mask &= !mask;
                        }
                    }
                }
                ui.monospace(format!("{:#x}", entry.mask));
                if ui.small_button(t!("remove")).clicked() {
                    remove = Some(index);
                }
            });
        });
    }
    if let Some(index) = remove {
        entries.remove(index);
    }
    ui.checkbox(
        protected,
        t!("properties-block-inheritance-from-the-parent"),
    );
    ui.horizontal(|ui| {
        egui::TextEdit::singleline(new.name)
            .hint_text(t!("properties-trustee-name"))
            .desired_width(160.0)
            .show(ui);
        ui.checkbox(new.allow, t!("allow"));
        if ui
            .add_enabled(
                !new.name.trim().is_empty(),
                egui::Button::new(t!("add-entry")),
            )
            .clicked()
        {
            let name = new.name.trim().to_owned();
            entries.push(NtAce {
                allow: *new.allow,
                trustee: Principal::named(name.clone(), name),
                mask: NT_RIGHTS[3].1,
                flags: 0,
            });
            new.name.clear();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ace(tag: PosixTag, name: Option<&str>, perms: u8) -> PosixAce {
        PosixAce {
            tag,
            qualifier: name.map(|name| Principal::named(name, name)),
            perms,
        }
    }

    fn base(mode: u32) -> Vec<PosixAce> {
        let mut list = vec![
            ace(PosixTag::UserObj, None, 0),
            ace(PosixTag::GroupObj, None, 0),
            ace(PosixTag::Other, None, 0),
        ];
        mode_into_list(mode, &mut list);
        list
    }

    fn perms_of(list: &[PosixAce], tag: PosixTag) -> Option<u8> {
        list.iter().find(|e| e.tag == tag).map(|e| e.perms)
    }

    #[test]
    fn without_named_entries_the_group_bits_are_the_owning_group_s() {
        let mut list = base(0o750);
        assert_eq!(perms_of(&list, PosixTag::UserObj), Some(7));
        assert_eq!(perms_of(&list, PosixTag::GroupObj), Some(5));
        assert_eq!(perms_of(&list, PosixTag::Other), Some(0));
        assert_eq!(list_into_mode(0o2000, &list), 0o2750, "setgid is kept");
        // The table's Owning group row is the grid's Group row.
        list[1].perms = 4;
        assert_eq!(list_into_mode(0o750, &list), 0o740);
    }

    #[test]
    fn with_named_entries_the_group_bits_are_the_mask() {
        // The frame: 0710, user qemu --x, mask --x, owning group ---.
        let mut list = base(0o700);
        add_named(&mut list, "qemu", false);
        assert_eq!(
            perms_of(&list, PosixTag::Mask),
            Some(4),
            "the mask covers the new entry"
        );
        assert_eq!(list_into_mode(0o700, &list), 0o740);
        // The Access grid's Group row now sets the mask, not the owning
        // group.
        mode_into_list(0o710, &mut list);
        assert_eq!(perms_of(&list, PosixTag::Mask), Some(1));
        assert_eq!(perms_of(&list, PosixTag::GroupObj), Some(0));
        // A second entry widens the mask it finds.
        add_named(&mut list, "video", true);
        assert_eq!(perms_of(&list, PosixTag::Mask), Some(5));
        // The mask goes with the last named entry, and the group bits are
        // the owning group's again.
        let first = list
            .iter()
            .position(named)
            .unwrap_or_else(|| panic!("qemu"));
        remove_entry(&mut list, first);
        assert!(perms_of(&list, PosixTag::Mask).is_some());
        let video = list
            .iter()
            .position(named)
            .unwrap_or_else(|| panic!("video"));
        remove_entry(&mut list, video);
        assert_eq!(perms_of(&list, PosixTag::Mask), None);
        assert_eq!(list_into_mode(0o710, &list), 0o700);
    }

    #[test]
    fn the_summary_names_the_extra_entries() {
        crate::english();
        let mut list = base(0o710);
        let acl = |access: &[PosixAce]| Acl::Posix {
            access: access.to_vec(),
            default: Vec::new(),
        };
        assert_eq!(summary(&acl(&list)), "No extra entries");
        list.push(ace(PosixTag::User, Some("qemu"), 1));
        list.push(ace(PosixTag::Mask, None, 1));
        assert_eq!(summary(&acl(&list)), "1 extra entry: user qemu  --x");
        list.push(ace(PosixTag::Group, Some("video"), 5));
        assert_eq!(
            summary(&acl(&list)),
            "2 extra entries: user qemu  --x, group video  r-x"
        );
        assert_eq!(summary(&Acl::Nfs4(Vec::new())), "0 entries");
    }

    #[test]
    fn the_table_lists_owner_owning_group_named_mask_others() {
        // As the kernel stores it: the named users before the owning group.
        let list = [
            ace(PosixTag::UserObj, None, 7),
            ace(PosixTag::User, Some("qemu"), 1),
            ace(PosixTag::GroupObj, None, 0),
            ace(PosixTag::Group, Some("video"), 4),
            ace(PosixTag::Mask, None, 5),
            ace(PosixTag::Other, None, 0),
        ];
        let tags: Vec<PosixTag> = shown_order(&list)
            .into_iter()
            .map(|index| list[index].tag)
            .collect();
        assert_eq!(
            tags,
            [
                PosixTag::UserObj,
                PosixTag::GroupObj,
                PosixTag::User,
                PosixTag::Group,
                PosixTag::Mask,
                PosixTag::Other,
            ]
        );
    }
}
