//! Disk-image listings through the archive provider's reader: the 7-Zip
//! library opens the container, the partition table and the file systems
//! inside it (`ext_vfs_archive::diskimage`), and the entries come back as
//! this previewer's [`RawEntry`]s within its [`Limit`]. The library is
//! found through the provider's [`Resolver`], the one setting for both
//! plugins; when it is missing, the previewer opens a [`MissingDoc`], whose
//! content names the library and how to install it, and the pane shows
//! that in place of the listing with *Browse…* and *Retry*.

use std::path::Path;
use std::sync::Arc;

use ext::platform::ImageFormat;
use ext::{
    Content, ControlId, ControlValue, Frame, Icon, InfoSection, PreviewControl, PreviewDoc,
    RenderRequest, Text, TextBlock, TextLayer, Transport,
};
use ext_vfs_archive::{MissingLibrary, Resolver, diskimage};
use types::{EntryKind, Error};

use crate::{Facts, Format, Limit, Listed, RawEntry, format_size};

/// Lists the disk image at `path` of `format`.
pub(crate) fn list(
    resolver: &Resolver,
    path: &Path,
    format: ImageFormat,
    limit: &Limit,
) -> Result<Listed, Error> {
    let mut entries = Vec::new();
    let mut truncated = false;
    let facts = diskimage::visit(resolver, path, format, &mut |found| {
        if limit.reached(entries.len()) {
            truncated = true;
            return Ok(true);
        }
        let kind = match found.item.kind {
            EntryKind::Dir => EntryKind::Dir,
            EntryKind::Symlink { target, broken } => EntryKind::Symlink { target, broken },
            _ => EntryKind::File,
        };
        entries.push(RawEntry {
            path: String::from_utf8_lossy(&found.path).into_owned(),
            size: found.item.size,
            compressed: None,
            modified: found.item.modified,
            encrypted: false,
            kind,
        });
        Ok(false)
    })?;
    let mut rows = vec![("Layout".to_owned(), facts.layout.join(" → "))];
    if !facts.labels.is_empty() {
        rows.push(("Volume label".to_owned(), facts.labels.join(", ")));
    }
    Ok(Listed {
        entries,
        truncated,
        facts: Facts {
            encrypted_headers: false,
            solid: false,
            rows,
        },
    })
}

/// The document shown for a disk image while the 7-Zip library is missing:
/// the file's facts and the message the pane lays out with *Browse…* and
/// *Retry*, with the text as the fallback.
pub struct MissingDoc {
    missing: Arc<ext::MissingLibrary>,
    message: String,
    format: Format,
    name: String,
    size: Option<u64>,
}

impl std::fmt::Debug for MissingDoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MissingDoc")
            .field("name", &self.name)
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

impl MissingDoc {
    /// The document for the image `name` of `format` and `size` bytes,
    /// from what the resolver reported.
    #[must_use]
    pub fn new(missing: &MissingLibrary, format: Format, name: String, size: Option<u64>) -> Self {
        Self {
            missing: Arc::new(missing.for_pane()),
            message: missing.to_string(),
            format,
            name,
            size,
        }
    }

    /// The content the pane lays out.
    #[must_use]
    pub fn missing(&self) -> &Arc<ext::MissingLibrary> {
        &self.missing
    }
}

impl PreviewDoc for MissingDoc {
    fn pages(&self) -> Option<usize> {
        None
    }

    fn render(&mut self, page: usize, _req: RenderRequest) -> Result<Frame, Error> {
        if page != 0 {
            return Err(Error::NotFound);
        }
        Ok(Frame::Text(TextBlock {
            text: self.message.clone(),
            language: None,
            spans: Vec::new(),
        }))
    }

    fn text_layer(&self) -> Option<&TextLayer> {
        None
    }

    fn content(&self) -> Option<Content> {
        Some(Content::MissingLibrary(Arc::clone(&self.missing)))
    }

    fn sections(&self) -> Vec<InfoSection> {
        let mut rows = vec![
            ("Name".to_owned(), self.name.clone()),
            ("Format".to_owned(), self.format.name().to_owned()),
        ];
        if let Some(size) = self.size {
            rows.push(("Size".to_owned(), format_size(size)));
        }
        rows.push((
            "Preview".to_owned(),
            format!("{} not found", self.missing.name),
        ));
        vec![InfoSection {
            id: "file",
            title: Text("preview-archive-file-info"),
            icon: Icon::Info,
            rows,
        }]
    }

    fn status(&self) -> Vec<String> {
        let mut status = vec![self.format.name().to_owned()];
        if let Some(size) = self.size {
            status.push(format_size(size));
        }
        status.push(format!("{} not found", self.missing.name));
        status
    }

    fn controls(&self) -> Vec<PreviewControl> {
        Vec::new()
    }

    fn set_control(&mut self, _id: ControlId, _value: ControlValue) -> Result<(), Error> {
        Err(Error::Unsupported("a missing library has no controls"))
    }

    fn transport(&mut self) -> Option<&mut dyn Transport> {
        None
    }
}
