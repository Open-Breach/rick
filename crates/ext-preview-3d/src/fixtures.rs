//! Fixtures for the tests: a cube as ASCII and binary STL and as 3MF in
//! any unit, a torus of any triangle count as binary STL, and an OBJ of
//! three coloured cubes with its material library.

use std::fmt::Write as _;
use std::io::Write as _;

use crate::math::Vec3;

/// The twelve triangles of a cube from the origin to `size` on each axis,
/// wound outwards.
#[must_use]
pub fn cube_triangles(size: f32) -> Vec<[Vec3; 3]> {
    let s = size;
    let p =
        |x: u8, y: u8, z: u8| -> Vec3 { [f32::from(x) * s, f32::from(y) * s, f32::from(z) * s] };
    vec![
        // Bottom (z = 0), facing down.
        [p(0, 0, 0), p(0, 1, 0), p(1, 1, 0)],
        [p(0, 0, 0), p(1, 1, 0), p(1, 0, 0)],
        // Top (z = s), facing up.
        [p(0, 0, 1), p(1, 0, 1), p(1, 1, 1)],
        [p(0, 0, 1), p(1, 1, 1), p(0, 1, 1)],
        // Front (y = 0).
        [p(0, 0, 0), p(1, 0, 0), p(1, 0, 1)],
        [p(0, 0, 0), p(1, 0, 1), p(0, 0, 1)],
        // Back (y = s).
        [p(0, 1, 0), p(0, 1, 1), p(1, 1, 1)],
        [p(0, 1, 0), p(1, 1, 1), p(1, 1, 0)],
        // Left (x = 0).
        [p(0, 0, 0), p(0, 0, 1), p(0, 1, 1)],
        [p(0, 0, 0), p(0, 1, 1), p(0, 1, 0)],
        // Right (x = s).
        [p(1, 0, 0), p(1, 1, 0), p(1, 1, 1)],
        [p(1, 0, 0), p(1, 1, 1), p(1, 0, 1)],
    ]
}

/// The face normal of a triangle, unit length.
fn normal(t: &[Vec3; 3]) -> Vec3 {
    let a = crate::math::sub(t[1], t[0]);
    let b = crate::math::sub(t[2], t[0]);
    crate::math::normalize(crate::math::cross(a, b))
}

/// Triangles as an ASCII STL named `name`.
#[must_use]
pub fn ascii_stl(name: &str, triangles: &[[Vec3; 3]]) -> Vec<u8> {
    let mut out = format!("solid {name}\n");
    for t in triangles {
        let n = normal(t);
        // Writing into a String cannot fail.
        let _ = writeln!(out, "  facet normal {} {} {}", n[0], n[1], n[2]);
        out.push_str("    outer loop\n");
        for v in t {
            let _ = writeln!(out, "      vertex {} {} {}", v[0], v[1], v[2]);
        }
        out.push_str("    endloop\n  endfacet\n");
    }
    let _ = writeln!(out, "endsolid {name}");
    out.into_bytes()
}

/// Triangles as a binary STL whose header starts with `solid`, as many
/// exporters write it.
#[must_use]
pub fn binary_stl(triangles: &[[Vec3; 3]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(84 + triangles.len() * 50);
    let mut header = [0u8; 80];
    let note = b"solid binary STL written by Rick's test fixtures";
    header[..note.len()].copy_from_slice(note);
    out.extend_from_slice(&header);
    out.extend_from_slice(
        &u32::try_from(triangles.len())
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    for t in triangles {
        let n = normal(t);
        for value in n.iter().chain(t.iter().flatten()) {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(&0u16.to_le_bytes());
    }
    out
}

/// A cube of `size` mm as an ASCII STL.
#[must_use]
pub fn cube_stl_ascii(size: f32) -> Vec<u8> {
    ascii_stl("cube", &cube_triangles(size))
}

/// A cube of `size` mm as a binary STL.
#[must_use]
pub fn cube_stl_binary(size: f32) -> Vec<u8> {
    binary_stl(&cube_triangles(size))
}

/// A torus (60 mm across, a 20 mm tube) tessellated into exactly
/// `triangles` triangles, as a binary STL; the last ring is cut where the
/// count runs out.
#[must_use]
pub fn torus_stl(triangles: usize) -> Vec<u8> {
    const MAJOR: f32 = 30.0;
    const MINOR: f32 = 10.0;
    // Triangle counts are far below 2^24, so the arithmetic is exact.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let around = ((triangles as f32 / 2.0).sqrt().ceil() as usize).max(3);
    let along = triangles.div_ceil(2 * around).max(3);
    let point = |i: usize, j: usize| -> Vec3 {
        // Grid indices, exact as floats.
        #[allow(clippy::cast_precision_loss)]
        let u = i as f32 / around as f32 * std::f32::consts::TAU;
        #[allow(clippy::cast_precision_loss)]
        let v = j as f32 / along as f32 * std::f32::consts::TAU;
        let ring = MAJOR + MINOR * v.cos();
        [ring * u.cos(), ring * u.sin(), MINOR * v.sin()]
    };
    let mut out = Vec::with_capacity(84 + triangles * 50);
    let mut header = [0u8; 80];
    header[..5].copy_from_slice(b"torus");
    out.extend_from_slice(&header);
    out.extend_from_slice(&u32::try_from(triangles).unwrap_or(u32::MAX).to_le_bytes());
    let mut written = 0;
    'rings: for j in 0..along {
        for i in 0..around {
            let quad = [
                point(i, j),
                point(i + 1, j),
                point(i + 1, j + 1),
                point(i, j + 1),
            ];
            for t in [[quad[0], quad[1], quad[2]], [quad[0], quad[2], quad[3]]] {
                if written == triangles {
                    break 'rings;
                }
                let n = normal(&t);
                for value in n.iter().chain(t.iter().flatten()) {
                    out.extend_from_slice(&value.to_le_bytes());
                }
                out.extend_from_slice(&0u16.to_le_bytes());
                written += 1;
            }
        }
    }
    out
}

/// The name of the material library the OBJ fixture names.
pub const OBJ_MATERIALS: &str = "cubes.mtl";

/// Three 10 mm cubes side by side, each its own object with its own
/// material (red, green, blue), as the OBJ text and the material library.
#[must_use]
pub fn three_cubes_obj() -> (Vec<u8>, Vec<u8>) {
    let mut obj = format!("# three cubes\nmtllib {OBJ_MATERIALS}\n");
    let mut index = 1usize;
    for (k, (name, material)) in [("red", "Red"), ("green", "Green"), ("blue", "Blue")]
        .into_iter()
        .enumerate()
    {
        // Three cubes, exact as floats.
        #[allow(clippy::cast_precision_loss)]
        let offset = k as f32 * 15.0;
        // Writing into a String cannot fail.
        let _ = writeln!(obj, "o {name}\nusemtl {material}");
        let mut faces = String::new();
        for t in cube_triangles(10.0) {
            for v in t {
                let _ = writeln!(obj, "v {} {} {}", v[0] + offset, v[1], v[2]);
            }
            let _ = writeln!(faces, "f {} {} {}", index, index + 1, index + 2);
            index += 3;
        }
        obj.push_str(&faces);
    }
    let mtl =
        "newmtl Red\nKd 1.0 0.0 0.0\nnewmtl Green\nKd 0.0 1.0 0.0\nnewmtl Blue\nKd 0.0 0.0 1.0\n";
    (obj.into_bytes(), mtl.as_bytes().to_vec())
}

/// The XML of a 3MF model part holding `objects` (each an element with its
/// id baked in) and `build`, in `unit`, with a title and a designer.
fn model_xml(unit: &str, objects: &str, build: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <model unit=\"{unit}\" xml:lang=\"en-US\" xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\">\n\
         <metadata name=\"Title\">Cube</metadata>\n\
         <metadata name=\"Designer\">Rick</metadata>\n\
         <resources>{objects}</resources>\n\
         <build>{build}</build>\n\
         </model>\n"
    )
}

/// A mesh object element of a cube of `size` units with the id `id`.
fn cube_object_xml(id: u32, size: f32) -> String {
    let mut vertices = String::new();
    for z in 0..2u8 {
        for y in 0..2u8 {
            for x in 0..2u8 {
                // Writing into a String cannot fail.
                let _ = write!(
                    vertices,
                    "<vertex x=\"{}\" y=\"{}\" z=\"{}\"/>",
                    f32::from(x) * size,
                    f32::from(y) * size,
                    f32::from(z) * size
                );
            }
        }
    }
    // Corner index: x + 2y + 4z.
    let faces = [
        [0, 2, 3],
        [0, 3, 1],
        [4, 5, 7],
        [4, 7, 6],
        [0, 1, 5],
        [0, 5, 4],
        [2, 6, 7],
        [2, 7, 3],
        [0, 4, 6],
        [0, 6, 2],
        [1, 3, 7],
        [1, 7, 5],
    ];
    let mut triangles = String::new();
    for f in &faces {
        let _ = write!(
            triangles,
            "<triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"/>",
            f[0], f[1], f[2]
        );
    }
    format!(
        "<object id=\"{id}\" type=\"model\"><mesh><vertices>{vertices}</vertices><triangles>{triangles}</triangles></mesh></object>"
    )
}

/// A 3MF package around the model part `model`.
#[must_use]
pub fn package_3mf(model: &str) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let files = [
        (
            "[Content_Types].xml",
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"model\" ContentType=\"application/vnd.ms-package.3dmanufacturing-3dmodel+xml\"/></Types>",
        ),
        (
            "_rels/.rels",
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Target=\"/3D/3dmodel.model\" Id=\"rel0\" Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel\"/></Relationships>",
        ),
        ("3D/3dmodel.model", model),
    ];
    for (name, text) in files {
        // Writing into memory cannot fail.
        if zip.start_file(name, options).is_ok() {
            let _ = zip.write_all(text.as_bytes());
        }
    }
    zip.finish().map_or_default(std::io::Cursor::into_inner)
}

/// A cube of `size` units as a 3MF in `unit` (`"millimeter"`, `"inch"`,
/// …), one build item, titled "Cube" and designed by "Rick".
#[must_use]
pub fn cube_3mf(size: f32, unit: &str) -> Vec<u8> {
    package_3mf(&model_xml(
        unit,
        &cube_object_xml(1, size),
        "<item objectid=\"1\"/>",
    ))
}

/// A 3MF in millimetres whose build item is a components object placing a
/// 10 mm cube twice, the second one 20 mm along Y, so the whole spans
/// 10 × 30 × 10 mm.
#[must_use]
pub fn components_3mf() -> Vec<u8> {
    let objects = format!(
        "{}<object id=\"2\"><components><component objectid=\"1\"/><component objectid=\"1\" transform=\"1 0 0 0 1 0 0 0 1 0 20 0\"/></components></object>",
        cube_object_xml(1, 10.0)
    );
    package_3mf(&model_xml("millimeter", &objects, "<item objectid=\"2\"/>"))
}
