// Copyright 2024 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

use std::marker::PhantomData;

use masonry::{kurbo::Rect, widgets};

use crate::core::{MessageContext, Mut, ViewMarker};
use crate::{MessageResult, Pod, View, ViewCtx, WidgetView};

/// A view which puts `child` into a scrollable region.
///
/// This corresponds to the Masonry [`Portal`](masonry::widgets::Portal) widget.
pub fn portal<Child, State, Action>(child: Child) -> Portal<Child, State, Action>
where
    Child: WidgetView<State, Action>,
{
    Portal {
        child,
        reveal_target: None,
        horizontal_anchor: None,
        horizontal_scrollbar_thumb_widths: None,
        phantom: PhantomData,
    }
}

/// The [`View`] created by [`portal`].
#[must_use = "View values do nothing unless provided to Xilem."]
pub struct Portal<V, State, Action> {
    child: V,
    reveal_target: Option<Rect>,
    horizontal_anchor: Option<(f64, u64)>,
    horizontal_scrollbar_thumb_widths: Option<(f64, f64)>,
    phantom: PhantomData<(State, Action)>,
}

impl<V, State, Action> Portal<V, State, Action> {
    /// Reveal the given child-coordinate rectangle on initial layout and when
    /// the target changes on a later rebuild.
    pub fn reveal_target(mut self, target: Option<Rect>) -> Self {
        self.reveal_target = target;
        self
    }

    /// Preserve a child-coordinate X position across relayouts.
    /// The revision forces a fresh capture after an explicit anchoring gesture.
    pub fn horizontal_anchor(mut self, anchor: Option<(f64, u64)>) -> Self {
        self.horizontal_anchor = anchor;
        self
    }

    /// Keep a generous scrollbar hit target while drawing a slimmer horizontal thumb.
    pub fn horizontal_scrollbar_thumb_widths(mut self, idle: f64, active: f64) -> Self {
        let idle = idle.max(1.0);
        self.horizontal_scrollbar_thumb_widths = Some((idle, active.max(idle)));
        self
    }
}

impl<V, State, Action> ViewMarker for Portal<V, State, Action> {}
impl<Child, State, Action> View<State, Action, ViewCtx> for Portal<Child, State, Action>
where
    Child: WidgetView<State, Action>,
    State: 'static,
    Action: 'static,
{
    type Element = Pod<widgets::Portal<Child::Widget>>;
    type ViewState = Child::ViewState;

    fn build(&self, ctx: &mut ViewCtx, app_state: &mut State) -> (Self::Element, Self::ViewState) {
        // The Portal `View` doesn't get any messages directly (yet - scroll events?), so doesn't need to
        // use ctx.with_id.
        let (child, child_state) = self.child.build(ctx, app_state);
        let mut widget = widgets::Portal::new(child.new_widget);
        if let Some(target) = self.reveal_target {
            widget = widget.initial_pan_to(target);
        }
        if let Some((anchor, _revision)) = self.horizontal_anchor {
            widget = widget.horizontal_anchor(anchor);
        }
        if let Some((idle, active)) = self.horizontal_scrollbar_thumb_widths {
            widget = widget.horizontal_scrollbar_thumb_widths(idle, active);
        }
        let widget_pod = ctx.create_pod(widget);
        (widget_pod, child_state)
    }

    fn rebuild(
        &self,
        prev: &Self,
        view_state: &mut Self::ViewState,
        ctx: &mut ViewCtx,
        mut element: Mut<'_, Self::Element>,
        app_state: &mut State,
    ) {
        if self.reveal_target != prev.reveal_target
            && let Some(target) = self.reveal_target
        {
            widgets::Portal::set_pending_pan_to(&mut element, target);
        }
        if self.horizontal_anchor != prev.horizontal_anchor {
            widgets::Portal::set_horizontal_anchor(
                &mut element,
                self.horizontal_anchor.map(|(anchor, _revision)| anchor),
            );
        }
        if self.horizontal_scrollbar_thumb_widths != prev.horizontal_scrollbar_thumb_widths {
            widgets::Portal::set_horizontal_scrollbar_thumb_widths(
                &mut element,
                self.horizontal_scrollbar_thumb_widths,
            );
        }
        let child_element = widgets::Portal::child_mut(&mut element);
        self.child
            .rebuild(&prev.child, view_state, ctx, child_element, app_state);
    }

    fn teardown(
        &self,
        view_state: &mut Self::ViewState,
        ctx: &mut ViewCtx,
        mut element: Mut<'_, Self::Element>,
    ) {
        let child_element = widgets::Portal::child_mut(&mut element);
        self.child.teardown(view_state, ctx, child_element);
    }

    fn message(
        &self,
        view_state: &mut Self::ViewState,
        message: &mut MessageContext,
        mut element: Mut<'_, Self::Element>,
        app_state: &mut State,
    ) -> MessageResult<Action> {
        let child_element = widgets::Portal::child_mut(&mut element);
        self.child
            .message(view_state, message, child_element, app_state)
    }
}
