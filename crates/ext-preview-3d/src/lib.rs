//! 3D model previews: STL, OBJ and 3MF meshes rendered through wgpu.
//!
//! `open` reads the file through its provider and parses it off the UI
//! thread ([`stl`], [`obj`], [`threemf`]) into a [`Mesh`] in millimetres;
//! a model over the triangle budget ([`settings`]) is decimated for the
//! preview and the status bar says so. `render` draws the mesh through
//! wgpu ([`render`]) into an off-screen target the size of the request, a
//! shaded solid in a neutral material over a ground grid on the theme's
//! background, and returns the pixels as [`Frame::Rgba`] for the host to
//! upload; the plugin never touches egui. The document claims three
//! pointer gestures as [`PreviewControl::Pointer`] controls, orbit by drag
//! ([`ORBIT`]), zoom by wheel ([`ZOOM`]) and reset by double-click
//! ([`RESET`]), which the pane forwards through `set_control`; the
//! [`Camera`] lives in the document, so the view is kept per file while it
//! is open. Without a graphics adapter `render` returns the facts as text
//! and says that rendering is unavailable.

#![deny(unsafe_code)]

pub mod camera;
pub mod fixtures;
pub mod math;
pub mod mesh;
pub mod obj;
pub mod render;
pub mod settings;
pub mod stl;
#[cfg(test)]
mod tests;
pub mod threemf;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::hash::BuildHasher;
use std::path::PathBuf;
use std::sync::Arc;

use ext::Text;
use ext::{
    API_VERSION, Capabilities, ControlId, ControlValue, Extension, Frame, Gesture, Icon,
    InfoSection, Manifest, Platforms, Points, PreviewControl, PreviewDoc, Previewer, Provider,
    Registry, RenderRequest, TextBlock, TextLayer, Transport, Version, async_trait,
};
use types::{CancelToken, Entry, EntryKind, Error, ExtId, Location};

pub use camera::Camera;
pub use mesh::{Aabb, Mesh};
pub use render::{GpuError, Grid, Scene};
pub use threemf::Unit;

/// The extension id.
pub const ID: ExtId = ExtId("preview-3d");
/// The drag that orbits the view.
pub const ORBIT: ControlId = ControlId("3d.orbit");
/// The wheel that zooms the view.
pub const ZOOM: ControlId = ControlId("3d.zoom");
/// The double-click that resets the view.
pub const RESET: ControlId = ControlId("3d.reset");
/// The longest side a frame is rendered at; a larger request is scaled
/// down to it, keeping its aspect.
pub const MAX_SIDE: u32 = 4096;
/// Files above this are not opened; the preview says so instead.
pub const MAX_BYTES: usize = 1024 * 1024 * 1024;
/// A material library above this is left unread.
pub const MATERIALS_LIMIT: usize = 16 * 1024 * 1024;
/// How much of a file the sniff covers.
const SNIFF: usize = 4096;

/// The extension: registers [`Previewer3d`] and the triangle budget
/// setting.
#[derive(Debug)]
pub struct Preview3d {
    manifest: Manifest,
}

impl Default for Preview3d {
    fn default() -> Self {
        Self::new()
    }
}

impl Preview3d {
    /// The extension with its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "3D model preview",
                version: Version::parse(env!("CARGO_PKG_VERSION"))
                    .unwrap_or_else(|_| Version::new(0, 0, 0)),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::PREVIEWER,
                platforms: Platforms::ALL,
            },
        }
    }
}

impl Extension for Preview3d {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.translations(&[("en", include_str!("../locales/en.ftl"))]);
        let config_dir = reg.platform().map(|platform| platform.config_dir());
        reg.previewer(Arc::new(Previewer3d::new(config_dir)));
        reg.settings(settings::schema());
        Ok(())
    }
}

ext::plugin!(|| Arc::new(Preview3d::new()) as Arc<dyn Extension>);

/// The three formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// `solid … endsolid`.
    StlAscii,
    /// A header, a count, 50 bytes a triangle.
    StlBinary,
    /// Wavefront OBJ.
    Obj,
    /// 3D Manufacturing Format: a zip with a model part.
    ThreeMf,
}

impl Format {
    /// The name shown in the status bar and the File info section.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::StlAscii => "STL (ASCII)",
            Self::StlBinary => "STL (binary)",
            Self::Obj => "OBJ",
            Self::ThreeMf => "3MF",
        }
    }
}

/// The lower-case extension of `name`, without the dot.
fn extension(name: &str) -> Option<String> {
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty() && !ext.is_empty()).then(|| ext.to_ascii_lowercase())
}

/// Whether the sniff reads as text: no NUL and mostly printable.
fn textual(sniff: &[u8]) -> bool {
    !sniff.contains(&0)
        && sniff
            .iter()
            .filter(|b| b.is_ascii_control() && !b.is_ascii_whitespace())
            .count()
            * 20
            < sniff.len().max(1)
}

/// The format of a file called `name` whose first bytes are `sniff` and
/// whose size is `size`, with how sure the claim is: 200 for a name and
/// content that agree, 150 for a name alone, 120 for content alone, 60 for
/// a name the content contradicts; `None` for a file this previewer does
/// not render.
#[must_use]
pub fn detect(name: &str, sniff: &[u8], size: Option<u64>) -> Option<(Format, u8)> {
    let size = size.unwrap_or(0);
    let stl = stl::detect(sniff, size);
    match extension(name).as_deref() {
        Some("stl") => Some(match stl {
            Some(stl::Kind::Ascii) => (Format::StlAscii, 200),
            Some(stl::Kind::Binary) => (Format::StlBinary, 200),
            None if sniff.is_empty() => (Format::StlBinary, 150),
            None if sniff.trim_ascii_start().starts_with(b"solid") => (Format::StlAscii, 150),
            None => (Format::StlBinary, 150),
        }),
        Some("obj") => Some((Format::Obj, if textual(sniff) { 200 } else { 60 })),
        Some("3mf") => Some((
            Format::ThreeMf,
            if sniff.starts_with(b"PK\x03\x04") {
                200
            } else if sniff.is_empty() {
                150
            } else {
                60
            },
        )),
        _ => match stl {
            Some(stl::Kind::Binary) => Some((Format::StlBinary, 120)),
            Some(stl::Kind::Ascii) if sniff.windows(12).any(|w| w == b"facet normal") => {
                Some((Format::StlAscii, 120))
            }
            _ => None,
        },
    }
}

/// Why a file could not be parsed.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    /// Not an STL.
    #[error("not an STL file: {0}")]
    Stl(#[source] std::io::Error),
    /// Not an OBJ.
    #[error("not an OBJ file: {0}")]
    Obj(tobj::LoadError),
    /// Not a 3MF.
    #[error("not a 3MF file: {0}")]
    ThreeMf(#[from] threemf::ThreeMfError),
}

/// A parsed model, before the budget.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    /// The format it came in.
    pub format: Format,
    /// The triangles, in millimetres.
    pub mesh: Mesh,
    /// The number of objects, for OBJ (and the build items of a 3MF).
    pub objects: Option<usize>,
    /// The unit a 3MF declared.
    pub unit: Option<Unit>,
    /// A 3MF's metadata, name and value.
    pub metadata: Vec<(String, String)>,
}

impl Model {
    /// Parses `bytes` as `format`; `materials` are the OBJ's material
    /// libraries by file name.
    ///
    /// # Errors
    ///
    /// [`ParseError`] when the bytes are not a file of that format.
    pub fn parse<S: BuildHasher>(
        format: Format,
        bytes: Vec<u8>,
        materials: &HashMap<String, Vec<u8>, S>,
    ) -> Result<Self, ParseError> {
        match format {
            Format::StlAscii | Format::StlBinary => {
                let (mesh, kind) = stl::parse(bytes).map_err(ParseError::Stl)?;
                Ok(Self {
                    format: match kind {
                        stl::Kind::Ascii => Format::StlAscii,
                        stl::Kind::Binary => Format::StlBinary,
                    },
                    mesh,
                    objects: None,
                    unit: None,
                    metadata: Vec::new(),
                })
            }
            Format::Obj => {
                let obj = obj::parse(&bytes, materials).map_err(ParseError::Obj)?;
                Ok(Self {
                    format,
                    mesh: obj.mesh,
                    objects: Some(obj.objects),
                    unit: None,
                    metadata: Vec::new(),
                })
            }
            Format::ThreeMf => {
                let model = threemf::parse(&bytes)?;
                Ok(Self {
                    format,
                    mesh: model.mesh,
                    objects: Some(model.objects),
                    unit: Some(model.unit),
                    metadata: model.metadata,
                })
            }
        }
    }
}

/// Previews 3D models.
#[derive(Debug, Default)]
pub struct Previewer3d {
    /// Where `preview-3d.toml` is read from, when the host lent the
    /// platform.
    config_dir: Option<PathBuf>,
    /// A budget fixed for the tests, over the settings file.
    budget: Option<usize>,
}

impl Previewer3d {
    /// A previewer reading its triangle budget from `config_dir`.
    #[must_use]
    pub fn new(config_dir: Option<PathBuf>) -> Self {
        Self {
            config_dir,
            budget: None,
        }
    }

    /// A previewer with a fixed triangle budget.
    #[must_use]
    pub fn with_budget(budget: usize) -> Self {
        Self {
            config_dir: None,
            budget: Some(budget),
        }
    }

    /// The budget in force for the next file.
    #[must_use]
    pub fn triangle_budget(&self) -> usize {
        self.budget
            .unwrap_or_else(|| settings::triangle_budget(self.config_dir.as_deref()))
    }
}

/// A file name an OBJ may name as a material library next to itself: no
/// directory part.
fn sibling_name(name: &str) -> Option<&str> {
    (!name.is_empty() && !name.contains(['/', '\\']) && name != "..").then_some(name)
}

/// Reads the material libraries `text`, an OBJ, names from the folder of
/// `loc`; one that is missing or too large is left out.
async fn materials_of(
    text: &[u8],
    loc: &Location,
    provider: &dyn Provider,
    cancel: &CancelToken,
) -> Result<HashMap<String, Vec<u8>>, Error> {
    let mut out = HashMap::new();
    let Some(folder) = loc.path.parent() else {
        return Ok(out);
    };
    for name in obj::material_libraries(text) {
        let Some(name) = sibling_name(&name) else {
            continue;
        };
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let target = Location::new(loc.provider, folder.join(name.as_bytes()));
        match ext::read_bytes(provider, &target, Some(MATERIALS_LIMIT + 1), cancel).await {
            Ok(bytes) if bytes.len() <= MATERIALS_LIMIT => {
                out.insert(name.to_owned(), bytes);
            }
            Ok(_) => tracing::debug!(name, "material library larger than the limit"),
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(error) => tracing::debug!(name, %error, "material library not read"),
        }
    }
    Ok(out)
}

#[async_trait]
impl Previewer for Previewer3d {
    fn id(&self) -> ExtId {
        ID
    }

    fn score(&self, entry: &Entry, sniff: &[u8]) -> u8 {
        if entry.kind != EntryKind::File {
            return 0;
        }
        detect(&entry.name, sniff, entry.size).map_or(0, |(_, score)| score)
    }

    async fn open(
        &self,
        loc: &Location,
        provider: Arc<dyn Provider>,
        cancel: CancelToken,
    ) -> Result<Box<dyn PreviewDoc>, Error> {
        let entry = provider.stat(loc).await?;
        let bytes = ext::read_bytes(&*provider, loc, Some(MAX_BYTES + 1), &cancel).await?;
        if bytes.len() > MAX_BYTES {
            return Err(Error::Unsupported("3D model larger than 1 GiB"));
        }
        let sniff = &bytes[..bytes.len().min(SNIFF)];
        let (format, _) = detect(&entry.name, sniff, Some(bytes.len() as u64))
            .ok_or(Error::Unsupported("not a 3D model this previewer renders"))?;
        let materials = if format == Format::Obj {
            materials_of(&bytes, loc, &*provider, &cancel).await?
        } else {
            HashMap::new()
        };
        let budget = self.triangle_budget();
        let size = entry.size;
        let doc = ext::blocking(move || {
            let model = Model::parse(format, bytes, &materials).map_err(|e| Error::Extension {
                ext: ID,
                source: Box::new(e),
            })?;
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            Ok(Doc3d::new(model, budget, size))
        })
        .await?;
        Ok(Box::new(doc))
    }
}

/// An opened model: its facts, the mesh shown (decimated when over the
/// budget), the camera and, once a frame was rendered, the scene on the
/// GPU.
pub struct Doc3d {
    format: Format,
    /// The file's triangle count.
    triangles: usize,
    bounds: Aabb,
    objects: Option<usize>,
    unit: Option<Unit>,
    metadata: Vec<(String, String)>,
    size: Option<u64>,
    budget: usize,
    /// The triangle count shown when the file was decimated.
    decimated: Option<usize>,
    /// The mesh to upload, until the scene holds it.
    mesh: Option<Mesh>,
    grid: Grid,
    camera: Camera,
    scene: Option<Scene>,
    /// Why the last frame could not be rendered.
    failure: Option<String>,
}

impl std::fmt::Debug for Doc3d {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Doc3d")
            .field("format", &self.format)
            .field("triangles", &self.triangles)
            .field("decimated", &self.decimated)
            .field("camera", &self.camera)
            .field("scene", &self.scene.is_some())
            .finish_non_exhaustive()
    }
}

impl Doc3d {
    /// The document of `model` under the triangle `budget`, for a file of
    /// `size` bytes.
    #[must_use]
    pub fn new(model: Model, budget: usize, size: Option<u64>) -> Self {
        let triangles = model.mesh.triangles();
        let bounds = model.mesh.bounds;
        let (mesh, decimated) = match model.mesh.decimate(budget) {
            Some(decimated) => {
                let shown = decimated.triangles();
                (decimated, Some(shown))
            }
            None => (model.mesh, None),
        };
        Self {
            format: model.format,
            triangles,
            bounds,
            objects: model.objects,
            unit: model.unit,
            metadata: model.metadata,
            size,
            budget,
            decimated,
            grid: Grid::under(&bounds),
            mesh: Some(mesh),
            camera: Camera::default(),
            scene: None,
            failure: None,
        }
    }

    /// The format.
    #[must_use]
    pub fn format(&self) -> Format {
        self.format
    }

    /// The file's triangle count.
    #[must_use]
    pub fn triangles(&self) -> usize {
        self.triangles
    }

    /// The bounding box, in millimetres.
    #[must_use]
    pub fn bounds(&self) -> Aabb {
        self.bounds
    }

    /// The triangle count of the decimated mesh shown, when the file was
    /// over the budget.
    #[must_use]
    pub fn decimated(&self) -> Option<usize> {
        self.decimated
    }

    /// The view.
    #[must_use]
    pub fn camera(&self) -> Camera {
        self.camera
    }

    /// Why the last frame could not be rendered, if it could not.
    #[must_use]
    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    /// The bounding box as `w × d × h mm`.
    #[must_use]
    pub fn dimensions(&self) -> String {
        let size = self.bounds.size();
        format!(
            "{} × {} × {} mm",
            millimetres(size[0]),
            millimetres(size[1]),
            millimetres(size[2])
        )
    }

    /// The frame, or why there is none.
    fn frame(&mut self, width: u32, height: u32, dark: bool) -> Result<Frame, GpuError> {
        let gpu = render::gpu()?;
        if self.scene.is_none() {
            let Some(mesh) = self.mesh.as_ref() else {
                return Err(GpuError::Validation("the mesh is gone".to_owned()));
            };
            // The device's buffer limit caps what can be shown whatever the
            // budget says.
            let limit = usize::try_from(
                gpu.max_buffer_size() / std::mem::size_of::<mesh::Vertex>() as u64 / 3,
            )
            .unwrap_or(usize::MAX);
            let scene = match mesh.decimate(limit.min(self.budget)) {
                Some(smaller) => {
                    self.decimated = Some(smaller.triangles());
                    Scene::new(gpu, &smaller, &self.grid)?
                }
                None => Scene::new(gpu, mesh, &self.grid)?,
            };
            self.scene = Some(scene);
            self.mesh = None;
        }
        let Some(scene) = self.scene.as_mut() else {
            return Err(GpuError::Validation("no scene".to_owned()));
        };
        scene.render(gpu, &self.camera, width, height, dark)
    }

    /// The facts as text, for a frame that could not be rendered.
    fn fallback_text(&self) -> String {
        let mut text = format!("{}\n\n", self.format.name());
        if let Some(reason) = &self.failure {
            // Writing into a String cannot fail.
            let _ = write!(text, "3D rendering is not available: {reason}\n\n");
        }
        for (key, value) in self.rows() {
            let _ = writeln!(text, "{key}: {value}");
        }
        text
    }

    /// The File info rows.
    fn rows(&self) -> Vec<(String, String)> {
        let mut rows = vec![
            ("Format".to_owned(), self.format.name().to_owned()),
            ("Triangles".to_owned(), count(self.triangles)),
            ("Bounding box".to_owned(), self.dimensions()),
        ];
        if let Some(objects) = self.objects {
            rows.push(("Objects".to_owned(), count(objects)));
        }
        if let Some(unit) = self.unit {
            rows.push(("Units".to_owned(), unit.name().to_owned()));
        }
        if let Some(shown) = self.decimated {
            rows.push((
                "Preview".to_owned(),
                format!(
                    "decimated to {} triangles (budget {})",
                    count(shown),
                    count(self.budget)
                ),
            ));
        }
        rows.push((
            "Renderer".to_owned(),
            match (&self.failure, render::describe()) {
                (Some(reason), _) => format!("not available: {reason}"),
                (None, Some(adapter)) => adapter,
                (None, None) => "wgpu".to_owned(),
            },
        ));
        if let Some(size) = self.size {
            rows.push(("Size".to_owned(), format_size(size)));
        }
        rows.extend(self.metadata.iter().cloned());
        rows
    }
}

/// A request's size capped at [`MAX_SIDE`] a side, keeping the aspect.
#[must_use]
pub fn capped(width: u32, height: u32) -> (u32, u32) {
    let (w, h) = (width.max(1), height.max(1));
    if w <= MAX_SIDE && h <= MAX_SIDE {
        return (w, h);
    }
    let scale = (f64::from(MAX_SIDE) / f64::from(w)).min(f64::from(MAX_SIDE) / f64::from(h));
    // Scaled down into 1..=MAX_SIDE, so the casts are exact.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let side = |n: u32| ((f64::from(n) * scale).round().max(1.0)) as u32;
    (side(w), side(h))
}

/// A length in millimetres to two decimals, trailing zeros dropped.
#[must_use]
pub fn millimetres(value: f32) -> String {
    let text = format!("{value:.2}");
    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
    if trimmed == "-0" {
        "0".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// A count with a space every three digits: `1 500 000`.
#[must_use]
pub fn count(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// A byte count as `n B`, `n.n KiB` or `n.n MiB`.
fn format_size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    // Sizes shown to one decimal; the precision loss is immaterial.
    #[allow(clippy::cast_precision_loss)]
    let value = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if value < KIB * KIB {
        format!("{:.1} KiB", value / KIB)
    } else if value < KIB * KIB * KIB {
        format!("{:.1} MiB", value / (KIB * KIB))
    } else {
        format!("{:.2} GiB", value / (KIB * KIB * KIB))
    }
}

impl PreviewDoc for Doc3d {
    fn pages(&self) -> Option<usize> {
        None
    }

    fn render(&mut self, page: usize, req: RenderRequest) -> Result<Frame, Error> {
        if page != 0 {
            return Err(Error::NotFound);
        }
        let (width, height) = capped(req.width, req.height);
        match self.frame(width, height, req.dark) {
            Ok(frame) => {
                self.failure = None;
                Ok(frame)
            }
            Err(error) => {
                tracing::warn!(%error, "3D preview not rendered");
                self.failure = Some(error.to_string());
                Ok(Frame::Text(TextBlock {
                    text: self.fallback_text(),
                    language: None,
                    spans: Vec::new(),
                }))
            }
        }
    }

    fn text_layer(&self) -> Option<&TextLayer> {
        None
    }

    fn sections(&self) -> Vec<InfoSection> {
        vec![InfoSection {
            id: "file",
            title: Text("preview-3d-file-info"),
            icon: Icon::Info,
            rows: self.rows(),
        }]
    }

    fn status(&self) -> Vec<String> {
        let mut facts = vec![
            self.format.name().to_owned(),
            format!("{} triangles", count(self.triangles)),
            self.dimensions(),
        ];
        if self.format == Format::Obj
            && let Some(objects) = self.objects
        {
            facts.push(format!(
                "{} object{}",
                count(objects),
                if objects == 1 { "" } else { "s" }
            ));
        }
        if let Some(shown) = self.decimated {
            facts.push(format!(
                "decimated to {} triangles for the preview",
                count(shown)
            ));
        }
        facts
    }

    fn controls(&self) -> Vec<PreviewControl> {
        vec![
            PreviewControl::Pointer {
                id: ORBIT,
                gesture: Gesture::Drag,
                label: "Orbit",
            },
            PreviewControl::Pointer {
                id: ZOOM,
                gesture: Gesture::Wheel,
                label: "Zoom",
            },
            PreviewControl::Pointer {
                id: RESET,
                gesture: Gesture::DoubleClick,
                label: "Reset view",
            },
        ]
    }

    fn set_control(&mut self, id: ControlId, value: ControlValue) -> Result<(), Error> {
        match (id, value) {
            (ORBIT, ControlValue::Delta { x, y }) => {
                self.camera.orbit(x, y);
                Ok(())
            }
            (ZOOM, ControlValue::Delta { y, .. }) => {
                self.camera.zoom_by(y);
                Ok(())
            }
            (RESET, ControlValue::Pressed) => {
                self.camera.reset();
                Ok(())
            }
            _ => Err(Error::Unsupported("no such control")),
        }
    }

    fn transport(&mut self) -> Option<&mut dyn Transport> {
        None
    }
}
