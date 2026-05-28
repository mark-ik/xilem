// Copyright 2025 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

use std::marker::PhantomData;

use masonry::widgets::{self, ExternalSurfaceSizeChanged};

use crate::core::{MessageCtx, MessageResult, Mut, View, ViewMarker};
use crate::{Pod, ViewCtx};

/// A region that fills its parent and is reserved for host-composited external
/// content (e.g. an imported GPU texture) rather than Masonry-drawn content.
///
/// The host reads the resulting `External` placeholder layer's bounds and
/// composites its content there (see `MasonryState::set_external_texture`).
pub fn external_surface<State>() -> ExternalSurface<State>
where
    State: 'static,
{
    ExternalSurface {
        phantom: PhantomData,
    }
}

/// The [`View`] created by [`external_surface`].
#[must_use = "View values do nothing unless provided to Xilem."]
pub struct ExternalSurface<State> {
    phantom: PhantomData<fn() -> State>,
}

impl<State> ViewMarker for ExternalSurface<State> {}

impl<State, Action> View<State, Action, ViewCtx> for ExternalSurface<State>
where
    State: 'static,
{
    type Element = Pod<widgets::ExternalSurface>;
    type ViewState = ();

    fn build(&self, ctx: &mut ViewCtx, _: &mut State) -> (Self::Element, Self::ViewState) {
        (
            ctx.with_action_widget(|ctx| ctx.create_pod(widgets::ExternalSurface::default())),
            (),
        )
    }

    fn rebuild(
        &self,
        _prev: &Self,
        (): &mut Self::ViewState,
        _ctx: &mut ViewCtx,
        _element: Mut<'_, Self::Element>,
        _state: &mut State,
    ) {
    }

    fn teardown(&self, (): &mut Self::ViewState, _: &mut ViewCtx, _: Mut<'_, Self::Element>) {}

    fn message(
        &self,
        (): &mut Self::ViewState,
        message: &mut MessageCtx,
        _element: Mut<'_, Self::Element>,
        _app_state: &mut State,
    ) -> MessageResult<Action> {
        debug_assert!(
            message.remaining_path().is_empty(),
            "id path should be empty in ExternalSurface::message"
        );
        match message.take_message::<ExternalSurfaceSizeChanged>() {
            Some(_) => MessageResult::RequestRebuild,
            None => {
                tracing::error!("Wrong message type in ExternalSurface::message, got {message:?}.");
                MessageResult::Stale
            }
        }
    }
}
