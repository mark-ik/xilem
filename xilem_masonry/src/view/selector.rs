// Copyright 2026 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

use std::marker::PhantomData;

use masonry::widgets::{self, SelectionChanged};

use crate::core::{MessageCtx, MessageResult, Mut, View, ViewMarker};
use crate::{Pod, ViewCtx};

/// A drop-down selector (combo box): shows the currently `selected` option,
/// and on click pops a menu of `options` as an overlay layer. Picking an
/// option calls `callback` with its index.
///
/// Corresponds to the Masonry [`Selector`](masonry::widgets::Selector) widget,
/// whose menu renders through a [`SelectorMenu`](masonry::layers::SelectorMenu)
/// layer, so it floats over sibling content rather than expanding inline.
///
/// # Example
/// ```ignore
/// use xilem::view::selector;
/// # use xilem::WidgetView;
/// struct State { options: Vec<String>, picked: usize }
/// # fn view(state: &mut State) -> impl WidgetView<State> {
/// selector(state.options.clone(), state.picked, |state: &mut State, idx: usize| {
///     state.picked = idx;
/// })
/// # }
/// ```
pub fn selector<F, State, Action>(
    options: Vec<String>,
    selected: usize,
    callback: F,
) -> Selector<State, Action, F>
where
    F: Fn(&mut State, usize) -> Action + Send + 'static,
    State: 'static,
{
    Selector {
        options,
        selected,
        callback,
        phantom: PhantomData,
    }
}

/// The [`View`] created by [`selector`].
///
/// See `selector` documentation for more context.
#[must_use = "View values do nothing unless provided to Xilem."]
pub struct Selector<State, Action, F> {
    options: Vec<String>,
    selected: usize,
    callback: F,
    phantom: PhantomData<fn(State) -> Action>,
}

/// Clamp a selection index to a valid option (or 0 for an empty list).
fn clamp(idx: usize, len: usize) -> usize {
    if len == 0 { 0 } else { idx.min(len - 1) }
}

impl<State, Action, F> ViewMarker for Selector<State, Action, F> {}
impl<F, State, Action> View<State, Action, ViewCtx> for Selector<State, Action, F>
where
    State: 'static,
    Action: 'static,
    F: Fn(&mut State, usize) -> Action + Send + Sync + 'static,
{
    type Element = Pod<widgets::Selector>;
    type ViewState = ();

    fn build(&self, ctx: &mut ViewCtx, _: &mut State) -> (Self::Element, Self::ViewState) {
        let selected = clamp(self.selected, self.options.len());
        let element = ctx.with_action_widget(|ctx| {
            ctx.create_pod(
                widgets::Selector::new(self.options.clone()).with_selected_option(selected),
            )
        });
        (element, ())
    }

    fn rebuild(
        &self,
        prev: &Self,
        (): &mut Self::ViewState,
        _ctx: &mut ViewCtx,
        mut element: Mut<'_, Self::Element>,
        _: &mut State,
    ) {
        // `set_options` resets the selection to 0, so always re-apply the
        // selection afterwards (and whenever it changes on its own).
        if prev.options != self.options {
            widgets::Selector::set_options(&mut element, self.options.clone());
        }
        if prev.options != self.options || prev.selected != self.selected {
            widgets::Selector::select_option(&mut element, clamp(self.selected, self.options.len()));
        }
    }

    fn teardown(
        &self,
        (): &mut Self::ViewState,
        ctx: &mut ViewCtx,
        element: Mut<'_, Self::Element>,
    ) {
        ctx.teardown_action_source(element);
    }

    fn message(
        &self,
        (): &mut Self::ViewState,
        message: &mut MessageCtx,
        _element: Mut<'_, Self::Element>,
        app_state: &mut State,
    ) -> MessageResult<Action> {
        debug_assert!(
            message.remaining_path().is_empty(),
            "id path should be empty in Selector::message"
        );
        match message.take_message::<SelectionChanged>() {
            Some(change) => MessageResult::Action((self.callback)(app_state, change.index)),
            None => {
                tracing::error!("Wrong message type in Selector::message, got {message:?}.");
                MessageResult::Stale
            }
        }
    }
}
