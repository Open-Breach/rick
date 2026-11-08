// The one shader of the 3D previewer: a vertex stage shared by the solid
// and the grid, a lit fragment stage for the solid (flat shading from the
// screen-space derivatives of the world position, so no normals cross the
// bus, two-sided so inconsistent winding never goes black) and a flat one
// for the grid lines, whose vertex colour word is the line's kind.

struct Uniforms {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    light: vec4<f32>,
    grid_minor: vec4<f32>,
    grid_major: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) color: u32,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) kind: u32,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = u.view_proj * vec4<f32>(in.position, 1.0);
    out.world = in.position;
    out.color = unpack4x8unorm(in.color);
    out.kind = in.color;
    return out;
}

@fragment
fn fs_solid(in: VertexOut) -> @location(0) vec4<f32> {
    var n = normalize(cross(dpdx(in.world), dpdy(in.world)));
    let to_eye = normalize(u.eye.xyz - in.world);
    if (dot(n, to_eye) < 0.0) {
        n = -n;
    }
    let light = normalize(u.light.xyz);
    let key = max(dot(n, light), 0.0);
    let fill = max(dot(n, to_eye), 0.0);
    let base = pow(in.color.rgb, vec3<f32>(2.2));
    let half_way = normalize(light + to_eye);
    let specular = pow(max(dot(n, half_way), 0.0), 32.0) * 0.12;
    let lit = base * (0.2 + 0.55 * key + 0.3 * fill) + vec3<f32>(specular);
    return vec4<f32>(lit, 1.0);
}

@fragment
fn fs_line(in: VertexOut) -> @location(0) vec4<f32> {
    if (in.kind == 1u) {
        return u.grid_major;
    }
    return u.grid_minor;
}
