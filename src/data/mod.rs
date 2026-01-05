slint::include_modules!();

pub struct FileItem {
    pub name: String,
    pub size: u64
}

impl From<FileItem> for UiFileItem {
    fn from(item: FileItem) -> Self {
        Self {
            name: item.name.into(),
            size: item.size as i32,
        }
    }
}