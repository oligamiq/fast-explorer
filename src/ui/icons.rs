use std::any::TypeId;

use xilem::core::{MessageContext, MessageResult, Mut, View, ViewMarker};
use xilem::masonry::accesskit::{Node, Role};
use xilem::masonry::core::{
    AccessCtx, BoxConstraints, ChildrenIds, LayoutCtx, NoAction, PaintCtx, PropertiesMut,
    PropertiesRef, RegisterCtx, Update, UpdateCtx, Widget, WidgetId, WidgetMut,
};
use xilem::masonry::kurbo::{Affine, Arc, BezPath, Cap, Circle, Join, Rect, Size, Stroke};
use xilem::masonry::peniko::Color;
use xilem::masonry::vello::Scene;
use xilem::{Pod, ViewCtx};

#[allow(
    dead_code,
    reason = "shared Lucide set is used by platform-specific UI paths"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LucideIcon {
    Settings,
    Plus,
    Minus,
    X,
    Square,
    ChevronLeft,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    House,
    RefreshCw,
    Search,
    Pause,
    Play,
    Share2,
    Pin,
    ArrowUpDown,
    Ellipsis,
    FolderPlus,
    Scissors,
    Copy,
    ClipboardPaste,
    Pencil,
    Trash2,
    LogIn,
    LogOut,
    ExternalLink,
    FolderOpen,
    File,
    FileText,
    FileImage,
    Video,
    Music,
    FileArchive,
    FileCode,
    FileSpreadsheet,
    Presentation,
    Braces,
    HardDrive,
    Network,
}

pub(super) struct IconSource {
    paths: &'static [&'static str],
    circles: &'static [(f64, f64, f64)],
    rects: &'static [(f64, f64, f64, f64, f64)],
}

include!("lucide_generated.rs");

const ELLIPSIS: IconSource = IconSource {
    paths: &[],
    circles: &[(5.0, 12.0, 1.0), (12.0, 12.0, 1.0), (19.0, 12.0, 1.0)],
    rects: &[],
};

const SEARCH: IconSource = IconSource {
    paths: &["m21 21-4.34-4.34"],
    circles: &[(11.0, 11.0, 8.0)],
    rects: &[],
};

impl LucideIcon {
    fn source(self) -> &'static IconSource {
        match self {
            Self::Settings => &SETTINGS,
            Self::Plus => &PLUS,
            Self::Minus => &MINUS,
            Self::X => &X,
            Self::Square => &SQUARE,
            Self::ChevronLeft => &CHEVRON_LEFT,
            Self::ArrowLeft => &ARROW_LEFT,
            Self::ArrowRight => &ARROW_RIGHT,
            Self::ArrowUp => &ARROW_UP,
            Self::House => &HOUSE,
            Self::RefreshCw => &REFRESH_CW,
            Self::Search => &SEARCH,
            Self::Pause => &PAUSE,
            Self::Play => &PLAY,
            Self::Share2 => &SHARE_2,
            Self::Pin => &PIN,
            Self::ArrowUpDown => &ARROW_UP_DOWN,
            Self::Ellipsis => &ELLIPSIS,
            Self::FolderPlus => &FOLDER_PLUS,
            Self::Scissors => &SCISSORS,
            Self::Copy => &COPY,
            Self::ClipboardPaste => &CLIPBOARD_PASTE,
            Self::Pencil => &PENCIL,
            Self::Trash2 => &TRASH_2,
            Self::LogIn => &LOG_IN,
            Self::LogOut => &LOG_OUT,
            Self::ExternalLink => &EXTERNAL_LINK,
            Self::FolderOpen => &FOLDER_OPEN,
            Self::File => &FILE,
            Self::FileText => &FILE_TEXT,
            Self::FileImage => &FILE_IMAGE,
            Self::Video => &VIDEO,
            Self::Music => &MUSIC,
            Self::FileArchive => &FILE_ARCHIVE,
            Self::FileCode => &FILE_CODE,
            Self::FileSpreadsheet => &FILE_SPREADSHEET,
            Self::Presentation => &PRESENTATION,
            Self::Braces => &BRACES,
            Self::HardDrive => &HARD_DRIVE,
            Self::Network => &NETWORK,
        }
    }
}

pub(super) fn icon(
    icon: LucideIcon,
    color: Color,
    size: f64,
    label: &'static str,
) -> LucideIconView {
    LucideIconView {
        icon,
        color,
        size,
        label,
    }
}

pub(super) struct LucideIconView {
    icon: LucideIcon,
    color: Color,
    size: f64,
    label: &'static str,
}

impl ViewMarker for LucideIconView {}
impl<State, Action> View<State, Action, ViewCtx> for LucideIconView {
    type Element = Pod<LucideIconWidget>;
    type ViewState = ();

    fn build(&self, ctx: &mut ViewCtx, _: &mut State) -> (Self::Element, Self::ViewState) {
        (
            ctx.create_pod(LucideIconWidget::new(
                self.icon, self.color, self.size, self.label,
            )),
            (),
        )
    }

    fn rebuild(
        &self,
        prev: &Self,
        (): &mut Self::ViewState,
        _ctx: &mut ViewCtx,
        mut element: Mut<'_, Self::Element>,
        _: &mut State,
    ) {
        if self.icon != prev.icon
            || self.color != prev.color
            || self.size != prev.size
            || self.label != prev.label
        {
            LucideIconWidget::set_style(&mut element, self.icon, self.color, self.size, self.label);
        }
    }

    fn teardown(&self, (): &mut (), ctx: &mut ViewCtx, element: Mut<'_, Self::Element>) {
        ctx.teardown_leaf(element);
    }

    fn message(
        &self,
        (): &mut (),
        message: &mut MessageContext,
        _element: Mut<'_, Self::Element>,
        _state: &mut State,
    ) -> MessageResult<Action> {
        tracing::error!(?message, "Lucide icon received an unexpected message");
        MessageResult::Stale
    }
}

pub(super) struct LucideIconWidget {
    icon: LucideIcon,
    color: Color,
    size: f64,
    label: &'static str,
}

impl LucideIconWidget {
    fn new(icon: LucideIcon, color: Color, size: f64, label: &'static str) -> Self {
        Self {
            icon,
            color,
            size,
            label,
        }
    }

    fn set_style(
        this: &mut WidgetMut<'_, Self>,
        icon: LucideIcon,
        color: Color,
        size: f64,
        label: &'static str,
    ) {
        this.widget.icon = icon;
        this.widget.color = color;
        this.widget.size = size;
        this.widget.label = label;
        this.ctx.request_layout();
        this.ctx.request_paint_only();
    }
}

pub(super) fn draw_icon(scene: &mut Scene, icon: LucideIcon, color: Color, rect: Rect) {
    let source = icon.source();
    let size = rect.width().min(rect.height());
    let scale = size / 24.0;
    let dx = rect.x0 + (rect.width() - 24.0 * scale) * 0.5;
    let dy = rect.y0 + (rect.height() - 24.0 * scale) * 0.5;
    let transform = Affine::translate((dx, dy)) * Affine::scale(scale);
    let stroke = Stroke::new(2.0)
        .with_caps(Cap::Round)
        .with_join(Join::Round);
    for data in source.paths {
        if let Ok(path) = BezPath::from_svg(data) {
            scene.stroke(&stroke, transform, color, None, &path);
        }
    }
    for &(cx, cy, radius) in source.circles {
        scene.stroke(
            &stroke,
            transform,
            color,
            None,
            &Circle::new((cx, cy), radius),
        );
    }
    for &(x, y, width, height, _radius) in source.rects {
        scene.stroke(
            &stroke,
            transform,
            color,
            None,
            &Rect::new(x, y, x + width, y + height),
        );
    }
}

impl Widget for LucideIconWidget {
    type Action = NoAction;

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}
    fn property_changed(&mut self, _ctx: &mut UpdateCtx<'_>, _property_type: TypeId) {}
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
        bc.constrain(Size::new(self.size, self.size))
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, scene: &mut Scene) {
        draw_icon(scene, self.icon, self.color, ctx.size().to_rect());
    }

    fn accepts_pointer_interaction(&self) -> bool {
        false
    }

    fn accessibility_role(&self) -> Role {
        Role::Image
    }
    fn accessibility(
        &mut self,
        _ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        node: &mut Node,
    ) {
        node.set_label(self.label);
    }
    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }
    fn make_trace_span(&self, id: WidgetId) -> tracing::Span {
        tracing::trace_span!("LucideIcon", id = id.trace())
    }
}

pub(super) fn progress_ring(
    fraction: f64,
    track_color: Color,
    progress_color: Color,
    size: f64,
    label: &'static str,
) -> ProgressRingView {
    ProgressRingView {
        fraction: fraction.clamp(0.0, 1.0),
        track_color,
        progress_color,
        size,
        label,
    }
}

pub(super) struct ProgressRingView {
    fraction: f64,
    track_color: Color,
    progress_color: Color,
    size: f64,
    label: &'static str,
}

impl ViewMarker for ProgressRingView {}
impl<State, Action> View<State, Action, ViewCtx> for ProgressRingView {
    type Element = Pod<ProgressRingWidget>;
    type ViewState = ();

    fn build(&self, ctx: &mut ViewCtx, _: &mut State) -> (Self::Element, Self::ViewState) {
        (
            ctx.create_pod(ProgressRingWidget::new(
                self.fraction,
                self.track_color,
                self.progress_color,
                self.size,
                self.label,
            )),
            (),
        )
    }

    fn rebuild(
        &self,
        prev: &Self,
        (): &mut Self::ViewState,
        _ctx: &mut ViewCtx,
        mut element: Mut<'_, Self::Element>,
        _: &mut State,
    ) {
        if self.fraction != prev.fraction
            || self.track_color != prev.track_color
            || self.progress_color != prev.progress_color
            || self.size != prev.size
            || self.label != prev.label
        {
            ProgressRingWidget::set_style(
                &mut element,
                self.fraction,
                self.track_color,
                self.progress_color,
                self.size,
                self.label,
            );
        }
    }

    fn teardown(&self, (): &mut (), ctx: &mut ViewCtx, element: Mut<'_, Self::Element>) {
        ctx.teardown_leaf(element);
    }

    fn message(
        &self,
        (): &mut (),
        message: &mut MessageContext,
        _element: Mut<'_, Self::Element>,
        _state: &mut State,
    ) -> MessageResult<Action> {
        tracing::error!(?message, "progress ring received an unexpected message");
        MessageResult::Stale
    }
}

pub(super) struct ProgressRingWidget {
    fraction: f64,
    track_color: Color,
    progress_color: Color,
    size: f64,
    label: &'static str,
}

impl ProgressRingWidget {
    fn new(
        fraction: f64,
        track_color: Color,
        progress_color: Color,
        size: f64,
        label: &'static str,
    ) -> Self {
        Self {
            fraction,
            track_color,
            progress_color,
            size,
            label,
        }
    }

    fn set_style(
        this: &mut WidgetMut<'_, Self>,
        fraction: f64,
        track_color: Color,
        progress_color: Color,
        size: f64,
        label: &'static str,
    ) {
        this.widget.fraction = fraction;
        this.widget.track_color = track_color;
        this.widget.progress_color = progress_color;
        this.widget.size = size;
        this.widget.label = label;
        this.ctx.request_layout();
        this.ctx.request_paint_only();
    }
}

impl Widget for ProgressRingWidget {
    type Action = NoAction;

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}
    fn property_changed(&mut self, _ctx: &mut UpdateCtx<'_>, _property_type: TypeId) {}
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
        bc.constrain(Size::new(self.size, self.size))
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, scene: &mut Scene) {
        let rect = ctx.size().to_rect();
        let radius = (rect.width().min(rect.height()) * 0.5 - 2.0).max(1.0);
        let center = rect.center();
        let stroke = Stroke::new(3.0).with_caps(Cap::Round);
        scene.stroke(
            &stroke,
            Affine::IDENTITY,
            self.track_color,
            None,
            &Circle::new(center, radius),
        );
        if self.fraction > 0.0 {
            let arc = Arc::new(
                center,
                (radius, radius),
                -std::f64::consts::FRAC_PI_2,
                std::f64::consts::TAU * self.fraction.clamp(0.0, 1.0),
                0.0,
            );
            scene.stroke(&stroke, Affine::IDENTITY, self.progress_color, None, &arc);
        }
    }

    fn accepts_pointer_interaction(&self) -> bool {
        false
    }

    fn accessibility_role(&self) -> Role {
        Role::ProgressIndicator
    }

    fn accessibility(
        &mut self,
        _ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        node: &mut Node,
    ) {
        node.set_label(self.label);
        node.set_numeric_value(self.fraction * 100.0);
        node.set_min_numeric_value(0.0);
        node.set_max_numeric_value(100.0);
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }

    fn make_trace_span(&self, id: WidgetId) -> tracing::Span {
        tracing::trace_span!("ProgressRing", id = id.trace())
    }
}
