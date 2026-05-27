// Copyright 2025 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

//! Simple helpers for managing wgpu state and surfaces.
//!
//! This module is based on [`vello::util`] module with modifications
//! for transparent surfaces.

use wgpu::util::{TextureBlitter, TextureBlitterBuilder};
use wgpu::{
    self, BlendComponent, BlendFactor, BlendState, CompositeAlphaMode, Device, Instance,
    MemoryBudgetThresholds, MemoryHints, PresentMode, Surface, SurfaceConfiguration, Texture,
    TextureFormat, TextureUsages, TextureView,
};

use crate::app_driver::WgpuLimits;

#[derive(Debug)]
pub(crate) enum RenderSurfaceError {
    CreateSurface(wgpu::CreateSurfaceError),
    NoCompatibleDevice,
    UnsupportedSurfaceFormat,
}

impl core::fmt::Display for RenderSurfaceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::CreateSurface(err) => write!(f, "creating surface failed: {err}"),
            Self::NoCompatibleDevice => write!(f, "no compatible WGPU device found"),
            Self::UnsupportedSurfaceFormat => write!(f, "unsupported surface format"),
        }
    }
}

impl std::error::Error for RenderSurfaceError {}

/// Simple render context that maintains wgpu state for rendering the pipeline.
pub(crate) struct RenderContext {
    pub instance: Instance,
    /// Created devices used by this context.
    ///
    /// Invariants:
    /// - Entries are append-only (never deleted or replaced).
    /// - Indices are stable for the lifetime of the `RenderContext`.
    ///
    /// Other parts of the library store indices into this vec (e.g. `RenderSurface::dev_id`) and
    /// assume they remain valid.
    pub devices: Vec<DeviceHandle>,
    requested_features: wgpu::Features,
    requested_limits: WgpuLimits,
}

pub(crate) struct DeviceHandle {
    pub(crate) adapter: wgpu::Adapter,
    pub device: Device,
    pub queue: wgpu::Queue,
}

impl RenderContext {
    pub(crate) fn new() -> Self {
        let backends = wgpu::Backends::from_env().unwrap_or_default();
        let flags = wgpu::InstanceFlags::from_build_config().with_env();
        let backend_options = wgpu::BackendOptions::from_env_or_default();
        let instance = Instance::new(&wgpu::InstanceDescriptor {
            backends,
            flags,
            backend_options,
            memory_budget_thresholds: MemoryBudgetThresholds::default(),
        });
        Self {
            instance,
            devices: Vec::new(),
            requested_features: wgpu::Features::empty(),
            requested_limits: WgpuLimits::Default,
        }
    }

    pub(crate) fn set_wgpu_device_options(
        &mut self,
        features: wgpu::Features,
        limits: WgpuLimits,
    ) -> bool {
        if !self.devices.is_empty() {
            return false;
        }
        self.requested_features = features;
        self.requested_limits = limits;
        true
    }

    pub(crate) fn add_wgpu_features(&mut self, features: wgpu::Features) -> bool {
        if !self.devices.is_empty() {
            return false;
        }
        self.requested_features |= features;
        true
    }

    pub(crate) fn set_wgpu_limits(&mut self, limits: WgpuLimits) -> bool {
        if !self.devices.is_empty() {
            return false;
        }
        self.requested_limits = limits;
        true
    }

    /// Creates a new surface for the specified window and dimensions.
    pub(crate) async fn create_surface<'w>(
        &mut self,
        window: impl Into<wgpu::SurfaceTarget<'w>>,
        width: u32,
        height: u32,
        present_mode: PresentMode,
    ) -> Result<RenderSurface<'w>, RenderSurfaceError> {
        self.create_render_surface(
            self.instance
                .create_surface(window.into())
                .map_err(RenderSurfaceError::CreateSurface)?,
            width,
            height,
            present_mode,
        )
        .await
    }

    /// Creates a new render surface for the specified window and dimensions.
    pub(crate) async fn create_render_surface<'w>(
        &mut self,
        surface: Surface<'w>,
        width: u32,
        height: u32,
        present_mode: PresentMode,
    ) -> Result<RenderSurface<'w>, RenderSurfaceError> {
        let dev_id = self
            .device(Some(&surface))
            .await
            .ok_or(RenderSurfaceError::NoCompatibleDevice)?;

        let device_handle = &self.devices[dev_id];
        let capabilities = surface.get_capabilities(&device_handle.adapter);
        let format = capabilities
            .formats
            .into_iter()
            .find(|it| matches!(it, TextureFormat::Rgba8Unorm | TextureFormat::Bgra8Unorm))
            .ok_or(RenderSurfaceError::UnsupportedSurfaceFormat)?;

        const PREMUL_BLEND_STATE: BlendState = BlendState {
            alpha: BlendComponent::REPLACE,
            color: BlendComponent {
                src_factor: BlendFactor::SrcAlpha,
                dst_factor: BlendFactor::Zero,
                operation: wgpu::BlendOperation::Add,
            },
        };
        // TODO: check if the window is transparent then set alpha_mode accordingly
        // also, Opaque mode may help in saving power.
        // blocked on winit not exposing a way to check for transparency
        let (alpha_mode, blitter) = if capabilities
            .alpha_modes
            .contains(&CompositeAlphaMode::PostMultiplied)
        {
            (
                CompositeAlphaMode::PostMultiplied,
                TextureBlitter::new(&device_handle.device, format),
            )
        } else if capabilities
            .alpha_modes
            .contains(&CompositeAlphaMode::PreMultiplied)
        {
            (
                CompositeAlphaMode::PreMultiplied,
                TextureBlitterBuilder::new(&device_handle.device, format)
                    .blend_state(PREMUL_BLEND_STATE)
                    .build(),
            )
        } else {
            // TODO: check if the only available mode is Inherit then log info that postmultipled blit is being used
            // TODO: check if non-opaque base color is used on unsupported device then warn
            let texture_blitter =
                if cfg!(windows) && device_handle.adapter.get_info().name.contains("AMD") {
                    tracing::info!(
                        "on Windows with AMD GPUs use premultiplied blitting even on opaque surface"
                    );
                    TextureBlitterBuilder::new(&device_handle.device, format)
                        .blend_state(PREMUL_BLEND_STATE)
                        .build()
                } else {
                    TextureBlitter::new(&device_handle.device, format)
                };
            (CompositeAlphaMode::Auto, texture_blitter)
        };

        let config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
        };
        let (target_texture, target_view) = create_targets(width, height, &device_handle.device);
        let external_compositor = ExternalCompositor::new(&device_handle.device);

        let surface = RenderSurface {
            surface,
            config,
            dev_id,
            format,
            target_texture,
            target_view,
            blitter,
            external_compositor,
            resizing: false,
        };
        self.configure_surface(&surface);
        Ok(surface)
    }

    /// Resizes the surface to the new dimensions.
    pub(crate) fn resize_surface(&self, surface: &mut RenderSurface<'_>, width: u32, height: u32) {
        let (texture, view) = create_targets(width, height, &self.devices[surface.dev_id].device);
        // TODO: Use clever resize semantics to avoid thrashing the memory allocator during a resize
        // especially important on metal.
        surface.target_texture = texture;
        surface.target_view = view;
        surface.config.width = width;
        surface.config.height = height;
        self.configure_surface(surface);
    }

    /// Handles changes of the window resizing state for a surface.
    ///
    /// On macOS/Metal, interactive window resizing is driven by `CoreAnimation` transactions. During
    /// that phase, enabling `present_with_transaction` avoids visible jitter.
    #[cfg(target_os = "macos")]
    pub(crate) fn on_window_resize_state_change(
        &self,
        surface: &mut RenderSurface<'_>,
        resizing: bool,
    ) {
        if surface.resizing == resizing {
            return;
        }

        #[allow(
            unsafe_code,
            reason = "We only mutate a backend-specific flag on the Metal HAL surface when the runtime backend matches Metal"
        )]
        unsafe {
            if let Some(hal_surface) = surface.surface.as_hal::<::wgpu::hal::api::Metal>() {
                let guard = hal_surface.render_layer().lock();
                guard.set_presents_with_transaction(resizing);
            }
        }

        // The flag affects presentation behavior and must be applied via reconfigure.
        self.configure_surface(surface);

        surface.resizing = resizing;
    }

    pub(crate) fn set_present_mode(
        &self,
        surface: &mut RenderSurface<'_>,
        present_mode: PresentMode,
    ) {
        surface.config.present_mode = present_mode;
        self.configure_surface(surface);
    }

    fn configure_surface(&self, surface: &RenderSurface<'_>) {
        let device = &self.devices[surface.dev_id].device;
        surface.surface.configure(device, &surface.config);
    }

    /// Finds or creates a compatible device handle id.
    pub(crate) async fn device(
        &mut self,
        compatible_surface: Option<&Surface<'_>>,
    ) -> Option<usize> {
        let compatible = match compatible_surface {
            Some(s) => self
                .devices
                .iter()
                .enumerate()
                .find(|(_, d)| d.adapter.is_surface_supported(s))
                .map(|(i, _)| i),
            None => (!self.devices.is_empty()).then_some(0),
        };
        if compatible.is_none() {
            return self.new_device(compatible_surface).await;
        }
        compatible
    }

    /// Creates a compatible device handle id.
    async fn new_device(&mut self, compatible_surface: Option<&Surface<'_>>) -> Option<usize> {
        let adapter =
            wgpu::util::initialize_adapter_from_env_or_default(&self.instance, compatible_surface)
                .await
                .ok()?;
        let supported_features = adapter.features();
        let required_limits = match &self.requested_limits {
            WgpuLimits::Default => wgpu::Limits::default(),
            WgpuLimits::Adapter => adapter.limits(),
            WgpuLimits::Custom(limits) => *limits.clone(),
        };

        let requested_features = wgpu::Features::CLEAR_TEXTURE | self.requested_features;
        #[cfg(feature = "tracy")]
        let requested_features =
            requested_features | wgpu_profiler::GpuProfiler::ALL_WGPU_TIMER_FEATURES;

        let required_features = supported_features & requested_features;

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: None,
                required_features,
                required_limits,
                memory_hints: MemoryHints::default(),
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
            })
            .await
            .ok()?;
        let device_handle = DeviceHandle {
            adapter,
            device,
            queue,
        };
        self.devices.push(device_handle);
        Some(self.devices.len() - 1)
    }
}

/// Masonry renders into an intermediate texture because surface textures are not suitable for all
/// backend paths directly.
///
/// The Vello path needs storage binding for compute-based rendering, while the Vello Hybrid path
/// renders through a color attachment. We therefore provision one intermediate texture that
/// supports both backends and then blit to the surface.
fn create_targets(width: u32, height: u32, device: &Device) -> (Texture, TextureView) {
    let target_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        usage: TextureUsages::STORAGE_BINDING
            | TextureUsages::TEXTURE_BINDING
            | TextureUsages::COPY_DST
            | TextureUsages::RENDER_ATTACHMENT,
        format: TextureFormat::Rgba8Unorm,
        view_formats: &[],
    });
    let target_view = target_texture.create_view(&wgpu::TextureViewDescriptor::default());
    (target_texture, target_view)
}

/// Composites a host-supplied wgpu texture into the window's target texture at
/// a given viewport rect, after Masonry's scene render and before the surface
/// blit. This realizes `VisualLayerKind::External` placeholder layers (e.g. an
/// imported Servo/GPU frame). Built once per surface; the pipeline is keyed to
/// the `create_targets` format (`Rgba8Unorm`).
pub(crate) struct ExternalCompositor {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl ExternalCompositor {
    fn new(device: &Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("external-composite-shader"),
            source: wgpu::ShaderSource::Wgsl(
                r#"
                struct VsOut {
                    @builtin(position) pos: vec4<f32>,
                    @location(0) uv: vec2<f32>,
                };
                @vertex
                fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
                    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
                    var out: VsOut;
                    out.uv = uv;
                    out.pos = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
                    return out;
                }
                @group(0) @binding(0) var src_tex: texture_2d<f32>;
                @group(0) @binding(1) var src_smp: sampler;
                @fragment
                fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
                    return textureSample(src_tex, src_smp, in.uv);
                }
                "#
                .into(),
            ),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("external-composite-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
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

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("external-composite-sampler"),
            compare: None,
            border_color: None,
            lod_min_clamp: 0.0,
            lod_max_clamp: 0.0,
            anisotropy_clamp: 1,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("external-composite-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("external-composite-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            bind_group_layout,
            sampler,
        }
    }

    /// Draw `source` into `target_view` within `viewport` (physical px,
    /// `[x, y, w, h]`), preserving existing content elsewhere.
    pub(crate) fn composite(
        &self,
        device: &Device,
        queue: &wgpu::Queue,
        target_view: &TextureView,
        source: &Texture,
        viewport: [f32; 4],
    ) {
        let source_view = source.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("external-composite-bind-group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&source_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("external-composite-encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("external-composite-pass"),
                timestamp_writes: None,
                occlusion_query_set: None,
                depth_stencil_attachment: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                multiview_mask: None,
            });
            pass.set_viewport(
                viewport[0],
                viewport[1],
                viewport[2],
                viewport[3],
                0.0,
                1.0,
            );
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit(std::iter::once(encoder.finish()));
    }
}

/// Combination of surface and its configuration.
pub(crate) struct RenderSurface<'s> {
    pub surface: Surface<'s>,
    pub config: SurfaceConfiguration,
    pub dev_id: usize,
    pub format: TextureFormat,
    pub target_texture: Texture,
    pub target_view: TextureView,
    pub blitter: TextureBlitter,
    pub external_compositor: ExternalCompositor,
    pub resizing: bool,
}

impl std::fmt::Debug for RenderSurface<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderSurface")
            .field("surface", &self.surface)
            .field("config", &self.config)
            .field("dev_id", &self.dev_id)
            .field("format", &self.format)
            .field("target_texture", &self.target_texture)
            .field("target_view", &self.target_view)
            .field("blitter", &"(Not Debug)")
            .field("resizing", &self.resizing)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requested_features_are_intersected_with_adapter_features() {
        let supported = wgpu::Features::CLEAR_TEXTURE;
        let options_features = wgpu::Features::TIMESTAMP_QUERY;

        let requested_features = wgpu::Features::CLEAR_TEXTURE | options_features;
        let required_features = supported & requested_features;
        assert_eq!(required_features, wgpu::Features::CLEAR_TEXTURE);
    }
}
