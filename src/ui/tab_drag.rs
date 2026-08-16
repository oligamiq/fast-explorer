use xilem::core::{MessageContext, MessageResult, Mut, View, ViewId, ViewMarker, ViewPathTracker};
use xilem::masonry::accesskit::{Action, Node, Role};
use xilem::masonry::core::{
    AccessCtx, AccessEvent, BoxConstraints, ChildrenIds, ComposeCtx, EventCtx, LayoutCtx,
    NewWidget, PaintCtx, PointerButton, PointerButtonEvent, PointerEvent, PointerId, PointerType,
    PointerUpdate, Properties, PropertiesMut, PropertiesRef, RegisterCtx, StyleProperty, TextEvent,
    Update, UpdateCtx, Widget, WidgetId, WidgetMut, WidgetPod,
};
use xilem::masonry::kurbo::{Point, Rect, Size, Vec2};
use xilem::masonry::peniko::Color;
use xilem::masonry::properties::types::AsUnit;
use xilem::masonry::properties::{
    Background, BorderColor, BorderWidth, ContentColor, CornerRadius, Padding,
};
use xilem::masonry::vello::Scene;
use xilem::masonry::widgets::{Label, SizedBox};
use xilem::{Pod, ViewCtx, WidgetView};

use crate::app::AppState;

const TAB_CONTENT_VIEW_ID: ViewId = ViewId::new(0);
const TAB_DRAG_THRESHOLD: f64 = 8.0;
const TOUCH_SCROLL_THRESHOLD: f64 = 10.0;
const TOUCH_DRAG_HOLD_NS: u64 = 350_000_000;
const HIDDEN_DRAG_CHILD_X: f64 = -100_000.0;

struct DragLayerRoot {
    child: WidgetPod<dyn Widget>,
    child_size: Size,
}

impl DragLayerRoot {
    fn new(child: NewWidget<impl Widget + ?Sized>, child_size: Size) -> Self {
        Self {
            child: child.erased().to_pod(),
            child_size,
        }
    }
}

impl Widget for DragLayerRoot {
    type Action = xilem::masonry::core::NoAction;

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        ctx.register_child(&mut self.child);
    }

    fn layout(
        &mut self,
        ctx: &mut LayoutCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        bc: &BoxConstraints,
    ) -> Size {
        let child_bc = BoxConstraints::tight(self.child_size);
        let _ = ctx.run_layout(&mut self.child, &child_bc);
        ctx.place_child(&mut self.child, Point::ORIGIN);
        bc.max()
    }

    fn paint(&mut self, _ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, _scene: &mut Scene) {}

    fn accessibility_role(&self) -> Role {
        Role::GenericContainer
    }

    fn accessibility(
        &mut self,
        _ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        _node: &mut Node,
    ) {
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::from_slice(&[self.child.id()])
    }
}

fn drag_layer_widget(
    title: &str,
    width: f64,
    height: f64,
    background: Color,
    border: Color,
    text_color: Color,
) -> NewWidget<dyn Widget> {
    let mut label_props = Properties::new();
    label_props.insert(ContentColor::new(text_color));
    let label = NewWidget::new_with_props(
        Label::new(format!("{title}   ×")).with_style(StyleProperty::FontSize(13.0)),
        label_props,
    );

    let mut box_props = Properties::new();
    box_props.insert(Background::Color(background));
    box_props.insert(BorderColor::new(border));
    box_props.insert(BorderWidth::all(1.0));
    box_props.insert(CornerRadius::all(4.0));
    box_props.insert(Padding::from_vh(5.0, 8.0));
    let tab = NewWidget::new_with_props(
        SizedBox::new(label).width(width.px()).height(height.px()),
        box_props,
    );
    NewWidget::new(DragLayerRoot::new(tab, Size::new(width, height))).erased()
}

fn tab_drop_target_index(widths: &[f64], source_index: usize, drag_offset: f64) -> usize {
    if widths.is_empty() {
        return 0;
    }
    let source_index = source_index.min(widths.len() - 1);
    let mut centers = Vec::with_capacity(widths.len());
    let mut cursor = 0.0;
    for width in widths.iter().copied() {
        centers.push(cursor + width * 0.5);
        cursor += width;
    }

    let source_center = centers[source_index];
    let dragged_center = source_center + drag_offset;
    let mut target = source_index;
    if dragged_center < source_center {
        for index in (0..source_index).rev() {
            if dragged_center < centers[index] {
                target = index;
            } else {
                break;
            }
        }
    } else if dragged_center > source_center {
        for (index, center) in centers.iter().copied().enumerate().skip(source_index + 1) {
            if dragged_center > center {
                target = index;
            } else {
                break;
            }
        }
    }
    target
}

#[derive(Debug)]
pub(super) enum TabDragAction {
    Select(u64),
    Drop { tab_id: u64, target_index: usize },
}

#[derive(Clone, PartialEq)]
pub(super) struct TabDragConfig {
    pub tab_id: u64,
    pub source_index: usize,
    pub drag_index: usize,
    pub tab_widths: Vec<f64>,
    pub drop_targets: Vec<usize>,
    pub scroll_leading: f64,
    pub drag_handle_right_inset: f64,
    pub accessibility_label: String,
    pub selected: bool,
    pub background: Color,
    pub border: Color,
    pub text_color: Color,
}

pub(super) fn tab_drag_button<V>(child: V, config: TabDragConfig) -> TabDragView<V>
where
    V: WidgetView<AppState>,
{
    TabDragView { child, config }
}

pub(super) struct TabDragView<V> {
    child: V,
    config: TabDragConfig,
}

impl<V> ViewMarker for TabDragView<V> {}

impl<V> View<AppState, (), ViewCtx> for TabDragView<V>
where
    V: WidgetView<AppState>,
{
    type Element = Pod<TabDragWidget>;
    type ViewState = V::ViewState;

    fn build(&self, ctx: &mut ViewCtx, state: &mut AppState) -> (Self::Element, Self::ViewState) {
        let (child, child_state) =
            ctx.with_id(TAB_CONTENT_VIEW_ID, |ctx| self.child.build(ctx, state));
        let widget = TabDragWidget::new(child.new_widget, self.config.clone());
        (
            ctx.with_action_widget(|ctx| ctx.create_pod(widget)),
            child_state,
        )
    }

    fn rebuild(
        &self,
        prev: &Self,
        view_state: &mut Self::ViewState,
        ctx: &mut ViewCtx,
        mut element: Mut<'_, Self::Element>,
        state: &mut AppState,
    ) {
        TabDragWidget::set_config(&mut element, self.config.clone());
        ctx.with_id(TAB_CONTENT_VIEW_ID, |ctx| {
            let mut child = TabDragWidget::child_mut(&mut element);
            self.child
                .rebuild(&prev.child, view_state, ctx, child.downcast(), state);
        });
    }

    fn teardown(
        &self,
        view_state: &mut Self::ViewState,
        ctx: &mut ViewCtx,
        mut element: Mut<'_, Self::Element>,
    ) {
        ctx.with_id(TAB_CONTENT_VIEW_ID, |ctx| {
            let mut child = TabDragWidget::child_mut(&mut element);
            self.child.teardown(view_state, ctx, child.downcast());
        });
        ctx.teardown_leaf(element);
    }

    fn message(
        &self,
        view_state: &mut Self::ViewState,
        message: &mut MessageContext,
        mut element: Mut<'_, Self::Element>,
        state: &mut AppState,
    ) -> MessageResult<()> {
        match message.take_first() {
            Some(TAB_CONTENT_VIEW_ID) => {
                let mut child = TabDragWidget::child_mut(&mut element);
                self.child
                    .message(view_state, message, child.downcast(), state)
            }
            None => match message.take_message::<TabDragAction>() {
                Some(action) => {
                    match *action {
                        TabDragAction::Select(tab_id) => state.select_tab_by_id(tab_id),
                        TabDragAction::Drop {
                            tab_id,
                            target_index,
                        } => {
                            // Keep app state stable during the gesture so Xilem
                            // cannot rebuild the tab strip underneath the drag
                            // preview. Commit reorder + activation only on drop.
                            state.move_tab_to_index(tab_id, target_index);
                            state.select_tab_by_id(tab_id);
                        }
                    }
                    MessageResult::Action(())
                }
                None => MessageResult::Stale,
            },
            _ => MessageResult::Stale,
        }
    }
}

pub(super) struct TabDragWidget {
    child: WidgetPod<dyn Widget>,
    config: TabDragConfig,
    drag_tab_id: Option<u64>,
    drag_pointer: Option<PointerId>,
    drag_start_local_x: f64,
    drag_origin_window: Point,
    drag_offset_x: f64,
    layer_root_id: Option<WidgetId>,
    pending_touch: bool,
    touch_down_time_ns: u64,
    touch_drag_armed: bool,
    touch_scroll_started: bool,
}

impl TabDragWidget {
    fn new(child: NewWidget<impl Widget + ?Sized>, config: TabDragConfig) -> Self {
        Self {
            child: child.erased().to_pod(),
            config,
            drag_tab_id: None,
            drag_pointer: None,
            drag_start_local_x: 0.0,
            drag_origin_window: Point::ORIGIN,
            drag_offset_x: 0.0,
            layer_root_id: None,
            pending_touch: false,
            touch_down_time_ns: 0,
            touch_drag_armed: false,
            touch_scroll_started: false,
        }
    }

    fn child_mut<'a>(this: &'a mut WidgetMut<'_, Self>) -> WidgetMut<'a, dyn Widget> {
        this.ctx.get_mut(&mut this.widget.child)
    }

    fn set_config(this: &mut WidgetMut<'_, Self>, config: TabDragConfig) {
        if this.widget.config != config {
            let should_reveal = config.selected
                && (!this.widget.config.selected
                    || this.widget.config.source_index != config.source_index
                    || this.widget.config.drag_index != config.drag_index
                    || this.widget.config.tab_widths != config.tab_widths
                    || this.widget.config.drop_targets != config.drop_targets
                    || (this.widget.config.scroll_leading - config.scroll_leading).abs()
                        > f64::EPSILON);
            this.widget.config = config;
            if should_reveal {
                let size = this.ctx.size();
                this.ctx.request_scroll_to(Rect::new(
                    -this.widget.config.scroll_leading,
                    0.0,
                    size.width,
                    size.height,
                ));
            }
            this.ctx.request_render();
            this.ctx.request_compose();
        }
    }

    fn drop_target_index(&self) -> usize {
        let visible_target = tab_drop_target_index(
            &self.config.tab_widths,
            self.config.drag_index,
            self.drag_offset_x,
        );
        self.config
            .drop_targets
            .get(visible_target)
            .copied()
            .unwrap_or(self.config.source_index)
    }

    fn clear_drag(&mut self) {
        self.drag_tab_id = None;
        self.drag_pointer = None;
        self.drag_offset_x = 0.0;
        self.pending_touch = false;
        self.touch_down_time_ns = 0;
        self.touch_drag_armed = false;
        self.touch_scroll_started = false;
    }
}

impl Widget for TabDragWidget {
    type Action = TabDragAction;

    fn on_pointer_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &PointerEvent,
    ) {
        match event {
            PointerEvent::Down(PointerButtonEvent {
                button,
                pointer,
                state,
                ..
            }) if button.is_none() || matches!(button, Some(PointerButton::Primary)) => {
                let local = ctx.local_position(state.position);
                let drag_limit = (ctx.size().width - self.config.drag_handle_right_inset).max(0.0);
                if local.x >= drag_limit {
                    return;
                }
                // Capture on Down (the only phase where Masonry permits capture).
                // Touch events are still allowed to bubble to the parent Portal until
                // a long-press arms reordering, so ordinary horizontal swipes scroll.
                ctx.capture_pointer();
                // Mouse/pen reordering starts after a small movement threshold.
                // Touch is intentionally different: a normal swipe belongs to the
                // horizontal Portal, while a press-and-hold arms tab reordering.
                // This is the same gesture split users expect from mobile browser
                // tab strips and prevents every horizontal swipe from moving tabs.
                self.drag_tab_id = None;
                self.drag_pointer = pointer.pointer_id;
                self.drag_start_local_x = local.x;
                self.drag_origin_window = ctx.to_window(Point::ORIGIN);
                self.drag_offset_x = 0.0;
                self.pending_touch = pointer.pointer_type == PointerType::Touch;
                self.touch_down_time_ns = 0;
                self.touch_drag_armed = !self.pending_touch;
                self.touch_scroll_started = false;
            }
            PointerEvent::Move(PointerUpdate {
                pointer, current, ..
            }) if self.drag_pointer == pointer.pointer_id => {
                let local = ctx.local_position(current.position);
                let offset = local.x - self.drag_start_local_x;
                if self.pending_touch && !self.touch_drag_armed {
                    // Decide the gesture from input timestamps, not animation
                    // frames. If the user moved before the long-press deadline,
                    // the parent Portal owns the horizontal swipe. If they held
                    // long enough first, the very next move begins tab dragging.
                    let held_ns = current.time.saturating_sub(self.touch_down_time_ns);
                    if held_ns >= TOUCH_DRAG_HOLD_NS {
                        self.touch_drag_armed = true;
                    } else {
                        if offset.abs() >= TOUCH_SCROLL_THRESHOLD {
                            self.touch_scroll_started = true;
                        }
                        return;
                    }
                }
                if self.pending_touch && self.touch_scroll_started {
                    return;
                }
                if self.drag_tab_id.is_none() {
                    let threshold = if self.pending_touch {
                        3.0
                    } else {
                        TAB_DRAG_THRESHOLD
                    };
                    if offset.abs() < threshold {
                        return;
                    }
                    if self.pending_touch {
                        ctx.set_handled();
                    }
                    self.drag_tab_id = Some(self.config.tab_id);
                    let overlay = drag_layer_widget(
                        &self.config.accessibility_label,
                        ctx.size().width,
                        ctx.size().height,
                        self.config.background,
                        self.config.border,
                        self.config.text_color,
                    );
                    self.layer_root_id = Some(overlay.id());
                    ctx.create_layer(overlay, self.drag_origin_window);
                    ctx.request_compose();
                }
                self.drag_offset_x = offset;
                if self.pending_touch {
                    ctx.set_handled();
                }
                if let Some(layer_id) = self.layer_root_id {
                    ctx.reposition_layer(
                        layer_id,
                        Point::new(
                            self.drag_origin_window.x + self.drag_offset_x,
                            self.drag_origin_window.y,
                        ),
                    );
                }
                ctx.request_compose();
                ctx.request_render();
            }
            PointerEvent::Up(PointerButtonEvent { pointer, .. })
                if self.drag_pointer == pointer.pointer_id =>
            {
                let Some(tab_id) = self.drag_tab_id else {
                    let should_select = !self.pending_touch || !self.touch_scroll_started;
                    let tab_id = self.config.tab_id;
                    self.clear_drag();
                    ctx.release_pointer();
                    if should_select {
                        ctx.submit_action::<TabDragAction>(TabDragAction::Select(tab_id));
                    }
                    return;
                };
                let source_index = self.config.source_index;
                let target_index = self.drop_target_index();
                if let Some(layer_id) = self.layer_root_id.take() {
                    ctx.remove_layer(layer_id);
                }
                self.clear_drag();
                ctx.request_compose();
                ctx.release_pointer();
                ctx.request_render();
                if target_index != source_index {
                    ctx.submit_action::<TabDragAction>(TabDragAction::Drop {
                        tab_id,
                        target_index,
                    });
                }
            }
            PointerEvent::Cancel(pointer) if self.drag_pointer == pointer.pointer_id => {
                if let Some(layer_id) = self.layer_root_id.take() {
                    ctx.remove_layer(layer_id);
                }
                self.clear_drag();
                ctx.request_compose();
                ctx.release_pointer();
                ctx.request_render();
            }
            _ => {}
        }
    }

    fn on_text_event(
        &mut self,
        _ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        _event: &TextEvent,
    ) {
    }

    fn on_access_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &AccessEvent,
    ) {
        if event.action == Action::Click {
            ctx.submit_action::<TabDragAction>(TabDragAction::Select(self.config.tab_id));
        }
    }
    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        if matches!(event, Update::WidgetAdded) && self.config.selected {
            let width = self
                .config
                .tab_widths
                .get(self.config.drag_index)
                .copied()
                .unwrap_or_else(|| ctx.size().width.max(1.0));
            ctx.request_scroll_to(Rect::new(-self.config.scroll_leading, 0.0, width, 1.0));
        }
        if matches!(event, Update::HoveredChanged(_) | Update::ActiveChanged(_)) {
            ctx.request_paint_only();
        }
    }

    fn layout(
        &mut self,
        ctx: &mut LayoutCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        bc: &BoxConstraints,
    ) -> Size {
        let size = ctx.run_layout(&mut self.child, bc);
        ctx.place_child(&mut self.child, Point::ORIGIN);
        bc.constrain(size)
    }

    fn compose(&mut self, ctx: &mut ComposeCtx<'_>) {
        // Keep the original slot in layout as a browser-style placeholder, but
        // paint the live drag preview in a top-level layer. Hiding the original
        // avoids the double-tab corruption that occurred with the old overlay.
        let translation = if self.drag_tab_id.is_some() {
            Vec2::new(HIDDEN_DRAG_CHILD_X, 0.0)
        } else {
            Vec2::ZERO
        };
        ctx.set_child_scroll_translation(&mut self.child, translation);
    }

    fn paint(&mut self, _ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, _scene: &mut Scene) {}

    fn accessibility_role(&self) -> Role {
        Role::Tab
    }
    fn accessibility(
        &mut self,
        _ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        node: &mut Node,
    ) {
        node.set_label(self.config.accessibility_label.clone());
        node.set_selected(self.config.selected);
        node.add_action(Action::Click);
    }

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        ctx.register_child(&mut self.child);
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::from_slice(&[self.child.id()])
    }

    fn accepts_pointer_interaction(&self) -> bool {
        // The tab title surface is not itself a Button. Without opting the
        // wrapper into hit-testing, only the close button receives pointer
        // events and dragging/tapping the tab body never reaches this widget.
        true
    }

    fn accepts_focus(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::tab_drop_target_index;

    #[test]
    fn drag_reorders_only_after_crossing_neighbor_center() {
        let widths = [100.0, 200.0, 80.0];

        assert_eq!(tab_drop_target_index(&widths, 1, -149.0), 1);
        assert_eq!(tab_drop_target_index(&widths, 1, -151.0), 0);
        assert_eq!(tab_drop_target_index(&widths, 1, 139.0), 1);
        assert_eq!(tab_drop_target_index(&widths, 1, 141.0), 2);
    }

    #[test]
    fn slight_left_drag_does_not_immediately_reorder() {
        let widths = [100.0, 200.0, 80.0];
        assert_eq!(tab_drop_target_index(&widths, 2, -1.0), 2);
        assert_eq!(tab_drop_target_index(&widths, 2, -139.0), 2);
        assert_eq!(tab_drop_target_index(&widths, 2, -141.0), 1);
    }
}
