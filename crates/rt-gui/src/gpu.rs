//! GPU presentation of the CPU-rasterized frame (ticket #23, behind the
//! `gpu` feature): the canonical engine representation stays `vello_cpu`
//! rasterization — this module never rasterizes. It uploads the painted
//! `u32` frame as a wgpu texture and blits it to the swapchain (GPU present
//! with a scale blit), so application and widget code are identical on both
//! backends.
//!
//! # Fallback discipline
//!
//! - [`GpuPresenter::new`] probes the adapter (LowPower, compatible with the
//!   window's surface) and returns `None` on any failure — the presenter
//!   falls back to CPU (softbuffer) presentation silently.
//! - [`GpuPresenter::present`] returns `false` on a wgpu error after logging
//!   it through `eprintln!`; the presenter then disables the GPU path and
//!   falls back to CPU for all subsequent frames. Transient surface states
//!   (timeout, occlusion, outdated swapchain) skip a frame without
//!   disabling the path.
//!
//! Zero-sized surfaces are never configured nor presented (matches the
//! presenter's minimization guard).
//!
//! True GPU rasterization (vello compute) is deferred: vello 0.11's retained
//! `Scene` encoding is not API-compatible with vello_cpu 0.3's immediate
//! `RenderContext` (stateful paint/fill + `Resources`-owning glyph runs), so
//! widgets cannot emit once into both. Revisit when Linebender ships aligned
//! Scene/RenderContext versions.

#![forbid(unsafe_code)]

use std::num::NonZeroU32;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::task::{Context, Poll, Wake, Waker};
use std::thread;

use winit::window::Window as PlatformWindow;

/// Presents the CPU-rasterized `u32` frame through a wgpu surface.
///
/// Frame pixel layout is softbuffer's `0x00RRGGBB` (top byte zero): in
/// little-endian memory that is `B, G, R, A` bytes, i.e. an exact
/// `Bgra8Unorm(Srgb)` upload — the alpha byte is always 0, so the blit
/// shader forces alpha to 1 and the surface is configured opaque when
/// possible. Uploading the texture with the *swapchain's own* format keeps
/// the blit byte-exact for RGB in both cases (`*Unorm`: identity;
/// `*UnormSrgb`: the sampled-then-written sRGB round trip cancels out).
pub(crate) struct GpuPresenter {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    bind_group_layout: wgpu::BindGroupLayout,
    /// Upload texture + bind group for the current frame dimensions
    /// (recreated only when the frame size changes).
    texture: Option<(wgpu::Texture, wgpu::BindGroup, [u32; 2])>,
    /// Reused frame bytes for texture upload (capacity grows once).
    scratch: Vec<u8>,
    /// Set by the uncaptured-error handler; checked every present.
    errored: Arc<AtomicBool>,
    /// Whether the surface has been configured with a real (non-zero) size.
    configured: bool,
}

impl GpuPresenter {
    /// Probe and initialize: adapter (LowPower, compatible with the window),
    /// device, swapchain, and the blit pipeline. `None` on any failure —
    /// silent, so the caller falls back to CPU without noise.
    pub(crate) fn new(window: &Arc<PlatformWindow>) -> Option<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        // The surface holds its own clone of the window ('static); the
        // instance itself may be dropped after setup.
        let surface = instance.create_surface(window.clone()).ok()?;
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            apply_limit_buckets: false,
        }))
        .ok()?;
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("rt-gui present"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
        }))
        .ok()?;

        // Prefer a Bgra8 format (the frame's byte order); every wgpu-capable
        // backend offers one. The chosen format is reused as the upload
        // texture's format (see the type-level doc for why that stays exact).
        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(|f| {
                matches!(
                    f,
                    wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
                )
            })
            .unwrap_or(capabilities.formats[0]);
        let alpha_mode = if capabilities
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::Opaque)
        {
            wgpu::CompositeAlphaMode::Opaque
        } else {
            capabilities.alpha_modes[0]
        };
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode,
            view_formats: Vec::new(),
            desired_maximum_frame_latency: 2,
        };

        let errored = Arc::new(AtomicBool::new(false));
        {
            let errored = Arc::clone(&errored);
            device.on_uncaptured_error(Arc::new(move |error| {
                eprintln!("rt-gui: wgpu error, falling back to CPU presentation: {error}");
                errored.store(true, Ordering::Relaxed);
            }));
        }

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rt-gui blit bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rt-gui blit layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rt-gui blit shader"),
            source: wgpu::ShaderSource::Wgsl(BLIT_SHADER.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rt-gui blit pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("rt-gui blit sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Some(Self {
            device,
            queue,
            surface,
            config,
            pipeline,
            sampler,
            bind_group_layout,
            texture: None,
            scratch: Vec::new(),
            errored,
            configured: false,
        })
    }

    /// Present one CPU-rasterized frame: reconfigure the surface on size
    /// change, upload the frame as a texture, blit to the swapchain, and
    /// present. Returns `false` on a wgpu error (already logged) — the
    /// caller then switches to CPU presentation permanently. A frame whose
    /// length does not match the window size is a stale in-flight resize
    /// frame and is skipped without error (same guard as the CPU path).
    pub(crate) fn present(&mut self, frame: &[u32], width: NonZeroU32, height: NonZeroU32) -> bool {
        if self.errored.load(Ordering::Relaxed) {
            return false;
        }
        let (width, height) = (width.get(), height.get());
        let pixel_count = usize::try_from(width).expect("u32 fits usize")
            * usize::try_from(height).expect("u32 fits usize");
        if frame.len() != pixel_count {
            return true; // stale resize frame: skip, keep the GPU path alive
        }
        if !self.configured || self.config.width != width || self.config.height != height {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
            self.texture = None;
            self.configured = true;
        }

        // (Re)create the upload texture only when the frame size changes.
        if self
            .texture
            .as_ref()
            .is_none_or(|(.., dims)| *dims != [width, height])
        {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("rt-gui frame upload"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.config.format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("rt-gui blit bind group"),
                layout: &self.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            self.texture = Some((texture, bind_group, [width, height]));
        }
        let Some((texture, bind_group, _)) = &self.texture else {
            return false;
        };

        // Upload: the u32 frame in little-endian memory is exactly the
        // format's byte layout (B, G, R, A with A = 0).
        self.scratch.clear();
        self.scratch
            .extend(frame.iter().flat_map(|p| p.to_ne_bytes()));
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &self.scratch,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        // Acquire: skip frames wgpu tells us to skip without killing the
        // path; surface loss/validation errors mean permanent CPU fallback.
        let frame_texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return true; // skip this frame, present the next one
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                // Swapchain no longer matches the window; reconfigure before
                // the next present (the size check at the top only covers
                // width/height changes).
                self.configured = false;
                return true;
            }
            status @ (wgpu::CurrentSurfaceTexture::Lost
            | wgpu::CurrentSurfaceTexture::Validation) => {
                eprintln!(
                    "rt-gui: wgpu surface error, falling back to CPU presentation: {status:?}"
                );
                return false;
            }
        };
        let view = frame_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("rt-gui blit encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("rt-gui blit pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        // wgpu 30 presents through the queue; presentation errors surface
        // through the uncaptured handler above, which trips the flag.
        self.queue.present(frame_texture);
        !self.errored.load(Ordering::Relaxed)
    }
}

/// A fullscreen triangle sampling the uploaded frame texture. The target's
/// alpha is forced opaque (the frame's alpha byte is always 0); RGB passes
/// through byte-exact when the texture format is the swapchain format.
const BLIT_SHADER: &str = r"
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VsOut {
    let x = f32(i32(vi & 1u) * 4 - 1);
    let y = f32(i32(vi & 2u) * 2 - 1);
    var out: VsOut;
    out.pos = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, 1.0 - (y + 1.0) * 0.5);
    return out;
}

@group(0) @binding(0) var frame: texture_2d<f32>;
@group(0) @binding(1) var nearest: sampler;

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let pixel = textureSample(frame, nearest, in.uv);
    return vec4<f32>(pixel.rgb, 1.0);
}
";

/// Blocks on a wgpu setup future without a runtime dependency: a
/// thread-unpark waker is enough, because the adapter/device requests are
/// resolved by the wgpu backend on the calling thread (pollster is not on
/// the ADR 0007 allowlist).
fn block_on<T>(future: impl Future<Output = T>) -> T {
    struct ThreadWake(thread::Thread);

    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(value) => return value,
            Poll::Pending => thread::park(),
        }
    }
}
