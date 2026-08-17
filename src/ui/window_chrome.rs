use xilem::core::{MessageContext, MessageResult, Mut, View, ViewMarker};
use xilem::masonry::accesskit::{Node, Role};
use xilem::masonry::core::keyboard::{Key, NamedKey};
use xilem::masonry::core::{
    AccessCtx, AccessEvent, BoxConstraints, ChildrenIds, CursorIcon, EventCtx, LayoutCtx, NoAction,
    PaintCtx, PointerButton, PointerButtonEvent, PointerEvent, PointerId, PointerType,
    PointerUpdate, PropertiesMut, PropertiesRef, QueryCtx, RegisterCtx, ResizeDirection, TextEvent,
    Update, UpdateCtx, Widget, WidgetMut,
};
use xilem::masonry::kurbo::{Affine, Point, Rect, Size, Stroke};
use xilem::masonry::peniko::{Color, Fill};
use xilem::masonry::vello::Scene;
use xilem::{Pod, ViewCtx};

use super::icons::{LucideIcon, draw_icon};
use crate::app::AppState;
use crate::theme::{Layout, ThemePalette};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CaptionButtonKind {
    Minimize,
    Maximize,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NavigationButtonKind {
    Back,
    Forward,
    Up,
    Home,
    Refresh,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CaptionPalette {
    background: Color,
    hover: Color,
    active: Color,
    foreground: Color,
    focus: Color,
}
impl CaptionPalette {
    fn from_theme(palette: ThemePalette) -> Self {
        Self {
            background: palette.chrome,
            hover: palette.accent_soft,
            active: palette.accent_pressed,
            foreground: palette.text,
            focus: palette.focus,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct NavigationPalette {
    background: Color,
    hover: Color,
    active: Color,
    foreground: Color,
    disabled: Color,
    focus: Color,
}

impl NavigationPalette {
    fn from_theme(palette: ThemePalette) -> Self {
        Self {
            background: palette.chrome,
            hover: palette.accent_soft,
            active: palette.accent_pressed,
            foreground: palette.text,
            disabled: palette.muted,
            focus: palette.focus,
        }
    }
}

#[derive(Debug)]
pub(super) struct CaptionButtonActivated;

pub(super) fn caption_button(kind: CaptionButtonKind, palette: ThemePalette) -> CaptionButtonView {
    CaptionButtonView {
        kind,
        palette: CaptionPalette::from_theme(palette),
    }
}

#[derive(Debug)]
pub(super) struct NavigationButtonActivated(pub NavigationButtonKind);

pub(super) fn navigation_button(
    kind: NavigationButtonKind,
    disabled: bool,
    palette: ThemePalette,
) -> NavigationButtonView {
    NavigationButtonView {
        kind,
        disabled,
        palette: NavigationPalette::from_theme(palette),
    }
}

pub(super) fn drag_region() -> DragRegionView {
    DragRegionView
}

pub(super) struct CaptionButtonView {
    kind: CaptionButtonKind,
    palette: CaptionPalette,
}

impl ViewMarker for CaptionButtonView {}
impl View<AppState, (), ViewCtx> for CaptionButtonView {
    type Element = Pod<WindowCaptionButton>;
    type ViewState = ();

    fn build(&self, ctx: &mut ViewCtx, _: &mut AppState) -> (Self::Element, Self::ViewState) {
        ctx.with_leaf_action_widget(|ctx| {
            ctx.create_pod(WindowCaptionButton::new(self.kind, self.palette))
        })
    }

    fn rebuild(
        &self,
        prev: &Self,
        (): &mut Self::ViewState,
        _ctx: &mut ViewCtx,
        mut element: Mut<'_, Self::Element>,
        _: &mut AppState,
    ) {
        if self.kind != prev.kind || self.palette != prev.palette {
            WindowCaptionButton::set_style(&mut element, self.kind, self.palette);
        }
    }

    fn teardown(
        &self,
        (): &mut Self::ViewState,
        ctx: &mut ViewCtx,
        element: Mut<'_, Self::Element>,
    ) {
        ctx.teardown_leaf(element);
    }
    fn message(
        &self,
        (): &mut Self::ViewState,
        message: &mut MessageContext,
        _element: Mut<'_, Self::Element>,
        app_state: &mut AppState,
    ) -> MessageResult<()> {
        if message.take_message::<CaptionButtonActivated>().is_some() {
            app_state.persist_session();
            crate::ipc::cleanup_owned_socket();
            MessageResult::Action(())
        } else {
            MessageResult::Stale
        }
    }
}

pub(super) struct NavigationButtonView {
    kind: NavigationButtonKind,
    disabled: bool,
    palette: NavigationPalette,
}

impl ViewMarker for NavigationButtonView {}
impl View<AppState, (), ViewCtx> for NavigationButtonView {
    type Element = Pod<NavigationButton>;
    type ViewState = ();

    fn build(&self, ctx: &mut ViewCtx, _: &mut AppState) -> (Self::Element, Self::ViewState) {
        let (mut element, ()) = ctx.with_leaf_action_widget(|ctx| {
            ctx.create_pod(NavigationButton::new(self.kind, self.palette))
        });
        element.new_widget.options.disabled = self.disabled;
        (element, ())
    }

    fn rebuild(
        &self,
        prev: &Self,
        (): &mut Self::ViewState,
        _ctx: &mut ViewCtx,
        mut element: Mut<'_, Self::Element>,
        _: &mut AppState,
    ) {
        if self.disabled != prev.disabled {
            element.ctx.set_disabled(self.disabled);
        }
        if self.kind != prev.kind || self.palette != prev.palette {
            NavigationButton::set_style(&mut element, self.kind, self.palette);
        }
    }

    fn teardown(
        &self,
        (): &mut Self::ViewState,
        ctx: &mut ViewCtx,
        element: Mut<'_, Self::Element>,
    ) {
        ctx.teardown_leaf(element);
    }

    fn message(
        &self,
        (): &mut Self::ViewState,
        message: &mut MessageContext,
        _element: Mut<'_, Self::Element>,
        state: &mut AppState,
    ) -> MessageResult<()> {
        let Some(action) = message.take_message::<NavigationButtonActivated>() else {
            return MessageResult::Stale;
        };
        match action.0 {
            NavigationButtonKind::Back => state.go_back(),
            NavigationButtonKind::Forward => state.go_forward(),
            NavigationButtonKind::Up => state.go_up(),
            NavigationButtonKind::Home => state.go_home(),
            NavigationButtonKind::Refresh => state.refresh(),
        }
        MessageResult::Action(())
    }
}

pub(super) struct DragRegionView;
impl ViewMarker for DragRegionView {}

impl View<AppState, (), ViewCtx> for DragRegionView {
    type Element = Pod<WindowDragRegion>;
    type ViewState = ();

    fn build(&self, ctx: &mut ViewCtx, _: &mut AppState) -> (Self::Element, Self::ViewState) {
        ctx.with_leaf_action_widget(|ctx| ctx.create_pod(WindowDragRegion))
    }

    fn rebuild(
        &self,
        _prev: &Self,
        (): &mut Self::ViewState,
        _ctx: &mut ViewCtx,
        _element: Mut<'_, Self::Element>,
        _: &mut AppState,
    ) {
    }

    fn teardown(
        &self,
        (): &mut Self::ViewState,
        ctx: &mut ViewCtx,
        element: Mut<'_, Self::Element>,
    ) {
        ctx.teardown_leaf(element);
    }

    fn message(
        &self,
        (): &mut Self::ViewState,
        _message: &mut MessageContext,
        _element: Mut<'_, Self::Element>,
        _: &mut AppState,
    ) -> MessageResult<()> {
        MessageResult::Stale
    }
}

pub(super) fn resize_region(direction: ResizeDirection) -> ResizeRegionView {
    ResizeRegionView { direction }
}

pub(super) struct ResizeRegionView {
    direction: ResizeDirection,
}

impl ViewMarker for ResizeRegionView {}

impl View<AppState, (), ViewCtx> for ResizeRegionView {
    type Element = Pod<WindowResizeRegion>;
    type ViewState = ();

    fn build(&self, ctx: &mut ViewCtx, _: &mut AppState) -> (Self::Element, Self::ViewState) {
        ctx.with_leaf_action_widget(|ctx| ctx.create_pod(WindowResizeRegion(self.direction)))
    }

    fn rebuild(
        &self,
        prev: &Self,
        (): &mut Self::ViewState,
        _ctx: &mut ViewCtx,
        mut element: Mut<'_, Self::Element>,
        _: &mut AppState,
    ) {
        if self.direction != prev.direction {
            element.widget.0 = self.direction;
            element.ctx.request_layout();
            element.ctx.request_cursor_icon_change();
        }
    }

    fn teardown(
        &self,
        (): &mut Self::ViewState,
        ctx: &mut ViewCtx,
        element: Mut<'_, Self::Element>,
    ) {
        ctx.teardown_leaf(element);
    }

    fn message(
        &self,
        (): &mut Self::ViewState,
        _message: &mut MessageContext,
        _element: Mut<'_, Self::Element>,
        _: &mut AppState,
    ) -> MessageResult<()> {
        MessageResult::Stale
    }
}

pub(super) struct WindowResizeRegion(ResizeDirection);

impl Widget for WindowResizeRegion {
    type Action = NoAction;

    fn on_pointer_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _: &mut PropertiesMut<'_>,
        event: &PointerEvent,
    ) {
        if let PointerEvent::Down(button) = event
            && (button.button == Some(PointerButton::Primary)
                || (button.button.is_none() && button.pointer.pointer_type == PointerType::Touch))
        {
            ctx.set_handled();
            ctx.drag_resize_window(self.0);
        }
    }

    fn on_text_event(&mut self, _: &mut EventCtx<'_>, _: &mut PropertiesMut<'_>, _: &TextEvent) {}
    fn on_access_event(
        &mut self,
        _: &mut EventCtx<'_>,
        _: &mut PropertiesMut<'_>,
        _: &AccessEvent,
    ) {
    }
    fn update(&mut self, _: &mut UpdateCtx<'_>, _: &mut PropertiesMut<'_>, _: &Update) {}

    fn layout(
        &mut self,
        _: &mut LayoutCtx<'_>,
        _: &mut PropertiesMut<'_>,
        bc: &BoxConstraints,
    ) -> Size {
        let max = bc.max();
        let edge = Layout::RESIZE_HIT_SIZE;
        let corner = Layout::RESIZE_CORNER_SIZE;
        let desired = match self.0 {
            ResizeDirection::North | ResizeDirection::South => Size::new(max.width, edge),
            ResizeDirection::East | ResizeDirection::West => Size::new(edge, max.height),
            _ => Size::new(corner, corner),
        };
        bc.constrain(desired)
    }

    fn paint(&mut self, _: &mut PaintCtx<'_>, _: &PropertiesRef<'_>, _: &mut Scene) {}

    fn get_cursor(&self, _: &QueryCtx<'_>, _: xilem::masonry::kurbo::Point) -> CursorIcon {
        match self.0 {
            ResizeDirection::East | ResizeDirection::West => CursorIcon::EwResize,
            ResizeDirection::North | ResizeDirection::South => CursorIcon::NsResize,
            ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::NeswResize,
            ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::NwseResize,
        }
    }

    fn accessibility_role(&self) -> Role {
        Role::GenericContainer
    }
    fn accessibility(&mut self, _: &mut AccessCtx<'_>, _: &PropertiesRef<'_>, _: &mut Node) {}
    fn register_children(&mut self, _: &mut RegisterCtx<'_>) {}
    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }
}

pub(super) struct WindowDragRegion;

impl Widget for WindowDragRegion {
    type Action = NoAction;

    fn on_pointer_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &PointerEvent,
    ) {
        if let PointerEvent::Down(button) = event {
            match button.button {
                Some(PointerButton::Primary) if button.state.count >= 2 => {
                    ctx.set_handled();
                    ctx.toggle_maximized();
                }
                Some(PointerButton::Primary) => {
                    ctx.set_handled();
                    ctx.drag_window();
                }
                None if button.pointer.pointer_type == PointerType::Touch => {
                    ctx.set_handled();
                    ctx.drag_window();
                }
                Some(PointerButton::Secondary) => {
                    ctx.set_handled();
                    ctx.show_window_menu(button.state.logical_position());
                }
                _ => {}
            }
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
        _ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        _event: &AccessEvent,
    ) {
    }
    fn update(
        &mut self,
        _ctx: &mut UpdateCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        _event: &Update,
    ) {
    }

    fn layout(
        &mut self,
        _ctx: &mut LayoutCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        bc: &BoxConstraints,
    ) -> Size {
        let max = bc.max();
        bc.constrain(Size::new(max.width, Layout::TAB_HEIGHT))
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

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}
    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }
}

pub(super) struct NavigationButton {
    kind: NavigationButtonKind,
    palette: NavigationPalette,
    touch_start: Option<(Option<PointerId>, Point)>,
    touch_moved: bool,
}

impl NavigationButton {
    fn new(kind: NavigationButtonKind, palette: NavigationPalette) -> Self {
        Self {
            kind,
            palette,
            touch_start: None,
            touch_moved: false,
        }
    }

    fn set_style(
        this: &mut WidgetMut<'_, Self>,
        kind: NavigationButtonKind,
        palette: NavigationPalette,
    ) {
        this.widget.kind = kind;
        this.widget.palette = palette;
        this.ctx.request_paint_only();
    }

    fn accessibility_label(&self) -> &'static str {
        match self.kind {
            NavigationButtonKind::Back => "Back",
            NavigationButtonKind::Forward => "Forward",
            NavigationButtonKind::Up => "Up one level",
            NavigationButtonKind::Home => "Home",
            NavigationButtonKind::Refresh => "Refresh",
        }
    }

    fn activate(&self, ctx: &mut EventCtx<'_>) {
        if !ctx.is_disabled() {
            ctx.submit_action::<NavigationButtonActivated>(NavigationButtonActivated(self.kind));
        }
    }
}

impl Widget for NavigationButton {
    type Action = NavigationButtonActivated;

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
            }) if !ctx.is_disabled()
                && (pointer.pointer_type == PointerType::Touch
                    || *button == Some(PointerButton::Primary)) =>
            {
                ctx.capture_pointer();
                if pointer.pointer_type == PointerType::Touch {
                    self.touch_start = Some((
                        pointer.pointer_id,
                        Point::new(state.position.x, state.position.y),
                    ));
                    self.touch_moved = false;
                }
                ctx.request_paint_only();
            }
            PointerEvent::Move(PointerUpdate {
                pointer, current, ..
            }) => {
                if let Some((id, start)) = self.touch_start
                    && id == pointer.pointer_id
                {
                    let current = Point::new(current.position.x, current.position.y);
                    if (current - start).hypot() > 12.0 {
                        self.touch_moved = true;
                    }
                }
            }
            PointerEvent::Up(PointerButtonEvent {
                button, pointer, ..
            }) => {
                let is_touch = pointer.pointer_type == PointerType::Touch;
                let valid_button = is_touch || *button == Some(PointerButton::Primary);
                let touch_tap = is_touch
                    && self
                        .touch_start
                        .is_some_and(|(id, _)| id == pointer.pointer_id)
                    && !self.touch_moved;
                if valid_button && (touch_tap || (!is_touch && ctx.is_active() && ctx.is_hovered()))
                {
                    self.activate(ctx);
                }
                self.touch_start = None;
                self.touch_moved = false;
                ctx.request_paint_only();
            }
            PointerEvent::Cancel(pointer)
                if self
                    .touch_start
                    .is_some_and(|(id, _)| id == pointer.pointer_id) =>
            {
                self.touch_start = None;
                self.touch_moved = false;
            }
            _ => {}
        }
    }

    fn on_text_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &TextEvent,
    ) {
        if let TextEvent::Keyboard(event) = event
            && event.state.is_up()
            && (matches!(&event.key, Key::Character(c) if c == " ")
                || event.key == Key::Named(NamedKey::Enter))
        {
            self.activate(ctx);
        }
    }

    fn on_access_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &AccessEvent,
    ) {
        if event.action == xilem::masonry::accesskit::Action::Click {
            self.activate(ctx);
        }
    }

    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _: &mut PropertiesMut<'_>, event: &Update) {
        if matches!(
            event,
            Update::HoveredChanged(_)
                | Update::ActiveChanged(_)
                | Update::FocusChanged(_)
                | Update::DisabledChanged(_)
        ) {
            ctx.request_paint_only();
        }
    }

    fn layout(
        &mut self,
        _ctx: &mut LayoutCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        bc: &BoxConstraints,
    ) -> Size {
        bc.constrain(Size::new(Layout::NAV_WIDTH, Layout::LOCATION_FIELD_HEIGHT))
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _: &PropertiesRef<'_>, scene: &mut Scene) {
        let size = ctx.size();
        let rect = size.to_rect();
        let background = if ctx.is_disabled() {
            self.palette.background
        } else if ctx.is_active() {
            self.palette.active
        } else if ctx.is_hovered() {
            self.palette.hover
        } else {
            self.palette.background
        };
        scene.fill(Fill::NonZero, Affine::IDENTITY, background, None, &rect);

        let foreground = if ctx.is_disabled() {
            self.palette.disabled
        } else {
            self.palette.foreground
        };
        let icon = match self.kind {
            NavigationButtonKind::Back => LucideIcon::ArrowLeft,
            NavigationButtonKind::Forward => LucideIcon::ArrowRight,
            NavigationButtonKind::Up => LucideIcon::ArrowUp,
            NavigationButtonKind::Home => LucideIcon::House,
            NavigationButtonKind::Refresh => LucideIcon::RefreshCw,
        };
        let icon_rect = Rect::new(
            (size.width - 18.0) * 0.5,
            (size.height - 18.0) * 0.5,
            (size.width + 18.0) * 0.5,
            (size.height + 18.0) * 0.5,
        );
        draw_icon(scene, icon, foreground, icon_rect);

        if ctx.is_focus_target() && !ctx.is_disabled() {
            let focus_rect = Rect::new(2.5, 2.5, size.width - 2.5, size.height - 2.5);
            scene.stroke(
                &Stroke::new(1.0),
                Affine::IDENTITY,
                self.palette.focus,
                None,
                &focus_rect,
            );
        }
    }

    fn accessibility_role(&self) -> Role {
        Role::Button
    }

    fn accessibility(
        &mut self,
        _ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        node: &mut Node,
    ) {
        node.set_label(self.accessibility_label());
        node.add_action(xilem::masonry::accesskit::Action::Click);
    }

    fn register_children(&mut self, _: &mut RegisterCtx<'_>) {}

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }

    fn accepts_focus(&self) -> bool {
        true
    }
}

pub(super) struct WindowCaptionButton {
    kind: CaptionButtonKind,
    palette: CaptionPalette,
}

impl WindowCaptionButton {
    fn new(kind: CaptionButtonKind, palette: CaptionPalette) -> Self {
        Self { kind, palette }
    }

    fn set_style(this: &mut WidgetMut<'_, Self>, kind: CaptionButtonKind, palette: CaptionPalette) {
        this.widget.kind = kind;
        this.widget.palette = palette;
        this.ctx.request_paint_only();
    }

    fn activate(&self, ctx: &mut EventCtx<'_>) {
        match self.kind {
            CaptionButtonKind::Minimize => ctx.minimize(),
            CaptionButtonKind::Maximize => ctx.toggle_maximized(),
            CaptionButtonKind::Close => {
                ctx.submit_action::<CaptionButtonActivated>(CaptionButtonActivated);
                ctx.exit();
            }
        }
    }

    fn accessibility_label(&self) -> &'static str {
        match self.kind {
            CaptionButtonKind::Minimize => "Minimize window",
            CaptionButtonKind::Maximize => "Maximize or restore window",
            CaptionButtonKind::Close => "Close window",
        }
    }
}

impl Widget for WindowCaptionButton {
    type Action = CaptionButtonActivated;

    fn on_pointer_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &PointerEvent,
    ) {
        match event {
            PointerEvent::Down(PointerButtonEvent {
                button: None | Some(PointerButton::Primary),
                ..
            }) => {
                ctx.capture_pointer();
                ctx.set_handled();
                ctx.request_paint_only();
            }
            PointerEvent::Up(PointerButtonEvent {
                button: None | Some(PointerButton::Primary),
                ..
            }) => {
                ctx.set_handled();
                if ctx.is_active() && ctx.is_hovered() {
                    self.activate(ctx);
                }
                ctx.request_paint_only();
            }
            _ => {}
        }
    }

    fn on_text_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &TextEvent,
    ) {
        if let TextEvent::Keyboard(event) = event
            && event.state.is_up()
            && (matches!(&event.key, Key::Character(c) if c == " ")
                || event.key == Key::Named(NamedKey::Enter))
        {
            self.activate(ctx);
        }
    }

    fn on_access_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &AccessEvent,
    ) {
        if event.action == xilem::masonry::accesskit::Action::Click {
            self.activate(ctx);
        }
    }
    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        if matches!(
            event,
            Update::HoveredChanged(_)
                | Update::ActiveChanged(_)
                | Update::FocusChanged(_)
                | Update::DisabledChanged(_)
        ) {
            ctx.request_paint_only();
        }
    }

    fn layout(
        &mut self,
        _ctx: &mut LayoutCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        bc: &BoxConstraints,
    ) -> Size {
        bc.constrain(Size::new(Layout::CAPTION_BUTTON_WIDTH, Layout::TAB_HEIGHT))
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, scene: &mut Scene) {
        let size = ctx.size();
        let rect = size.to_rect();
        let close_hover = Color::from_rgb8(196, 43, 28);
        let close_active = Color::from_rgb8(145, 27, 20);
        let background = if self.kind == CaptionButtonKind::Close && ctx.is_active() {
            close_active
        } else if self.kind == CaptionButtonKind::Close && ctx.is_hovered() {
            close_hover
        } else if ctx.is_active() {
            self.palette.active
        } else if ctx.is_hovered() {
            self.palette.hover
        } else {
            self.palette.background
        };
        scene.fill(Fill::NonZero, Affine::IDENTITY, background, None, &rect);

        let foreground = if self.kind == CaptionButtonKind::Close && ctx.is_hovered() {
            Color::WHITE
        } else {
            self.palette.foreground
        };
        let icon = match self.kind {
            CaptionButtonKind::Minimize => LucideIcon::Minus,
            CaptionButtonKind::Maximize => LucideIcon::Square,
            CaptionButtonKind::Close => LucideIcon::X,
        };
        let icon_rect = Rect::new(
            (size.width - 16.0) * 0.5,
            (size.height - 16.0) * 0.5,
            (size.width + 16.0) * 0.5,
            (size.height + 16.0) * 0.5,
        );
        draw_icon(scene, icon, foreground, icon_rect);

        if ctx.is_focus_target() {
            let focus_rect = Rect::new(2.5, 2.5, size.width - 2.5, size.height - 2.5);
            scene.stroke(
                &Stroke::new(1.0),
                Affine::IDENTITY,
                self.palette.focus,
                None,
                &focus_rect,
            );
        }
    }

    fn accessibility_role(&self) -> Role {
        Role::Button
    }

    fn accessibility(
        &mut self,
        _ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        node: &mut Node,
    ) {
        node.set_label(self.accessibility_label());
        node.add_action(xilem::masonry::accesskit::Action::Click);
    }

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }

    fn accepts_focus(&self) -> bool {
        true
    }
}
