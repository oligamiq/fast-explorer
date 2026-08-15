use std::path::PathBuf;

use xilem::core::{MessageContext, MessageResult, Mut, View, ViewId, ViewMarker, ViewPathTracker};
use xilem::masonry::accesskit::{Action, Node, Role};
use xilem::masonry::core::{
    AccessCtx, AccessEvent, BoxConstraints, ChildrenIds, EventCtx, LayoutCtx, NewWidget, PaintCtx,
    PointerButton, PointerButtonEvent, PointerEvent, PointerId, PointerType, PointerUpdate,
    PropertiesMut, PropertiesRef, RegisterCtx, TextEvent, Update, UpdateCtx, Widget, WidgetMut,
    WidgetPod,
};
use xilem::masonry::kurbo::{Affine, Point, Size, Stroke};
use xilem::masonry::peniko::{Color, Fill};
use xilem::masonry::vello::Scene;
use xilem::{Pod, ViewCtx, WidgetView};

use crate::app::AppState;
use crate::theme::ThemePalette;

const FILE_ROW_CONTENT_VIEW_ID: ViewId = ViewId::new(0);

#[derive(Debug)]
pub(super) struct FileRowPressed(Option<PointerButton>);

pub(super) fn file_row_button<V>(
    child: V,
    path: PathBuf,
    accessibility_label: String,
    selected: bool,
    palette: ThemePalette,
) -> FileRowButtonView<V>
where
    V: WidgetView<AppState>,
{
    FileRowButtonView {
        child,
        path,
        accessibility_label,
        selected,
        background: if selected {
            palette.accent_soft
        } else {
            palette.surface
        },
        active_background: palette.accent_soft,
        border: if selected {
            palette.accent
        } else {
            palette.surface
        },
        hovered_border: palette.border_strong,
    }
}

pub(super) struct FileRowButtonView<V> {
    child: V,
    path: PathBuf,
    accessibility_label: String,
    selected: bool,
    background: Color,
    active_background: Color,
    border: Color,
    hovered_border: Color,
}

impl<V> ViewMarker for FileRowButtonView<V> {}

impl<V> View<AppState, (), ViewCtx> for FileRowButtonView<V>
where
    V: WidgetView<AppState>,
{
    type Element = Pod<FileRowButton>;
    type ViewState = V::ViewState;

    fn build(&self, ctx: &mut ViewCtx, state: &mut AppState) -> (Self::Element, Self::ViewState) {
        let (child, child_state) =
            ctx.with_id(FILE_ROW_CONTENT_VIEW_ID, |ctx| self.child.build(ctx, state));
        let widget = FileRowButton::new(
            child.new_widget,
            self.accessibility_label.clone(),
            self.selected,
            self.background,
            self.active_background,
            self.border,
            self.hovered_border,
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
        FileRowButton::set_style(
            &mut element,
            self.accessibility_label.clone(),
            self.selected,
            self.background,
            self.active_background,
            self.border,
            self.hovered_border,
        );
        ctx.with_id(FILE_ROW_CONTENT_VIEW_ID, |ctx| {
            let mut child = FileRowButton::child_mut(&mut element);
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
        ctx.with_id(FILE_ROW_CONTENT_VIEW_ID, |ctx| {
            let mut child = FileRowButton::child_mut(&mut element);
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
            Some(FILE_ROW_CONTENT_VIEW_ID) => {
                let mut child = FileRowButton::child_mut(&mut element);
                self.child
                    .message(view_state, message, child.downcast(), state)
            }
            None => match message.take_message::<FileRowPressed>() {
                Some(press) => {
                    match press.0 {
                        None => state.touch_entry(self.path.clone()),
                        Some(PointerButton::Primary) => state.click_entry(self.path.clone()),
                        Some(PointerButton::Secondary) => {
                            state.context_click_entry(self.path.clone())
                        }
                        _ => {}
                    }
                    MessageResult::Action(())
                }
                None => MessageResult::Stale,
            },
            _ => MessageResult::Stale,
        }
    }
}

pub(super) struct FileRowButton {
    child: WidgetPod<dyn Widget>,
    accessibility_label: String,
    selected: bool,
    background: Color,
    active_background: Color,
    border: Color,
    hovered_border: Color,
    touch_start: Option<(Option<PointerId>, Point, u64)>,
    touch_moved: bool,
}

impl FileRowButton {
    fn new(
        child: NewWidget<impl Widget + ?Sized>,
        accessibility_label: String,
        selected: bool,
        background: Color,
        active_background: Color,
        border: Color,
        hovered_border: Color,
    ) -> Self {
        Self {
            child: child.erased().to_pod(),
            accessibility_label,
            selected,
            background,
            active_background,
            border,
            hovered_border,
            touch_start: None,
            touch_moved: false,
        }
    }

    fn set_style(
        this: &mut WidgetMut<'_, Self>,
        accessibility_label: String,
        selected: bool,
        background: Color,
        active_background: Color,
        border: Color,
        hovered_border: Color,
    ) {
        let accessibility_changed = this.widget.accessibility_label != accessibility_label
            || this.widget.selected != selected;
        let paint_changed = this.widget.background != background
            || this.widget.active_background != active_background
            || this.widget.border != border
            || this.widget.hovered_border != hovered_border;

        this.widget.accessibility_label = accessibility_label;
        this.widget.selected = selected;
        this.widget.background = background;
        this.widget.active_background = active_background;
        this.widget.border = border;
        this.widget.hovered_border = hovered_border;

        if accessibility_changed || paint_changed {
            this.ctx.request_render();
        }
    }

    fn child_mut<'a>(this: &'a mut WidgetMut<'_, Self>) -> WidgetMut<'a, dyn Widget> {
        this.ctx.get_mut(&mut this.widget.child)
    }

    fn activate(&self, ctx: &mut EventCtx<'_>, button: Option<PointerButton>) {
        ctx.submit_action::<FileRowPressed>(FileRowPressed(button));
    }
}

impl Widget for FileRowButton {
    type Action = FileRowPressed;

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
            }) if button.is_none()
                || matches!(
                    button,
                    Some(PointerButton::Primary | PointerButton::Secondary)
                ) =>
            {
                ctx.capture_pointer();
                if pointer.pointer_type == PointerType::Touch {
                    self.touch_start = Some((
                        pointer.pointer_id,
                        Point::new(state.position.x, state.position.y),
                        state.time,
                    ));
                    self.touch_moved = false;
                }
                ctx.request_paint_only();
            }
            PointerEvent::Move(PointerUpdate {
                pointer, current, ..
            }) => {
                if let Some((id, start, _)) = self.touch_start
                    && pointer.pointer_id == id
                {
                    let current = Point::new(current.position.x, current.position.y);
                    if (current - start).hypot() > 12.0 {
                        self.touch_moved = true;
                    }
                }
            }
            PointerEvent::Up(PointerButtonEvent {
                button,
                pointer,
                state,
                ..
            }) => {
                let is_touch = pointer.pointer_type == PointerType::Touch;
                let touch_tap = is_touch
                    && self
                        .touch_start
                        .is_some_and(|(id, _, _)| id == pointer.pointer_id)
                    && !self.touch_moved;
                let touch_long_press = touch_tap
                    && self.touch_start.is_some_and(|(_, _, down_time)| {
                        state.time.saturating_sub(down_time) >= 500_000_000
                    });
                if touch_tap || (!is_touch && ctx.is_active() && ctx.is_hovered()) {
                    let activation_button = if touch_long_press {
                        Some(PointerButton::Secondary)
                    } else if is_touch {
                        // Android touch events can report a normal tap with either no
                        // button or Primary. Always route touch taps through touch_entry
                        // so folders open on one tap while files are selected.
                        None
                    } else {
                        *button
                    };
                    self.activate(ctx, activation_button);
                }
                self.touch_start = None;
                self.touch_moved = false;
                ctx.request_paint_only();
            }
            PointerEvent::Cancel(pointer)
                if self
                    .touch_start
                    .is_some_and(|(id, _, _)| id == pointer.pointer_id) =>
            {
                self.touch_start = None;
                self.touch_moved = false;
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
            self.activate(ctx, None);
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

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, scene: &mut Scene) {
        let rect = ctx.size().to_rect();
        let background = if ctx.is_active() {
            self.active_background
        } else {
            self.background
        };
        scene.fill(Fill::NonZero, Affine::IDENTITY, background, None, &rect);
        let border = if ctx.is_hovered() {
            self.hovered_border
        } else {
            self.border
        };
        scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, border, None, &rect);
    }

    fn accessibility_role(&self) -> Role {
        Role::ListBoxOption
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
