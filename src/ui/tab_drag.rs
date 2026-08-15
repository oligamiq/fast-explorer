use xilem::core::{MessageContext, MessageResult, Mut, View, ViewId, ViewMarker, ViewPathTracker};
use xilem::masonry::accesskit::{Action, Node, Role};
use xilem::masonry::core::{
    AccessCtx, AccessEvent, BoxConstraints, ChildrenIds, ComposeCtx, EventCtx, LayoutCtx,
    NewWidget, PaintCtx, PointerButton, PointerButtonEvent, PointerEvent, PointerId, PointerUpdate,
    Properties, PropertiesMut, PropertiesRef, RegisterCtx, StyleProperty, TextEvent, Update,
    UpdateCtx, Widget, WidgetId, WidgetMut, WidgetPod,
};
use xilem::masonry::kurbo::{Point, Size, Vec2};
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
    box_props.insert(Padding::from_vh(5.0, 5.0));
    NewWidget::new_with_props(
        SizedBox::new(label).width(width.px()).height(height.px()),
        box_props,
    )
    .erased()
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

pub(super) fn tab_drag_button<V>(
    child: V,
    tab_id: u64,
    tab_index: usize,
    tab_widths: Vec<f64>,
    drag_handle_right_inset: f64,
    accessibility_label: String,
    selected: bool,
    background: Color,
    border: Color,
    text_color: Color,
) -> TabDragView<V>
where
    V: WidgetView<AppState>,
{
    TabDragView {
        child,
        tab_id,
        tab_index,
        tab_widths,
        drag_handle_right_inset,
        accessibility_label,
        selected,
        background,
        border,
        text_color,
    }
}

pub(super) struct TabDragView<V> {
    child: V,
    tab_id: u64,
    tab_index: usize,
    tab_widths: Vec<f64>,
    drag_handle_right_inset: f64,
    accessibility_label: String,
    selected: bool,
    background: Color,
    border: Color,
    text_color: Color,
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
        let widget = TabDragWidget::new(
            child.new_widget,
            self.tab_id,
            self.tab_index,
            self.tab_widths.clone(),
            self.drag_handle_right_inset,
            self.accessibility_label.clone(),
            self.selected,
            self.background,
            self.border,
            self.text_color,
        );
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
        TabDragWidget::set_style(
            &mut element,
            self.tab_id,
            self.tab_index,
            self.tab_widths.clone(),
            self.drag_handle_right_inset,
            self.accessibility_label.clone(),
            self.selected,
            self.background,
            self.border,
            self.text_color,
        );
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
                        } => state.move_tab_to_index(tab_id, target_index),
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
    tab_id: u64,
    tab_index: usize,
    tab_widths: Vec<f64>,
    drag_handle_right_inset: f64,
    accessibility_label: String,
    selected: bool,
    background: Color,
    border: Color,
    text_color: Color,
    drag_tab_id: Option<u64>,
    drag_pointer: Option<PointerId>,
    drag_start_local_x: f64,
    drag_grab_offset: Point,
    drag_offset_x: f64,
    layer_root_id: Option<WidgetId>,
}
impl TabDragWidget {
    fn new(
        child: NewWidget<impl Widget + ?Sized>,
        tab_id: u64,
        tab_index: usize,
        tab_widths: Vec<f64>,
        drag_handle_right_inset: f64,
        accessibility_label: String,
        selected: bool,
        background: Color,
        border: Color,
        text_color: Color,
    ) -> Self {
        Self {
            child: child.erased().to_pod(),
            tab_id,
            tab_index,
            tab_widths,
            drag_handle_right_inset,
            accessibility_label,
            selected,
            background,
            border,
            text_color,
            drag_tab_id: None,
            drag_pointer: None,
            drag_start_local_x: 0.0,
            drag_grab_offset: Point::ORIGIN,
            drag_offset_x: 0.0,
            layer_root_id: None,
        }
    }

    fn child_mut<'a>(this: &'a mut WidgetMut<'_, Self>) -> WidgetMut<'a, dyn Widget> {
        this.ctx.get_mut(&mut this.widget.child)
    }

    fn set_style(
        this: &mut WidgetMut<'_, Self>,
        tab_id: u64,
        tab_index: usize,
        tab_widths: Vec<f64>,
        drag_handle_right_inset: f64,
        accessibility_label: String,
        selected: bool,
        background: Color,
        border: Color,
        text_color: Color,
    ) {
        let changed = this.widget.tab_id != tab_id
            || this.widget.tab_index != tab_index
            || this.widget.tab_widths != tab_widths
            || this.widget.drag_handle_right_inset != drag_handle_right_inset
            || this.widget.accessibility_label != accessibility_label
            || this.widget.selected != selected
            || this.widget.background != background
            || this.widget.border != border
            || this.widget.text_color != text_color;
        this.widget.tab_id = tab_id;
        this.widget.tab_index = tab_index;
        this.widget.tab_widths = tab_widths;
        this.widget.drag_handle_right_inset = drag_handle_right_inset;
        this.widget.accessibility_label = accessibility_label;
        this.widget.selected = selected;
        this.widget.background = background;
        this.widget.border = border;
        this.widget.text_color = text_color;
        if changed {
            this.ctx.request_render();
            this.ctx.request_compose();
        }
    }

    fn clamp_drag_offset(&self, offset: f64) -> f64 {
        let left = self.tab_widths.iter().take(self.tab_index).sum::<f64>();
        let width = self.tab_widths.get(self.tab_index).copied().unwrap_or(0.0);
        let total = self.tab_widths.iter().sum::<f64>();
        offset.clamp(-left, (total - left - width).max(0.0))
    }

    fn drop_target_index(&self) -> usize {
        tab_drop_target_index(&self.tab_widths, self.tab_index, self.drag_offset_x)
    }

    fn clear_drag(&mut self) {
        self.drag_tab_id = None;
        self.drag_pointer = None;
        self.drag_offset_x = 0.0;
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
                let drag_limit = (ctx.size().width - self.drag_handle_right_inset).max(0.0);
                if local.x >= drag_limit {
                    return;
                }
                ctx.capture_pointer();
                self.drag_tab_id = Some(self.tab_id);
                self.drag_pointer = pointer.pointer_id;
                self.drag_start_local_x = local.x;
                self.drag_grab_offset = local;
                self.drag_offset_x = 0.0;
                ctx.submit_action::<TabDragAction>(TabDragAction::Select(self.tab_id));
                ctx.request_render();
            }
            PointerEvent::Move(PointerUpdate {
                pointer, current, ..
            }) if self.drag_tab_id.is_some() && self.drag_pointer == pointer.pointer_id => {
                let local = ctx.local_position(current.position);
                self.drag_offset_x = self.clamp_drag_offset(local.x - self.drag_start_local_x);

                let pointer_pos = current.logical_point();
                let layer_pos = Point::new(
                    pointer_pos.x - self.drag_grab_offset.x,
                    pointer_pos.y - self.drag_grab_offset.y,
                );
                if let Some(layer_id) = self.layer_root_id {
                    ctx.reposition_layer(layer_id, layer_pos);
                } else {
                    let overlay = drag_layer_widget(
                        &self.accessibility_label,
                        ctx.size().width,
                        ctx.size().height,
                        self.background,
                        self.border,
                        self.text_color,
                    );
                    self.layer_root_id = Some(overlay.id());
                    ctx.create_layer(overlay, layer_pos);
                    ctx.request_compose();
                }
                ctx.request_render();
            }
            PointerEvent::Up(PointerButtonEvent { pointer, .. })
                if self.drag_tab_id.is_some() && self.drag_pointer == pointer.pointer_id =>
            {
                let tab_id = self.drag_tab_id.expect("drag id checked above");
                let source_index = self.tab_index;
                let target_index = self.drop_target_index();
                if let Some(layer_id) = self.layer_root_id.take() {
                    ctx.remove_layer(layer_id);
                    ctx.request_compose();
                }
                self.clear_drag();
                ctx.release_pointer();
                ctx.request_render();
                if target_index != source_index {
                    ctx.submit_action::<TabDragAction>(TabDragAction::Drop {
                        tab_id,
                        target_index,
                    });
                }
            }
            PointerEvent::Cancel(pointer)
                if self.drag_tab_id.is_some() && self.drag_pointer == pointer.pointer_id =>
            {
                if let Some(layer_id) = self.layer_root_id.take() {
                    ctx.remove_layer(layer_id);
                    ctx.request_compose();
                }
                self.clear_drag();
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
            ctx.submit_action::<TabDragAction>(TabDragAction::Select(self.tab_id));
        }
    }
    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
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
        let translation = if self.layer_root_id.is_some() {
            Vec2::new(-100_000.0, 0.0)
        } else {
            Vec2::new(0.0, 0.0)
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
        node.set_label(self.accessibility_label.clone());
        node.set_selected(self.selected);
        node.add_action(Action::Click);
    }

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        ctx.register_child(&mut self.child);
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::from_slice(&[self.child.id()])
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
