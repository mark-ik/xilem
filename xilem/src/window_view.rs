// Copyright 2025 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use masonry::core::DefaultProperties;
use masonry::{theme::BACKGROUND_COLOR, util::debug_panic};
use masonry_winit::app::{NewWindow, Window, WindowId};

use crate::core::{MessageCtx, Mut, View, ViewElement, ViewMarker};
use crate::{
    AnyWidgetView, Color, InitialRootWidget, MasonryRoot, ViewCtx, WidgetView, WindowOptions,
};

/// A view representing a window.
pub struct WindowView<State: 'static> {
    pub(crate) id: WindowId,
    pub(crate) options: WindowOptions<State>,
    pub(crate) masonry_root: MasonryRoot<State>,
    /// The base color of the window.
    pub(crate) base_color: Option<Color>,
    /// Tree-wide default properties. `None` keeps whatever the app
    /// builder set at startup. When `Some` and the `Arc` identity
    /// changes between rebuilds, it is pushed to the render root —
    /// enabling runtime theme swaps. Compared by `Arc` pointer, so a
    /// host should cache the `Arc` and only rebuild it when the theme
    /// actually changes (else every frame re-applies).
    pub(crate) default_properties: Option<Arc<DefaultProperties>>,
}

pub(crate) type WindowViewState = <Box<AnyWidgetView<(), ()>> as View<(), (), ViewCtx>>::ViewState;

/// A view representing a window.
///
/// `id` can be created using the [`WindowId::next()`] method and _must_ be the
/// same each frame for the same window. Usually it should be stored in app
/// state somewhere.
///
/// `title` initializes [`WindowOptions`].
pub fn window<V: WidgetView<State>, State: 'static>(
    id: WindowId,
    title: impl Into<String>,
    root_view: V,
) -> WindowView<State> {
    WindowView {
        id,
        options: WindowOptions::new(title),
        masonry_root: MasonryRoot::new(root_view),
        base_color: None,
        default_properties: None,
    }
}

impl<State> WindowView<State> {
    /// Modify window options in-place.
    pub fn with_options(
        mut self,
        f: impl FnOnce(WindowOptions<State>) -> WindowOptions<State>,
    ) -> Self {
        self.options = f(self.options);
        self
    }

    /// Set base color of the window.
    ///
    /// This is [`masonry::theme::BACKGROUND_COLOR`] by default.
    pub fn with_base_color(mut self, color: Color) -> Self {
        self.base_color = Some(color);
        self
    }

    /// Set the tree-wide default properties reactively.
    ///
    /// Pass the same `Arc` each frame and only swap it when the theme
    /// changes; on swap the new set is applied to the live render root
    /// and every widget repaints. `None` (the default) leaves the
    /// startup set untouched.
    pub fn with_default_properties(mut self, default_properties: Arc<DefaultProperties>) -> Self {
        self.default_properties = Some(default_properties);
        self
    }
}

/// A newtype wrapper around [`NewWindow`] for implementing [`ViewElement`].
pub struct PodWindow(pub NewWindow);

impl ViewElement for PodWindow {
    type Mut<'a> = &'a mut Window;
}

impl<State> ViewMarker for WindowView<State> where State: 'static {}

// TODO: Reconsider how this works.
// There are *reasonable* arguments for making this be `View<()>`, i.e. the root state is just another local.
impl<State> View<State, (), ViewCtx> for WindowView<State> {
    type Element = PodWindow;

    type ViewState = WindowViewState;

    fn build(&self, ctx: &mut ViewCtx, app_state: &mut State) -> (Self::Element, Self::ViewState) {
        let (InitialRootWidget(root_widget), view_state) = self.masonry_root.build(ctx, app_state);
        let initial_attributes = self.options.build_initial_attrs();
        let base_color = self.base_color.unwrap_or_else(|| {
            debug_panic!("base_color should be set already in `MasonryDriver::build_window`");
            BACKGROUND_COLOR
        });
        (
            PodWindow(
                NewWindow::new_with_id(
                    self.id,
                    initial_attributes,
                    root_widget.new_widget.erased(),
                )
                .with_base_color(base_color),
            ),
            view_state,
        )
    }

    fn rebuild(
        &self,
        prev: &Self,
        root_widget_view_state: &mut Self::ViewState,
        ctx: &mut ViewCtx,
        window: Mut<'_, Self::Element>,
        app_state: &mut State,
    ) {
        self.options.rebuild(&prev.options, window.handle());
        if self.base_color != prev.base_color
            && let Some(base_color) = self.base_color
        {
            *window.base_color() = base_color;
        }

        // Runtime default-properties swap: apply only when the `Arc`
        // identity changed (a host caches it and rebuilds on theme
        // change), so steady-state frames don't re-apply.
        if let Some(props) = &self.default_properties {
            let changed = match &prev.default_properties {
                Some(prev_props) => !Arc::ptr_eq(prev_props, props),
                None => true,
            };
            if changed {
                window.render_root().set_default_properties(props.clone());
            }
        }

        self.masonry_root.rebuild(
            &prev.masonry_root,
            root_widget_view_state,
            ctx,
            window.render_root(),
            app_state,
        );
    }

    fn teardown(
        &self,
        view_state: &mut Self::ViewState,
        ctx: &mut ViewCtx,
        window: Mut<'_, Self::Element>,
    ) {
        self.masonry_root
            .teardown(view_state, ctx, window.render_root());
    }

    fn message(
        &self,
        view_state: &mut Self::ViewState,
        message: &mut MessageCtx,
        window: Mut<'_, Self::Element>,
        app_state: &mut State,
    ) -> xilem_core::MessageResult<()> {
        self.masonry_root
            .message(view_state, message, window.render_root(), app_state)
    }
}

impl<State> WindowView<State>
where
    State: 'static,
{
    pub(crate) fn on_close(&self, state: &mut State) {
        if let Some(on_close) = &self.options.callbacks.on_close {
            on_close(state);
        }
    }
}
