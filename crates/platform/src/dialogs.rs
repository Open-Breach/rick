//! Native file dialogues through `rfd`: the XDG desktop portal on Linux,
//! `NSOpenPanel` on macOS, `IFileDialog` on Windows.

use std::path::PathBuf;

use crate::traits::DialogFilter;

fn dialog(title: &str, filters: &[DialogFilter]) -> rfd::AsyncFileDialog {
    let mut dialog = rfd::AsyncFileDialog::new().set_title(title);
    for filter in filters {
        dialog = dialog.add_filter(filter.name, filter.extensions);
    }
    dialog
}

pub(crate) async fn pick_folder(title: &str) -> Option<PathBuf> {
    let handle = dialog(title, &[]).pick_folder().await?;
    Some(handle.path().to_path_buf())
}

pub(crate) async fn pick_files(title: &str, filters: &[DialogFilter]) -> Vec<PathBuf> {
    dialog(title, filters)
        .pick_files()
        .await
        .unwrap_or_default()
        .iter()
        .map(|handle| handle.path().to_path_buf())
        .collect()
}

pub(crate) async fn save_file(title: &str, suggested: &str) -> Option<PathBuf> {
    let handle = dialog(title, &[])
        .set_file_name(suggested)
        .save_file()
        .await?;
    Some(handle.path().to_path_buf())
}
