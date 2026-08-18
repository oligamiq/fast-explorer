use xilem::core::{MessageContext, MessageResult, Mut, View, ViewId, ViewMarker, ViewPathTracker};
use xilem::masonry::accesskit::{Action, Node, Role};
use xilem::masonry::core::keyboard::{Key, NamedKey};
use xilem::masonry::core::{
    AccessCtx, AccessEvent, BoxConstraints, ChildrenIds, ComposeCtx, EventCtx, LayoutCtx,
    NewWidget, PaintCtx, PointerButton, PointerButtonEvent, PointerEvent, PointerId, PointerType,
    PointerUpdate, Properties, PropertiesMut, PropertiesRef, RegisterCtx, StyleProperty, TextEvent,
    Update, UpdateCtx, Widget, WidgetId, WidgetMut, WidgetPod,
};
use xilem::masonry::kurbo::{Affine, Point, Rect, RoundedRect, Size, Stroke, Vec2};
use xilem::masonry::peniko::{Color, Fill};
use xilem::masonry::properties::types::AsUnit;
use xilem::masonry::properties::{
    Background, BorderColor, BorderWidth, ContentColor, CornerRadius,
};
use xilem::masonry::vello::Scene;
use xilem::masonry::widgets::{Flex, Label, SizedBox};
use xilem::{Pod, ViewCtx, WidgetView};

use crate::app::{AppState, TabStripDragPreviewHandle};
use crate::theme::Layout;

const TAB_CONTENT_VIEW_ID: ViewId = ViewId::new(0);
const GROUP_CONTENT_VIEW_ID: ViewId = ViewId::new(1);
const TAB_DRAG_THRESHOLD: f64 = 8.0;
const TOUCH_HOLD_SLOP_PX: f64 = 8.0;
const TOUCH_SCROLL_THRESHOLD: f64 = 12.0;
const TOUCH_DRAG_HOLD_NS: u64 = 350_000_000;
const TAB_GROUP_HOVER_NS: u64 = 300_000_000;
const TAB_GROUP_HOVER_STABILITY_PX: f64 = 8.0;
const TAB_GROUP_MIN_OVERLAP_RATIO: f64 = 0.20;
const GROUP_DROP_HYSTERESIS_PX: f64 = 12.0;
const GROUP_DROP_ENTER_INSET_PX: f64 = 4.0;
const HIDDEN_DRAG_CHILD_X: f64 = -100_000.0;
const PREVIEW_EASE_RATE: f64 = 24.0;
const PREVIEW_SNAP_DISTANCE: f64 = 0.2;
const TAB_STATE_EASE_RATE: f64 = 18.0;
const TAB_STATE_SNAP_DISTANCE: f64 = 0.01;
const TAB_APPEAR_OFFSET_PX: f64 = 8.0;

fn tab_layout_transition_offset(
    old_ids: &[u64],
    old_centers: &[f64],
    new_ids: &[u64],
    new_centers: &[f64],
    tab_id: u64,
    identity_changed: bool,
) -> Option<f64> {
    let old_index = old_ids.iter().position(|id| *id == tab_id);
    let new_index = new_ids.iter().position(|id| *id == tab_id);
    match (old_index, new_index) {
        (Some(old_index), Some(new_index)) => old_centers
            .get(old_index)
            .zip(new_centers.get(new_index))
            .map(|(old_center, new_center)| old_center - new_center),
        (None, Some(_)) if identity_changed => Some(-TAB_APPEAR_OFFSET_PX),
        _ => None,
    }
}

fn compose_layout_transition_offset(
    current_offset: f64,
    transition_offset: f64,
    identity_changed: bool,
) -> f64 {
    if identity_changed {
        transition_offset
    } else {
        current_offset + transition_offset
    }
}

fn eased_unit_progress(current: f64, target: f64, interval_ns: u64) -> f64 {
    if (target - current).abs() <= TAB_STATE_SNAP_DISTANCE {
        return target;
    }
    let dt = if interval_ns == 0 {
        1.0 / 60.0
    } else {
        (interval_ns as f64 / 1_000_000_000.0).min(0.05)
    };
    let alpha = 1.0 - (-TAB_STATE_EASE_RATE * dt).exp();
    current + (target - current) * alpha
}

fn eased_preview_offset(current: f64, target: f64, interval_ns: u64, direct: bool) -> f64 {
    if direct || (target - current).abs() <= PREVIEW_SNAP_DISTANCE {
        return target;
    }
    let dt = if interval_ns == 0 {
        1.0 / 60.0
    } else {
        (interval_ns as f64 / 1_000_000_000.0).min(0.05)
    };
    let alpha = 1.0 - (-PREVIEW_EASE_RATE * dt).exp();
    current + (target - current) * alpha
}

fn touch_hold_elapsed(down_time_ns: u64, current_time_ns: u64) -> bool {
    current_time_ns.saturating_sub(down_time_ns) >= TOUCH_DRAG_HOLD_NS
}

fn touch_displacement(start: Point, current: Point) -> f64 {
    let delta = current - start;
    delta.x.hypot(delta.y)
}

fn touch_hold_stable(start: Point, current: Point) -> bool {
    touch_displacement(start, current) <= TOUCH_HOLD_SLOP_PX
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroupLongPressIntent {
    Move,
    ContextMenu,
}

fn group_long_press_intent(collapsed: bool) -> GroupLongPressIntent {
    if collapsed {
        GroupLongPressIntent::Move
    } else {
        GroupLongPressIntent::ContextMenu
    }
}

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
    close_width: f64,
    background: Color,
    border: Color,
    text_color: Color,
) -> NewWidget<dyn Widget> {
    let mut title_props = Properties::new();
    title_props.insert(ContentColor::new(text_color));
    let title_label = NewWidget::new_with_props(
        Label::new(title.to_owned()).with_style(StyleProperty::FontSize(13.0)),
        title_props,
    );
    let title = NewWidget::new(
        Flex::row()
            .with_gap(0.0.px())
            .with_spacer(5.0.px())
            .with_flex_child(title_label, 1.0)
            .with_spacer(5.0.px())
            .must_fill_main_axis(true),
    );

    let mut close_props = Properties::new();
    close_props.insert(ContentColor::new(text_color));
    let close_label = NewWidget::new_with_props(
        Label::new("×").with_style(StyleProperty::FontSize(13.0)),
        close_props,
    );
    let close_centered = NewWidget::new(
        Flex::row()
            .with_gap(0.0.px())
            .with_flex_spacer(1.0)
            .with_child(close_label)
            .with_flex_spacer(1.0)
            .must_fill_main_axis(true),
    );
    let close = NewWidget::new(
        SizedBox::new(close_centered)
            .width(close_width.px())
            .height(close_width.px()),
    );
    let row = NewWidget::new(
        Flex::row()
            .with_gap(0.0.px())
            .with_flex_child(title, 1.0)
            .with_child(close)
            .must_fill_main_axis(true),
    );

    let mut box_props = Properties::new();
    box_props.insert(Background::Color(background));
    box_props.insert(BorderColor::new(border));
    box_props.insert(BorderWidth::all(1.0));
    box_props.insert(CornerRadius::all(4.0));
    let tab = NewWidget::new_with_props(
        SizedBox::new(row).width(width.px()).height(height.px()),
        box_props,
    );
    NewWidget::new(DragLayerRoot::new(tab, Size::new(width, height))).erased()
}

fn group_candidate_layer_widget(width: f64, height: f64, border: Color) -> NewWidget<dyn Widget> {
    let mut props = Properties::new();
    props.insert(Background::Color(Color::TRANSPARENT));
    props.insert(BorderColor::new(border));
    props.insert(BorderWidth::all(3.0));
    props.insert(CornerRadius::all(7.0));
    let outline = NewWidget::new_with_props(
        SizedBox::empty().width(width.px()).height(height.px()),
        props,
    );
    NewWidget::new(DragLayerRoot::new(outline, Size::new(width, height))).erased()
}

#[derive(Clone, PartialEq)]
pub(super) struct TabDropGroupSpan {
    pub group_id: u64,
    pub start: f64,
    pub header_center: f64,
    pub end: f64,
    pub color: Color,
}

fn tab_drop_group_with_hysteresis(
    spans: &[TabDropGroupSpan],
    dragged_center: f64,
    dragged_width: f64,
    source_group: Option<u64>,
    latched: Option<u64>,
) -> Option<u64> {
    // The slot preview moves a group header to the right as soon as the dragged
    // tab's leading edge crosses the header center. At that exact point the tab
    // is visually before its source group, so keeping the source membership would
    // make the preview and committed drop disagree. Exclude only the source group;
    // another group farther left can still become the drop target normally.
    let exited_source_to_left = source_group.filter(|group_id| {
        spans
            .iter()
            .find(|span| span.group_id == *group_id)
            .is_some_and(|span| dragged_center - dragged_width * 0.5 < span.header_center)
    });

    if let Some(group_id) = latched
        && Some(group_id) != exited_source_to_left
        && let Some(span) = spans.iter().find(|span| span.group_id == group_id)
        && dragged_center >= span.start - GROUP_DROP_HYSTERESIS_PX
        && dragged_center <= span.end + GROUP_DROP_HYSTERESIS_PX
    {
        return Some(group_id);
    }

    spans
        .iter()
        .filter(|span| Some(span.group_id) != exited_source_to_left)
        .find(|span| {
            let inset = GROUP_DROP_ENTER_INSET_PX.min((span.end - span.start).max(0.0) * 0.25);
            dragged_center >= span.start + inset && dragged_center <= span.end - inset
        })
        .map(|span| span.group_id)
}

fn tab_drop_group_color(spans: &[TabDropGroupSpan], group_id: Option<u64>) -> Option<Color> {
    let group_id = group_id?;
    spans
        .iter()
        .find(|span| span.group_id == group_id)
        .map(|span| span.color)
}

fn tab_drag_drop_border(
    spans: &[TabDropGroupSpan],
    target_group: Option<u64>,
    create_group: bool,
    ungrouped_border: Color,
    new_group_border: Color,
) -> Color {
    if create_group {
        return new_group_border;
    }
    tab_drop_group_color(spans, target_group).unwrap_or(ungrouped_border)
}

fn tab_drop_target_index(
    widths: &[f64],
    centers: &[f64],
    source_index: usize,
    drag_offset: f64,
) -> usize {
    let count = widths.len().min(centers.len());
    if count == 0 {
        return 0;
    }
    let source_index = source_index.min(count - 1);
    let source_center = centers[source_index];
    let source_half = widths[source_index] * 0.5;
    let mut target = source_index;

    if drag_offset < 0.0 {
        let dragged_leading = source_center + drag_offset - source_half;
        for index in (0..source_index).rev() {
            if dragged_leading < centers[index] {
                target = index;
            } else {
                break;
            }
        }
    } else if drag_offset > 0.0 {
        let dragged_trailing = source_center + drag_offset + source_half;
        for (index, center) in centers
            .iter()
            .copied()
            .take(count)
            .enumerate()
            .skip(source_index + 1)
        {
            if dragged_trailing > center {
                target = index;
            } else {
                break;
            }
        }
    }
    target
}

struct TabGroupCandidateGeometry<'a> {
    widths: &'a [f64],
    centers: &'a [f64],
    tab_ids: &'a [u64],
    tab_groups: &'a [Option<u64>],
    tab_slot_indices: &'a [usize],
    preview_slot_offsets: &'a [f64],
}

fn tab_group_candidate(
    geometry: TabGroupCandidateGeometry<'_>,
    source_index: usize,
    source_group: Option<u64>,
    drag_offset: f64,
) -> Option<(u64, usize)> {
    let TabGroupCandidateGeometry {
        widths,
        centers,
        tab_ids,
        tab_groups,
        tab_slot_indices,
        preview_slot_offsets,
    } = geometry;
    if source_group.is_some() {
        return None;
    }
    let count = widths
        .len()
        .min(centers.len())
        .min(tab_ids.len())
        .min(tab_groups.len())
        .min(tab_slot_indices.len());
    if count < 2 {
        return None;
    }
    let source = source_index.min(count - 1);
    let source_width = widths[source];
    let source_center = centers[source] + drag_offset;
    let source_left = source_center - source_width * 0.5;
    let source_right = source_center + source_width * 0.5;

    let mut best: Option<(f64, f64, u64, usize)> = None;
    for index in 0..count {
        if index == source || tab_groups[index].is_some() {
            continue;
        }
        let target_slot = tab_slot_indices[index];
        if preview_slot_offsets
            .get(target_slot)
            .copied()
            .unwrap_or(0.0)
            .abs()
            > PREVIEW_SNAP_DISTANCE
        {
            // Match the reference gesture: once the prospective layout pushes a
            // target tab out of its original position, it is no longer a grouping
            // candidate. The user must settle over the tab's new visible position.
            continue;
        }
        let target_width = widths[index];
        let target_left = centers[index] - target_width * 0.5;
        let target_right = centers[index] + target_width * 0.5;
        let overlap = source_right.min(target_right) - source_left.max(target_left);
        if overlap <= 0.0 {
            continue;
        }
        let ratio = overlap / source_width.min(target_width).max(1.0);
        if ratio < TAB_GROUP_MIN_OVERLAP_RATIO {
            continue;
        }
        let distance = (source_center - centers[index]).abs();
        let candidate = (ratio, -distance, tab_ids[index], index);
        if best.is_none_or(|current| {
            candidate.0 > current.0 || (candidate.0 == current.0 && candidate.1 > current.1)
        }) {
            best = Some(candidate);
        }
    }
    best.map(|(_, _, tab_id, index)| (tab_id, index))
}

fn tab_preview_targets(
    slot_widths: &[f64],
    slot_centers: &[f64],
    source_slot: usize,
    drag_offset: f64,
) -> (Vec<f64>, Vec<bool>) {
    let slot_count = slot_widths.len().min(slot_centers.len());
    let mut offsets = vec![0.0; slot_count];
    let direct_slots = vec![false; slot_count];
    if slot_count == 0 {
        return (offsets, direct_slots);
    }
    let source = source_slot.min(slot_count - 1);
    let target =
        tab_drop_target_index(slot_widths, slot_centers, source, drag_offset).min(slot_count - 1);
    let source_width = slot_widths[source];
    if target > source {
        for offset in &mut offsets[source + 1..=target] {
            *offset = -source_width;
        }
    } else if target < source {
        for offset in &mut offsets[target..source] {
            *offset = source_width;
        }
    }
    (offsets, direct_slots)
}

fn group_preview_targets(
    slot_widths: &[f64],
    block_slot_ranges: &[(usize, usize)],
    source_slot_start: usize,
    source_slot_end: usize,
    source_block: usize,
    target_block: usize,
    drag_offset: f64,
) -> (Vec<f64>, Vec<bool>) {
    let slot_count = slot_widths.len();
    let mut offsets = vec![0.0; slot_count];
    let mut direct_slots = vec![false; slot_count];
    if slot_count == 0 {
        return (offsets, direct_slots);
    }

    let source_start = source_slot_start.min(slot_count - 1);
    let source_end = source_slot_end.min(slot_count - 1).max(source_start);
    let source_width = slot_widths[source_start..=source_end].iter().sum::<f64>();
    for slot in source_start..=source_end {
        offsets[slot] = drag_offset;
        direct_slots[slot] = true;
    }

    if target_block > source_block {
        if let Some((_, target_end)) = block_slot_ranges.get(target_block).copied() {
            let target_end = target_end.min(slot_count - 1);
            if source_end < target_end {
                for offset in &mut offsets[source_end + 1..=target_end] {
                    *offset = -source_width;
                }
            }
        }
    } else if target_block < source_block
        && let Some((target_start, _)) = block_slot_ranges.get(target_block).copied()
    {
        let target_start = target_start.min(source_start);
        if target_start < source_start {
            for offset in &mut offsets[target_start..source_start] {
                *offset = source_width;
            }
        }
    }

    (offsets, direct_slots)
}

#[derive(Debug, Clone, Copy)]
pub(super) enum TabDragAction {
    PreviewStart,
    PreviewEnd,
    Select(u64),
    OpenMenu {
        tab_id: u64,
        anchor_x: f64,
    },
    OpenGroupMenu {
        group_id: u64,
        anchor_x: f64,
    },
    Drop {
        tab_id: u64,
        target_index: usize,
        target_group: Option<u64>,
        moving_right: bool,
    },
    CreateGroup {
        tab_id: u64,
        target_tab_id: u64,
        anchor_x: f64,
    },
    ToggleGroup(u64),
    MoveGroup {
        group_id: u64,
        target_tab_id: u64,
        after: bool,
    },
}

fn apply_drag_action(state: &mut AppState, action: TabDragAction) {
    match action {
        TabDragAction::PreviewStart => state.begin_tab_strip_drag_preview(),
        TabDragAction::PreviewEnd => state.end_tab_strip_drag_preview(),
        TabDragAction::Select(tab_id) => {
            state.end_tab_strip_drag_preview();
            state.select_tab_by_id(tab_id);
        }
        TabDragAction::OpenMenu { tab_id, anchor_x } => {
            state.end_tab_strip_drag_preview();
            state.open_tab_context_menu_at(tab_id, anchor_x)
        }
        TabDragAction::OpenGroupMenu { group_id, anchor_x } => {
            state.end_tab_strip_drag_preview();
            state.open_tab_group_editor_at(group_id, anchor_x)
        }
        TabDragAction::Drop {
            tab_id,
            target_index,
            target_group,
            moving_right,
        } => {
            state.end_tab_strip_drag_preview();
            state.drop_tab_at(tab_id, target_index, target_group, moving_right);
            state.select_tab_by_id(tab_id);
        }
        TabDragAction::CreateGroup {
            tab_id,
            target_tab_id,
            anchor_x,
        } => {
            state.end_tab_strip_drag_preview();
            state.create_tab_group_from_drag(tab_id, target_tab_id, anchor_x);
            state.select_tab_by_id(tab_id);
        }
        TabDragAction::ToggleGroup(group_id) => {
            state.end_tab_strip_drag_preview();
            state.toggle_tab_group_collapsed(group_id);
        }
        TabDragAction::MoveGroup {
            group_id,
            target_tab_id,
            after,
        } => {
            state.end_tab_strip_drag_preview();
            state.move_tab_group_near(group_id, target_tab_id, after);
        }
    }
}

#[derive(Clone, PartialEq)]
pub(super) struct TabDragConfig {
    pub tab_id: u64,
    pub source_index: usize,
    pub source_group: Option<u64>,
    pub drag_index: usize,
    pub tab_widths: Vec<f64>,
    pub tab_centers: Vec<f64>,
    pub tab_ids: Vec<u64>,
    pub tab_groups: Vec<Option<u64>>,
    pub tab_slot_indices: Vec<usize>,
    pub drop_targets: Vec<usize>,
    pub group_spans: Vec<TabDropGroupSpan>,
    pub slot_index: usize,
    pub slot_widths: Vec<f64>,
    pub slot_centers: Vec<f64>,
    pub preview_handle: TabStripDragPreviewHandle,
    pub preview_active: bool,
    pub scroll_leading: f64,
    pub drag_handle_right_inset: f64,
    pub accessibility_label: String,
    pub selected: bool,
    pub inactive_background: Color,
    pub active_background: Color,
    pub inactive_border: Color,
    pub active_border: Color,
    pub ungrouped_border: Color,
    pub new_group_borders: Vec<Color>,
    pub armed_border: Color,
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
        if let Some(layer_id) = element.widget.candidate_layer_root_id.take() {
            element.ctx.remove_layer(layer_id);
        }
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
                    apply_drag_action(state, *action);
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
    touch_start_local: Point,
    drag_origin_window: Point,
    drag_offset_x: f64,
    preview_offset_x: f64,
    layout_offset_x: f64,
    selection_progress: f64,
    hover_progress: f64,
    hover_target: f64,
    preview_announced: bool,
    layer_root_id: Option<WidgetId>,
    candidate_layer_root_id: Option<WidgetId>,
    layer_border: Option<Color>,
    latched_target_group: Option<u64>,
    group_candidate_tab_id: Option<u64>,
    group_candidate_slot: Option<usize>,
    group_candidate_elapsed_ns: u64,
    group_candidate_anchor_offset_x: f64,
    group_candidate_ready: bool,
    pending_touch: bool,
    touch_down_time_ns: u64,
    touch_hold_frame_ns: u64,
    touch_drag_armed: bool,
    touch_drag_moved: bool,
    touch_hold_cancelled: bool,
    touch_scroll_started: bool,
}

impl TabDragWidget {
    fn new(child: NewWidget<impl Widget + ?Sized>, config: TabDragConfig) -> Self {
        let selection_progress = if config.selected { 1.0 } else { 0.0 };
        Self {
            child: child.erased().to_pod(),
            config,
            drag_tab_id: None,
            drag_pointer: None,
            drag_start_local_x: 0.0,
            touch_start_local: Point::ORIGIN,
            drag_origin_window: Point::ORIGIN,
            drag_offset_x: 0.0,
            preview_offset_x: 0.0,
            layout_offset_x: 0.0,
            selection_progress,
            hover_progress: 0.0,
            hover_target: 0.0,
            preview_announced: false,
            layer_root_id: None,
            candidate_layer_root_id: None,
            layer_border: None,
            latched_target_group: None,
            group_candidate_tab_id: None,
            group_candidate_slot: None,
            group_candidate_elapsed_ns: 0,
            group_candidate_anchor_offset_x: 0.0,
            group_candidate_ready: false,
            pending_touch: false,
            touch_down_time_ns: 0,
            touch_hold_frame_ns: 0,
            touch_drag_armed: false,
            touch_drag_moved: false,
            touch_hold_cancelled: false,
            touch_scroll_started: false,
        }
    }

    fn child_mut<'a>(this: &'a mut WidgetMut<'_, Self>) -> WidgetMut<'a, dyn Widget> {
        this.ctx.get_mut(&mut this.widget.child)
    }

    fn set_config(this: &mut WidgetMut<'_, Self>, config: TabDragConfig) {
        if this.widget.config != config {
            let old_config = &this.widget.config;
            let identity_changed = old_config.tab_id != config.tab_id;
            let preview_changed = old_config.preview_active != config.preview_active;
            let selection_changed = old_config.selected != config.selected;
            let layout_offset = if !old_config.preview_active && !config.preview_active {
                tab_layout_transition_offset(
                    &old_config.tab_ids,
                    &old_config.tab_centers,
                    &config.tab_ids,
                    &config.tab_centers,
                    config.tab_id,
                    identity_changed,
                )
            } else {
                None
            };
            let should_reveal = config.selected
                && (!this.widget.config.selected
                    || this.widget.config.source_index != config.source_index
                    || this.widget.config.drag_index != config.drag_index
                    || this.widget.config.tab_widths != config.tab_widths
                    || this.widget.config.tab_centers != config.tab_centers
                    || this.widget.config.drop_targets != config.drop_targets
                    || (this.widget.config.scroll_leading - config.scroll_leading).abs()
                        > f64::EPSILON);
            this.widget.config = config;
            if let Some(offset) =
                layout_offset.filter(|offset| offset.abs() > PREVIEW_SNAP_DISTANCE)
            {
                // The layout has already committed its new geometry. Preserve the
                // current visual center across consecutive structural changes by
                // composing the new layout delta with any in-flight easing offset.
                // A recycled slot representing a different tab must not inherit the
                // previous tab's transient translation.
                this.widget.layout_offset_x = compose_layout_transition_offset(
                    this.widget.layout_offset_x,
                    offset,
                    identity_changed,
                );
            } else if preview_changed || identity_changed {
                this.widget.layout_offset_x = 0.0;
            }
            if identity_changed || (preview_changed && !this.widget.config.preview_active) {
                // Vec-based Xilem children are rebuilt in visual slot order. After a
                // committed reorder a wrapper may now represent a different tab, so
                // never carry the previous tab's transient drag translation into it.
                // The separate layout offset above is only retained for non-drag
                // structural changes such as closing a tab or toggling a group.
                this.widget.preview_offset_x = 0.0;
                if identity_changed {
                    this.widget.selection_progress = if this.widget.config.selected {
                        1.0
                    } else {
                        0.0
                    };
                    this.widget.hover_progress = 0.0;
                    this.widget.hover_target = 0.0;
                }
                if let Some(layer_id) = this.widget.candidate_layer_root_id.take() {
                    this.ctx.remove_layer(layer_id);
                }
            }
            if should_reveal {
                let size = this.ctx.size();
                this.ctx.request_scroll_to(Rect::new(
                    -this.widget.config.scroll_leading,
                    0.0,
                    size.width,
                    size.height,
                ));
            }
            if preview_changed
                || selection_changed
                || this.widget.config.preview_active
                || this.widget.preview_offset_x != 0.0
                || this.widget.layout_offset_x.abs() > PREVIEW_SNAP_DISTANCE
            {
                this.ctx.request_anim_frame();
            }
            this.ctx.request_render();
            this.ctx.request_compose();
        }
    }

    fn animated_background(&self) -> Color {
        self.config.inactive_background.lerp_rect(
            self.config.active_background,
            self.selection_progress.clamp(0.0, 1.0) as f32,
        )
    }

    fn animated_border(&self) -> Color {
        self.config.inactive_border.lerp_rect(
            self.config.active_border,
            self.selection_progress.clamp(0.0, 1.0) as f32,
        )
    }

    fn dragged_center(&self) -> f64 {
        self.config
            .tab_centers
            .get(self.config.drag_index)
            .copied()
            .unwrap_or(0.0)
            + self.drag_offset_x
    }

    fn drop_target_index(&self) -> usize {
        let visible_target = tab_drop_target_index(
            &self.config.tab_widths,
            &self.config.tab_centers,
            self.config.drag_index,
            self.drag_offset_x,
        );
        self.config
            .drop_targets
            .get(visible_target)
            .copied()
            .unwrap_or(self.config.source_index)
    }

    fn drop_target_group(&self) -> Option<u64> {
        self.latched_target_group
    }

    fn update_target_group_latch(&mut self) {
        let dragged_width = self
            .config
            .tab_widths
            .get(self.config.drag_index)
            .copied()
            .unwrap_or(0.0);
        self.latched_target_group = tab_drop_group_with_hysteresis(
            &self.config.group_spans,
            self.dragged_center(),
            dragged_width,
            self.config.source_group,
            self.latched_target_group,
        );
    }

    fn new_group_border(&self) -> Color {
        let Some(target_tab_id) = self.group_candidate_tab_id else {
            return self.config.armed_border;
        };
        let Some(index) = self
            .config
            .tab_ids
            .iter()
            .position(|tab_id| *tab_id == target_tab_id)
        else {
            return self.config.armed_border;
        };
        self.config
            .new_group_borders
            .get(index)
            .copied()
            .unwrap_or(self.config.armed_border)
    }

    fn preview_border(&self) -> Color {
        tab_drag_drop_border(
            &self.config.group_spans,
            self.drop_target_group(),
            self.group_candidate_ready,
            self.config.ungrouped_border,
            self.new_group_border(),
        )
    }

    fn update_group_candidate(&mut self) -> bool {
        let (preview_slot_offsets, _) = tab_preview_targets(
            &self.config.slot_widths,
            &self.config.slot_centers,
            self.config.slot_index,
            self.drag_offset_x,
        );
        let candidate = if self.latched_target_group.is_none() {
            tab_group_candidate(
                TabGroupCandidateGeometry {
                    widths: &self.config.tab_widths,
                    centers: &self.config.tab_centers,
                    tab_ids: &self.config.tab_ids,
                    tab_groups: &self.config.tab_groups,
                    tab_slot_indices: &self.config.tab_slot_indices,
                    preview_slot_offsets: &preview_slot_offsets,
                },
                self.config.drag_index,
                self.config.source_group,
                self.drag_offset_x,
            )
        } else {
            None
        };
        let candidate_tab_id = candidate.map(|(tab_id, _)| tab_id);
        let candidate_slot = candidate.and_then(|(_, visible_index)| {
            self.config.tab_slot_indices.get(visible_index).copied()
        });

        if candidate_tab_id != self.group_candidate_tab_id {
            self.group_candidate_tab_id = candidate_tab_id;
            self.group_candidate_slot = candidate_slot;
            self.group_candidate_elapsed_ns = 0;
            self.group_candidate_anchor_offset_x = self.drag_offset_x;
            self.group_candidate_ready = false;
            self.config.preview_handle.set_group_candidate(None, None);
            return candidate_tab_id.is_some();
        }

        if candidate_tab_id.is_some()
            && (self.drag_offset_x - self.group_candidate_anchor_offset_x).abs()
                > TAB_GROUP_HOVER_STABILITY_PX
        {
            self.group_candidate_elapsed_ns = 0;
            self.group_candidate_anchor_offset_x = self.drag_offset_x;
            if self.group_candidate_ready {
                self.group_candidate_ready = false;
                self.config.preview_handle.set_group_candidate(None, None);
            }
        }
        candidate_tab_id.is_some()
    }

    fn update_preview_targets(&self) {
        let (offsets, direct_slots) = tab_preview_targets(
            &self.config.slot_widths,
            &self.config.slot_centers,
            self.config.slot_index,
            self.drag_offset_x,
        );
        self.config
            .preview_handle
            .set_targets(offsets, direct_slots);
    }

    fn layer_position(&self) -> Point {
        Point::new(
            self.drag_origin_window.x + self.drag_offset_x,
            self.drag_origin_window.y - 2.0,
        )
    }

    fn clear_drag(&mut self) {
        self.config.preview_handle.set_group_candidate(None, None);
        self.drag_tab_id = None;
        self.drag_pointer = None;
        self.drag_offset_x = 0.0;
        self.preview_announced = false;
        self.layer_root_id = None;
        self.layer_border = None;
        self.latched_target_group = None;
        self.group_candidate_tab_id = None;
        self.group_candidate_slot = None;
        self.group_candidate_elapsed_ns = 0;
        self.group_candidate_anchor_offset_x = 0.0;
        self.group_candidate_ready = false;
        self.pending_touch = false;
        self.touch_down_time_ns = 0;
        self.touch_hold_frame_ns = 0;
        self.touch_drag_armed = false;
        self.touch_drag_moved = false;
        self.touch_hold_cancelled = false;
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
                button: Some(PointerButton::Secondary),
                state,
                ..
            }) => {
                self.clear_drag();
                let local = ctx.local_position(state.position);
                let anchor_x = ctx.to_window(local).x;
                ctx.submit_action::<TabDragAction>(TabDragAction::OpenMenu {
                    tab_id: self.config.tab_id,
                    anchor_x,
                });
                ctx.set_handled();
            }
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
                ctx.capture_pointer();
                self.drag_tab_id = None;
                self.drag_pointer = pointer.pointer_id;
                self.drag_start_local_x = local.x;
                self.touch_start_local = local;
                self.drag_origin_window = ctx.to_window(Point::ORIGIN);
                self.drag_offset_x = 0.0;
                self.preview_announced = false;
                self.layer_root_id = None;
                self.layer_border = None;
                self.latched_target_group = self.config.source_group;
                self.group_candidate_tab_id = None;
                self.group_candidate_slot = None;
                self.group_candidate_elapsed_ns = 0;
                self.group_candidate_anchor_offset_x = 0.0;
                self.group_candidate_ready = false;
                self.config.preview_handle.set_group_candidate(None, None);
                self.pending_touch = pointer.pointer_type == PointerType::Touch;
                self.touch_down_time_ns = if self.pending_touch { state.time } else { 0 };
                self.touch_hold_frame_ns = 0;
                self.touch_drag_armed = !self.pending_touch;
                self.touch_drag_moved = false;
                self.touch_hold_cancelled = false;
                self.touch_scroll_started = false;
                if self.pending_touch {
                    // A stationary long press must visibly enter move mode; do not
                    // wait for a later Move event to discover that the hold elapsed.
                    ctx.request_anim_frame();
                }
            }
            PointerEvent::Move(PointerUpdate {
                pointer, current, ..
            }) if self.drag_pointer == pointer.pointer_id => {
                let local = ctx.local_position(current.position);
                let offset = local.x - self.drag_start_local_x;
                if self.pending_touch && !self.touch_drag_armed {
                    let displacement = touch_displacement(self.touch_start_local, local);
                    if !touch_hold_stable(self.touch_start_local, local) {
                        self.touch_hold_cancelled = true;
                    }
                    if displacement >= TOUCH_SCROLL_THRESHOLD {
                        self.touch_scroll_started = true;
                    }
                    if !self.touch_hold_cancelled
                        && touch_hold_elapsed(self.touch_down_time_ns, current.time)
                    {
                        self.touch_drag_armed = true;
                        self.touch_hold_frame_ns = TOUCH_DRAG_HOLD_NS;
                        self.drag_tab_id = Some(self.config.tab_id);
                        self.drag_offset_x = offset;
                        self.update_preview_targets();
                        if !self.preview_announced {
                            self.preview_announced = true;
                            ctx.submit_action::<TabDragAction>(TabDragAction::PreviewStart);
                        }
                        if self.layer_root_id.is_none() {
                            let preview_border = self.preview_border();
                            let overlay = drag_layer_widget(
                                &self.config.accessibility_label,
                                ctx.size().width,
                                ctx.size().height,
                                self.config.drag_handle_right_inset,
                                self.animated_background(),
                                preview_border,
                                self.config.text_color,
                            );
                            self.layer_root_id = Some(overlay.id());
                            self.layer_border = Some(preview_border);
                            ctx.create_layer(overlay, self.layer_position());
                            ctx.request_compose();
                            ctx.request_render();
                        }
                    } else {
                        return;
                    }
                }
                if self.pending_touch && self.touch_scroll_started {
                    return;
                }
                if self.pending_touch && self.touch_drag_armed && offset.abs() > TOUCH_HOLD_SLOP_PX
                {
                    self.touch_drag_moved = true;
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
                    self.drag_tab_id = Some(self.config.tab_id);
                    self.drag_offset_x = offset;
                    self.update_preview_targets();
                    if !self.preview_announced {
                        self.preview_announced = true;
                        ctx.submit_action::<TabDragAction>(TabDragAction::PreviewStart);
                    }
                    if self.pending_touch {
                        ctx.set_handled();
                    }
                    if self.layer_root_id.is_none() {
                        let preview_border = self.preview_border();
                        let overlay = drag_layer_widget(
                            &self.config.accessibility_label,
                            ctx.size().width,
                            ctx.size().height,
                            self.config.drag_handle_right_inset,
                            self.animated_background(),
                            preview_border,
                            self.config.text_color,
                        );
                        self.layer_root_id = Some(overlay.id());
                        self.layer_border = Some(preview_border);
                        ctx.create_layer(overlay, self.layer_position());
                    }
                    ctx.request_compose();
                }
                self.drag_offset_x = offset;
                self.update_target_group_latch();
                let group_candidate_active = self.update_group_candidate();
                self.update_preview_targets();
                if group_candidate_active {
                    ctx.request_anim_frame();
                }
                if self.pending_touch {
                    ctx.set_handled();
                }

                // The floating tab mirrors the state that would be committed on
                // release: target-group color, prospective new-group color, or the
                // ordinary ungrouped-tab border when no group would be assigned.
                let preview_border = self.preview_border();
                if self.layer_border != Some(preview_border) {
                    if let Some(layer_id) = self.layer_root_id.take() {
                        ctx.remove_layer(layer_id);
                    }
                    let overlay = drag_layer_widget(
                        &self.config.accessibility_label,
                        ctx.size().width,
                        ctx.size().height,
                        self.config.drag_handle_right_inset,
                        self.animated_background(),
                        preview_border,
                        self.config.text_color,
                    );
                    self.layer_root_id = Some(overlay.id());
                    self.layer_border = Some(preview_border);
                    ctx.create_layer(overlay, self.layer_position());
                } else if let Some(layer_id) = self.layer_root_id {
                    ctx.reposition_layer(layer_id, self.layer_position());
                }
                ctx.request_compose();
                ctx.request_render();
            }
            PointerEvent::Up(PointerButtonEvent { pointer, state, .. })
                if self.drag_pointer == pointer.pointer_id =>
            {
                let layer_id = self.layer_root_id.take();
                if let Some(layer_id) = layer_id {
                    ctx.remove_layer(layer_id);
                }
                let Some(tab_id) = self.drag_tab_id else {
                    let should_select = !self.pending_touch
                        || (!self.touch_scroll_started && !self.touch_hold_cancelled);
                    let open_touch_menu = self.pending_touch
                        && !self.touch_scroll_started
                        && !self.touch_hold_cancelled
                        && (self.touch_drag_armed
                            || touch_hold_elapsed(self.touch_down_time_ns, state.time));
                    let tab_id = self.config.tab_id;
                    let local = ctx.local_position(state.position);
                    let anchor_x = ctx.to_window(local).x;
                    self.clear_drag();
                    ctx.request_compose();
                    ctx.release_pointer();
                    ctx.request_render();
                    if open_touch_menu {
                        ctx.submit_action::<TabDragAction>(TabDragAction::OpenMenu {
                            tab_id,
                            anchor_x,
                        });
                    } else if should_select {
                        ctx.submit_action::<TabDragAction>(TabDragAction::Select(tab_id));
                    }
                    return;
                };
                if self.pending_touch && self.touch_drag_armed && !self.touch_drag_moved {
                    let local = ctx.local_position(state.position);
                    let anchor_x = ctx.to_window(local).x;
                    let tab_id = self.config.tab_id;
                    self.clear_drag();
                    ctx.request_compose();
                    ctx.release_pointer();
                    ctx.request_render();
                    ctx.submit_action::<TabDragAction>(TabDragAction::OpenMenu {
                        tab_id,
                        anchor_x,
                    });
                    return;
                }
                let source_index = self.config.source_index;
                let source_group = self.config.source_group;
                let target_index = self.drop_target_index();
                let target_group = self.drop_target_group();
                let moving_right = self.drag_offset_x >= 0.0;
                let changed = target_index != source_index || target_group != source_group;
                let group_target = self
                    .group_candidate_ready
                    .then_some(self.group_candidate_tab_id)
                    .flatten();
                let local = ctx.local_position(state.position);
                let anchor_x = ctx.to_window(local).x;
                let preview_announced = self.preview_announced;
                self.clear_drag();
                ctx.request_compose();
                ctx.release_pointer();
                ctx.request_render();
                if let Some(target_tab_id) = group_target {
                    ctx.submit_action::<TabDragAction>(TabDragAction::CreateGroup {
                        tab_id,
                        target_tab_id,
                        anchor_x,
                    });
                } else if changed {
                    ctx.submit_action::<TabDragAction>(TabDragAction::Drop {
                        tab_id,
                        target_index,
                        target_group,
                        moving_right,
                    });
                } else if preview_announced {
                    ctx.submit_action::<TabDragAction>(TabDragAction::PreviewEnd);
                }
            }
            PointerEvent::Cancel(pointer) if self.drag_pointer == pointer.pointer_id => {
                if let Some(layer_id) = self.layer_root_id.take() {
                    ctx.remove_layer(layer_id);
                }
                let preview_announced = self.preview_announced;
                self.clear_drag();
                ctx.request_compose();
                ctx.release_pointer();
                ctx.request_render();
                if preview_announced {
                    ctx.submit_action::<TabDragAction>(TabDragAction::PreviewEnd);
                }
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
            && (event.key == Key::Named(NamedKey::Enter)
                || matches!(&event.key, Key::Character(value) if value == " "))
        {
            ctx.submit_action::<TabDragAction>(TabDragAction::Select(self.config.tab_id));
            ctx.set_handled();
        }
    }

    fn on_access_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &AccessEvent,
    ) {
        match event.action {
            Action::Click => {
                ctx.submit_action::<TabDragAction>(TabDragAction::Select(self.config.tab_id));
            }
            Action::ShowContextMenu => {
                ctx.submit_action::<TabDragAction>(TabDragAction::OpenMenu {
                    tab_id: self.config.tab_id,
                    anchor_x: 8.0,
                });
            }
            _ => {}
        }
    }
    fn on_anim_frame(
        &mut self,
        ctx: &mut UpdateCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        interval: u64,
    ) {
        let mut needs_next_frame = false;

        let selection_target = if self.config.selected { 1.0 } else { 0.0 };
        let next_selection =
            eased_unit_progress(self.selection_progress, selection_target, interval);
        if (next_selection - self.selection_progress).abs() > f64::EPSILON {
            self.selection_progress = next_selection;
            ctx.request_render();
        }
        if (selection_target - self.selection_progress).abs() > TAB_STATE_SNAP_DISTANCE {
            needs_next_frame = true;
        }

        let next_hover = eased_unit_progress(self.hover_progress, self.hover_target, interval);
        if (next_hover - self.hover_progress).abs() > f64::EPSILON {
            self.hover_progress = next_hover;
            ctx.request_render();
        }
        if (self.hover_target - self.hover_progress).abs() > TAB_STATE_SNAP_DISTANCE {
            needs_next_frame = true;
        }

        if self.pending_touch
            && !self.touch_drag_armed
            && !self.touch_hold_cancelled
            && !self.touch_scroll_started
            && self.drag_pointer.is_some()
        {
            self.touch_hold_frame_ns = self.touch_hold_frame_ns.saturating_add(interval);
            if self.touch_hold_frame_ns < TOUCH_DRAG_HOLD_NS {
                needs_next_frame = true;
            } else {
                self.touch_drag_armed = true;
                self.drag_tab_id = Some(self.config.tab_id);
                self.update_preview_targets();
                if !self.preview_announced {
                    self.preview_announced = true;
                    ctx.submit_action::<TabDragAction>(TabDragAction::PreviewStart);
                }
                if self.layer_root_id.is_none() {
                    let preview_border = self.preview_border();
                    let overlay = drag_layer_widget(
                        &self.config.accessibility_label,
                        ctx.size().width,
                        ctx.size().height,
                        self.config.drag_handle_right_inset,
                        self.animated_background(),
                        preview_border,
                        self.config.text_color,
                    );
                    self.layer_root_id = Some(overlay.id());
                    self.layer_border = Some(preview_border);
                    ctx.create_layer(overlay, self.layer_position());
                }
                ctx.request_compose();
                ctx.request_render();
            }
        }

        if self.drag_tab_id.is_some()
            && self.group_candidate_tab_id.is_some()
            && !self.group_candidate_ready
        {
            self.group_candidate_elapsed_ns =
                self.group_candidate_elapsed_ns.saturating_add(interval);
            if self.group_candidate_elapsed_ns >= TAB_GROUP_HOVER_NS {
                self.group_candidate_ready = true;
                let preview_border = self.preview_border();
                self.config
                    .preview_handle
                    .set_group_candidate(self.group_candidate_slot, Some(preview_border));

                if self.layer_border != Some(preview_border) {
                    if let Some(layer_id) = self.layer_root_id.take() {
                        ctx.remove_layer(layer_id);
                    }
                    let overlay = drag_layer_widget(
                        &self.config.accessibility_label,
                        ctx.size().width,
                        ctx.size().height,
                        self.config.drag_handle_right_inset,
                        self.animated_background(),
                        preview_border,
                        self.config.text_color,
                    );
                    self.layer_root_id = Some(overlay.id());
                    self.layer_border = Some(preview_border);
                    ctx.create_layer(overlay, self.layer_position());
                }
                ctx.request_render();
            } else {
                needs_next_frame = true;
            }
        }

        let (target, direct) = if self.config.preview_active {
            self.config
                .preview_handle
                .target_for_slot(self.config.slot_index)
        } else {
            (0.0, false)
        };
        let next = eased_preview_offset(self.preview_offset_x, target, interval, direct);
        if (next - self.preview_offset_x).abs() > f64::EPSILON {
            self.preview_offset_x = next;
            ctx.request_compose();
            ctx.request_render();
        }
        if self.config.preview_active
            || (target - self.preview_offset_x).abs() > PREVIEW_SNAP_DISTANCE
        {
            needs_next_frame = true;
        }

        let next_layout = eased_preview_offset(self.layout_offset_x, 0.0, interval, false);
        if (next_layout - self.layout_offset_x).abs() > f64::EPSILON {
            self.layout_offset_x = next_layout;
            ctx.request_compose();
            ctx.request_render();
        }
        if self.layout_offset_x.abs() > PREVIEW_SNAP_DISTANCE {
            needs_next_frame = true;
        }

        let is_candidate_target = self.config.preview_active
            && self.config.preview_handle.group_candidate_slot() == Some(self.config.slot_index)
            && self.drag_tab_id.is_none();
        if is_candidate_target {
            let margin = 3.0;
            let size = ctx.size();
            let position = ctx.window_origin() + Vec2::new(self.preview_offset_x - margin, -margin);
            if let Some(layer_id) = self.candidate_layer_root_id {
                ctx.reposition_layer(layer_id, position);
            } else {
                let overlay = group_candidate_layer_widget(
                    size.width + margin * 2.0,
                    size.height + margin * 2.0,
                    self.config
                        .preview_handle
                        .group_candidate_border()
                        .unwrap_or(self.config.armed_border),
                );
                self.candidate_layer_root_id = Some(overlay.id());
                ctx.create_layer(overlay, position);
            }
        } else if let Some(layer_id) = self.candidate_layer_root_id.take() {
            ctx.remove_layer(layer_id);
        }

        if needs_next_frame {
            ctx.request_anim_frame();
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
            ctx.request_anim_frame();
        }
        if let Update::HoveredChanged(hovered) = event {
            self.hover_target = if *hovered { 1.0 } else { 0.0 };
            ctx.request_anim_frame();
        }
        if matches!(event, Update::ActiveChanged(_) | Update::FocusChanged(_)) {
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
        // Keep the original slot in layout as a browser-style placeholder. The
        // grouping candidate outline is a separate top-level layer, so both the
        // tab and its outline use exactly this same X translation.
        let translation = if self.layer_root_id.is_some() {
            Vec2::new(HIDDEN_DRAG_CHILD_X, 0.0)
        } else {
            Vec2::new(self.preview_offset_x + self.layout_offset_x, 0.0)
        };
        ctx.set_child_scroll_translation(&mut self.child, translation);
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, scene: &mut Scene) {
        let paint_transform =
            Affine::translate(Vec2::new(self.preview_offset_x + self.layout_offset_x, 0.0));
        if self.layer_root_id.is_none() {
            let rect = ctx.size().to_rect().inset(0.5);
            let rounded = RoundedRect::from_rect(rect, Layout::RADIUS);
            scene.fill(
                Fill::NonZero,
                paint_transform,
                self.animated_background(),
                None,
                &rounded,
            );
            if self.hover_progress > TAB_STATE_SNAP_DISTANCE {
                scene.fill(
                    Fill::NonZero,
                    paint_transform,
                    self.config
                        .armed_border
                        .multiply_alpha((0.055 * self.hover_progress) as f32),
                    None,
                    &rounded,
                );
            }
            scene.stroke(
                &Stroke::new(1.0),
                paint_transform,
                self.animated_border(),
                None,
                &rounded,
            );

            if self.selection_progress > TAB_STATE_SNAP_DISTANCE {
                let max_width = (rect.width() - 14.0).max(0.0);
                let width = max_width * self.selection_progress;
                if width > 0.0 {
                    let center = rect.center().x;
                    let indicator = Rect::new(
                        center - width * 0.5,
                        rect.y1 - 2.0,
                        center + width * 0.5,
                        rect.y1,
                    )
                    .to_rounded_rect(1.0);
                    scene.fill(
                        Fill::NonZero,
                        paint_transform,
                        self.config
                            .armed_border
                            .multiply_alpha(self.selection_progress as f32),
                        None,
                        &indicator,
                    );
                }
            }
        }

        if ctx.is_focus_target() {
            let rect = ctx.size().to_rect().inset(2.0);
            scene.stroke(
                &Stroke::new(1.5),
                paint_transform,
                self.config.armed_border,
                None,
                &rect,
            );
        }
    }

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
        node.add_action(Action::ShowContextMenu);
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
        true
    }
}

#[derive(Clone, PartialEq)]
pub(super) struct GroupDragConfig {
    pub group_id: u64,
    pub collapsed: bool,
    pub source_block_index: usize,
    pub block_widths: Vec<f64>,
    pub block_centers: Vec<f64>,
    pub block_target_tab_ids: Vec<u64>,
    pub block_slot_ranges: Vec<(usize, usize)>,
    pub source_slot_start: usize,
    pub source_slot_end: usize,
    pub slot_widths: Vec<f64>,
    pub preview_handle: TabStripDragPreviewHandle,
    pub preview_active: bool,
    pub accessibility_label: String,
    pub background: Color,
    pub border: Color,
    pub text_color: Color,
}

pub(super) fn group_drag_button<V>(child: V, config: GroupDragConfig) -> GroupDragView<V>
where
    V: WidgetView<AppState>,
{
    GroupDragView { child, config }
}

pub(super) struct GroupDragView<V> {
    child: V,
    config: GroupDragConfig,
}

impl<V> ViewMarker for GroupDragView<V> {}

impl<V> View<AppState, (), ViewCtx> for GroupDragView<V>
where
    V: WidgetView<AppState>,
{
    type Element = Pod<GroupDragWidget>;
    type ViewState = V::ViewState;

    fn build(&self, ctx: &mut ViewCtx, state: &mut AppState) -> (Self::Element, Self::ViewState) {
        let (child, child_state) =
            ctx.with_id(GROUP_CONTENT_VIEW_ID, |ctx| self.child.build(ctx, state));
        let widget = GroupDragWidget::new(child.new_widget, self.config.clone());
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
        GroupDragWidget::set_config(&mut element, self.config.clone());
        ctx.with_id(GROUP_CONTENT_VIEW_ID, |ctx| {
            let mut child = GroupDragWidget::child_mut(&mut element);
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
        ctx.with_id(GROUP_CONTENT_VIEW_ID, |ctx| {
            let mut child = GroupDragWidget::child_mut(&mut element);
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
            Some(GROUP_CONTENT_VIEW_ID) => {
                let mut child = GroupDragWidget::child_mut(&mut element);
                self.child
                    .message(view_state, message, child.downcast(), state)
            }
            None => match message.take_message::<TabDragAction>() {
                Some(action) => {
                    apply_drag_action(state, *action);
                    MessageResult::Action(())
                }
                None => MessageResult::Stale,
            },
            _ => MessageResult::Stale,
        }
    }
}

fn group_layer_widget(
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
        Label::new(title.to_owned()).with_style(StyleProperty::FontSize(12.0)),
        label_props,
    );
    let row = NewWidget::new(
        Flex::row()
            .with_gap(0.0.px())
            .with_spacer(10.0.px())
            .with_flex_child(label, 1.0)
            .with_spacer(10.0.px())
            .must_fill_main_axis(true),
    );
    let mut props = Properties::new();
    props.insert(Background::Color(background));
    props.insert(BorderColor::new(border));
    props.insert(BorderWidth::all(2.0));
    props.insert(CornerRadius::all(4.0));
    let surface = NewWidget::new_with_props(
        SizedBox::new(row).width(width.px()).height(height.px()),
        props,
    );
    NewWidget::new(DragLayerRoot::new(surface, Size::new(width, height))).erased()
}

pub(super) struct GroupDragWidget {
    child: WidgetPod<dyn Widget>,
    config: GroupDragConfig,
    pointer: Option<PointerId>,
    start_local_x: f64,
    touch_start_local: Point,
    origin_window: Point,
    offset_x: f64,
    preview_offset_x: f64,
    collapse_progress: f64,
    hover_progress: f64,
    hover_target: f64,
    preview_announced: bool,
    dragging: bool,
    layer_root_id: Option<WidgetId>,
    pending_touch: bool,
    touch_down_time_ns: u64,
    touch_hold_frame_ns: u64,
    touch_drag_armed: bool,
    touch_hold_cancelled: bool,
    touch_context_armed: bool,
    touch_scroll_started: bool,
}

impl GroupDragWidget {
    fn new(child: NewWidget<impl Widget + ?Sized>, config: GroupDragConfig) -> Self {
        let collapse_progress = if config.collapsed { 1.0 } else { 0.0 };
        Self {
            child: child.erased().to_pod(),
            config,
            pointer: None,
            start_local_x: 0.0,
            touch_start_local: Point::ORIGIN,
            origin_window: Point::ORIGIN,
            offset_x: 0.0,
            preview_offset_x: 0.0,
            collapse_progress,
            hover_progress: 0.0,
            hover_target: 0.0,
            preview_announced: false,
            dragging: false,
            layer_root_id: None,
            pending_touch: false,
            touch_down_time_ns: 0,
            touch_hold_frame_ns: 0,
            touch_drag_armed: false,
            touch_hold_cancelled: false,
            touch_context_armed: false,
            touch_scroll_started: false,
        }
    }

    fn child_mut<'a>(this: &'a mut WidgetMut<'_, Self>) -> WidgetMut<'a, dyn Widget> {
        this.ctx.get_mut(&mut this.widget.child)
    }

    fn set_config(this: &mut WidgetMut<'_, Self>, config: GroupDragConfig) {
        if this.widget.config != config {
            let identity_changed = this.widget.config.group_id != config.group_id;
            let preview_changed = this.widget.config.preview_active != config.preview_active;
            let collapsed_changed = this.widget.config.collapsed != config.collapsed;
            this.widget.config = config;
            if identity_changed || (preview_changed && !this.widget.config.preview_active) {
                this.widget.preview_offset_x = 0.0;
            }
            if identity_changed {
                this.widget.collapse_progress = if this.widget.config.collapsed {
                    1.0
                } else {
                    0.0
                };
                this.widget.hover_progress = 0.0;
                this.widget.hover_target = 0.0;
            }
            if preview_changed
                || collapsed_changed
                || this.widget.config.preview_active
                || this.widget.preview_offset_x != 0.0
            {
                this.ctx.request_anim_frame();
            }
            this.ctx.request_render();
            this.ctx.request_compose();
        }
    }

    fn target_block_index(&self) -> usize {
        tab_drop_target_index(
            &self.config.block_widths,
            &self.config.block_centers,
            self.config.source_block_index,
            self.offset_x,
        )
    }

    fn update_preview_targets(&self) {
        let target_block = self.target_block_index();
        let (offsets, direct_slots) = group_preview_targets(
            &self.config.slot_widths,
            &self.config.block_slot_ranges,
            self.config.source_slot_start,
            self.config.source_slot_end,
            self.config.source_block_index,
            target_block,
            self.offset_x,
        );
        self.config
            .preview_handle
            .set_targets(offsets, direct_slots);
    }

    fn layer_position(&self) -> Point {
        Point::new(
            self.origin_window.x + self.offset_x,
            self.origin_window.y - 2.0,
        )
    }

    fn clear(&mut self) {
        self.pointer = None;
        self.offset_x = 0.0;
        self.preview_announced = false;
        self.dragging = false;
        self.layer_root_id = None;
        self.pending_touch = false;
        self.touch_down_time_ns = 0;
        self.touch_hold_frame_ns = 0;
        self.touch_drag_armed = false;
        self.touch_hold_cancelled = false;
        self.touch_context_armed = false;
        self.touch_scroll_started = false;
    }
}

impl Widget for GroupDragWidget {
    type Action = TabDragAction;

    fn on_pointer_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &PointerEvent,
    ) {
        match event {
            PointerEvent::Down(PointerButtonEvent {
                button: Some(PointerButton::Secondary),
                state,
                ..
            }) => {
                self.clear();
                let local = ctx.local_position(state.position);
                let anchor_x = ctx.to_window(local).x;
                ctx.submit_action::<TabDragAction>(TabDragAction::OpenGroupMenu {
                    group_id: self.config.group_id,
                    anchor_x,
                });
                ctx.set_handled();
            }
            PointerEvent::Down(PointerButtonEvent {
                button,
                pointer,
                state,
                ..
            }) if button.is_none() || matches!(button, Some(PointerButton::Primary)) => {
                let local = ctx.local_position(state.position);
                ctx.capture_pointer();
                self.pointer = pointer.pointer_id;
                self.start_local_x = local.x;
                self.touch_start_local = local;
                self.origin_window = ctx.to_window(Point::ORIGIN);
                self.offset_x = 0.0;
                self.preview_announced = false;
                self.dragging = false;
                self.layer_root_id = None;
                self.pending_touch = pointer.pointer_type == PointerType::Touch;
                self.touch_down_time_ns = if self.pending_touch { state.time } else { 0 };
                self.touch_hold_frame_ns = 0;
                self.touch_drag_armed = !self.pending_touch;
                self.touch_hold_cancelled = false;
                self.touch_context_armed = false;
                self.touch_scroll_started = false;
                if self.pending_touch {
                    ctx.request_anim_frame();
                }
            }
            PointerEvent::Move(PointerUpdate {
                pointer, current, ..
            }) if self.pointer == pointer.pointer_id => {
                let local = ctx.local_position(current.position);
                let offset = local.x - self.start_local_x;
                if self.pending_touch && !self.touch_drag_armed && !self.touch_context_armed {
                    let displacement = touch_displacement(self.touch_start_local, local);
                    if !touch_hold_stable(self.touch_start_local, local) {
                        self.touch_hold_cancelled = true;
                    }
                    if displacement >= TOUCH_SCROLL_THRESHOLD {
                        self.touch_scroll_started = true;
                    }
                    if !self.touch_hold_cancelled
                        && touch_hold_elapsed(self.touch_down_time_ns, current.time)
                    {
                        self.touch_hold_frame_ns = TOUCH_DRAG_HOLD_NS;
                        if group_long_press_intent(self.config.collapsed)
                            == GroupLongPressIntent::Move
                        {
                            self.touch_drag_armed = true;
                            self.dragging = true;
                            self.offset_x = offset;
                            self.update_preview_targets();
                            if !self.preview_announced {
                                self.preview_announced = true;
                                ctx.submit_action::<TabDragAction>(TabDragAction::PreviewStart);
                            }
                            if self.layer_root_id.is_none() {
                                let overlay = group_layer_widget(
                                    &self.config.accessibility_label,
                                    ctx.size().width,
                                    ctx.size().height,
                                    self.config.background,
                                    self.config.border,
                                    self.config.text_color,
                                );
                                self.layer_root_id = Some(overlay.id());
                                ctx.create_layer(overlay, self.layer_position());
                            }
                        } else {
                            self.touch_context_armed = true;
                            let anchor_x = self.origin_window.x + self.touch_start_local.x;
                            ctx.submit_action::<TabDragAction>(TabDragAction::OpenGroupMenu {
                                group_id: self.config.group_id,
                                anchor_x,
                            });
                            ctx.set_handled();
                            ctx.request_render();
                        }
                    } else {
                        return;
                    }
                }
                if self.pending_touch && (self.touch_scroll_started || self.touch_context_armed) {
                    return;
                }
                if !self.dragging {
                    if self.pending_touch {
                        return;
                    }
                    if offset.abs() < TAB_DRAG_THRESHOLD {
                        return;
                    }
                    self.dragging = true;
                    self.offset_x = offset;
                    self.update_preview_targets();
                    if !self.preview_announced {
                        self.preview_announced = true;
                        ctx.submit_action::<TabDragAction>(TabDragAction::PreviewStart);
                    }
                    if self.layer_root_id.is_none() {
                        let overlay = group_layer_widget(
                            &self.config.accessibility_label,
                            ctx.size().width,
                            ctx.size().height,
                            self.config.background,
                            self.config.border,
                            self.config.text_color,
                        );
                        self.layer_root_id = Some(overlay.id());
                        ctx.create_layer(overlay, self.layer_position());
                    }
                    ctx.request_compose();
                }
                self.offset_x = offset;
                self.update_preview_targets();
                if self.pending_touch {
                    ctx.set_handled();
                }
                if let Some(layer_id) = self.layer_root_id {
                    ctx.reposition_layer(layer_id, self.layer_position());
                }
                ctx.request_render();
            }
            PointerEvent::Up(PointerButtonEvent { pointer, .. })
                if self.pointer == pointer.pointer_id =>
            {
                if let Some(layer_id) = self.layer_root_id.take() {
                    ctx.remove_layer(layer_id);
                }
                let was_dragging = self.dragging;
                let was_scroll = self.touch_scroll_started;
                let was_touch = self.pending_touch;
                let was_armed = self.touch_drag_armed;
                let was_cancelled = self.touch_hold_cancelled;
                let open_context = self.touch_context_armed && !self.config.collapsed;
                let target_block = self.target_block_index();
                let source_block = self.config.source_block_index;
                let target_tab_id = self.config.block_target_tab_ids.get(target_block).copied();
                let after = self.offset_x >= 0.0;
                let group_id = self.config.group_id;
                let preview_announced = self.preview_announced;
                self.clear();
                ctx.request_compose();
                ctx.release_pointer();
                ctx.request_render();
                if open_context {
                    // The expanded-group long press already opened the context
                    // menu at the hold threshold. Releasing must not toggle or move it.
                } else if was_dragging && target_block != source_block {
                    if let Some(target_tab_id) = target_tab_id {
                        ctx.submit_action::<TabDragAction>(TabDragAction::MoveGroup {
                            group_id,
                            target_tab_id,
                            after,
                        });
                    } else if preview_announced {
                        ctx.submit_action::<TabDragAction>(TabDragAction::PreviewEnd);
                    }
                } else if preview_announced {
                    ctx.submit_action::<TabDragAction>(TabDragAction::PreviewEnd);
                } else if !was_dragging
                    && !was_scroll
                    && !was_cancelled
                    && (!was_touch || !was_armed)
                {
                    ctx.submit_action::<TabDragAction>(TabDragAction::ToggleGroup(group_id));
                }
            }
            PointerEvent::Cancel(pointer) if self.pointer == pointer.pointer_id => {
                if let Some(layer_id) = self.layer_root_id.take() {
                    ctx.remove_layer(layer_id);
                }
                let preview_announced = self.preview_announced;
                self.clear();
                ctx.request_compose();
                ctx.release_pointer();
                ctx.request_render();
                if preview_announced {
                    ctx.submit_action::<TabDragAction>(TabDragAction::PreviewEnd);
                }
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
            && (event.key == Key::Named(NamedKey::Enter)
                || matches!(&event.key, Key::Character(value) if value == " "))
        {
            ctx.submit_action::<TabDragAction>(TabDragAction::ToggleGroup(self.config.group_id));
            ctx.set_handled();
        }
    }

    fn on_access_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &AccessEvent,
    ) {
        match event.action {
            Action::Click => {
                ctx.submit_action::<TabDragAction>(TabDragAction::ToggleGroup(
                    self.config.group_id,
                ));
            }
            Action::ShowContextMenu => {
                ctx.submit_action::<TabDragAction>(TabDragAction::OpenGroupMenu {
                    group_id: self.config.group_id,
                    anchor_x: 8.0,
                });
            }
            _ => {}
        }
    }

    fn on_anim_frame(
        &mut self,
        ctx: &mut UpdateCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        interval: u64,
    ) {
        let mut needs_next_frame = false;

        let collapse_target = if self.config.collapsed { 1.0 } else { 0.0 };
        let next_collapse = eased_unit_progress(self.collapse_progress, collapse_target, interval);
        if (next_collapse - self.collapse_progress).abs() > f64::EPSILON {
            self.collapse_progress = next_collapse;
            ctx.request_render();
        }
        if (collapse_target - self.collapse_progress).abs() > TAB_STATE_SNAP_DISTANCE {
            needs_next_frame = true;
        }

        let next_hover = eased_unit_progress(self.hover_progress, self.hover_target, interval);
        if (next_hover - self.hover_progress).abs() > f64::EPSILON {
            self.hover_progress = next_hover;
            ctx.request_render();
        }
        if (self.hover_target - self.hover_progress).abs() > TAB_STATE_SNAP_DISTANCE {
            needs_next_frame = true;
        }

        if self.pending_touch
            && !self.touch_drag_armed
            && !self.touch_context_armed
            && !self.touch_hold_cancelled
            && !self.touch_scroll_started
            && self.pointer.is_some()
        {
            self.touch_hold_frame_ns = self.touch_hold_frame_ns.saturating_add(interval);
            if self.touch_hold_frame_ns < TOUCH_DRAG_HOLD_NS {
                needs_next_frame = true;
            } else if group_long_press_intent(self.config.collapsed) == GroupLongPressIntent::Move {
                self.touch_drag_armed = true;
                self.dragging = true;
                self.update_preview_targets();
                if !self.preview_announced {
                    self.preview_announced = true;
                    ctx.submit_action::<TabDragAction>(TabDragAction::PreviewStart);
                }
                if self.layer_root_id.is_none() {
                    let overlay = group_layer_widget(
                        &self.config.accessibility_label,
                        ctx.size().width,
                        ctx.size().height,
                        self.config.background,
                        self.config.border,
                        self.config.text_color,
                    );
                    self.layer_root_id = Some(overlay.id());
                    ctx.create_layer(overlay, self.layer_position());
                }
                ctx.request_compose();
                ctx.request_render();
            } else {
                self.touch_context_armed = true;
                let anchor_x = self.origin_window.x + self.touch_start_local.x;
                ctx.submit_action::<TabDragAction>(TabDragAction::OpenGroupMenu {
                    group_id: self.config.group_id,
                    anchor_x,
                });
                ctx.request_render();
            }
        }

        let (target, direct) = if self.config.preview_active {
            self.config
                .preview_handle
                .target_for_slot(self.config.source_slot_start)
        } else {
            (0.0, false)
        };
        let next = eased_preview_offset(self.preview_offset_x, target, interval, direct);
        if (next - self.preview_offset_x).abs() > f64::EPSILON {
            self.preview_offset_x = next;
            ctx.request_compose();
            ctx.request_render();
        }
        if self.config.preview_active
            || (target - self.preview_offset_x).abs() > PREVIEW_SNAP_DISTANCE
        {
            needs_next_frame = true;
        }

        if needs_next_frame {
            ctx.request_anim_frame();
        }
    }

    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        if let Update::HoveredChanged(hovered) = event {
            self.hover_target = if *hovered { 1.0 } else { 0.0 };
            ctx.request_anim_frame();
        }
        if matches!(event, Update::ActiveChanged(_) | Update::FocusChanged(_)) {
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
            Vec2::new(HIDDEN_DRAG_CHILD_X, 0.0)
        } else {
            Vec2::new(self.preview_offset_x, 0.0)
        };
        ctx.set_child_scroll_translation(&mut self.child, translation);
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, scene: &mut Scene) {
        let transform = Affine::translate(Vec2::new(self.preview_offset_x, 0.0));
        if self.layer_root_id.is_none() {
            let rect = ctx.size().to_rect().inset(0.5);
            let rounded = RoundedRect::from_rect(rect, Layout::RADIUS);
            let collapsed_tint = self.config.background.lerp_rect(self.config.border, 0.07);
            let background = self.config.background.lerp_rect(
                collapsed_tint,
                self.collapse_progress.clamp(0.0, 1.0) as f32,
            );
            scene.fill(Fill::NonZero, transform, background, None, &rounded);
            if self.hover_progress > TAB_STATE_SNAP_DISTANCE {
                scene.fill(
                    Fill::NonZero,
                    transform,
                    self.config
                        .border
                        .multiply_alpha((0.045 * self.hover_progress) as f32),
                    None,
                    &rounded,
                );
            }
            scene.stroke(
                &Stroke::new(1.0),
                transform,
                self.config.border,
                None,
                &rounded,
            );
        }
        if ctx.is_focus_target() {
            let rect = ctx.size().to_rect().inset(2.0);
            scene.stroke(
                &Stroke::new(1.5),
                transform,
                self.config.text_color,
                None,
                &rect,
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
        node.set_label(self.config.accessibility_label.clone());
        node.add_action(Action::Click);
        node.add_action(Action::ShowContextMenu);
    }

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        ctx.register_child(&mut self.child);
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::from_slice(&[self.child.id()])
    }

    fn accepts_pointer_interaction(&self) -> bool {
        true
    }

    fn accepts_focus(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GroupLongPressIntent, TOUCH_DRAG_HOLD_NS, TabDropGroupSpan, TabGroupCandidateGeometry,
        compose_layout_transition_offset, eased_preview_offset, group_long_press_intent,
        group_preview_targets, tab_drag_drop_border, tab_drop_group_with_hysteresis,
        tab_drop_target_index, tab_group_candidate, tab_layout_transition_offset,
        tab_preview_targets, touch_hold_elapsed, touch_hold_stable,
    };
    use xilem::masonry::kurbo::Point;
    use xilem::masonry::peniko::Color;

    #[test]
    fn touch_hold_uses_relative_input_timestamp() {
        let down = 9_000_000_000_u64;
        assert!(!touch_hold_elapsed(down, down + TOUCH_DRAG_HOLD_NS - 1));
        assert!(touch_hold_elapsed(down, down + TOUCH_DRAG_HOLD_NS));
    }

    #[test]
    fn touch_hold_requires_two_dimensional_stability() {
        let start = Point::ORIGIN;
        assert!(touch_hold_stable(start, Point::new(5.0, 5.0)));
        assert!(!touch_hold_stable(start, Point::new(7.0, 6.0)));
    }

    #[test]
    fn group_long_press_depends_on_collapsed_state() {
        assert_eq!(group_long_press_intent(true), GroupLongPressIntent::Move);
        assert_eq!(
            group_long_press_intent(false),
            GroupLongPressIntent::ContextMenu
        );
    }

    #[test]
    fn closing_a_tab_keeps_survivors_at_their_previous_visual_centers() {
        let old_ids = [1, 2, 3, 4];
        let old_centers = [50.0, 150.0, 250.0, 350.0];
        let new_ids = [1, 3, 4];
        let new_centers = [50.0, 150.0, 250.0];
        assert_eq!(
            tab_layout_transition_offset(&old_ids, &old_centers, &new_ids, &new_centers, 3, true,),
            Some(100.0)
        );
        assert_eq!(
            tab_layout_transition_offset(&old_ids, &old_centers, &new_ids, &new_centers, 4, true,),
            Some(100.0)
        );
    }

    #[test]
    fn newly_visible_group_member_gets_only_a_small_entrance_offset() {
        let old_ids = [1, 4];
        let old_centers = [50.0, 150.0];
        let new_ids = [1, 2, 3, 4];
        let new_centers = [50.0, 150.0, 250.0, 350.0];
        assert_eq!(
            tab_layout_transition_offset(&old_ids, &old_centers, &new_ids, &new_centers, 2, true,),
            Some(-8.0)
        );
    }

    #[test]
    fn consecutive_layout_transitions_preserve_the_current_visual_center() {
        assert_eq!(compose_layout_transition_offset(50.0, 100.0, false), 150.0);
    }

    #[test]
    fn recycled_tab_slot_does_not_inherit_the_previous_tabs_offset() {
        assert_eq!(compose_layout_transition_offset(50.0, -8.0, true), -8.0);
    }

    #[test]
    fn drag_reorders_when_dragged_edge_crosses_neighbor_center() {
        let widths = [100.0, 120.0, 80.0];
        let centers = [50.0, 160.0, 260.0];

        // Video behavior: moving right swaps when the dragged tab's trailing
        // edge crosses the next tab's center, not when center crosses center.
        assert_eq!(tab_drop_target_index(&widths, &centers, 1, 39.0), 1);
        assert_eq!(tab_drop_target_index(&widths, &centers, 1, 41.0), 2);

        // The same rule is symmetric when moving left.
        assert_eq!(tab_drop_target_index(&widths, &centers, 1, -49.0), 1);
        assert_eq!(tab_drop_target_index(&widths, &centers, 1, -51.0), 0);
    }

    #[test]
    fn slight_left_drag_does_not_immediately_reorder() {
        let widths = [100.0, 120.0, 80.0];
        let centers = [50.0, 160.0, 260.0];
        assert_eq!(tab_drop_target_index(&widths, &centers, 2, -1.0), 2);
        assert_eq!(tab_drop_target_index(&widths, &centers, 2, -59.0), 2);
        assert_eq!(tab_drop_target_index(&widths, &centers, 2, -61.0), 1);
    }

    #[test]
    fn live_tab_preview_opens_exactly_one_tab_width() {
        let widths = [100.0, 120.0, 80.0];
        let centers = [50.0, 160.0, 260.0];
        let (right, direct) = tab_preview_targets(&widths, &centers, 0, 61.0);
        assert_eq!(right, vec![0.0, -100.0, 0.0]);
        assert_eq!(direct, vec![false, false, false]);

        let (left, _) = tab_preview_targets(&widths, &centers, 2, -61.0);
        assert_eq!(left, vec![0.0, 80.0, 0.0]);
    }

    #[test]
    fn ungrouped_tab_overlap_can_become_a_group_candidate() {
        let widths = [100.0, 100.0];
        let centers = [50.0, 150.0];
        let ids = [10, 20];
        let ungrouped = [None, None];

        let slot_indices = [0, 1];
        let preview_offsets = [0.0, 0.0];

        assert_eq!(
            tab_group_candidate(
                TabGroupCandidateGeometry {
                    widths: &widths,
                    centers: &centers,
                    tab_ids: &ids,
                    tab_groups: &ungrouped,
                    tab_slot_indices: &slot_indices,
                    preview_slot_offsets: &preview_offsets,
                },
                0,
                None,
                19.0,
            ),
            None
        );
        assert_eq!(
            tab_group_candidate(
                TabGroupCandidateGeometry {
                    widths: &widths,
                    centers: &centers,
                    tab_ids: &ids,
                    tab_groups: &ungrouped,
                    tab_slot_indices: &slot_indices,
                    preview_slot_offsets: &preview_offsets,
                },
                0,
                None,
                21.0,
            ),
            Some((20, 1))
        );
        assert_eq!(
            tab_group_candidate(
                TabGroupCandidateGeometry {
                    widths: &widths,
                    centers: &centers,
                    tab_ids: &ids,
                    tab_groups: &[None, Some(7)],
                    tab_slot_indices: &slot_indices,
                    preview_slot_offsets: &preview_offsets,
                },
                0,
                None,
                50.0,
            ),
            None
        );
    }

    #[test]
    fn moved_preview_tab_is_not_a_group_candidate() {
        let widths = [100.0, 100.0];
        let centers = [50.0, 150.0];
        let ids = [10, 20];
        let ungrouped = [None, None];
        let slot_indices = [0, 1];
        let moved_target = [0.0, -100.0];

        assert_eq!(
            tab_group_candidate(
                TabGroupCandidateGeometry {
                    widths: &widths,
                    centers: &centers,
                    tab_ids: &ids,
                    tab_groups: &ungrouped,
                    tab_slot_indices: &slot_indices,
                    preview_slot_offsets: &moved_target,
                },
                0,
                None,
                61.0,
            ),
            None
        );
    }

    #[test]
    fn drag_border_matches_the_state_committed_on_release() {
        let group_color = Color::from_rgb8(210, 67, 67);
        let ungrouped = Color::from_rgb8(90, 90, 90);
        let new_group = Color::from_rgb8(50, 115, 220);
        let spans = [TabDropGroupSpan {
            group_id: 7,
            start: 100.0,
            header_center: 120.0,
            end: 200.0,
            color: group_color,
        }];

        assert_eq!(
            tab_drag_drop_border(&spans, Some(7), false, ungrouped, new_group),
            group_color
        );
        assert_eq!(
            tab_drag_drop_border(&spans, None, false, ungrouped, new_group),
            ungrouped
        );
        assert_eq!(
            tab_drag_drop_border(&spans, None, true, ungrouped, new_group),
            new_group
        );
    }

    #[test]
    fn group_drop_boundary_uses_hysteresis() {
        let spans = [TabDropGroupSpan {
            group_id: 7,
            start: 100.0,
            header_center: 120.0,
            end: 200.0,
            color: Color::BLACK,
        }];

        assert_eq!(
            tab_drop_group_with_hysteresis(&spans, 101.0, 0.0, None, None),
            None
        );
        assert_eq!(
            tab_drop_group_with_hysteresis(&spans, 105.0, 0.0, None, None),
            Some(7)
        );
        assert_eq!(
            tab_drop_group_with_hysteresis(&spans, 209.0, 0.0, None, Some(7)),
            Some(7)
        );
        assert_eq!(
            tab_drop_group_with_hysteresis(&spans, 213.0, 0.0, None, Some(7)),
            None
        );
    }

    #[test]
    fn dragging_group_member_before_its_header_ungroups_it() {
        let spans = [TabDropGroupSpan {
            group_id: 7,
            start: 100.0,
            header_center: 120.0,
            end: 360.0,
            color: Color::BLACK,
        }];

        // With a 100px tab, its left edge at 121px is still to the right of
        // the header center, so the source-group latch remains intact.
        assert_eq!(
            tab_drop_group_with_hysteresis(&spans, 171.0, 100.0, Some(7), Some(7)),
            Some(7)
        );
        // Crossing the header center is exactly when the slot preview places
        // the tab before the group header; membership must disappear too.
        assert_eq!(
            tab_drop_group_with_hysteresis(&spans, 169.0, 100.0, Some(7), Some(7)),
            None
        );
    }

    #[test]
    fn live_group_preview_moves_members_together_and_opens_block_width() {
        let widths = [90.0, 100.0, 110.0, 80.0, 70.0];
        let ranges = [(0, 2), (3, 3), (4, 4)];
        let (right, direct) = group_preview_targets(&widths, &ranges, 0, 2, 0, 1, 240.0);
        assert_eq!(right, vec![240.0, 240.0, 240.0, -300.0, 0.0]);
        assert_eq!(direct, vec![true, true, true, false, false]);

        let (left, direct) = group_preview_targets(&widths, &ranges, 3, 3, 1, 0, -180.0);
        assert_eq!(left, vec![80.0, 80.0, 80.0, -180.0, 0.0]);
        assert_eq!(direct, vec![false, false, false, true, false]);
    }

    #[test]
    fn preview_easing_is_smooth_but_direct_slots_follow_pointer() {
        let eased = eased_preview_offset(0.0, 100.0, 16_000_000, false);
        assert!(eased > 0.0 && eased < 100.0);
        assert_eq!(eased_preview_offset(0.0, 100.0, 16_000_000, true), 100.0);
    }
}
