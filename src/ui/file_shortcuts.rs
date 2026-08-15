use xilem::core::{MessageContext, MessageResult, Mut, View, ViewId, ViewMarker, ViewPathTracker};
use xilem::masonry::accesskit::{Node, Role};
use xilem::masonry::core::keyboard::{Key, NamedKey};
use xilem::masonry::core::{
    AccessCtx, AccessEvent, BoxConstraints, ChildrenIds, EventCtx, KeyboardEvent, LayoutCtx,
    NewWidget, PaintCtx, PointerButton, PointerEvent, PropertiesMut, PropertiesRef, RegisterCtx,
    TextEvent, Update, UpdateCtx, Widget, WidgetMut, WidgetPod,
};
use xilem::masonry::kurbo::{Point, Rect, Size};
use xilem::masonry::vello::Scene;
use xilem::{Pod, ViewCtx, WidgetView};

use crate::app::AppState;
use crate::theme::Layout;

const PAGE_JUMP_ROWS: isize = 10;
const FILE_SHORTCUT_CONTENT_VIEW_ID: ViewId = ViewId::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShortcutScope {
    FileList,
    Browser,
    Global,
}

#[derive(Clone, Debug)]
pub(super) enum FileShortcutAction {
    Move(isize),
    First,
    Last,
    Activate,
    Rename,
    Delete,
    Copy,
    Cut,
    Paste,
    NewFolder,
    Refresh,
    Back,
    SystemBack,
    Forward,
    Up,
    ApplyRename,
    CancelRename,
    RenameText(String),
    RenameBackspace,
    TypeAhead(String),
}

pub(super) fn file_list_shortcuts<V>(
    inner: V,
    item_count: usize,
    selected_index: Option<usize>,
    rename_active: bool,
) -> FileShortcutView<V>
where
    V: WidgetView<AppState>,
{
    FileShortcutView {
        inner,
        item_count,
        selected_index,
        scope: ShortcutScope::FileList,
        rename_active,
    }
}

pub(super) fn browser_shortcuts<V>(inner: V, rename_active: bool) -> FileShortcutView<V>
where
    V: WidgetView<AppState>,
{
    FileShortcutView {
        inner,
        item_count: 0,
        selected_index: None,
        scope: ShortcutScope::Browser,
        rename_active,
    }
}

#[cfg(target_os = "android")]
pub(super) fn system_back_shortcuts<V>(inner: V) -> FileShortcutView<V>
where
    V: WidgetView<AppState>,
{
    FileShortcutView {
        inner,
        item_count: 0,
        selected_index: None,
        scope: ShortcutScope::Global,
        rename_active: false,
    }
}

pub(super) struct FileShortcutView<V> {
    inner: V,
    item_count: usize,
    selected_index: Option<usize>,
    scope: ShortcutScope,
    rename_active: bool,
}

impl<V> ViewMarker for FileShortcutView<V> {}
impl<V> View<AppState, (), ViewCtx> for FileShortcutView<V>
where
    V: WidgetView<AppState>,
{
    type Element = Pod<FileShortcutWidget>;
    type ViewState = V::ViewState;

    fn build(&self, ctx: &mut ViewCtx, state: &mut AppState) -> (Self::Element, Self::ViewState) {
        let (child, child_state) = ctx.with_id(FILE_SHORTCUT_CONTENT_VIEW_ID, |ctx| {
            self.inner.build(ctx, state)
        });
        let widget = FileShortcutWidget::new(
            child.new_widget,
            self.item_count,
            self.selected_index,
            self.scope,
            self.rename_active,
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
        FileShortcutWidget::set_state(
            &mut element,
            self.item_count,
            self.selected_index,
            self.scope,
            self.rename_active,
        );
        ctx.with_id(FILE_SHORTCUT_CONTENT_VIEW_ID, |ctx| {
            let mut child = FileShortcutWidget::child_mut(&mut element);
            self.inner
                .rebuild(&prev.inner, view_state, ctx, child.downcast(), state);
        });
    }

    fn teardown(
        &self,
        view_state: &mut Self::ViewState,
        ctx: &mut ViewCtx,
        mut element: Mut<'_, Self::Element>,
    ) {
        ctx.with_id(FILE_SHORTCUT_CONTENT_VIEW_ID, |ctx| {
            let mut child = FileShortcutWidget::child_mut(&mut element);
            self.inner.teardown(view_state, ctx, child.downcast());
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
            Some(FILE_SHORTCUT_CONTENT_VIEW_ID) => {
                let mut child = FileShortcutWidget::child_mut(&mut element);
                self.inner
                    .message(view_state, message, child.downcast(), state)
            }
            None => match message.take_message::<FileShortcutAction>() {
                Some(action) => {
                    apply_action(state, *action);
                    MessageResult::Action(())
                }
                None => MessageResult::Stale,
            },
            _ => MessageResult::Stale,
        }
    }
}

fn apply_action(state: &mut AppState, action: FileShortcutAction) -> Option<usize> {
    match action {
        FileShortcutAction::Move(delta) => state.move_selection(delta),
        FileShortcutAction::First => state.select_first_entry(),
        FileShortcutAction::Last => state.select_last_entry(),
        FileShortcutAction::Activate => {
            state.activate_selected();
            None
        }
        FileShortcutAction::Rename => {
            state.begin_rename();
            None
        }
        FileShortcutAction::Delete => {
            state.delete_selected();
            None
        }
        FileShortcutAction::Copy => {
            state.copy_selected();
            None
        }
        FileShortcutAction::Cut => {
            state.cut_selected();
            None
        }
        FileShortcutAction::Paste => {
            state.paste();
            state.selected_entry_index()
        }
        FileShortcutAction::NewFolder => {
            state.new_folder();
            state.selected_entry_index()
        }
        FileShortcutAction::Refresh => {
            state.refresh();
            state.selected_entry_index()
        }
        FileShortcutAction::Back => {
            state.go_back();
            None
        }
        FileShortcutAction::SystemBack => {
            if state.handle_system_back() {
                #[cfg(target_os = "android")]
                state.background_android_task();
            }
            None
        }
        FileShortcutAction::Forward => {
            state.go_forward();
            None
        }
        FileShortcutAction::Up => {
            state.go_up();
            None
        }
        FileShortcutAction::ApplyRename => {
            state.apply_rename();
            state.selected_entry_index()
        }
        FileShortcutAction::CancelRename => {
            state.cancel_rename();
            state.selected_entry_index()
        }
        FileShortcutAction::RenameText(text) => {
            state.type_rename_text(text);
            state.selected_entry_index()
        }
        FileShortcutAction::RenameBackspace => {
            state.backspace_rename();
            state.selected_entry_index()
        }
        FileShortcutAction::TypeAhead(text) => state.typeahead_select(text),
    }
}

pub(super) struct FileShortcutWidget {
    child: WidgetPod<dyn Widget>,
    item_count: usize,
    selected_index: Option<usize>,
    scope: ShortcutScope,
    rename_active: bool,
}
impl FileShortcutWidget {
    fn new(
        child: NewWidget<impl Widget + ?Sized>,
        item_count: usize,
        selected_index: Option<usize>,
        scope: ShortcutScope,
        rename_active: bool,
    ) -> Self {
        Self {
            child: child.erased().to_pod(),
            item_count,
            selected_index,
            scope,
            rename_active,
        }
    }

    fn set_state(
        this: &mut WidgetMut<'_, Self>,
        item_count: usize,
        selected_index: Option<usize>,
        scope: ShortcutScope,
        rename_active: bool,
    ) {
        let restore_list_focus = this.widget.scope == ShortcutScope::FileList
            && this.widget.rename_active
            && !rename_active;
        this.widget.item_count = item_count;
        this.widget.selected_index = selected_index;
        this.widget.scope = scope;
        this.widget.rename_active = rename_active;
        if restore_list_focus {
            let id = this.ctx.widget_id();
            this.ctx.set_focus(id);
        }
    }

    fn child_mut<'a>(this: &'a mut WidgetMut<'_, Self>) -> WidgetMut<'a, dyn Widget> {
        this.ctx.get_mut(&mut this.widget.child)
    }

    fn selection_index_after(&self, action: &FileShortcutAction) -> Option<usize> {
        if self.item_count == 0 {
            return None;
        }
        match action {
            FileShortcutAction::Move(delta) => Some(self.selected_index.map_or(0, |current| {
                (current as isize + *delta).clamp(0, self.item_count as isize - 1) as usize
            })),
            FileShortcutAction::First => Some(0),
            FileShortcutAction::Last => Some(self.item_count - 1),
            _ => None,
        }
    }
}
impl Widget for FileShortcutWidget {
    type Action = FileShortcutAction;

    fn on_pointer_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &PointerEvent,
    ) {
        if self.scope == ShortcutScope::FileList
            && let PointerEvent::Down(button) = event
            && matches!(
                button.button,
                Some(PointerButton::Primary | PointerButton::Secondary)
            )
        {
            ctx.request_focus();
        }
    }

    fn on_text_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &TextEvent,
    ) {
        if ctx.is_handled() {
            return;
        }
        if self.scope == ShortcutScope::FileList
            && !self.rename_active
            && matches!(event, TextEvent::ClipboardPaste(_))
        {
            ctx.submit_action::<FileShortcutAction>(FileShortcutAction::Paste);
            ctx.set_handled();
            return;
        }
        let TextEvent::Keyboard(event) = event else {
            return;
        };
        if let Some(action) = shortcut_action(event, self.scope, self.rename_active) {
            if matches!(action, FileShortcutAction::Activate) && ctx.target() != ctx.widget_id() {
                return;
            }
            if let Some(index) = self.selection_index_after(&action) {
                let top = index as f64 * Layout::ROW_HEIGHT;
                ctx.request_scroll_to(Rect::new(
                    0.0,
                    top,
                    ctx.size().width,
                    top + Layout::ROW_HEIGHT,
                ));
            }
            ctx.submit_action::<FileShortcutAction>(action);
            ctx.set_handled();
        }
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
        ctx: &mut LayoutCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        bc: &BoxConstraints,
    ) -> Size {
        let size = ctx.run_layout(&mut self.child, bc);
        ctx.place_child(&mut self.child, Point::ORIGIN);
        size
    }

    fn paint(&mut self, _ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, _scene: &mut Scene) {}

    fn accessibility_role(&self) -> Role {
        match self.scope {
            ShortcutScope::FileList => Role::ListBox,
            ShortcutScope::Browser | ShortcutScope::Global => Role::Group,
        }
    }

    fn accessibility(
        &mut self,
        _ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        node: &mut Node,
    ) {
        node.set_label("Files");
    }
    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        ctx.register_child(&mut self.child);
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::from_slice(&[self.child.id()])
    }

    fn accepts_focus(&self) -> bool {
        self.scope == ShortcutScope::FileList
    }
}

fn shortcut_action(
    event: &KeyboardEvent,
    scope: ShortcutScope,
    rename_active: bool,
) -> Option<FileShortcutAction> {
    if !event.state.is_down() || event.is_composing {
        return None;
    }

    let ctrl = event.modifiers.ctrl();
    let shift = event.modifiers.shift();
    let alt = event.modifiers.alt();
    let meta = event.modifiers.meta();

    if scope == ShortcutScope::Global {
        return (!ctrl
            && !shift
            && !alt
            && !meta
            && !event.repeat
            && matches!(event.key, Key::Named(NamedKey::BrowserBack)))
        .then_some(FileShortcutAction::SystemBack);
    }

    if scope == ShortcutScope::Browser {
        if rename_active
            && !ctrl
            && !shift
            && !alt
            && !meta
            && !event.repeat
            && event.key == Key::Named(NamedKey::Escape)
        {
            return Some(FileShortcutAction::CancelRename);
        }
        if alt && !ctrl && !shift && !meta && !event.repeat {
            return match event.key {
                Key::Named(NamedKey::ArrowLeft) => Some(FileShortcutAction::Back),
                Key::Named(NamedKey::ArrowRight) => Some(FileShortcutAction::Forward),
                Key::Named(NamedKey::ArrowUp) => Some(FileShortcutAction::Up),
                _ => None,
            };
        }
        if !ctrl
            && !shift
            && !alt
            && !meta
            && !event.repeat
            && event.key == Key::Named(NamedKey::F5)
        {
            return Some(FileShortcutAction::Refresh);
        }
        return None;
    }

    if rename_active {
        if ctrl || alt || meta {
            return None;
        }
        return match &event.key {
            Key::Named(NamedKey::Enter) if !event.repeat => Some(FileShortcutAction::ApplyRename),
            Key::Named(NamedKey::Escape) if !event.repeat => Some(FileShortcutAction::CancelRename),
            Key::Named(NamedKey::Backspace) => Some(FileShortcutAction::RenameBackspace),
            Key::Character(text) => Some(FileShortcutAction::RenameText(text.to_string())),
            _ => None,
        };
    }

    let primary = ctrl || (cfg!(target_os = "macos") && meta);
    if primary && !alt {
        if shift && key_char(event, "n") && !event.repeat {
            return Some(FileShortcutAction::NewFolder);
        }
        if !shift && !event.repeat {
            if key_char(event, "c") {
                return Some(FileShortcutAction::Copy);
            }
            if key_char(event, "x") {
                return Some(FileShortcutAction::Cut);
            }
        }
    }
    if ctrl || shift || alt || meta {
        return None;
    }
    if let Key::Character(text) = &event.key
        && !text.is_empty()
        && !text.chars().any(char::is_control)
    {
        return Some(FileShortcutAction::TypeAhead(text.to_string()));
    }

    match event.key {
        Key::Named(NamedKey::ArrowUp) => Some(FileShortcutAction::Move(-1)),
        Key::Named(NamedKey::ArrowDown) => Some(FileShortcutAction::Move(1)),
        Key::Named(NamedKey::Home) => Some(FileShortcutAction::First),
        Key::Named(NamedKey::End) => Some(FileShortcutAction::Last),
        Key::Named(NamedKey::PageUp) => Some(FileShortcutAction::Move(-PAGE_JUMP_ROWS)),
        Key::Named(NamedKey::PageDown) => Some(FileShortcutAction::Move(PAGE_JUMP_ROWS)),
        Key::Named(NamedKey::Enter) if !event.repeat => Some(FileShortcutAction::Activate),
        Key::Named(NamedKey::F2) if !event.repeat => Some(FileShortcutAction::Rename),
        Key::Named(NamedKey::Delete) if !event.repeat => Some(FileShortcutAction::Delete),
        Key::Named(NamedKey::Backspace) if !event.repeat => Some(FileShortcutAction::Back),
        _ => None,
    }
}

fn key_char(event: &KeyboardEvent, expected: &str) -> bool {
    matches!(
        &event.key,
        Key::Character(value) if value.as_str().eq_ignore_ascii_case(expected)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use xilem::masonry::core::Modifiers;
    use xilem::masonry::core::keyboard::Code;

    fn key_event(key: Key) -> KeyboardEvent {
        KeyboardEvent::key_down(key, Code::Unidentified)
    }

    #[test]
    fn list_navigation_keys_are_list_scoped() {
        let down = key_event(Key::Named(NamedKey::ArrowDown));
        assert!(matches!(
            shortcut_action(&down, ShortcutScope::FileList, false),
            Some(FileShortcutAction::Move(1))
        ));
        assert!(shortcut_action(&down, ShortcutScope::Browser, false).is_none());
    }

    #[test]
    fn android_system_back_is_global() {
        let back = key_event(Key::Named(NamedKey::BrowserBack));
        assert!(matches!(
            shortcut_action(&back, ShortcutScope::Global, false),
            Some(FileShortcutAction::SystemBack)
        ));
        assert!(shortcut_action(&back, ShortcutScope::Browser, false).is_none());
        assert!(shortcut_action(&back, ShortcutScope::FileList, false).is_none());
    }

    #[test]
    fn browser_navigation_does_not_become_a_list_edit_command() {
        let mut back = key_event(Key::Named(NamedKey::ArrowLeft));
        back.modifiers = Modifiers::ALT;
        assert!(matches!(
            shortcut_action(&back, ShortcutScope::Browser, false),
            Some(FileShortcutAction::Back)
        ));
        assert!(shortcut_action(&back, ShortcutScope::FileList, false).is_none());
    }

    #[test]
    fn refresh_is_browser_scoped_and_copy_is_list_scoped() {
        let refresh = key_event(Key::Named(NamedKey::F5));
        assert!(matches!(
            shortcut_action(&refresh, ShortcutScope::Browser, false),
            Some(FileShortcutAction::Refresh)
        ));
        assert!(shortcut_action(&refresh, ShortcutScope::FileList, false).is_none());

        let mut copy = key_event(Key::Character("c".into()));
        copy.modifiers = Modifiers::CONTROL;
        assert!(matches!(
            shortcut_action(&copy, ShortcutScope::FileList, false),
            Some(FileShortcutAction::Copy)
        ));
        assert!(shortcut_action(&copy, ShortcutScope::Browser, false).is_none());
    }

    #[test]
    fn rename_mode_blocks_destructive_list_commands() {
        let delete = key_event(Key::Named(NamedKey::Delete));
        assert!(shortcut_action(&delete, ShortcutScope::FileList, true).is_none());

        let enter = key_event(Key::Named(NamedKey::Enter));
        assert!(matches!(
            shortcut_action(&enter, ShortcutScope::FileList, true),
            Some(FileShortcutAction::ApplyRename)
        ));

        let escape = key_event(Key::Named(NamedKey::Escape));
        assert!(matches!(
            shortcut_action(&escape, ShortcutScope::FileList, true),
            Some(FileShortcutAction::CancelRename)
        ));
        assert!(matches!(
            shortcut_action(&escape, ShortcutScope::Browser, true),
            Some(FileShortcutAction::CancelRename)
        ));

        let rename_text = key_event(Key::Character("A".into()));
        assert!(matches!(
            shortcut_action(&rename_text, ShortcutScope::FileList, true),
            Some(FileShortcutAction::RenameText(text)) if text == "A"
        ));

        let rename_backspace = key_event(Key::Named(NamedKey::Backspace));
        assert!(matches!(
            shortcut_action(&rename_backspace, ShortcutScope::FileList, true),
            Some(FileShortcutAction::RenameBackspace)
        ));

        let mut raw_paste = key_event(Key::Character("v".into()));
        raw_paste.modifiers = Modifiers::CONTROL;
        assert!(shortcut_action(&raw_paste, ShortcutScope::FileList, false).is_none());
    }
}
