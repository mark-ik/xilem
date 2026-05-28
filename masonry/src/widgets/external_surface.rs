// Copyright 2025 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

use accesskit::{Node, Role};
use tracing::{Span, trace_span};

use crate::core::{
    AccessCtx, ChildrenIds, LayoutCtx, MeasureCtx, PaintCtx, PaintLayerMode, PropertiesRef,
    RegisterCtx, Widget, WidgetId,
};
use crate::imaging::Painter;
use crate::kurbo::{Axis, Size};
use crate::layout::{LenReq, Length};

/// The preferred size of the surface when it has no constraint to fill.
const DEFAULT_LENGTH: Length = Length::const_px(100.);

/// A widget that reserves a region for host-composited external content.
///
/// It paints nothing of its own. Instead, during paint it marks its subtree as
/// an [`External`](PaintLayerMode::External) paint layer, so the windowing host
/// can composite externally-produced content (e.g. an imported GPU texture)
/// into the widget's bounds. The host obtains the bounds from the resulting
/// `VisualLayerKind::External` layer and supplies the content via its own
/// mechanism (e.g. `MasonryState::set_external_texture`).
///
/// The widget fills the space offered by its parent.
#[derive(Default)]
pub struct ExternalSurface {
    /// The drawable area size, which matches the widget's content-box.
    size: Size,
}

// --- MARK: METHODS
impl ExternalSurface {
    /// Returns the current size of the surface, which matches its content-box.
    pub fn size(&self) -> Size {
        self.size
    }
}

/// The size of the [`ExternalSurface`]'s content box has changed.
#[derive(Debug)]
pub struct ExternalSurfaceSizeChanged {
    /// The new size of the surface.
    pub size: Size,
}

// --- MARK: IMPL WIDGET
impl Widget for ExternalSurface {
    type Action = ExternalSurfaceSizeChanged;

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fn measure(
        &mut self,
        _ctx: &mut MeasureCtx<'_>,
        _props: &PropertiesRef<'_>,
        _axis: Axis,
        len_req: LenReq,
        _cross_length: Option<Length>,
    ) -> Length {
        // Use all the available space or fall back to our preferred size.
        match len_req {
            LenReq::FitContent(space) => space,
            _ => DEFAULT_LENGTH,
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        if self.size != size {
            self.size = size;
            ctx.submit_action::<Self::Action>(ExternalSurfaceSizeChanged { size });
        }
        ctx.set_clip_path(size.to_rect());
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, _painter: &mut Painter<'_>) {
        // Reserve this region for host-composited content; draw nothing.
        ctx.set_paint_layer_mode(PaintLayerMode::External);
    }

    fn accessibility_role(&self) -> Role {
        Role::Canvas
    }

    fn accessibility(&mut self, _ctx: &mut AccessCtx<'_>, _props: &PropertiesRef<'_>, _node: &mut Node) {}

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }

    fn make_trace_span(&self, widget_id: WidgetId) -> Span {
        trace_span!("ExternalSurface", id = widget_id.trace())
    }
}
