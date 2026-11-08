//! 3MF: the model part of the zip, found through the package relationships,
//! read with `roxmltree`; mesh objects, components and the build items
//! with their transforms; the declared unit converted to millimetres; the
//! metadata kept for the File info section.

use std::collections::HashMap;
use std::io::{Cursor, Read as _};

use crate::math::Vec3;
use crate::mesh::{self, Mesh};

/// The largest model part read, uncompressed.
pub const MAX_MODEL_BYTES: u64 = 512 * 1024 * 1024;
/// The relationship type of the model part.
pub const MODEL_RELATIONSHIP: &str = "3dmodel";
/// Components may nest this deep; a cycle stops here.
const MAX_DEPTH: usize = 32;

/// The unit a 3MF declares on its `model` element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// 0.001 mm.
    Micron,
    /// The default.
    Millimeter,
    /// 10 mm.
    Centimeter,
    /// 25.4 mm.
    Inch,
    /// 304.8 mm.
    Foot,
    /// 1000 mm.
    Meter,
}

impl Unit {
    /// The unit named `name` as the format spells it.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "micron" => Self::Micron,
            "millimeter" => Self::Millimeter,
            "centimeter" => Self::Centimeter,
            "inch" => Self::Inch,
            "foot" => Self::Foot,
            "meter" => Self::Meter,
            _ => return None,
        })
    }

    /// Millimetres per unit.
    #[must_use]
    pub fn millimetres(self) -> f32 {
        match self {
            Self::Micron => 0.001,
            Self::Millimeter => 1.0,
            Self::Centimeter => 10.0,
            Self::Inch => 25.4,
            Self::Foot => 304.8,
            Self::Meter => 1000.0,
        }
    }

    /// The name as the format spells it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Micron => "micron",
            Self::Millimeter => "millimeter",
            Self::Centimeter => "centimeter",
            Self::Inch => "inch",
            Self::Foot => "foot",
            Self::Meter => "meter",
        }
    }
}

/// What a 3MF file gives.
#[derive(Debug, Clone, PartialEq)]
pub struct ThreeMf {
    /// The triangles of every build item, in millimetres.
    pub mesh: Mesh,
    /// The unit the file declares.
    pub unit: Unit,
    /// The `metadata` elements, name and value, in order.
    pub metadata: Vec<(String, String)>,
    /// The number of build items.
    pub objects: usize,
}

/// Why a file is not a 3MF this previewer reads.
#[derive(Debug, thiserror::Error)]
pub enum ThreeMfError {
    /// Not a zip.
    #[error("not a zip archive: {0}")]
    Zip(#[from] zip::result::ZipError),
    /// No model part.
    #[error("no 3D model part in the package")]
    NoModel,
    /// The model part is too large.
    #[error("the model part is larger than {MAX_MODEL_BYTES} bytes")]
    TooLarge,
    /// The model part could not be read.
    #[error("reading the model part: {0}")]
    Io(#[from] std::io::Error),
    /// The model part is not UTF-8.
    #[error("the model part is not UTF-8")]
    Encoding,
    /// The model part is not well-formed XML.
    #[error("the model part is not well-formed XML: {0}")]
    Xml(#[from] roxmltree::Error),
    /// The model part has no `model` element.
    #[error("the model part has no model element")]
    NotAModel,
}

/// A row-major affine transform as 3MF writes it: three rows of the
/// linear part, then the translation; a point is a row vector on the left.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Transform([[f32; 3]; 4]);

impl Transform {
    const IDENTITY: Self = Self([
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 0.0],
    ]);

    /// The twelve numbers of a `transform` attribute; the identity when the
    /// attribute is absent or not twelve numbers.
    fn parse(attribute: Option<&str>) -> Self {
        let Some(text) = attribute else {
            return Self::IDENTITY;
        };
        let numbers: Vec<f32> = text
            .split_whitespace()
            .filter_map(|n| n.parse::<f32>().ok())
            .collect();
        if numbers.len() != 12 {
            return Self::IDENTITY;
        }
        let mut rows = [[0.0; 3]; 4];
        for (i, row) in rows.iter_mut().enumerate() {
            row.copy_from_slice(&numbers[i * 3..i * 3 + 3]);
        }
        Self(rows)
    }

    fn apply(&self, p: Vec3) -> Vec3 {
        let mut out = self.0[3];
        for (axis, value) in out.iter_mut().enumerate() {
            *value += p[0] * self.0[0][axis] + p[1] * self.0[1][axis] + p[2] * self.0[2][axis];
        }
        out
    }

    /// The transform that applies `self`, then `outer`.
    fn then(&self, outer: &Self) -> Self {
        let linear = |row: [f32; 3]| -> [f32; 3] {
            let mut out = [0.0; 3];
            for (axis, value) in out.iter_mut().enumerate() {
                *value = row[0] * outer.0[0][axis]
                    + row[1] * outer.0[1][axis]
                    + row[2] * outer.0[2][axis];
            }
            out
        };
        Self([
            linear(self.0[0]),
            linear(self.0[1]),
            linear(self.0[2]),
            outer.apply(self.0[3]),
        ])
    }
}

/// A resource object: triangles of its own, or components placing other
/// objects.
enum Object {
    Mesh {
        vertices: Vec<Vec3>,
        triangles: Vec<[usize; 3]>,
    },
    Components(Vec<(String, Transform)>),
}

/// Parses `bytes`, the whole package.
///
/// # Errors
///
/// [`ThreeMfError`] when the bytes are not a 3MF package with a model part.
pub fn parse(bytes: &[u8]) -> Result<ThreeMf, ThreeMfError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    let part = model_part(&mut archive).ok_or(ThreeMfError::NoModel)?;
    let text = {
        let file = archive.by_name(&part)?;
        if file.size() > MAX_MODEL_BYTES {
            return Err(ThreeMfError::TooLarge);
        }
        let mut buffer = Vec::with_capacity(usize::try_from(file.size()).unwrap_or(0));
        file.take(MAX_MODEL_BYTES).read_to_end(&mut buffer)?;
        String::from_utf8(buffer).map_err(|_| ThreeMfError::Encoding)?
    };
    parse_model(&text)
}

/// The path of the model part inside the package: the `3dmodel`
/// relationship's target, else the first `.model` file.
fn model_part(archive: &mut zip::ZipArchive<Cursor<&[u8]>>) -> Option<String> {
    let from_rels = archive.by_name("_rels/.rels").ok().and_then(|rels| {
        let mut text = String::new();
        rels.take(1024 * 1024).read_to_string(&mut text).ok()?;
        let doc = roxmltree::Document::parse(&text).ok()?;
        doc.descendants()
            .filter(|n| n.has_tag_name("Relationship"))
            .find(|n| {
                n.attribute("Type")
                    .is_some_and(|t| t.ends_with(MODEL_RELATIONSHIP))
            })
            .and_then(|n| n.attribute("Target"))
            .map(|target| target.trim_start_matches('/').to_owned())
    });
    if let Some(part) = from_rels
        && archive.by_name(&part).is_ok()
    {
        return Some(part);
    }
    let names: Vec<String> = archive
        .file_names()
        .filter_map(Result::ok)
        .map(std::borrow::Cow::into_owned)
        .collect();
    names.into_iter().find(|n| {
        std::path::Path::new(n)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("model"))
    })
}

/// Parses the model part's XML.
///
/// # Errors
///
/// [`ThreeMfError`] when the text is not a 3MF model.
pub fn parse_model(text: &str) -> Result<ThreeMf, ThreeMfError> {
    let doc = roxmltree::Document::parse(text)?;
    let model = doc.root_element();
    if !model.has_tag_name("model") {
        return Err(ThreeMfError::NotAModel);
    }
    let unit = model
        .attribute("unit")
        .and_then(Unit::parse)
        .unwrap_or(Unit::Millimeter);
    let metadata = model
        .children()
        .filter(|n| n.has_tag_name("metadata"))
        .filter_map(|n| {
            Some((
                n.attribute("name")?.to_owned(),
                n.text().unwrap_or_default().trim().to_owned(),
            ))
        })
        .collect();
    let mut objects: HashMap<String, Object> = HashMap::new();
    for resources in model.children().filter(|n| n.has_tag_name("resources")) {
        for object in resources.children().filter(|n| n.has_tag_name("object")) {
            let Some(id) = object.attribute("id") else {
                continue;
            };
            if let Some(mesh) = object.children().find(|n| n.has_tag_name("mesh")) {
                objects.insert(id.to_owned(), read_mesh(mesh));
            } else if let Some(components) =
                object.children().find(|n| n.has_tag_name("components"))
            {
                let parts = components
                    .children()
                    .filter(|n| n.has_tag_name("component"))
                    .filter_map(|n| {
                        Some((
                            n.attribute("objectid")?.to_owned(),
                            Transform::parse(n.attribute("transform")),
                        ))
                    })
                    .collect();
                objects.insert(id.to_owned(), Object::Components(parts));
            }
        }
    }
    let color = mesh::pack(mesh::NEUTRAL);
    let mut mesh = Mesh::default();
    let mut items = 0;
    for build in model.children().filter(|n| n.has_tag_name("build")) {
        for item in build.children().filter(|n| n.has_tag_name("item")) {
            let Some(id) = item.attribute("objectid") else {
                continue;
            };
            items += 1;
            place(
                &objects,
                id,
                &Transform::parse(item.attribute("transform")),
                &mut mesh,
                color,
                0,
            );
        }
    }
    mesh.scale(unit.millimetres());
    Ok(ThreeMf {
        mesh,
        unit,
        metadata,
        objects: items,
    })
}

/// The vertices and triangles of a `mesh` element; a triangle naming a
/// vertex the element lacks is dropped.
fn read_mesh(node: roxmltree::Node<'_, '_>) -> Object {
    let number = |n: roxmltree::Node<'_, '_>, name: &str| -> Option<f32> {
        n.attribute(name)?.trim().parse::<f32>().ok()
    };
    let index = |n: roxmltree::Node<'_, '_>, name: &str| -> Option<usize> {
        n.attribute(name)?.trim().parse::<usize>().ok()
    };
    let vertices: Vec<Vec3> = node
        .children()
        .filter(|n| n.has_tag_name("vertices"))
        .flat_map(|v| v.children().filter(|n| n.has_tag_name("vertex")))
        .filter_map(|v| Some([number(v, "x")?, number(v, "y")?, number(v, "z")?]))
        .collect();
    let triangles = node
        .children()
        .filter(|n| n.has_tag_name("triangles"))
        .flat_map(|t| t.children().filter(|n| n.has_tag_name("triangle")))
        .filter_map(|t| Some([index(t, "v1")?, index(t, "v2")?, index(t, "v3")?]))
        .filter(|t| t.iter().all(|i| *i < vertices.len()))
        .collect();
    Object::Mesh {
        vertices,
        triangles,
    }
}

/// Adds the object `id` transformed by `transform` to `mesh`, following
/// components.
fn place(
    objects: &HashMap<String, Object>,
    id: &str,
    transform: &Transform,
    mesh: &mut Mesh,
    color: u32,
    depth: usize,
) {
    if depth > MAX_DEPTH {
        return;
    }
    match objects.get(id) {
        Some(Object::Mesh {
            vertices,
            triangles,
        }) => {
            for triangle in triangles {
                mesh.push(
                    [
                        transform.apply(vertices[triangle[0]]),
                        transform.apply(vertices[triangle[1]]),
                        transform.apply(vertices[triangle[2]]),
                    ],
                    color,
                );
            }
        }
        Some(Object::Components(parts)) => {
            for (part, inner) in parts {
                place(
                    objects,
                    part,
                    &inner.then(transform),
                    mesh,
                    color,
                    depth + 1,
                );
            }
        }
        None => {}
    }
}
