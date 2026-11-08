//! Wavefront OBJ through `tobj`: every face triangulated, the material
//! library read for its diffuse colour and nothing else.

use std::collections::HashMap;
use std::hash::BuildHasher;
use std::io::Cursor;
use std::path::Path;

use crate::mesh::{self, Mesh};

/// What an OBJ file gives.
#[derive(Debug, Clone, PartialEq)]
pub struct Obj {
    /// The triangles, coloured by their material.
    pub mesh: Mesh,
    /// How many objects the file declares (`o` lines; groups when it has
    /// none; one when it has neither).
    pub objects: usize,
}

/// The material libraries the file names on its `mtllib` lines, in order,
/// file names only: a name with a directory part is left out.
#[must_use]
pub fn material_libraries(text: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    for line in text.split(|b| *b == b'\n') {
        let line = line.trim_ascii();
        let Some(rest) = line.strip_prefix(b"mtllib") else {
            continue;
        };
        if !rest.first().is_some_and(u8::is_ascii_whitespace) {
            continue;
        }
        for name in String::from_utf8_lossy(rest).split_whitespace() {
            if !name.contains(['/', '\\']) && name != ".." && !names.iter().any(|n| n == name) {
                names.push(name.to_owned());
            }
        }
    }
    names
}

/// How many objects `text` declares.
#[must_use]
pub fn count_objects(text: &[u8]) -> usize {
    let starts = |prefix: u8| {
        text.split(|b| *b == b'\n')
            .map(<[u8]>::trim_ascii_start)
            .filter(|line| {
                line.first() == Some(&prefix) && line.get(1).is_some_and(u8::is_ascii_whitespace)
            })
            .count()
    };
    match starts(b'o') {
        0 => starts(b'g').max(1),
        n => n,
    }
}

/// Parses `bytes`, the OBJ text, with `materials` the contents of the
/// material libraries it names, by file name.
///
/// # Errors
///
/// `tobj`'s error when the text is not an OBJ file.
pub fn parse<S: BuildHasher>(
    bytes: &[u8],
    materials: &HashMap<String, Vec<u8>, S>,
) -> Result<Obj, tobj::LoadError> {
    let options = tobj::LoadOptions {
        triangulate: true,
        single_index: true,
        ignore_points: true,
        ignore_lines: true,
    };
    let (models, library) =
        tobj::load_obj_buf(&mut Cursor::new(bytes), &options, |path: &Path| {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            match materials.get(name) {
                Some(text) => tobj::load_mtl_buf(&mut Cursor::new(text)),
                None => Err(tobj::LoadError::OpenFileFailed),
            }
        })?;
    let library = library.unwrap_or_default();
    let neutral = mesh::pack(mesh::NEUTRAL);
    let total: usize = models.iter().map(|m| m.mesh.indices.len() / 3).sum();
    let mut mesh = Mesh::with_capacity(total);
    for model in &models {
        let color = model
            .mesh
            .material_id
            .and_then(|id| library.get(id))
            .and_then(|material| material.diffuse)
            .map_or(neutral, |diffuse| mesh::pack(rgba(diffuse)));
        let positions = &model.mesh.positions;
        let corner = |index: u32| -> Option<[f32; 3]> {
            let at = usize::try_from(index).ok()?.checked_mul(3)?;
            Some([
                *positions.get(at)?,
                *positions.get(at + 1)?,
                *positions.get(at + 2)?,
            ])
        };
        for face in model.mesh.indices.as_chunks::<3>().0 {
            if let (Some(a), Some(b), Some(c)) = (corner(face[0]), corner(face[1]), corner(face[2]))
            {
                mesh.push([a, b, c], color);
            }
        }
    }
    Ok(Obj {
        mesh,
        objects: count_objects(bytes),
    })
}

/// A material colour as sRGB bytes.
fn rgba(diffuse: [f32; 3]) -> [u8; 4] {
    // Clamped into 0..=255 before the cast, so it is exact.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    [byte(diffuse[0]), byte(diffuse[1]), byte(diffuse[2]), 0xFF]
}
