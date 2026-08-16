// Copyright 2018 the Xilem Authors and the Druid Authors
// SPDX-License-Identifier: Apache-2.0

use std::any::TypeId;

use accesskit::{Node, Role};
use masonry_core::core::HasProperty;
use tracing::{Span, trace_span};
use vello::Scene;
use vello::kurbo::{Affine, Point, Rect, Size, Vec2};

use crate::core::{
    AccessCtx, ArcStr, BoxConstraints, ChildrenIds, ComposeCtx, LayoutCtx, NewWidget, NoAction,
    PaintCtx, Properties, PropertiesMut, PropertiesRef, RegisterCtx, Update, UpdateCtx, Widget,
    WidgetId, WidgetMut, WidgetPod,
};
use crate::properties::{
    Background, BorderColor, BorderWidth, BoxShadow, CaretColor, ContentColor, CornerRadius,
    DisabledBackground, Padding, PlaceholderColor, SelectionColor, UnfocusedSelectionColor,
};
use crate::util::{fill, stroke};
use crate::widgets::{Label, TextArea};

const CARET_SCROLL_MARGIN: f64 = 8.0;
const MARQUEE_START_HOLD_SECONDS: f64 = 0.8;
const DEFAULT_MARQUEE_END_HOLD_SECONDS: f64 = 3.0;
const MARQUEE_SPEED: f64 = 30.0;

/// The text input widget displays text which can be edited by the user,
/// inside a surrounding box.
///
/// This currently does not support newlines entered by the user,
/// although pre-existing newlines are handled correctly.
///
/// This widget itself does not emit any actions.
/// However, the child widget will do so, as it is user editable.
/// The ID of the child can be accessed using [`area_pod`](Self::area_pod).
///
/// At runtime, most properties of the text will be set using [`text_mut`](Self::text_mut).
/// This is because `TextInput` largely serves as a wrapper around a [`TextArea`].
pub struct TextInput {
    text: WidgetPod<TextArea<true>>,
    placeholder: WidgetPod<Label>,
    placeholder_text: ArcStr,

    /// Whether to clip the contained text.
    clip: bool,
    auto_focus: bool,
    initial_selection: Option<(usize, usize)>,
    /// Current horizontal translation used to keep the caret visible or run a marquee.
    scroll_x: f64,
    /// Maximum scroll based on the laid-out text width and current viewport.
    max_scroll_x: f64,
    /// Animate overflowing unfocused text from start to end and back.
    marquee_when_unfocused: bool,
    marquee_end_hold_seconds: f64,
    marquee_elapsed: f64,
    child_focused: bool,
}

impl TextInput {
    /// Create a new `TextInput` with the given text.
    ///
    /// To use non-default text properties, use [`from_text_area`](Self::from_text_area) instead.
    pub fn new(text: &str) -> Self {
        Self::from_text_area(TextArea::new_editable(text).with_auto_id())
    }

    /// Create a new `TextInput` from a styled text area.
    pub fn from_text_area(text: NewWidget<TextArea<true>>) -> Self {
        Self {
            text: text.to_pod(),
            placeholder: NewWidget::new_with_props(Label::new(""), Properties::new()).to_pod(),
            placeholder_text: "".into(),
            clip: false,
            auto_focus: false,
            initial_selection: None,
            scroll_x: 0.0,
            max_scroll_x: 0.0,
            marquee_when_unfocused: false,
            marquee_end_hold_seconds: DEFAULT_MARQUEE_END_HOLD_SECONDS,
            marquee_elapsed: 0.0,
            child_focused: false,
        }
    }

    /// The text that will be displayed when this input is empty.
    ///
    /// To modify this on active text input, use [`set_placeholder`](Self::set_placeholder).
    pub fn with_placeholder(mut self, placeholder_text: impl Into<ArcStr>) -> Self {
        let placeholder_text = placeholder_text.into();
        self.placeholder =
            NewWidget::new_with_props(Label::new(placeholder_text.clone()), Properties::new())
                .to_pod();
        self.placeholder_text = placeholder_text;
        self
    }

    /// Whether to clip the text to the drawn boundaries.
    ///
    /// If this is set to true, it is recommended, but not required, that this
    /// wraps a text area with [word wrapping](TextArea::with_word_wrap) enabled.
    ///
    /// To modify this on active text input, use [`set_clip`](Self::set_clip).
    pub fn with_clip(mut self, clip: bool) -> Self {
        self.clip = clip;
        self
    }

    /// Animate overflowing text while this input is unfocused.
    ///
    /// The animation moves only from start to end, pauses there, then snaps
    /// back to the start. Focusing the editor suspends the animation.
    pub fn with_marquee_when_unfocused(mut self, enabled: bool) -> Self {
        self.marquee_when_unfocused = enabled;
        self
    }

    /// Set how long an unfocused marquee waits at the end before snapping back.
    pub fn with_marquee_end_hold_seconds(mut self, seconds: f64) -> Self {
        self.marquee_end_hold_seconds = seconds.clamp(0.5, 10.0);
        self
    }

    /// Give the inner editor focus as soon as this input is inserted.
    pub fn with_auto_focus(mut self, auto_focus: bool) -> Self {
        self.auto_focus = auto_focus;
        self
    }

    /// Select the given byte range when this input is inserted.
    pub fn with_initial_selection(mut self, start: usize, end: usize) -> Self {
        self.initial_selection = Some((start, end));
        self
    }

    /// Read the underlying text area.
    ///
    /// Useful for getting its ID, as most actions from the text input will be sent by the child.
    pub fn area_pod(&self) -> &WidgetPod<TextArea<true>> {
        &self.text
    }
}

// --- MARK: WIDGETMUT
impl TextInput {
    /// Edit the underlying text area.
    ///
    /// Used to modify most properties of the text.
    pub fn text_mut<'t>(this: &'t mut WidgetMut<'_, Self>) -> WidgetMut<'t, TextArea<true>> {
        this.ctx.get_mut(&mut this.widget.text)
    }

    /// Edit the child label representing the placeholder text.
    pub fn placeholder_mut<'t>(this: &'t mut WidgetMut<'_, Self>) -> WidgetMut<'t, Label> {
        this.ctx.get_mut(&mut this.widget.placeholder)
    }

    /// The text that will be displayed when this input is empty.
    ///
    /// The runtime equivalent of [`with_placeholder`](Self::with_placeholder).
    pub fn set_placeholder(this: &mut WidgetMut<'_, Self>, placeholder_text: impl Into<ArcStr>) {
        Label::set_text(&mut Self::placeholder_mut(this), placeholder_text);
    }

    /// Whether to clip the text to the drawn boundaries.
    ///
    /// If this is set to true, it is recommended, but not required, that this
    /// wraps a text area with [word wrapping](TextArea::set_word_wrap) enabled.
    ///
    /// The runtime equivalent of [`with_clip`](Self::with_clip).
    pub fn set_clip(this: &mut WidgetMut<'_, Self>, clip: bool) {
        this.widget.clip = clip;
        this.ctx.request_layout();
    }

    /// Enable or disable the unfocused marquee for an active input.
    pub fn set_marquee_when_unfocused(this: &mut WidgetMut<'_, Self>, enabled: bool) {
        if this.widget.marquee_when_unfocused == enabled {
            return;
        }
        this.widget.marquee_when_unfocused = enabled;
        this.widget.marquee_elapsed = 0.0;
        if !enabled && !this.widget.child_focused {
            this.widget.scroll_x = 0.0;
        }
        if enabled && !this.widget.child_focused {
            this.ctx.request_anim_frame();
        }
        this.ctx.request_compose();
    }

    /// Change how long the marquee waits at the end before resetting.
    pub fn set_marquee_end_hold_seconds(this: &mut WidgetMut<'_, Self>, seconds: f64) {
        let seconds = seconds.clamp(0.5, 10.0);
        if (this.widget.marquee_end_hold_seconds - seconds).abs() <= f64::EPSILON {
            return;
        }
        this.widget.marquee_end_hold_seconds = seconds;
        this.widget.marquee_elapsed = 0.0;
        if this.widget.marquee_when_unfocused && !this.widget.child_focused {
            this.ctx.request_anim_frame();
        }
    }

    /// Restart the unfocused marquee after the displayed value changes.
    pub fn restart_marquee(this: &mut WidgetMut<'_, Self>) {
        this.widget.marquee_elapsed = 0.0;
        if this.widget.marquee_when_unfocused && !this.widget.child_focused {
            this.widget.scroll_x = 0.0;
            this.ctx.request_compose();
            this.ctx.request_anim_frame();
        }
    }
}

impl HasProperty<Background> for TextInput {}
impl HasProperty<CaretColor> for TextInput {}
impl HasProperty<DisabledBackground> for TextInput {}
impl HasProperty<BorderColor> for TextInput {}
impl HasProperty<BorderWidth> for TextInput {}
impl HasProperty<BoxShadow> for TextInput {}
impl HasProperty<CornerRadius> for TextInput {}
impl HasProperty<Padding> for TextInput {}
impl HasProperty<PlaceholderColor> for TextInput {}
impl HasProperty<SelectionColor> for TextInput {}
impl HasProperty<UnfocusedSelectionColor> for TextInput {}

// --- MARK: IMPL WIDGET
impl Widget for TextInput {
    type Action = NoAction;

    fn on_anim_frame(
        &mut self,
        ctx: &mut UpdateCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        interval: u64,
    ) {
        if !self.marquee_when_unfocused || self.child_focused || self.max_scroll_x <= 0.0 {
            return;
        }

        self.marquee_elapsed += interval as f64 * 1e-9;
        let travel_seconds = self.max_scroll_x / MARQUEE_SPEED;
        let cycle =
            MARQUEE_START_HOLD_SECONDS + travel_seconds + self.marquee_end_hold_seconds;
        let phase = self.marquee_elapsed.rem_euclid(cycle);
        self.scroll_x = if phase < MARQUEE_START_HOLD_SECONDS {
            0.0
        } else if phase < MARQUEE_START_HOLD_SECONDS + travel_seconds {
            (phase - MARQUEE_START_HOLD_SECONDS) * MARQUEE_SPEED
        } else {
            self.max_scroll_x
        }
        .clamp(0.0, self.max_scroll_x);

        ctx.request_compose();
        ctx.request_anim_frame();
    }

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        ctx.register_child(&mut self.text);
        ctx.register_child(&mut self.placeholder);
    }

    fn property_changed(&mut self, ctx: &mut UpdateCtx<'_>, property_type: TypeId) {
        DisabledBackground::prop_changed(ctx, property_type);
        Background::prop_changed(ctx, property_type);
        BorderColor::prop_changed(ctx, property_type);
        BorderWidth::prop_changed(ctx, property_type);
        CornerRadius::prop_changed(ctx, property_type);
        Padding::prop_changed(ctx, property_type);
        // TODO: Draw shadows in post_paint.
        BoxShadow::prop_changed(ctx, property_type);

        // FIXME - Find more elegant way to propagate property to child.
        if property_type == TypeId::of::<CaretColor>() {
            ctx.mutate_self_later(|mut input| {
                let mut input = input.downcast::<Self>();
                let color = *input.get_prop::<CaretColor>();
                let mut text_area = Self::text_mut(&mut input);
                text_area.insert_prop(color);
            });
        } else if property_type == TypeId::of::<SelectionColor>() {
            ctx.mutate_self_later(|mut input| {
                let mut input = input.downcast::<Self>();
                let color = *input.get_prop::<SelectionColor>();
                let mut text_area = Self::text_mut(&mut input);
                text_area.insert_prop(color);
            });
        } else if property_type == TypeId::of::<UnfocusedSelectionColor>() {
            ctx.mutate_self_later(|mut input| {
                let mut input = input.downcast::<Self>();
                let color = *input.get_prop::<UnfocusedSelectionColor>();
                let mut text_area = Self::text_mut(&mut input);
                text_area.insert_prop(color);
            });
        } else if property_type == TypeId::of::<PlaceholderColor>() {
            ctx.mutate_self_later(|mut input| {
                let mut input = input.downcast::<Self>();
                let color = input.get_prop::<PlaceholderColor>().color;
                let mut label = Self::placeholder_mut(&mut input);
                label.insert_prop(ContentColor::new(color));
            });
        }
    }

    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        match event {
            Update::WidgetAdded => {
                // FIXME - Find more elegant way to propagate property to child.
                ctx.mutate_self_later(|mut input| {
                    let mut input = input.downcast::<Self>();
                    let color = *input.get_prop::<CaretColor>();
                    let mut text_area = Self::text_mut(&mut input);
                    text_area.insert_prop(color);
                });
                ctx.mutate_self_later(|mut input| {
                    let mut input = input.downcast::<Self>();
                    let color = *input.get_prop::<SelectionColor>();
                    let mut text_area = Self::text_mut(&mut input);
                    text_area.insert_prop(color);
                });
                ctx.mutate_self_later(|mut input| {
                    let mut input = input.downcast::<Self>();
                    let color = *input.get_prop::<UnfocusedSelectionColor>();
                    let mut text_area = Self::text_mut(&mut input);
                    text_area.insert_prop(color);
                });
                ctx.mutate_self_later(|mut input| {
                    let mut input = input.downcast::<Self>();
                    let color = input.get_prop::<PlaceholderColor>().color;
                    let mut label = Self::placeholder_mut(&mut input);
                    label.insert_prop(ContentColor::new(color));
                });
                if self.auto_focus {
                    ctx.set_focus(self.text.id());
                }
                if let Some((start, end)) = self.initial_selection {
                    ctx.mutate_later(&mut self.text, move |mut text_area| {
                        TextArea::select_byte_range(&mut text_area, start, end);
                    });
                }
                if self.marquee_when_unfocused && !self.auto_focus {
                    ctx.request_anim_frame();
                }
            }
            // We check for `ChildFocusChanged` instead of `FocusChanged`
            // because the actual widget that receives focus is the child `TextArea`.
            Update::ChildFocusChanged(focused) => {
                self.child_focused = *focused;
                self.marquee_elapsed = 0.0;
                if !focused {
                    self.scroll_x = 0.0;
                    if self.marquee_when_unfocused {
                        ctx.request_anim_frame();
                    }
                }
                ctx.request_layout();
                ctx.request_paint_only();
            }
            _ => {}
        }
    }

    fn layout(
        &mut self,
        ctx: &mut LayoutCtx<'_>,
        props: &mut PropertiesMut<'_>,
        bc: &BoxConstraints,
    ) -> Size {
        let border = props.get::<BorderWidth>();
        let padding = props.get::<Padding>();
        let shadow = props.get::<BoxShadow>();

        let bc = *bc;
        let bc = border.layout_down(bc);
        let bc = padding.layout_down(bc);

        // Keep single-line text at its natural height even when the surrounding
        // field has a fixed height. The child is centered below instead of
        // inheriting the field's minimum height and painting from its top edge.
        let text_bc = BoxConstraints::new(Size::new(bc.min().width, 0.0), bc.max());
        let text_size = ctx.run_layout(&mut self.text, &text_bc);
        let inner_size = bc.constrain(text_size);
        let text_y = ((inner_size.height - text_size.height) * 0.5).max(0.0);
        let baseline = ctx.child_baseline_offset(&self.text) + text_y;

        let viewport_width = if inner_size.width.is_finite() {
            inner_size.width.max(0.0)
        } else {
            text_size.width.max(0.0)
        };
        let (full_text_width, caret) = ctx.get_raw(&mut self.text).0.horizontal_scroll_metrics();
        self.max_scroll_x = (full_text_width - viewport_width).max(0.0);
        if self.max_scroll_x <= f64::EPSILON {
            self.scroll_x = 0.0;
        } else if self.child_focused {
            if let Some(caret) = caret {
                let margin = CARET_SCROLL_MARGIN.min(viewport_width * 0.25);
                let visible_left = self.scroll_x + margin;
                let visible_right = self.scroll_x + (viewport_width - margin).max(margin);
                if caret.x0 < visible_left {
                    self.scroll_x = (caret.x0 - margin).max(0.0);
                } else if caret.x1 > visible_right {
                    self.scroll_x = (caret.x1 - viewport_width + margin).min(self.max_scroll_x);
                }
            }
        } else if !self.marquee_when_unfocused {
            self.scroll_x = 0.0;
        }
        self.scroll_x = self.scroll_x.clamp(0.0, self.max_scroll_x);
        let (size, baseline) = padding.layout_up(inner_size, baseline);
        let (size, baseline) = border.layout_up(size, baseline);

        let pos = Point::ORIGIN;
        let pos = border.place_down(pos);
        let pos = padding.place_down(pos);
        ctx.place_child(&mut self.text, pos + Vec2::new(0.0, text_y));

        let text_is_empty = ctx.get_raw(&mut self.text).0.is_empty();

        ctx.set_stashed(&mut self.placeholder, !text_is_empty);
        if text_is_empty {
            let placeholder_size = ctx.run_layout(&mut self.placeholder, &text_bc);
            let placeholder_y = ((inner_size.height - placeholder_size.height) * 0.5).max(0.0);
            ctx.place_child(&mut self.placeholder, pos + Vec2::new(0.0, placeholder_y));
        }

        if shadow.is_visible() {
            ctx.set_paint_insets(shadow.get_insets());
        }

        if self.clip {
            // Clip horizontally at the content box, not merely at the border.
            // A marquee translates the text child, so without the padding in
            // this clip it can slide through the normal left/right inset and
            // appear to touch or overlap the outline.
            let inset = border.width.max(0.0);
            let left = (inset + padding.left).min(size.width);
            let right = (size.width - inset - padding.right).max(left);
            ctx.set_clip_path(Rect::new(
                left,
                inset,
                right,
                (size.height - inset).max(inset),
            ));
        }

        ctx.set_baseline_offset(baseline);
        size
    }

    fn compose(&mut self, ctx: &mut ComposeCtx<'_>) {
        ctx.set_child_scroll_translation(&mut self.text, Vec2::new(-self.scroll_x, 0.0));
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, props: &PropertiesRef<'_>, scene: &mut Scene) {
        let size = ctx.size();

        let border_width = props.get::<BorderWidth>();
        let border_radius = props.get::<CornerRadius>();
        let shadow = props.get::<BoxShadow>();
        let mut border_color = props.get::<BorderColor>();

        let bg = if ctx.is_disabled() {
            &props.get::<DisabledBackground>().0
        } else {
            props.get::<Background>()
        };

        let bg_rect = border_width.bg_rect(size, border_radius);
        let border_rect = border_width.border_rect(size, border_radius);

        let focus_border;
        if ctx.has_focus_target() {
            focus_border = BorderColor {
                color: props.get::<CaretColor>().color,
            };
            border_color = &focus_border;
        }

        shadow.paint(scene, Affine::IDENTITY, bg_rect);

        let brush = bg.get_peniko_brush_for_rect(bg_rect.rect());
        fill(scene, &bg_rect, &brush);
        stroke(scene, &border_rect, border_color.color, border_width.width);
    }

    fn accessibility_role(&self) -> Role {
        Role::GenericContainer
    }

    fn accessibility(
        &mut self,
        _ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        node: &mut Node,
    ) {
        node.set_placeholder(self.placeholder_text.to_string());
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::from_slice(&[self.text.id(), self.placeholder.id()])
    }

    fn make_trace_span(&self, id: WidgetId) -> Span {
        trace_span!("Prose", id = id.trace())
    }

    fn get_debug_text(&self) -> Option<String> {
        self.clip.then(|| "(clip)".into())
    }
}

// TODO - Add more tests
#[cfg(test)]
mod tests {
    use masonry_core::core::TextEvent;
    use vello::kurbo::Size;

    use super::*;
    use crate::core::StyleProperty;
    use crate::testing::{TestHarness, assert_render_snapshot};
    use crate::theme::default_property_set;
    use crate::widgets::TextArea;

    #[test]
    fn text_input_outline() {
        let text_input = NewWidget::new(TextInput::from_text_area(
            TextArea::new_editable("TextInput contents")
                .with_style(StyleProperty::FontSize(14.0))
                .with_auto_id(),
        ));
        let mut harness = TestHarness::create_with_size(
            default_property_set(),
            text_input,
            Size::new(150.0, 40.0),
        );

        assert_render_snapshot!(harness, "text_input_outline");

        let mut text_area_id = None;
        harness.edit_root_widget(|mut text_input| {
            let mut text_input = TextInput::text_mut(&mut text_input);
            text_area_id = Some(text_input.ctx.widget_id());

            TextArea::select_text(&mut text_input, "contents");
        });
        harness.focus_on(text_area_id);

        assert_render_snapshot!(harness, "text_input_selection");

        harness.process_text_event(TextEvent::WindowFocusChange(false));

        assert_render_snapshot!(harness, "text_input_selection_unfocused");

        harness.process_text_event(TextEvent::WindowFocusChange(true));
        harness.animate_ms(500 + 1);

        assert_render_snapshot!(harness, "text_input_cursor_blink");
    }

    #[test]
    fn placeholder() {
        let text_input = NewWidget::new(
            TextInput::from_text_area(
                TextArea::new_editable("")
                    .with_style(StyleProperty::FontSize(14.0))
                    .with_auto_id(),
            )
            .with_placeholder("HELLO WORLD"),
        );

        let mut harness = TestHarness::create_with_size(
            default_property_set(),
            text_input,
            Size::new(150.0, 40.0),
        );

        assert_render_snapshot!(harness, "text_input_placeholder");
    }

    #[test]
    fn text_input_clips() {
        let text_input = NewWidget::new(
            TextInput::from_text_area(
                TextArea::new_editable("TextInput contents")
                    .with_style(StyleProperty::FontSize(14.0))
                    .with_word_wrap(false)
                    .with_auto_id(),
            )
            .with_clip(true),
        );
        let mut harness = TestHarness::create_with_size(
            default_property_set(),
            text_input,
            Size::new(80.0, 30.0),
        );

        assert_render_snapshot!(harness, "text_input_clip");
    }
}
