//! Thumbnails for the icon view: textures by entry and pixel size, filled
//! from [`app::Event::Thumbnail`] and asked for through
//! [`app::Command::Thumbnail`]. Until one arrives, or when nothing can draw
//! it, the cell shows the file-type icon.

use std::collections::{HashMap, HashSet};

use egui::TextureHandle;
use ext::Frame;
use types::EntryId;

/// What the cache knows about one thumbnail.
#[derive(Clone, Copy)]
pub enum Thumb<'a> {
    /// Never asked for.
    Missing,
    /// Asked for, not answered yet.
    Pending,
    /// No previewer could draw it; the icon stays.
    Unavailable,
    /// Ready to draw.
    Ready(&'a TextureHandle),
}

/// The cache.
#[derive(Default)]
pub struct Thumbnails {
    textures: HashMap<(EntryId, u32), Option<TextureHandle>>,
    pending: HashSet<(EntryId, u32)>,
}

/// Textures kept before the cache starts over.
const CAPACITY: usize = 2048;

impl std::fmt::Debug for Thumbnails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Thumbnails")
            .field("textures", &self.textures.len())
            .field("pending", &self.pending.len())
            .finish()
    }
}

impl Thumbnails {
    /// The state of the thumbnail of `id` at `size` pixels.
    #[must_use]
    pub fn get(&self, id: EntryId, size: u32) -> Thumb<'_> {
        match self.textures.get(&(id, size)) {
            Some(Some(texture)) => Thumb::Ready(texture),
            Some(None) => Thumb::Unavailable,
            None if self.pending.contains(&(id, size)) => Thumb::Pending,
            None => Thumb::Missing,
        }
    }

    /// Records that a render was asked for.
    pub fn requested(&mut self, id: EntryId, size: u32) {
        self.pending.insert((id, size));
    }

    /// How many are ready.
    #[must_use]
    pub fn ready(&self) -> usize {
        self.textures.values().filter(|t| t.is_some()).count()
    }

    /// Takes a finished render: uploads the frame as a texture, or records
    /// that nothing draws this entry.
    pub fn arrived(&mut self, ctx: &egui::Context, id: EntryId, size: u32, frame: Option<Frame>) {
        self.pending.remove(&(id, size));
        if self.textures.len() >= CAPACITY {
            self.textures.clear();
        }
        let texture = match frame {
            Some(Frame::Rgba {
                width,
                height,
                data,
            }) if width > 0 && height > 0 => {
                let (w, h) = (width as usize, height as usize);
                if data.len() < w * h * 4 {
                    None
                } else {
                    let image =
                        egui::ColorImage::from_rgba_unmultiplied([w, h], &data[..w * h * 4]);
                    Some(ctx.load_texture(
                        format!("thumbnail-{size}"),
                        image,
                        egui::TextureOptions::LINEAR,
                    ))
                }
            }
            _ => None,
        };
        self.textures.insert((id, size), texture);
    }
}
