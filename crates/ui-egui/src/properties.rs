//! The Properties dialogue: the general rows of an entry and, for a local
//! one, its owner, group, mode bits and access control list, edited in
//! place and applied as one job (`Command::SetPermissions`), on the
//! enclosed entries too when the folder's *Apply to enclosed items* is
//! ticked, with files and folders chosen separately. The dialogue follows
//! the entry: after a change the application sends the new state and the
//! controls reset to it. Where the platform has a dialogue of its own
//! (Windows, macOS), *System properties…* opens it.

use app::{Command, PermissionRequest, PropertiesInfo, Recursion};
use platform::{Acl, DarwinAce, NtAce, PermissionChange, PosixAce, PosixTag, Principal};

use crate::shell::Request;

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

/// The dialogue's state between frames.
#[derive(Debug, Default)]
pub struct PropertiesDialog {
    /// What the application sent; `None` while closed.
    info: Option<PropertiesInfo>,
    /// The controls' values.
    edit: Edit,
    /// A change was sent and its answer is awaited.
    pub applying: bool,
}

/// The editable state, as the controls hold it.
// The flags are the dialogue's checkboxes, one field each.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Edit {
    owner: Option<Principal>,
    group: Option<Principal>,
    /// A name typed for the owner or group instead of one picked.
    owner_text: String,
    group_text: String,
    mode: u32,
    /// The mode as typed in the octal field.
    octal: String,
    acl: Option<Acl>,
    recursive: bool,
    files: bool,
    folders: bool,
    /// The name of a new ACL entry being added.
    new_name: String,
    new_group: bool,
    new_allow: bool,
}

impl PropertiesDialog {
    /// Shows `info`, or refreshes the dialogue with the entry's new state.
    pub fn open(&mut self, info: PropertiesInfo) {
        self.edit = Edit::from(&info);
        self.info = Some(info);
        self.applying = false;
    }

    /// Whether the dialogue is on screen.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.info.is_some()
    }

    /// The entry shown, if any.
    #[must_use]
    pub fn info(&self) -> Option<&PropertiesInfo> {
        self.info.as_ref()
    }

    /// The change the controls stand for right now, against the entry's
    /// state; `None` when nothing changed.
    #[must_use]
    pub fn pending_change(&self) -> Option<PermissionChange> {
        self.info
            .as_ref()
            .and_then(|info| pending(info, &self.edit))
    }

    /// Draws the dialogue, when open.
    pub fn show(&mut self, ctx: &egui::Context, requests: &mut Vec<Request>) {
        let Self {
            info: shown,
            edit,
            applying,
        } = self;
        let Some(info) = shown.as_ref() else {
            return;
        };
        let mut open = true;
        let mut apply = false;
        let mut revert = false;
        let mut system = false;
        egui::Window::new(format!("Properties of {}", info.entry.name))
            .id(egui::Id::new("properties"))
            .open(&mut open)
            .default_width(460.0)
            .resizable(true)
            .show(ctx, |ui| {
                egui::Grid::new("properties-grid")
                    .num_columns(2)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        for (key, value) in &info.rows {
                            ui.weak(key);
                            ui.label(value);
                            ui.end_row();
                        }
                    });
                if let Some(current) = &info.permissions {
                    ui.separator();
                    ui.heading("Permissions");
                    permissions_section(ui, info, current, edit);
                    ui.add_space(8.0);
                    let dirty = pending(info, edit).is_some();
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(dirty && !*applying, egui::Button::new("Apply"))
                            .clicked()
                        {
                            apply = true;
                        }
                        if ui
                            .add_enabled(dirty && !*applying, egui::Button::new("Revert"))
                            .clicked()
                        {
                            revert = true;
                        }
                        if *applying {
                            ui.spinner();
                            ui.weak("Applying…");
                        }
                    });
                }
                if info.native {
                    ui.add_space(4.0);
                    if ui.button("System properties…").clicked() {
                        system = true;
                    }
                }
            });
        if system {
            requests.push(Request::Command(Command::SystemProperties(info.pane)));
        }
        if revert {
            *edit = Edit::from(info);
        }
        if apply
            && let Some(change) = pending(info, edit)
            && let Some(path) = info.path.clone()
        {
            let recursive =
                (edit.recursive && info.entry.kind == types::EntryKind::Dir).then_some(Recursion {
                    files: edit.files,
                    folders: edit.folders,
                });
            requests.push(Request::Command(Command::SetPermissions {
                pane: info.pane,
                request: PermissionRequest {
                    targets: vec![path],
                    change,
                    recursive,
                },
            }));
            *applying = true;
        }
        if !open {
            *shown = None;
            requests.push(Request::Command(Command::PropertiesClosed));
        }
    }
}

/// The difference between the controls and the entry's state.
fn pending(info: &PropertiesInfo, edit: &Edit) -> Option<PermissionChange> {
    let current = info.permissions.as_ref()?;
    let change = PermissionChange {
        owner: edit
            .owner
            .clone()
            .filter(|o| o.id != current.owner.id && o.id != current.owner.display()),
        group: edit
            .group
            .clone()
            .filter(|g| g.id != current.group.id && g.id != current.group.display()),
        mode: current.mode.filter(|m| *m != edit.mode).map(|_| edit.mode),
        acl: edit.acl.clone().filter(|a| Some(a) != current.acl.as_ref()),
    };
    (!change.is_empty()).then_some(change)
}

impl From<&PropertiesInfo> for Edit {
    fn from(info: &PropertiesInfo) -> Self {
        let Some(current) = &info.permissions else {
            return Self::default();
        };
        Self {
            owner: Some(current.owner.clone()),
            group: Some(current.group.clone()),
            owner_text: String::new(),
            group_text: String::new(),
            mode: current.mode.unwrap_or(0),
            octal: format!("{:04o}", current.mode.unwrap_or(0)),
            acl: current.acl.clone(),
            recursive: false,
            files: true,
            folders: true,
            new_name: String::new(),
            new_group: false,
            new_allow: true,
        }
    }
}

/// The owner and group rows, the mode grid, the ACL and the recursion.
fn permissions_section(
    ui: &mut egui::Ui,
    info: &PropertiesInfo,
    current: &platform::FilePermissions,
    edit: &mut Edit,
) {
    egui::Grid::new("permissions-grid")
        .num_columns(2)
        .spacing([16.0, 6.0])
        .show(ui, |ui| {
            ui.weak("Owner");
            principal_picker(
                ui,
                "owner",
                &info.principals.users,
                &mut edit.owner,
                &mut edit.owner_text,
            );
            ui.end_row();
            ui.weak("Group");
            principal_picker(
                ui,
                "group",
                &info.principals.groups,
                &mut edit.group,
                &mut edit.group_text,
            );
            ui.end_row();
        });
    if current.mode.is_some() {
        ui.add_space(4.0);
        mode_grid(ui, edit);
    }
    if let Some(acl) = &mut edit.acl {
        ui.add_space(4.0);
        ui.strong(acl.flavour());
        acl_section(
            ui,
            acl,
            &mut edit_names(&mut edit.new_name, &mut edit.new_group, &mut edit.new_allow),
        );
    }
    if info.entry.kind == types::EntryKind::Dir {
        ui.add_space(4.0);
        ui.checkbox(&mut edit.recursive, "Apply to enclosed items");
        if edit.recursive {
            ui.indent("recursion", |ui| {
                ui.checkbox(&mut edit.files, "Files");
                ui.checkbox(&mut edit.folders, "Folders");
            });
        }
    }
}

/// The three fields a new ACL entry is typed into.
struct NewEntry<'a> {
    name: &'a mut String,
    group: &'a mut bool,
    allow: &'a mut bool,
}

fn edit_names<'a>(name: &'a mut String, group: &'a mut bool, allow: &'a mut bool) -> NewEntry<'a> {
    NewEntry { name, group, allow }
}

/// A combo box over the known principals, with a field to type another
/// name or id.
fn principal_picker(
    ui: &mut egui::Ui,
    id: &str,
    known: &[Principal],
    chosen: &mut Option<Principal>,
    typed: &mut String,
) {
    ui.horizontal(|ui| {
        let selected = chosen
            .as_ref()
            .map_or_else(|| "—".to_owned(), |p| p.display().to_owned());
        egui::ComboBox::from_id_salt(("principal", id))
            .selected_text(selected)
            .width(180.0)
            .show_ui(ui, |ui| {
                for principal in known {
                    let is = chosen.as_ref().is_some_and(|c| c.id == principal.id);
                    if ui.selectable_label(is, principal.display()).clicked() {
                        *chosen = Some(principal.clone());
                    }
                }
            });
        let field = egui::TextEdit::singleline(typed)
            .hint_text("Other…")
            .desired_width(120.0)
            .show(ui);
        if field.response.lost_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter))
            && !typed.trim().is_empty()
        {
            // A name the platform resolves when the change is applied; a
            // known one is taken from the list for its id.
            let text = typed.trim().to_owned();
            *chosen = Some(
                known
                    .iter()
                    .find(|p| p.display() == text || p.id == text)
                    .cloned()
                    .unwrap_or_else(|| Principal::named(text.clone(), text.clone())),
            );
            typed.clear();
        }
    });
}

/// The `rwx` checkboxes for user, group and other, the special bits and
/// the octal field, all one value.
fn mode_grid(ui: &mut egui::Ui, edit: &mut Edit) {
    let mut mode = edit.mode;
    egui::Grid::new("mode-grid")
        .num_columns(4)
        .spacing([16.0, 2.0])
        .show(ui, |ui| {
            ui.weak("");
            ui.weak("Read");
            ui.weak("Write");
            ui.weak("Execute");
            ui.end_row();
            for (label, shift) in [("User", 6u32), ("Group", 3), ("Other", 0)] {
                ui.label(label);
                for bit in [4u32, 2, 1] {
                    let mask = bit << shift;
                    let mut set = mode & mask != 0;
                    if ui.checkbox(&mut set, "").changed() {
                        if set {
                            mode |= mask;
                        } else {
                            mode &= !mask;
                        }
                    }
                }
                ui.end_row();
            }
        });
    ui.horizontal(|ui| {
        for (label, mask) in [
            ("Set user ID", 0o4000u32),
            ("Set group ID", 0o2000),
            ("Sticky", 0o1000),
        ] {
            let mut set = mode & mask != 0;
            if ui.checkbox(&mut set, label).changed() {
                if set {
                    mode |= mask;
                } else {
                    mode &= !mask;
                }
            }
        }
    });
    ui.horizontal(|ui| {
        ui.weak("Octal");
        if mode != edit.mode {
            edit.octal = format!("{mode:04o}");
        }
        let field = egui::TextEdit::singleline(&mut edit.octal)
            .desired_width(60.0)
            .show(ui);
        if field.response.changed()
            && let Ok(typed) = u32::from_str_radix(edit.octal.trim(), 8)
            && typed <= 0o7777
        {
            mode = typed;
        }
    });
    edit.mode = mode;
}

/// The ACL in its flavour: the entries with their controls, and the row
/// that adds one.
fn acl_section(ui: &mut egui::Ui, acl: &mut Acl, new: &mut NewEntry<'_>) {
    match acl {
        Acl::Posix { access, .. } => posix_section(ui, access, new),
        Acl::Nfs4(entries) => {
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
                        if ui.small_button("Remove").clicked() {
                            // Marked for removal below; the index is kept.
                            ui.data_mut(|d| d.insert_temp(egui::Id::new("nfs4-remove"), index));
                        }
                        ui.end_row();
                    }
                });
            if let Some(index) =
                ui.data_mut(|d| d.remove_temp::<usize>(egui::Id::new("nfs4-remove")))
                && index < entries.len()
            {
                entries.remove(index);
            }
            ui.weak("NFSv4 entries are removed here and added with the server's tools.");
        }
        Acl::Darwin(entries) => darwin_section(ui, entries, new),
        Acl::Nt { entries, protected } => nt_section(ui, entries, protected, new),
    }
}

fn posix_section(ui: &mut egui::Ui, access: &mut Vec<PosixAce>, new: &mut NewEntry<'_>) {
    let mut remove: Option<usize> = None;
    egui::Grid::new("posix-grid")
        .num_columns(5)
        .spacing([12.0, 2.0])
        .show(ui, |ui| {
            for (index, entry) in access.iter_mut().enumerate() {
                let (tag, who) = match entry.tag {
                    PosixTag::UserObj => ("Owner", String::new()),
                    PosixTag::User => (
                        "User",
                        entry
                            .qualifier
                            .as_ref()
                            .map_or_else(String::new, |p| p.display().to_owned()),
                    ),
                    PosixTag::GroupObj => ("Owning group", String::new()),
                    PosixTag::Group => (
                        "Group",
                        entry
                            .qualifier
                            .as_ref()
                            .map_or_else(String::new, |p| p.display().to_owned()),
                    ),
                    PosixTag::Mask => ("Mask", String::new()),
                    PosixTag::Other => ("Others", String::new()),
                };
                ui.label(tag);
                ui.label(who);
                ui.horizontal(|ui| {
                    for (label, bit) in [("r", 4u8), ("w", 2), ("x", 1)] {
                        let mut set = entry.perms & bit != 0;
                        if ui.checkbox(&mut set, label).changed() {
                            if set {
                                entry.perms |= bit;
                            } else {
                                entry.perms &= !bit;
                            }
                        }
                    }
                });
                let named = matches!(entry.tag, PosixTag::User | PosixTag::Group);
                if named && ui.small_button("Remove").clicked() {
                    remove = Some(index);
                }
                ui.end_row();
            }
        });
    if let Some(index) = remove {
        access.remove(index);
        // The mask goes with the last named entry.
        if !access
            .iter()
            .any(|e| matches!(e.tag, PosixTag::User | PosixTag::Group))
        {
            access.retain(|e| e.tag != PosixTag::Mask);
        }
    }
    ui.horizontal(|ui| {
        egui::TextEdit::singleline(new.name)
            .hint_text("User or group name")
            .desired_width(160.0)
            .show(ui);
        ui.checkbox(new.group, "Group");
        if ui
            .add_enabled(!new.name.trim().is_empty(), egui::Button::new("Add entry"))
            .clicked()
        {
            let name = new.name.trim().to_owned();
            access.push(PosixAce {
                tag: if *new.group {
                    PosixTag::Group
                } else {
                    PosixTag::User
                },
                qualifier: Some(Principal::named(name.clone(), name)),
                perms: 4,
            });
            if !access.iter().any(|e| e.tag == PosixTag::Mask) {
                access.push(PosixAce {
                    tag: PosixTag::Mask,
                    qualifier: None,
                    perms: 7,
                });
            }
            new.name.clear();
        }
    });
}

fn darwin_section(ui: &mut egui::Ui, entries: &mut Vec<DarwinAce>, new: &mut NewEntry<'_>) {
    let mut remove: Option<usize> = None;
    for (index, entry) in entries.iter_mut().enumerate() {
        ui.horizontal_wrapped(|ui| {
            ui.label(if entry.allow { "Allow" } else { "Deny" });
            ui.strong(entry.principal.display());
            ui.weak(if entry.group { "(group)" } else { "(user)" });
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
            if ui.small_button("Remove").clicked() {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        entries.remove(index);
    }
    ui.horizontal(|ui| {
        egui::TextEdit::singleline(new.name)
            .hint_text("User or group name")
            .desired_width(160.0)
            .show(ui);
        ui.checkbox(new.group, "Group");
        ui.checkbox(new.allow, "Allow");
        if ui
            .add_enabled(!new.name.trim().is_empty(), egui::Button::new("Add entry"))
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
                ui.label(if entry.allow { "Allow" } else { "Deny" });
                ui.strong(entry.trustee.display());
                if inherited {
                    ui.weak("(inherited)");
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
                if ui.small_button("Remove").clicked() {
                    remove = Some(index);
                }
            });
        });
    }
    if let Some(index) = remove {
        entries.remove(index);
    }
    ui.checkbox(protected, "Block inheritance from the parent folder");
    ui.horizontal(|ui| {
        egui::TextEdit::singleline(new.name)
            .hint_text("Trustee name")
            .desired_width(160.0)
            .show(ui);
        ui.checkbox(new.allow, "Allow");
        if ui
            .add_enabled(!new.name.trim().is_empty(), egui::Button::new("Add entry"))
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
    use model::PaneId;
    use types::{Attrs, Entry, EntryId, EntryKind, ProviderId, VPath};

    fn info(mode: u32) -> PropertiesInfo {
        let entry = Entry {
            id: EntryId::new(ProviderId("local"), 1),
            name: "f".to_owned(),
            path: VPath::from("/tmp/f"),
            kind: EntryKind::File,
            size: Some(1),
            created: None,
            modified: None,
            accessed: None,
            attrs: Attrs::empty(),
            mime: None,
        };
        PropertiesInfo {
            pane: PaneId(1),
            entry,
            path: Some(std::path::PathBuf::from("/tmp/f")),
            rows: Vec::new(),
            permissions: Some(platform::FilePermissions {
                owner: Principal::named("1000", "me"),
                group: Principal::named("1000", "me"),
                mode: Some(mode),
                acl: None,
                is_dir: false,
            }),
            principals: platform::Principals::default(),
            native: false,
        }
    }

    #[test]
    fn the_pending_change_is_the_difference_to_the_entry() {
        let mut dialog = PropertiesDialog::default();
        assert!(dialog.pending_change().is_none());
        dialog.open(info(0o644));
        assert!(dialog.pending_change().is_none(), "nothing edited yet");
        dialog.edit.mode = 0o600;
        let change = dialog
            .pending_change()
            .unwrap_or_else(|| panic!("a change"));
        assert_eq!(change.mode, Some(0o600));
        assert!(change.owner.is_none() && change.group.is_none() && change.acl.is_none());
        dialog.edit.owner = Some(Principal::named("0", "root"));
        let change = dialog
            .pending_change()
            .unwrap_or_else(|| panic!("a change"));
        assert_eq!(change.owner.as_ref().map(|p| p.id.as_str()), Some("0"));
        // Opening again resets the controls.
        dialog.open(info(0o600));
        assert!(dialog.pending_change().is_none());
    }
}
