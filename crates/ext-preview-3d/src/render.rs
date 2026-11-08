//! Off-screen rendering through wgpu: one device per process, opened on
//! the first frame, and per document the mesh on the GPU, a ground grid
//! and a render target the size of the request, drawn with 4× multisampling
//! and read back as tightly packed RGBA.
//!
//! The adapter is whatever wgpu ranks first (`WGPU_BACKEND` and
//! `WGPU_POWER_PREF` steer it, as everywhere in wgpu); when none is found
//! the fallback adapter is asked for, which is the software rasteriser
//! where the platform has one (lavapipe on Linux, WARP on Windows). Every
//! frame runs inside error scopes, so a validation error or an exhausted
//! memory comes back as an error instead of a panic.

use std::sync::OnceLock;

use ext::Frame;
use wgpu::util::DeviceExt as _;

use crate::camera::Camera;
use crate::mesh::{Aabb, Mesh, Vertex};

/// Samples per pixel.
pub const MSAA: u32 = 4;
/// The frame's format: 8-bit sRGB with alpha, read back as is.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// The depth format.
pub const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Why a frame could not be rendered.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum GpuError {
    /// No adapter at all, not even a software one.
    #[error("no graphics adapter: {0}")]
    NoAdapter(String),
    /// The adapter gave no device.
    #[error("no graphics device: {0}")]
    NoDevice(String),
    /// The shader did not compile or a call was invalid.
    #[error("graphics error: {0}")]
    Validation(String),
    /// The device ran out of memory.
    #[error("graphics memory exhausted: {0}")]
    OutOfMemory(String),
    /// The frame did not come back from the device.
    #[error("the frame could not be read back: {0}")]
    Readback(String),
}

/// The process's device and the two pipelines.
pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    solid: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    info: wgpu::AdapterInfo,
}

impl std::fmt::Debug for Gpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gpu")
            .field("adapter", &self.info.name)
            .field("backend", &self.info.backend)
            .finish_non_exhaustive()
    }
}

static GPU: OnceLock<Result<Gpu, GpuError>> = OnceLock::new();

/// The process's GPU, opened on the first call; the same error on every
/// call when there is none.
///
/// # Errors
///
/// [`GpuError::NoAdapter`], [`GpuError::NoDevice`] or
/// [`GpuError::Validation`] when the shader does not compile there.
pub fn gpu() -> Result<&'static Gpu, GpuError> {
    GPU.get_or_init(Gpu::open).as_ref().map_err(Clone::clone)
}

/// The adapter's name and backend, once the GPU was opened: `"llvmpipe
/// (LLVM 20.1.2, 256 bits) through Vulkan"`.
#[must_use]
pub fn describe() -> Option<String> {
    GPU.get()
        .and_then(|gpu| gpu.as_ref().ok())
        .map(|gpu| format!("{} through {}", gpu.info.name, gpu.info.backend.to_str()))
}

/// The uniforms as the shader lays them out.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    view_proj: [f32; 16],
    eye: [f32; 4],
    light: [f32; 4],
    grid_minor: [f32; 4],
    grid_major: [f32; 4],
}

impl Gpu {
    fn open() -> Result<Self, GpuError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let request = |fallback: bool| {
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference:
                    wgpu::PowerPreference::from_env().unwrap_or(wgpu::PowerPreference::LowPower),
                force_fallback_adapter: fallback,
                compatible_surface: None,
                ..Default::default()
            }))
        };
        let adapter = request(false)
            .or_else(|_| request(true))
            .map_err(|e| GpuError::NoAdapter(e.to_string()))?;
        let info = adapter.get_info();
        tracing::info!(adapter = %info.name, backend = %info.backend, "3D previews render here");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("rick 3D preview"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|e| GpuError::NoDevice(e.to_string()))?;
        device.on_uncaptured_error(std::sync::Arc::new(|error: wgpu::Error| {
            tracing::error!(%error, "3D preview graphics error outside a frame");
        }));
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("3d preview"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3d uniforms"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("3d"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let solid = pipeline(
            &device,
            &shader,
            &pipeline_layout,
            "fs_solid",
            wgpu::PrimitiveTopology::TriangleList,
        );
        let lines = pipeline(
            &device,
            &shader,
            &pipeline_layout,
            "fs_line",
            wgpu::PrimitiveTopology::LineList,
        );
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(GpuError::Validation(error.to_string()));
        }
        Ok(Self {
            device,
            queue,
            solid,
            lines,
            layout,
            info,
        })
    }

    /// The largest buffer the device takes, in bytes.
    #[must_use]
    pub fn max_buffer_size(&self) -> u64 {
        self.device.limits().max_buffer_size
    }
}

/// One of the two pipelines: the shared vertex stage, the fragment stage
/// `entry`, depth tested, multisampled, drawing `topology`.
fn pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    entry: &str,
    topology: wgpu::PrimitiveTopology,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Uint32],
            })],
        },
        primitive: wgpu::PrimitiveState {
            topology,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: MSAA,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// The ground grid under a model: lines on the plane of the model's lowest
/// point, a nice step apart, out to twice the model's footprint.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    /// Two vertices per line; the colour word is 1 for a major line.
    pub vertices: Vec<Vertex>,
    /// The distance between lines, in millimetres.
    pub step: f32,
    /// How far the grid reaches from the model's centre.
    pub half: f32,
}

impl Grid {
    /// The grid for a model within `bounds`.
    #[must_use]
    pub fn under(bounds: &Aabb) -> Self {
        let size = bounds.size();
        let centre = bounds.centre();
        let footprint = size[0].max(size[1]).max(1.0);
        let step = nice_step(footprint / 10.0);
        let count = (footprint / step).ceil().max(1.0);
        // A whole number of steps, at most a few hundred lines a side.
        #[allow(clippy::cast_possible_truncation)]
        let lines = (count as i32).min(400);
        // Back to a length; small whole numbers are exact.
        #[allow(clippy::cast_precision_loss)]
        let half = lines as f32 * step;
        let z = if bounds.is_empty() {
            0.0
        } else {
            bounds.min[2]
        };
        let mut vertices = Vec::with_capacity(usize::try_from(lines).unwrap_or(0) * 8 + 4);
        for k in -lines..=lines {
            // Small whole numbers are exact.
            #[allow(clippy::cast_precision_loss)]
            let offset = k as f32 * step;
            let kind = u32::from(k % 5 == 0);
            let x = centre[0] + offset;
            let y = centre[1] + offset;
            vertices.push(Vertex {
                position: [x, centre[1] - half, z],
                color: kind,
            });
            vertices.push(Vertex {
                position: [x, centre[1] + half, z],
                color: kind,
            });
            vertices.push(Vertex {
                position: [centre[0] - half, y, z],
                color: kind,
            });
            vertices.push(Vertex {
                position: [centre[0] + half, y, z],
                color: kind,
            });
        }
        Self {
            vertices,
            step,
            half,
        }
    }
}

/// The step of 1, 2 or 5 times a power of ten nearest to `raw`.
#[must_use]
pub fn nice_step(raw: f32) -> f32 {
    let raw = raw.max(1e-3);
    let exponent = raw.log10().floor();
    let magnitude = 10f32.powf(exponent);
    let mantissa = raw / magnitude;
    let nice = if mantissa < 1.5 {
        1.0
    } else if mantissa < 3.5 {
        2.0
    } else if mantissa < 7.5 {
        5.0
    } else {
        10.0
    };
    nice * magnitude
}

/// A linear colour from sRGB bytes, for the uniforms and the clear.
fn linear(srgb: [u8; 3]) -> [f32; 4] {
    let channel = |c: u8| -> f32 {
        let c = f32::from(c) / 255.0;
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    [channel(srgb[0]), channel(srgb[1]), channel(srgb[2]), 1.0]
}

/// The theme's background and grid colours, linear.
fn palette(dark: bool) -> ([f32; 4], [f32; 4], [f32; 4]) {
    if dark {
        (
            linear([0x1F, 0x1F, 0x22]),
            linear([0x3A, 0x3A, 0x40]),
            linear([0x55, 0x55, 0x5D]),
        )
    } else {
        (
            linear([0xF5, 0xF5, 0xF7]),
            linear([0xD9, 0xD9, 0xDE]),
            linear([0xBE, 0xBE, 0xC5]),
        )
    }
}

/// A render target of one size, with its readback buffer.
struct Target {
    width: u32,
    height: u32,
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    msaa_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    readback: wgpu::Buffer,
    padded_row: u32,
}

impl Target {
    fn new(gpu: &Gpu, width: u32, height: u32) -> Self {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = |label, sample_count, format, usage| {
            gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let color = texture(
            "3d frame",
            1,
            FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let msaa = texture(
            "3d msaa",
            MSAA,
            FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let depth = texture(
            "3d depth",
            MSAA,
            DEPTH,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let padded_row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("3d readback"),
            size: u64::from(padded_row) * u64::from(height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            width,
            height,
            color_view: color.create_view(&wgpu::TextureViewDescriptor::default()),
            color,
            msaa_view: msaa.create_view(&wgpu::TextureViewDescriptor::default()),
            depth_view: depth.create_view(&wgpu::TextureViewDescriptor::default()),
            readback,
            padded_row,
        }
    }
}

/// One document's mesh and grid on the GPU, and its render target.
pub struct Scene {
    vertices: Option<wgpu::Buffer>,
    vertex_count: u32,
    grid: Option<wgpu::Buffer>,
    grid_count: u32,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    target: Option<Target>,
    bounds: Aabb,
    reach: f32,
}

impl std::fmt::Debug for Scene {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scene")
            .field("vertices", &self.vertex_count)
            .field("grid", &self.grid_count)
            .field("target", &self.target.as_ref().map(|t| (t.width, t.height)))
            .finish_non_exhaustive()
    }
}

impl Scene {
    /// Uploads `mesh` and `grid`.
    ///
    /// # Errors
    ///
    /// [`GpuError::OutOfMemory`] or [`GpuError::Validation`] when the
    /// device refuses the buffers (a mesh over its buffer limit).
    pub fn new(gpu: &Gpu, mesh: &Mesh, grid: &Grid) -> Result<Self, GpuError> {
        let memory = gpu.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let upload = |label: &str, vertices: &[Vertex]| -> Option<wgpu::Buffer> {
            (!vertices.is_empty()).then(|| {
                gpu.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(label),
                        contents: bytemuck::cast_slice(vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    })
            })
        };
        let vertices = upload("3d mesh", &mesh.vertices);
        let grid_buffer = upload("3d grid", &grid.vertices);
        let uniforms = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("3d uniforms"),
            size: std::mem::size_of::<Uniforms>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("3d uniforms"),
            layout: &gpu.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        if let Some(error) = pollster::block_on(validation.pop()) {
            return Err(GpuError::Validation(error.to_string()));
        }
        if let Some(error) = pollster::block_on(memory.pop()) {
            return Err(GpuError::OutOfMemory(error.to_string()));
        }
        Ok(Self {
            vertices,
            vertex_count: u32::try_from(mesh.vertices.len()).unwrap_or(u32::MAX),
            grid: grid_buffer,
            grid_count: u32::try_from(grid.vertices.len()).unwrap_or(u32::MAX),
            uniforms,
            bind_group,
            target: None,
            bounds: mesh.bounds,
            reach: grid.half * std::f32::consts::SQRT_2 + mesh.bounds.radius(),
        })
    }

    /// Draws the scene from `camera` into a `width` × `height` frame on
    /// the theme's background.
    ///
    /// # Errors
    ///
    /// [`GpuError`] when the device refuses the frame or does not hand it
    /// back.
    pub fn render(
        &mut self,
        gpu: &Gpu,
        camera: &Camera,
        width: u32,
        height: u32,
        dark: bool,
    ) -> Result<Frame, GpuError> {
        let (width, height) = (width.max(1), height.max(1));
        let memory = gpu.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        if self
            .target
            .as_ref()
            .is_none_or(|t| t.width != width || t.height != height)
        {
            self.target = Some(Target::new(gpu, width, height));
        }
        let Some(target) = self.target.as_ref() else {
            return Err(GpuError::Validation("no render target".to_owned()));
        };
        let (background, minor, major) = palette(dark);
        let uniforms = self.uniforms_for(camera, width, height, minor, major);
        gpu.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("3d frame"),
            });
        self.draw(gpu, &mut encoder, target, background);
        encoder.copy_texture_to_buffer(
            target.color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &target.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(target.padded_row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit(Some(encoder.finish()));
        let (tx, rx) = std::sync::mpsc::channel();
        target
            .readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        if let Some(error) = pollster::block_on(validation.pop()) {
            return Err(GpuError::Validation(error.to_string()));
        }
        if let Some(error) = pollster::block_on(memory.pop()) {
            return Err(GpuError::OutOfMemory(error.to_string()));
        }
        rx.recv()
            .map_err(|e| GpuError::Readback(e.to_string()))?
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        let data = read_back(target)?;
        Ok(Frame::Rgba {
            width,
            height,
            data: data.into(),
        })
    }

    /// The uniforms of a frame from `camera`.
    fn uniforms_for(
        &self,
        camera: &Camera,
        width: u32,
        height: u32,
        grid_minor: [f32; 4],
        grid_major: [f32; 4],
    ) -> Uniforms {
        let centre = self.bounds.centre();
        let radius = self.bounds.radius();
        // Pixel counts are small enough to be exact.
        #[allow(clippy::cast_precision_loss)]
        let aspect = width as f32 / height as f32;
        let eye = camera.eye(centre, radius);
        let light = camera.light();
        Uniforms {
            view_proj: camera
                .view_projection(centre, radius, aspect, self.reach)
                .to_array(),
            eye: [eye[0], eye[1], eye[2], 1.0],
            light: [light[0], light[1], light[2], 0.0],
            grid_minor,
            grid_major,
        }
    }

    /// Records the render pass: the background, the solid, the grid.
    fn draw(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        target: &Target,
        background: [f32; 4],
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("3d frame"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.msaa_view,
                depth_slice: None,
                resolve_target: Some(&target.color_view),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(background[0]),
                        g: f64::from(background[1]),
                        b: f64::from(background[2]),
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Discard,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &target.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &self.bind_group, &[]);
        if let Some(vertices) = &self.vertices {
            pass.set_pipeline(&gpu.solid);
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.draw(0..self.vertex_count, 0..1);
        }
        if let Some(grid) = &self.grid {
            pass.set_pipeline(&gpu.lines);
            pass.set_vertex_buffer(0, grid.slice(..));
            pass.draw(0..self.grid_count, 0..1);
        }
    }
}

/// The mapped frame as tightly packed rows; unmaps the buffer.
fn read_back(target: &Target) -> Result<Vec<u8>, GpuError> {
    let data = {
        let view = target
            .readback
            .slice(..)
            .get_mapped_range()
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        let row = target.width as usize * 4;
        let mut data = Vec::with_capacity(row * target.height as usize);
        for y in 0..target.height as usize {
            let start = y * target.padded_row as usize;
            data.extend_from_slice(&view[start..start + row]);
        }
        data
    };
    target.readback.unmap();
    Ok(data)
}
