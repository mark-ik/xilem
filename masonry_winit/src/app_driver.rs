// Copyright 2024 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

use std::fmt::Debug;
use std::hash::Hash;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};

use masonry_core::app::RenderRoot;
use masonry_core::core::{ErasedAction, WidgetId};
use tracing::field::DisplayValue;
use winit::event_loop::ActiveEventLoop;

use crate::app::MasonryState;
use crate::event_loop_runner::{NewWindow, Window};

/// A unique and persistent identifier for a window.
///
/// [`MasonryState`] internally maps these to winit window ids ([`winit::window::WindowId`]).
/// Applications should only use this struct and not be concerned with the winit window ids.
/// When the application is suspended and resumed this id will stay the same, while the
/// winit window id will change.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct WindowId(pub(crate) NonZeroU64);

impl WindowId {
    /// Allocate a new, unique `WindowId`.
    pub fn next() -> Self {
        static WINDOW_ID_COUNTER: AtomicU64 = AtomicU64::new(1);
        let id = WINDOW_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
        Self(id.try_into().unwrap())
    }

    /// A serialized representation of the `WindowId` for debugging purposes.
    pub fn trace(self) -> DisplayValue<NonZeroU64> {
        tracing::field::display(self.0)
    }
}

/// Context for the [`AppDriver`] trait.
#[derive(Debug)]
pub struct DriverCtx<'a, 's> {
    state: &'a mut MasonryState<'s>,
    event_loop: &'a ActiveEventLoop,
}

impl<'a, 's> DriverCtx<'a, 's> {
    pub(crate) fn new(state: &'a mut MasonryState<'s>, event_loop: &'a ActiveEventLoop) -> Self {
        Self { state, event_loop }
    }
}

/// Access to Masonry's WGPU device state.
///
/// This is provided via [`AppDriver::on_wgpu_ready`] so applications can create GPU resources
/// (textures, pipelines, etc.) using the same `Device`/`Queue` as Masonry.
#[derive(Debug)]
pub struct WgpuContext<'a> {
    /// The WGPU instance used by Masonry.
    pub instance: &'a wgpu::Instance,
    /// The WGPU adapter used to create the device.
    pub adapter: &'a wgpu::Adapter,
    /// The shared WGPU device.
    pub device: &'a wgpu::Device,
    /// The shared WGPU queue.
    pub queue: &'a wgpu::Queue,
}

/// One `VisualLayerKind::External` layer to be realized by the embedder,
/// in painter order. Its content is drawn into [`Self::bounds`] on the
/// surface during [`AppDriver::composite_external_layers`].
#[derive(Clone, Copy, Debug)]
pub struct ExternalLayer {
    /// The widget that requested the external layer (via
    /// `RenderRootOptions`/`push_external_layer`). The embedder uses this
    /// to decide which content belongs in this region.
    pub widget_id: WidgetId,
    /// Destination on the surface in physical pixels, `[x, y, width, height]`.
    pub bounds: [u32; 4],
}

/// Per-frame access for compositing externally-realized layers
/// (`VisualLayerKind::External`) onto the window surface.
///
/// Masonry calls [`AppDriver::composite_external_layers`] after rendering
/// its own widget content into [`Self::target_texture`] and before
/// presenting. The embedder draws each [`ExternalLayer`]'s content —
/// rendered on the **shared** [`Self::device`] (the same device handed
/// out by [`AppDriver::on_wgpu_ready`]) — into the layer's bounds, e.g.
/// via `copy_texture_to_texture` from a content texture. Because the
/// device is shared, this needs no GPU→CPU readback.
///
/// `target_texture` is `Rgba8Unorm` with `COPY_DST` + `RENDER_ATTACHMENT`
/// usage; Masonry blits it to the swapchain on present, so anything
/// composited here appears in the frame.
pub struct ExternalCompositeCtx<'a> {
    /// The shared WGPU device (same as [`WgpuContext::device`]).
    pub device: &'a wgpu::Device,
    /// The shared WGPU queue.
    pub queue: &'a wgpu::Queue,
    /// Masonry's intermediate render target. Composite content into here.
    pub target_texture: &'a wgpu::Texture,
    /// A default view of [`Self::target_texture`].
    pub target_view: &'a wgpu::TextureView,
    /// Surface size in physical pixels, `(width, height)`.
    pub surface_size: (u32, u32),
    /// External layers to realize this frame, in painter order.
    pub layers: &'a [ExternalLayer],
    /// The host window backing this surface. Embedders that realize an external
    /// layer with a child surface (e.g. a system WebView via `scrying`) need the
    /// platform window handle (HWND / NSView / etc.) as the parent; the device
    /// alone is not enough. Use [`raw_window_handle`] on this to obtain it.
    pub window: &'a winit::window::Window,
}

/// Per-tick access handed to [`AppDriver::on_tick`], which runs on the main
/// thread **outside** any `render()` pass (forwarded from winit's
/// `about_to_wait`). This is the safe place to drive work that pumps the OS
/// message loop — e.g. constructing or navigating a system WebView (`scrying`),
/// which would re-enter `render()` if done from
/// [`AppDriver::composite_external_layers`].
///
/// `device`/`queue` are the shared WGPU device (the same ones handed to
/// [`AppDriver::on_wgpu_ready`] / [`ExternalCompositeCtx`]); they are `None`
/// until the first frame creates the device. `windows` exposes the open windows
/// for parent-handle access (`raw_window_handle`) and `request_redraw`.
pub struct TickCtx<'a> {
    /// The shared WGPU device, once created (`None` before the first frame).
    pub device: Option<&'a wgpu::Device>,
    /// The shared WGPU queue, once created.
    pub queue: Option<&'a wgpu::Queue>,
    /// Currently-open windows, in arbitrary order. Use for parent-window
    /// handles and to `request_redraw` when new external content is ready.
    pub windows: &'a [&'a winit::window::Window],
}

/// Strategy for selecting `wgpu::Limits` when requesting the WGPU device.
#[derive(Clone, Debug, Default)]
pub enum WgpuLimits {
    /// Use `wgpu::Limits::default()`.
    #[default]
    Default,
    /// Use `adapter.limits()` (maximum supported by the selected adapter).
    Adapter,
    /// Use the provided limits.
    Custom(Box<wgpu::Limits>),
}

/// A trait for defining how your app interacts with the Masonry widget tree.
///
/// When launching your app with [`crate::app::run`], you need to provide
/// a type that implements this trait.
#[expect(unused_variables, reason = "Default impls doesn't use arguments")]
pub trait AppDriver {
    /// A hook which will be executed when a widget emits an `action`.
    ///
    /// This action is type-erased, and the type of action emitted will depend on.
    /// Each widget should document which types of action it might emit.
    fn on_action(
        &mut self,
        window_id: WindowId,
        ctx: &mut DriverCtx<'_, '_>,
        widget_id: WidgetId,
        action: ErasedAction,
    );

    /// A hook which will be executed for async actions sent outside the widget tree.
    ///
    /// This is called when the winit event loop gets a [`MasonryUserEvent::AsyncAction`] event.
    ///
    /// [`MasonryUserEvent::AsyncAction`]: crate::app::MasonryUserEvent::AsyncAction
    fn on_async_action(
        &mut self,
        window_id: WindowId,
        ctx: &mut DriverCtx<'_, '_>,
        action: ErasedAction,
    ) {
    }

    /// A hook which will be executed when the application starts, to allow initial configuration of the `MasonryState`.
    ///
    /// Use cases include loading fonts.
    ///
    /// There are circumstances under which this will be called multiple times during the lifecycle of your app.
    /// This is not intended to be the behaviour of Masonry Winit long-term, but this method should currently
    /// not assume it will only be called once (but should feel free to waste work if it is called multiple times,
    /// for example, as the mentioned circumstances are very rare).
    // TODO: Turn into something like on window created, or split into two.
    fn on_start(&mut self, state: &mut MasonryState<'_>) {}

    /// A hook called when a user has requested to close a window.
    fn on_close_requested(&mut self, window_id: WindowId, ctx: &mut DriverCtx<'_, '_>) {
        ctx.exit();
    }

    /// Called when Masonry has created its WGPU device.
    fn on_wgpu_ready(&mut self, _wgpu: &WgpuContext<'_>) {}

    /// Called each frame, after Masonry renders its widget content and
    /// before present, when the frame contains `VisualLayerKind::External`
    /// layers. The embedder composites each layer's externally-realized
    /// content (rendered on the shared device) into its bounds on
    /// [`ExternalCompositeCtx::target_texture`]. Default: no-op (the
    /// layers are left as reserved holes).
    fn composite_external_layers(&mut self, _ctx: &mut ExternalCompositeCtx<'_>) {}

    /// Called on the main thread once per event-loop iteration (forwarded from
    /// winit's `about_to_wait`), **outside** any `render()` pass. The safe place
    /// to drive work that pumps the OS message loop — constructing/navigating a
    /// system WebView, polling it for frames — which must *not* happen inside
    /// [`Self::composite_external_layers`] (that runs inside `render()`; pumping
    /// there re-enters the event loop and thus `render()`). Default: no-op.
    fn on_tick(&mut self, _ctx: &mut TickCtx<'_>) {}
}

impl DriverCtx<'_, '_> {
    // TODO - Add method to create timer

    /// Access the [`RenderRoot`] of the given window.
    ///
    /// # Panics
    ///
    /// Panics if the window cannot be found.
    pub fn render_root(&mut self, window_id: WindowId) -> &mut RenderRoot {
        &mut self.window(window_id).render_root
    }

    /// Access the [`Window`] state of the given window.
    ///
    /// # Panics
    ///
    /// Panics if the window cannot be found.
    pub fn window(&mut self, window_id: WindowId) -> &mut Window {
        self.state.window_mut(window_id)
    }

    /// Creates a new window.
    ///
    /// # Panics
    ///
    /// Panics if the window id is already used by another window.
    pub fn create_window(&mut self, new_window: NewWindow) {
        self.state.create_window(self.event_loop, new_window);
    }

    /// Closes the given window.
    ///
    /// # Panics
    ///
    /// Panics if the window cannot be found.
    pub fn close_window(&mut self, window_id: WindowId) {
        self.state.close_window(window_id);
    }

    /// Exits the application (stops the event loop).
    pub fn exit(&mut self) {
        self.state.exit = true;
    }
}
