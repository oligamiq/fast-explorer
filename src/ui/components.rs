use std::path::PathBuf;

use xilem::core::one_of::Either;
use xilem::masonry::kurbo::Rect;
use xilem::masonry::peniko::{Color, ImageData};
use xilem::masonry::properties::LineBreaking;
use xilem::masonry::properties::ObjectFit;
use xilem::masonry::properties::types::{AsUnit, UnitPoint};
use xilem::style::{Padding, Style as _};
use xilem::view::image;
use xilem::view::{
    CrossAxisAlignment, FlexExt as _, FlexSpacer, MainAxisAlignment, ZStackExt as _, button,
    flex_col, flex_row, portal, sized_box, slider, virtual_scroll, zstack,
};
use xilem::{AnyWidgetView, InsertNewline, TextAlign, WidgetView};

use super::file_row::file_row_button;
use super::file_shortcuts::file_list_shortcuts;
use super::font::{label, prose, text_input};
use super::icons::{LucideIcon, icon, progress_ring};
use super::tab_drag::{TabDragConfig, tab_drag_button};
#[cfg(not(target_os = "android"))]
use super::window_chrome::{CaptionButtonKind, caption_button, drag_region};
use super::window_chrome::{NavigationButtonKind, navigation_button};
use crate::app::{
    AppState, EntryKind, FileCategory, FileEntry, SortDirection, SortField, TaildriveLocation,
    parse_taildrive_path, taildrive_path,
};
use crate::settings::{PathOverflowBehavior, SearchMode, UiFont};
use crate::theme::{AppearanceMode, Layout, ThemeColor, ThemePalette};

fn compact_tab_width(title: &str, min_width: f64, max_width: f64) -> f64 {
    let label_width = title
        .chars()
        .map(|ch| if ch.is_ascii() { 7.2 } else { 13.0 })
        .sum::<f64>();
    (label_width + Layout::TAB_CLOSE_WIDTH + 18.0).clamp(min_width, max_width)
}

fn tab_strip_views(
    state: &AppState,
    min_tab_width: f64,
    palette: ThemePalette,
) -> (Vec<Box<AnyWidgetView<AppState>>>, f64, Option<Rect>) {
    let tab_count = state.tab_count().max(1);
    let active = state.active_tab_index();
    let max_tab_width =
        (Layout::TAB_LAYOUT_BUDGET / tab_count as f64).clamp(min_tab_width, Layout::TAB_WIDTH);
    let titles = state
        .tabs()
        .iter()
        .map(|tab| tab.title())
        .collect::<Vec<_>>();
    let ids = state.tabs().iter().map(|tab| tab.id()).collect::<Vec<_>>();
    let widths = titles
        .iter()
        .map(|title| compact_tab_width(title, min_tab_width, max_tab_width))
        .collect::<Vec<_>>();
    let drop_targets = (0..titles.len()).collect::<Vec<_>>();
    let total_width = widths.iter().sum::<f64>();

    let mut x = 0.0;
    let mut reveal_target = None;
    for (index, width) in widths.iter().copied().enumerate() {
        if index == active {
            reveal_target = Some(Rect::new(x, 0.0, x + width, Layout::TOOL_HEIGHT));
            break;
        }
        x += width;
    }

    let views = titles
        .into_iter()
        .enumerate()
        .map(|(index, title)| {
            tab_item(
                TabItemSpec {
                    index,
                    drag_index: index,
                    drop_targets: drop_targets.clone(),
                    tab_id: ids[index],
                    title,
                    active: index == active,
                    width: widths[index],
                    widths: widths.clone(),
                    scroll_leading: 0.0,
                },
                palette,
            )
            .boxed()
        })
        .collect::<Vec<_>>();

    (views, total_width, reveal_target)
}

#[cfg(not(target_os = "android"))]
pub fn tab_bar(state: &AppState) -> impl WidgetView<AppState> + use<> {
    let palette = state.palette();
    let (tabs, total_width, reveal_target) = tab_strip_views(state, 78.0, palette);
    let strip = flex_row(tabs)
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Center);
    let tab_strip_width = (total_width + 16.0).min(Layout::TAB_STRIP_MAX);
    let tabs_scroll = sized_box(
        sized_box(portal(strip).reveal_target(reveal_target)).padding(Padding::from_vh(4.0, 8.0)),
    )
    .width(tab_strip_width.px());
    let drag = sized_box(drag_region())
        .height(Layout::TAB_HEIGHT.px())
        .flex(1.0);

    sized_box(
        flex_row((
            tabs_scroll,
            icon_button(
                LucideIcon::Plus,
                "New tab",
                false,
                AppState::new_tab,
                palette,
            ),
            drag,
            transfer_progress_button(state, palette),
            caption_button(CaptionButtonKind::Minimize, palette),
            caption_button(CaptionButtonKind::Maximize, palette),
            caption_button(CaptionButtonKind::Close, palette),
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .must_fill_major_axis(true),
    )
    .height(Layout::TAB_HEIGHT.px())
    .expand_width()
    .background_color(palette.chrome)
}

#[cfg(target_os = "android")]
pub fn tab_bar(state: &AppState) -> impl WidgetView<AppState> + use<> {
    let palette = state.palette();
    let (tabs, _, reveal_target) = tab_strip_views(state, 88.0, palette);
    let strip = flex_row(tabs)
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Center);
    let tabs_scroll = sized_box(portal(strip).reveal_target(reveal_target))
        .height(Layout::TAB_HEIGHT.px())
        .flex(1.0);
    sized_box(
        flex_row((
            tabs_scroll,
            icon_button(
                LucideIcon::Plus,
                "New tab",
                false,
                AppState::new_tab,
                palette,
            ),
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Center),
    )
    .height(Layout::TAB_HEIGHT.px())
    .expand_width()
    .padding(Padding::horizontal(4.0))
    .background_color(palette.chrome)
    .border(palette.border, 1.0)
}

pub fn restore_warning_banner(state: &AppState) -> impl WidgetView<AppState> + use<> {
    let palette = state.palette();
    if let Some(text) = state.restore_warning() {
        Either::A(
            sized_box(
                flex_row((
                    sized_box(
                        prose(text.to_owned())
                            .text_size(11.0)
                            .text_color(palette.text),
                    )
                    .flex(1.0),
                    icon_button(
                        LucideIcon::X,
                        "Dismiss restored-path warning",
                        false,
                        AppState::dismiss_restore_warning,
                        palette,
                    ),
                ))
                .gap(6.px())
                .cross_axis_alignment(CrossAxisAlignment::Center),
            )
            .expand_width()
            .padding(Padding::from_vh(6.0, 10.0))
            .background_color(palette.accent_soft)
            .border(palette.border_strong, 1.0),
        )
    } else {
        Either::B(sized_box(label("")).height(0.px()))
    }
}

fn settings_header_view(palette: ThemePalette) -> Box<AnyWidgetView<AppState>> {
    sized_box(
        flex_row((
            icon_text_button(
                LucideIcon::ChevronLeft,
                "Files",
                false,
                AppState::close_settings,
                palette,
            ),
            FlexSpacer::Fixed(12.px()),
            label("Settings").text_size(22.0).color(palette.text),
            FlexSpacer::Flex(1.0),
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Center),
    )
    .height(52.px())
    .expand_width()
    .padding(Padding::horizontal(14.0))
    .background_color(palette.chrome)
    .boxed()
}

#[cfg(not(target_os = "android"))]
fn settings_slider(
    min: f64,
    max: f64,
    value: f64,
    callback: impl Fn(&mut AppState, f64) + Send + Sync + 'static,
) -> impl WidgetView<AppState> {
    sized_box(slider(min, max, value, callback).step(1.0)).width(480.px())
}

#[cfg(target_os = "android")]
fn settings_slider(
    min: f64,
    max: f64,
    value: f64,
    callback: impl Fn(&mut AppState, f64) + Send + Sync + 'static,
) -> impl WidgetView<AppState> {
    sized_box(slider(min, max, value, callback).step(1.0)).expand_width()
}

fn appearance_settings_view(
    state: &AppState,
    palette: ThemePalette,
) -> Box<AnyWidgetView<AppState>> {
    let appearance_mode = state.effective_theme_settings().appearance;
    settings_card(
        "Appearance",
        "Tune the window appearance without changing file-management behavior.",
        flex_col((
            settings_row(
                "Color scheme",
                flex_row((
                    fill_choice_button(
                        "System",
                        appearance_mode == AppearanceMode::System,
                        |state| state.set_appearance_mode(AppearanceMode::System),
                        palette,
                    )
                    .flex(1.0),
                    FlexSpacer::Fixed(6.px()),
                    fill_choice_button(
                        "Light",
                        appearance_mode == AppearanceMode::Light,
                        |state| state.set_appearance_mode(AppearanceMode::Light),
                        palette,
                    )
                    .flex(1.0),
                    FlexSpacer::Fixed(6.px()),
                    fill_choice_button(
                        "Dark",
                        appearance_mode == AppearanceMode::Dark,
                        |state| state.set_appearance_mode(AppearanceMode::Dark),
                        palette,
                    )
                    .flex(1.0),
                ))
                .gap(0.px()),
                palette,
            ),
            FlexSpacer::Fixed(12.px()),
            settings_row("Theme color", theme_color_picker(state, palette), palette),
            FlexSpacer::Fixed(12.px()),
            settings_row("UI font", ui_font_picker(state, palette), palette),
            FlexSpacer::Fixed(12.px()),
            settings_row(
                "Tint strength",
                flex_row((
                    settings_slider(
                        0.0,
                        100.0,
                        f64::from(state.theme_intensity()),
                        AppState::set_theme_intensity_value,
                    )
                    .flex(1.0),
                    FlexSpacer::Fixed(10.px()),
                    sized_box(
                        label(format!("{}%", state.theme_intensity()))
                            .text_size(12.0)
                            .color(palette.text),
                    )
                    .width(52.px()),
                ))
                .gap(0.px())
                .cross_axis_alignment(CrossAxisAlignment::Center),
                palette,
            ),
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Start),
        palette,
    )
    .boxed()
}

fn files_settings_view(state: &AppState, palette: ThemePalette) -> Box<AnyWidgetView<AppState>> {
    let show_hidden = state.active_tab().show_hidden;
    let hidden_files_row = settings_row(
        "Hidden files",
        flex_row((
            fill_choice_button(
                "Hide",
                !show_hidden,
                |state| {
                    if state.active_tab().show_hidden {
                        state.toggle_hidden();
                    }
                },
                palette,
            )
            .flex(1.0),
            FlexSpacer::Fixed(6.px()),
            fill_choice_button(
                "Show",
                show_hidden,
                |state| {
                    if !state.active_tab().show_hidden {
                        state.toggle_hidden();
                    }
                },
                palette,
            )
            .flex(1.0),
        ))
        .gap(0.px()),
        palette,
    );

    let path_overflow = state.path_overflow_behavior();
    let path_overflow_row = settings_row(
        "Long path",
        flex_row((
            fill_choice_button(
                "Don't move",
                path_overflow == PathOverflowBehavior::Static,
                |state| state.set_path_overflow_behavior(PathOverflowBehavior::Static),
                palette,
            )
            .flex(1.0),
            FlexSpacer::Fixed(6.px()),
            fill_choice_button(
                "Move then reset",
                path_overflow == PathOverflowBehavior::ForwardReset,
                |state| state.set_path_overflow_behavior(PathOverflowBehavior::ForwardReset),
                palette,
            )
            .flex(1.0),
        ))
        .gap(0.px()),
        palette,
    );
    let reset_delay = state.path_reset_delay_ms();
    let path_reset_delay_row = settings_row(
        "Reset wait",
        flex_row((
            fill_choice_button(
                "1 s",
                reset_delay == 1000,
                |state| state.set_path_reset_delay_ms(1000),
                palette,
            )
            .flex(1.0),
            FlexSpacer::Fixed(6.px()),
            fill_choice_button(
                "3 s",
                reset_delay == 3000,
                |state| state.set_path_reset_delay_ms(3000),
                palette,
            )
            .flex(1.0),
            FlexSpacer::Fixed(6.px()),
            fill_choice_button(
                "5 s",
                reset_delay == 5000,
                |state| state.set_path_reset_delay_ms(5000),
                palette,
            )
            .flex(1.0),
        ))
        .gap(0.px()),
        palette,
    );

    #[cfg(not(target_os = "android"))]
    {
        settings_card(
            "Files",
            "Less common view preferences for the active tab.",
            flex_col((
                hidden_files_row,
                FlexSpacer::Fixed(12.px()),
                path_overflow_row,
                FlexSpacer::Fixed(12.px()),
                path_reset_delay_row,
            ))
            .gap(0.px())
            .cross_axis_alignment(CrossAxisAlignment::Start),
            palette,
        )
        .boxed()
    }
    #[cfg(target_os = "android")]
    {
        settings_card(
            "Files",
            "View preferences and safety options for file operations.",
            flex_col((
                hidden_files_row,
                FlexSpacer::Fixed(12.px()),
                path_overflow_row,
                FlexSpacer::Fixed(12.px()),
                path_reset_delay_row,
                FlexSpacer::Fixed(12.px()),
                settings_row(
                    "Delete confirmation",
                    flex_row((
                        fill_choice_button(
                            "Warn",
                            state.confirm_mobile_delete_enabled(),
                            |state| state.set_confirm_mobile_delete_enabled(true),
                            palette,
                        )
                        .flex(1.0),
                        FlexSpacer::Fixed(6.px()),
                        fill_choice_button(
                            "Don't warn",
                            !state.confirm_mobile_delete_enabled(),
                            |state| state.set_confirm_mobile_delete_enabled(false),
                            palette,
                        )
                        .flex(1.0),
                    ))
                    .gap(0.px()),
                    palette,
                ),
            ))
            .gap(0.px())
            .cross_axis_alignment(CrossAxisAlignment::Start),
            palette,
        )
        .boxed()
    }
}

#[cfg(target_os = "windows")]
fn explorer_settings_view(state: &AppState, palette: ThemePalette) -> Box<AnyWidgetView<AppState>> {
    settings_card(
        "File Explorer integration",
        "Use FastExplorer for filesystem folders, drives, and Win+E. FastExplorer backs up the current per-user shell registration before changing it and restores that backup when disabled.",
        settings_row(
            "Default manager",
            flex_row((
                fill_choice_button(
                    "FastExplorer",
                    state.explorer_replacement_enabled(),
                    AppState::enable_explorer_replacement,
                    palette,
                ),
                FlexSpacer::Fixed(6.px()),
                fill_choice_button(
                    "Windows Explorer",
                    !state.explorer_replacement_enabled(),
                    AppState::disable_explorer_replacement,
                    palette,
                ),
            ))
            .gap(0.px()),
            palette,
        ),
        palette,
    )
    .boxed()
}

fn remote_cache_settings_view(
    state: &AppState,
    palette: ThemePalette,
) -> Box<AnyWidgetView<AppState>> {
    settings_card(
        "Remote file cache",
        "TailDrive files opened in other apps are cached locally. Expiration is renewed on every access. One file larger than the limit is kept so it can still be opened.",
        flex_col((
            settings_row(
                "Usage",
                flex_row((
                    label(state.remote_cache_usage_label())
                        .text_size(12.0)
                        .color(palette.text),
                    FlexSpacer::Flex(1.0),
                    toolbar_button("Clear now", false, AppState::clear_remote_cache, palette),
                ))
                .gap(0.px())
                .cross_axis_alignment(CrossAxisAlignment::Center),
                palette,
            ),
            FlexSpacer::Fixed(12.px()),
            settings_row(
                "Cache limit",
                flex_row((
                    settings_slider(
                        0.0,
                        6.0,
                        state.remote_cache_limit_slider_value(),
                        AppState::set_remote_cache_limit_slider_value,
                    )
                    .flex(1.0),
                    FlexSpacer::Fixed(10.px()),
                    sized_box(label(state.remote_cache_limit_label()).text_size(12.0).color(palette.text))
                        .width(82.px()),
                ))
                .gap(0.px())
                .cross_axis_alignment(CrossAxisAlignment::Center),
                palette,
            ),
            FlexSpacer::Fixed(12.px()),
            settings_row(
                "Expiration",
                flex_row((
                    settings_slider(
                        0.0,
                        6.0,
                        state.remote_cache_expiration_slider_value(),
                        AppState::set_remote_cache_expiration_slider_value,
                    )
                    .flex(1.0),
                    FlexSpacer::Fixed(10.px()),
                    sized_box(label(state.remote_cache_expiration_label()).text_size(12.0).color(palette.text))
                        .width(82.px()),
                ))
                .gap(0.px())
                .cross_axis_alignment(CrossAxisAlignment::Center),
                palette,
            ),
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Start),
        palette,
    )
    .boxed()
}

fn search_settings_view(state: &AppState, palette: ThemePalette) -> Box<AnyWidgetView<AppState>> {
    let search_mode = state.search_mode();
    let everything_available = state.everything_search_available();
    let search_note = if everything_available {
        "Default searches below the current folder. Everything uses the Everything index through ES."
    } else if cfg!(target_os = "windows") {
        "Everything is unavailable because ES.exe was not found. Install the Everything command-line interface to enable it."
    } else {
        "Everything is unavailable on this platform. Default searches below the current folder."
    };
    settings_card(
        "Search",
        search_note,
        settings_row(
            "Backend",
            flex_row((
                fill_choice_button(
                    "Default",
                    search_mode == SearchMode::Default || !everything_available,
                    |state| state.set_search_mode(SearchMode::Default, true),
                    palette,
                )
                .flex(1.0),
                FlexSpacer::Fixed(6.px()),
                fill_choice_button_disabled(
                    "Everything",
                    search_mode == SearchMode::Everything,
                    !everything_available,
                    |state| state.set_search_mode(SearchMode::Everything, true),
                    palette,
                )
                .flex(1.0),
            ))
            .gap(0.px()),
            palette,
        ),
        palette,
    )
    .boxed()
}

fn tailscale_settings_view(
    state: &AppState,
    palette: ThemePalette,
) -> Box<AnyWidgetView<AppState>> {
    tailscale_settings_card(state, palette).boxed()
}

fn external_settings_view(palette: ThemePalette) -> Box<AnyWidgetView<AppState>> {
    settings_card(
        "External control",
        "Startup flags can override saved settings for one run. Live control uses fast-explorer/1 over the local IPC socket.",
        prose("Protocol reference: docs/control-protocol.md")
            .text_size(12.0)
            .text_color(palette.text),
        palette,
    )
    .boxed()
}

pub fn settings_page(state: &AppState) -> impl WidgetView<AppState> + use<> {
    let palette = state.palette();
    let header = settings_header_view(palette);
    let mut cards: Vec<Box<AnyWidgetView<AppState>>> = vec![
        appearance_settings_view(state, palette),
        files_settings_view(state, palette),
    ];
    #[cfg(target_os = "windows")]
    cards.push(explorer_settings_view(state, palette));
    cards.extend([
        remote_cache_settings_view(state, palette),
        search_settings_view(state, palette),
        tailscale_settings_view(state, palette),
        external_settings_view(palette),
    ]);

    let settings_stack = flex_col(cards)
        .gap(14.px())
        .cross_axis_alignment(CrossAxisAlignment::Start);

    #[cfg(not(target_os = "android"))]
    let centered = flex_row((
        FlexSpacer::Flex(1.0),
        sized_box(settings_stack).width(Layout::SETTINGS_CONTENT_WIDTH.px()),
        FlexSpacer::Flex(1.0),
    ))
    .gap(0.px())
    .cross_axis_alignment(CrossAxisAlignment::Start);

    #[cfg(target_os = "android")]
    let centered = sized_box(settings_stack)
        .expand_width()
        .padding(Padding::from_vh(10.0, 10.0));

    sized_box(flex_col((header, sized_box(portal(centered)).flex(1.0))).gap(0.px()))
        .expand()
        .background_color(palette.window)
}

fn tailscale_settings_card(
    state: &AppState,
    palette: ThemePalette,
) -> impl WidgetView<AppState> + use<> {
    let cards = state
        .tailscale_profiles()
        .iter()
        .cloned()
        .map(|profile| {
            let profile_id = profile.config.id.clone();
            tailscale_profile_card(
                profile,
                state.tailscale_profile_hostname(&profile_id),
                state.tailscale_offline_peers_expanded(&profile_id),
                state.tailscale_offline_devices_expanded(&profile_id),
                palette,
            )
        })
        .collect::<Vec<_>>();
    let profiles = if cards.is_empty() {
        Either::A(
            prose("No Tailnets configured. Add one to sign in to another network.")
                .text_size(12.0)
                .text_color(palette.muted),
        )
    } else {
        Either::B(
            flex_col(cards)
                .gap(10.px())
                .cross_axis_alignment(CrossAxisAlignment::Start),
        )
    };

    settings_card(
        "Tailscale",
        "Each Tailnet runs as an independent embedded tsnet node. Add multiple Tailnets to keep them connected at the same time.",
        flex_col((
            toolbar_button("Add Tailnet", false, AppState::add_tailnet_profile, palette),
            FlexSpacer::Fixed(12.px()),
            profiles,
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Start),
        palette,
    )
}

fn compact_identity(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars || max_chars < 8 {
        return value.to_owned();
    }
    let side = (max_chars.saturating_sub(1)) / 2;
    let start = value.chars().take(side).collect::<String>();
    let end = value
        .chars()
        .rev()
        .take(side)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{start}…{end}")
}

fn tailscale_profile_card(
    profile: crate::app::TailnetProfileState,
    hostname_value: String,
    offline_peers_expanded: bool,
    offline_devices_expanded: bool,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    let profile_id = profile.config.id.clone();
    let label_value = profile.config.label.clone();
    let label_change_id = profile_id.clone();
    let label_enter_id = profile_id.clone();
    let label_input = settings_text_input(
        label_value,
        "Tailnet label",
        move |state, value| state.set_tailnet_label(&label_change_id, value),
        move |state, value| state.set_tailnet_label(&label_enter_id, value),
        palette,
    );
    let hostname_change_id = profile_id.clone();
    let hostname_enter_id = profile_id.clone();
    let hostname_input = settings_text_input(
        hostname_value,
        "fe-device",
        move |state, value| state.set_tailnet_hostname(&hostname_change_id, value),
        move |state, value| state.apply_tailnet_hostname(&hostname_enter_id, value),
        palette,
    );
    let enabled = profile.config.enabled;
    let status = profile.status;
    let network_name = if status.tailnet_name.is_empty() {
        "Not signed in".to_owned()
    } else {
        // The editable FastExplorer label is the primary name. Keep the actual
        // Tailscale identities available as secondary information, but compact
        // both so a long organization/MagicDNS name cannot dominate the card.
        let tailnet = compact_identity(&status.tailnet_name, 24);
        let suffix = compact_identity(&status.magic_dns_suffix, 24);
        if suffix.is_empty() || suffix == tailnet {
            tailnet
        } else {
            format!("{tailnet} · {suffix}")
        }
    };
    let identity = if !status.dns_name.is_empty() {
        compact_identity(&status.dns_name, 32)
    } else {
        compact_identity(&status.hostname, 32)
    };
    let address = status.ips.first().cloned().unwrap_or_default();
    let mut status_text = if !enabled {
        "Off".to_owned()
    } else if !status.error.is_empty() {
        format!("{} — {}", status.state, status.error)
    } else if identity.is_empty() {
        status.state.clone()
    } else if address.is_empty() {
        format!("{} — {identity}", status.state)
    } else {
        format!("{} — {identity} — {address}", status.state)
    };
    if enabled && status.service_ready {
        status_text.push_str(" — service ready");
    }
    if enabled && !status.library_version.is_empty() {
        status_text.push_str(" — ");
        status_text.push_str(&status.library_version);
    }
    let webdav_text = if status.webdav_url.is_empty() {
        "WebDAV: waiting for Tailnet address".to_owned()
    } else {
        format!("WebDAV: {}", status.webdav_url)
    };

    let actions = if enabled {
        let login_id = profile_id.clone();
        let refresh_id = profile_id.clone();
        let disconnect_id = profile_id.clone();
        let signout_id = profile_id.clone();
        Either::A(
            sized_box(portal(
                flex_row((
                    toolbar_button(
                        "Open sign-in",
                        status.auth_url.is_empty(),
                        move |state| state.open_tailscale_login(&login_id),
                        palette,
                    ),
                    FlexSpacer::Fixed(6.px()),
                    toolbar_button(
                        "Refresh",
                        false,
                        move |state| state.refresh_tailscale(&refresh_id),
                        palette,
                    ),
                    FlexSpacer::Fixed(6.px()),
                    toolbar_button(
                        "Disconnect",
                        false,
                        move |state| state.disconnect_tailscale(&disconnect_id),
                        palette,
                    ),
                    FlexSpacer::Fixed(6.px()),
                    toolbar_button(
                        "Sign out",
                        false,
                        move |state| state.sign_out_tailscale(&signout_id),
                        palette,
                    ),
                ))
                .gap(0.px())
                .cross_axis_alignment(CrossAxisAlignment::Center),
            ))
            .height(Layout::TOOL_HEIGHT.px())
            .expand_width(),
        )
    } else {
        let connect_id = profile_id.clone();
        Either::B(toolbar_button(
            "Connect",
            false,
            move |state| state.connect_tailscale(&connect_id),
            palette,
        ))
    };

    let taildrive_scanning = status.taildrive_scanning;
    let taildrive_error = status.taildrive_error.clone();
    let open_taildrive_id = profile_id.clone();
    let open_taildrive = icon_text_button(
        LucideIcon::Network,
        "Open TailDrive",
        !status.service_ready,
        move |state| {
            state.close_settings();
            state.navigate_to(taildrive_path(&TaildriveLocation::Profile {
                profile_id: open_taildrive_id.clone(),
            }));
        },
        palette,
    );
    let (online_peers, offline_peers): (Vec<_>, Vec<_>) =
        status.peers.into_iter().partition(|peer| peer.online);
    let mut peer_rows: Vec<Box<AnyWidgetView<AppState>>> = online_peers
        .into_iter()
        .map(|peer| tailscale_peer_row(profile_id.clone(), peer, palette).boxed())
        .collect();
    if !offline_peers.is_empty() {
        let offline_count = offline_peers.len();
        let accordion_id = profile_id.clone();
        peer_rows.push(
            accordion_button(
                format!("Offline ({offline_count})"),
                offline_peers_expanded,
                move |state| state.toggle_tailscale_offline_peers(&accordion_id),
                palette,
            )
            .boxed(),
        );
        if offline_peers_expanded {
            peer_rows.extend(
                offline_peers
                    .into_iter()
                    .map(|peer| tailscale_peer_row(profile_id.clone(), peer, palette).boxed()),
            );
        }
    }
    let peers = if peer_rows.is_empty() {
        Either::A(
            prose("No other Tailnet devices are visible yet.")
                .text_size(11.0)
                .text_color(palette.muted),
        )
    } else {
        Either::B(
            flex_col(peer_rows)
                .gap(0.px())
                .cross_axis_alignment(CrossAxisAlignment::Start),
        )
    };

    let (online_taildrive, offline_taildrive): (Vec<_>, Vec<_>) = status
        .taildrive_devices
        .into_iter()
        .partition(|device| device.online);
    let mut taildrive_rows: Vec<Box<AnyWidgetView<AppState>>> = online_taildrive
        .into_iter()
        .map(|device| taildrive_device_row(device, palette).boxed())
        .collect();
    if !offline_taildrive.is_empty() {
        let offline_count = offline_taildrive.len();
        let accordion_id = profile_id.clone();
        taildrive_rows.push(
            accordion_button(
                format!("Offline ({offline_count})"),
                offline_devices_expanded,
                move |state| state.toggle_tailscale_offline_devices(&accordion_id),
                palette,
            )
            .boxed(),
        );
        if offline_devices_expanded {
            taildrive_rows.extend(
                offline_taildrive
                    .into_iter()
                    .map(|device| taildrive_device_row(device, palette).boxed()),
            );
        }
    }
    let taildrive_devices = if taildrive_rows.is_empty() {
        let message = if taildrive_scanning {
            "Scanning Taildrive shares…".to_owned()
        } else if !taildrive_error.is_empty() {
            format!("Taildrive scan failed: {taildrive_error}")
        } else {
            "No Taildrive shares discovered on this Tailnet yet.".to_owned()
        };
        Either::A(prose(message).text_size(11.0).text_color(palette.muted))
    } else {
        Either::B(
            flex_col(taildrive_rows)
                .gap(0.px())
                .cross_axis_alignment(CrossAxisAlignment::Start),
        )
    };
    let devices_section = flex_col((
        prose("Tailnet devices")
            .text_size(12.0)
            .text_color(palette.text),
        peers,
        FlexSpacer::Fixed(10.px()),
        flex_row((
            prose("Taildrive devices")
                .text_size(12.0)
                .text_color(palette.text)
                .flex(1.0),
            open_taildrive,
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Center),
        taildrive_devices,
    ))
    .gap(0.px())
    .cross_axis_alignment(CrossAxisAlignment::Start);
    let remove_id = profile_id.clone();

    sized_box(
        flex_col((
            flex_row((
                sized_box(label_input).height(34.px()).flex(1.0),
                toolbar_button(
                    "Remove",
                    false,
                    move |state| state.remove_tailnet_profile(&remove_id),
                    palette,
                ),
            ))
            .gap(0.px())
            .cross_axis_alignment(CrossAxisAlignment::Center),
            FlexSpacer::Fixed(7.px()),
            settings_row("Tailscale device name", hostname_input, palette),
            FlexSpacer::Fixed(6.px()),
            prose("Press Enter after changing the device name to reconnect with the new name.")
                .text_size(11.0)
                .text_color(palette.muted),
            FlexSpacer::Fixed(6.px()),
            flex_col((
                prose(format!("Network: {network_name}"))
                    .text_size(12.0)
                    .text_color(palette.muted),
                FlexSpacer::Fixed(3.px()),
                prose(status_text).text_size(12.0).text_color(palette.text),
                FlexSpacer::Fixed(3.px()),
                prose(webdav_text).text_size(11.0).text_color(palette.muted),
            ))
            .gap(0.px())
            .cross_axis_alignment(CrossAxisAlignment::Start),
            FlexSpacer::Fixed(8.px()),
            actions,
            FlexSpacer::Fixed(10.px()),
            devices_section,
            FlexSpacer::Fixed(5.px()),
            prose(profile.ping_status)
                .text_size(11.0)
                .text_color(palette.muted),
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Start),
    )
    .expand_width()
    .padding(Padding::from_vh(10.0, 12.0))
    .background_color(palette.surface)
    .border(palette.border, 1.0)
    .corner_radius(Layout::RADIUS)
}

fn tailscale_peer_row(
    profile_id: String,
    peer: crate::tailscale::TailscalePeer,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    let display_name = if peer.hostname.is_empty() {
        peer.target.clone()
    } else {
        peer.hostname.clone()
    };
    let target = peer.target.clone();
    let callback_label = display_name.clone();
    let endpoint = if !peer.dns_name.is_empty() {
        peer.dns_name.clone()
    } else {
        peer.ips
            .first()
            .cloned()
            .unwrap_or_else(|| peer.target.clone())
    };
    let availability = if peer.online { "Online" } else { "Offline" };
    let detail = if peer.os.is_empty() {
        format!("{availability} — {endpoint}")
    } else {
        format!("{availability} — {endpoint} — {}", peer.os)
    };
    let test = toolbar_button(
        "Test",
        !peer.online,
        move |state| {
            state.ping_tailscale_peer(&profile_id, target.clone(), callback_label.clone());
        },
        palette,
    );
    sized_box(
        flex_col((
            flex_row((
                sized_box(label(display_name).text_size(13.0).color(palette.text)).flex(1.0),
                test,
            ))
            .gap(0.px())
            .cross_axis_alignment(CrossAxisAlignment::Center),
            FlexSpacer::Fixed(2.px()),
            prose(detail).text_size(11.0).text_color(palette.muted),
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Start),
    )
    .expand_width()
    .padding(Padding::vertical(6.0))
}

fn taildrive_device_row(
    device: crate::tailscale::TaildriveDevice,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    let display_name = if !device.hostname.is_empty() {
        device.hostname.clone()
    } else if !device.target.is_empty() {
        device.target.clone()
    } else {
        device.id.clone()
    };
    let endpoint = if !device.dns_name.is_empty() {
        device.dns_name.clone()
    } else {
        device
            .ips
            .first()
            .cloned()
            .unwrap_or_else(|| device.target.clone())
    };
    let availability = if device.online { "Online" } else { "Offline" };
    let detail = if device.os.is_empty() {
        format!("{availability} — {endpoint}")
    } else {
        format!("{availability} — {endpoint} — {}", device.os)
    };
    let shares = if device.shares.is_empty() {
        "No accessible shares".to_owned()
    } else {
        format!("Shares: {}", device.shares.join(", "))
    };

    sized_box(
        flex_col((
            label(display_name).text_size(13.0).color(palette.text),
            FlexSpacer::Fixed(2.px()),
            prose(detail).text_size(11.0).text_color(palette.muted),
            FlexSpacer::Fixed(2.px()),
            prose(shares).text_size(11.0).text_color(palette.text),
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Start),
    )
    .expand_width()
    .padding(Padding::vertical(6.0))
}

fn settings_card(
    title: &'static str,
    description: &'static str,
    body: impl WidgetView<AppState>,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    sized_box(
        flex_col((
            label(title).text_size(17.0).color(palette.text),
            FlexSpacer::Fixed(4.px()),
            prose(description).text_size(12.0).text_color(palette.muted),
            FlexSpacer::Fixed(16.px()),
            body,
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Fill),
    )
    .expand_width()
    .padding(18.0)
    .background_color(palette.surface)
    .border(palette.border, 1.0)
    .corner_radius(8.0)
}

#[cfg(not(target_os = "android"))]
fn settings_row(
    title: &'static str,
    control: impl WidgetView<AppState>,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    flex_row((
        sized_box(label(title).text_size(12.0).color(palette.muted)).width(132.px()),
        sized_box(control).expand_width().flex(1.0),
    ))
    .gap(0.px())
    .cross_axis_alignment(CrossAxisAlignment::Center)
}

#[cfg(target_os = "android")]
fn settings_row(
    title: &'static str,
    control: impl WidgetView<AppState>,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    flex_col((
        label(title).text_size(12.0).color(palette.muted),
        FlexSpacer::Fixed(6.px()),
        sized_box(control).expand_width(),
    ))
    .gap(0.px())
    .cross_axis_alignment(CrossAxisAlignment::Start)
}

#[cfg(not(target_os = "android"))]
pub fn address_bar(state: &AppState) -> impl WidgetView<AppState> + use<> {
    let palette = state.palette();
    let address = state.active_tab().address_input.clone();
    let search_text = state.active_tab().search_input.clone();
    let search_expanded = state.active_tab().search_field_expanded;

    let address_input = themed_text_input(
        address,
        "Location",
        AppState::set_address_input,
        AppState::submit_address,
        false,
        state.path_overflow_behavior() == PathOverflowBehavior::ForwardReset,
        state.path_reset_delay_ms() as f64 / 1000.0,
        10.0,
        state.is_dark_theme(),
        palette,
    );
    let search_input = themed_text_input(
        search_text,
        "Search this folder…",
        AppState::set_search_input,
        AppState::submit_search,
        true,
        false,
        3.0,
        14.0,
        state.is_dark_theme(),
        palette,
    );

    let address_group = sized_box(
        flex_row((address_input.flex(1.0),))
            .gap(0.px())
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .gap(0.px()),
    )
    .height(Layout::LOCATION_FIELD_HEIGHT.px())
    .background_color(palette.surface)
    .border(palette.border, 1.0)
    .corner_radius(Layout::RADIUS)
    .flex(1.0);

    let search_group = if search_expanded {
        Either::A(
            sized_box(
                flex_row((
                    sized_box(search_input).expand_width().flex(1.0),
                    field_icon_button(
                        LucideIcon::X,
                        "Clear or close search",
                        false,
                        AppState::clear_or_collapse_search,
                        palette,
                    ),
                ))
                .gap(0.px())
                .cross_axis_alignment(CrossAxisAlignment::Center),
            )
            .width(Layout::SEARCH_GROUP_WIDTH.px())
            .height(Layout::LOCATION_FIELD_HEIGHT.px())
            .background_color(palette.surface)
            .border(palette.border, 1.0)
            .corner_radius(Layout::RADIUS),
        )
    } else {
        Either::B(
            sized_box(field_icon_button(
                LucideIcon::Search,
                "Search this folder",
                false,
                AppState::expand_search_field,
                palette,
            ))
            .width(Layout::LOCATION_FIELD_HEIGHT.px())
            .height(Layout::LOCATION_FIELD_HEIGHT.px())
            .background_color(palette.surface)
            .border(palette.border, 1.0)
            .corner_radius(Layout::RADIUS),
        )
    };

    let address_controls = sized_box(
        flex_row((
            navigation_button(NavigationButtonKind::Back, !state.can_go_back(), palette),
            navigation_button(
                NavigationButtonKind::Forward,
                !state.can_go_forward(),
                palette,
            ),
            navigation_button(NavigationButtonKind::Up, !state.can_go_up(), palette),
            navigation_button(NavigationButtonKind::Home, !state.can_go_home(), palette),
            navigation_button(NavigationButtonKind::Refresh, false, palette),
            FlexSpacer::Fixed(8.px()),
            address_group,
            FlexSpacer::Fixed(8.px()),
            search_group,
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Center),
    )
    .flex(1.0);

    sized_box(
        flex_row((
            address_controls,
            FlexSpacer::Fixed(8.px()),
            icon_button(
                LucideIcon::Settings,
                "Settings",
                false,
                AppState::open_settings,
                palette,
            ),
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Center),
    )
    .height(Layout::ADDRESS_HEIGHT.px())
    .expand_width()
    .padding(Padding::from_vh(6.0, 10.0))
    .background_color(palette.chrome)
}

#[cfg(target_os = "android")]
pub fn address_bar(state: &AppState) -> impl WidgetView<AppState> + use<> {
    let palette = state.palette();
    let address = state.active_tab().address_input.clone();
    let search_text = state.active_tab().search_input.clone();
    let search_expanded = state.active_tab().search_field_expanded;

    let address_input = themed_text_input(
        address,
        "Location",
        AppState::set_address_input,
        AppState::submit_address,
        false,
        state.path_overflow_behavior() == PathOverflowBehavior::ForwardReset,
        state.path_reset_delay_ms() as f64 / 1000.0,
        10.0,
        state.is_dark_theme(),
        palette,
    );
    let search_input = themed_text_input(
        search_text,
        "Search this folder…",
        AppState::set_search_input,
        AppState::submit_search,
        true,
        false,
        3.0,
        14.0,
        state.is_dark_theme(),
        palette,
    );

    let address_group = sized_box(address_input)
        .height(Layout::LOCATION_FIELD_HEIGHT.px())
        .expand_width()
        .background_color(palette.surface)
        .border(palette.border, 1.0)
        .corner_radius(Layout::RADIUS);
    let navigation_left = sized_box(portal(
        flex_row((
            navigation_button(NavigationButtonKind::Back, !state.can_go_back(), palette),
            navigation_button(
                NavigationButtonKind::Forward,
                !state.can_go_forward(),
                palette,
            ),
            navigation_button(NavigationButtonKind::Up, !state.can_go_up(), palette),
            navigation_button(NavigationButtonKind::Home, !state.can_go_home(), palette),
            navigation_button(NavigationButtonKind::Refresh, false, palette),
            icon_button(
                LucideIcon::Network,
                "TailDrive",
                false,
                AppState::open_taildrive_root,
                palette,
            ),
            transfer_progress_button(state, palette),
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Center),
    ))
    .height(Layout::TOOL_HEIGHT.px())
    .flex(1.0);
    // Settings is structural navigation, not overflow content. Keep it pinned to
    // the physical right edge even when the navigation group needs to scroll.
    let navigation = sized_box(
        flex_row((
            navigation_left,
            icon_button(
                LucideIcon::Settings,
                "Settings",
                false,
                AppState::open_settings,
                palette,
            ),
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Center),
    )
    .height(Layout::TOOL_HEIGHT.px())
    .expand_width();

    let pinned_buttons = state
        .pinned_paths()
        .iter()
        .cloned()
        .map(|path| {
            let label_text = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| crate::app::display_path(&path));
            toolbar_button(
                label_text,
                false,
                move |state| state.navigate_to(path.clone()),
                palette,
            )
        })
        .collect::<Vec<_>>();
    let pinned_bar = if pinned_buttons.is_empty() {
        Either::A(sized_box(label("")).height(0.px()))
    } else {
        Either::B(
            sized_box(portal(
                flex_row(pinned_buttons)
                    .gap(4.px())
                    .cross_axis_alignment(CrossAxisAlignment::Center),
            ))
            .height(Layout::TOOL_HEIGHT.px())
            .expand_width(),
        )
    };

    let search_group = if search_expanded {
        Either::A(
            sized_box(
                flex_row((
                    sized_box(search_input).expand_width().flex(1.0),
                    field_icon_button(
                        LucideIcon::X,
                        "Clear or close search",
                        false,
                        AppState::clear_or_collapse_search,
                        palette,
                    ),
                ))
                .gap(0.px())
                .cross_axis_alignment(CrossAxisAlignment::Center),
            )
            .width(180.px())
            .height(Layout::LOCATION_FIELD_HEIGHT.px())
            .background_color(palette.surface)
            .border(palette.border, 1.0)
            .corner_radius(Layout::RADIUS),
        )
    } else {
        Either::B(
            sized_box(field_icon_button(
                LucideIcon::Search,
                "Search this folder",
                false,
                AppState::expand_search_field,
                palette,
            ))
            .width(Layout::LOCATION_FIELD_HEIGHT.px())
            .height(Layout::LOCATION_FIELD_HEIGHT.px())
            .background_color(palette.surface)
            .border(palette.border, 1.0)
            .corner_radius(Layout::RADIUS),
        )
    };
    let location_row = sized_box(
        flex_row((
            address_group.flex(1.0),
            FlexSpacer::Fixed(8.px()),
            search_group,
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Center),
    )
    .expand_width();

    sized_box(
        flex_col((
            navigation,
            pinned_bar,
            FlexSpacer::Fixed(4.px()),
            location_row,
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Start),
    )
    .expand_width()
    .padding(Padding::from_vh(4.0, 4.0))
    .background_color(palette.chrome)
}

fn settings_text_input(
    value: String,
    placeholder: &'static str,
    on_change: impl Fn(&mut AppState, String) + Send + Sync + 'static,
    on_enter: impl Fn(&mut AppState, String) + Send + Sync + 'static,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    text_input(value, on_change)
        .placeholder(placeholder)
        .insert_newline(InsertNewline::Never)
        .on_enter(on_enter)
        .text_color(palette.text)
        .caret_color(palette.focus)
        .padding(Padding::from_vh(5.0, 9.0))
        .background_color(palette.surface)
        .border(palette.border, 1.0)
        .corner_radius(Layout::RADIUS)
}

#[allow(
    clippy::too_many_arguments,
    reason = "UI text field helper carries callbacks and field behavior explicitly"
)]
fn themed_text_input(
    value: String,
    placeholder: &'static str,
    on_change: impl Fn(&mut AppState, String) + Send + Sync + 'static,
    on_enter: impl Fn(&mut AppState, String) + Send + Sync + 'static,
    auto_focus: bool,
    marquee_when_unfocused: bool,
    marquee_end_hold_seconds: f64,
    horizontal_padding: f64,
    dark: bool,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    if dark {
        Either::A(
            text_input(value, on_change)
                .placeholder(placeholder)
                .insert_newline(InsertNewline::Never)
                .on_enter(on_enter)
                .auto_focus(auto_focus)
                .marquee_when_unfocused(marquee_when_unfocused)
                .marquee_end_hold_seconds(marquee_end_hold_seconds)
                .text_color(palette.text)
                .caret_color(palette.focus)
                .padding(Padding::from_vh(6.0, horizontal_padding))
                .background_color(palette.surface)
                .border(palette.border, 1.0)
                .corner_radius(Layout::RADIUS),
        )
    } else {
        Either::B(
            text_input(value, on_change)
                .placeholder(placeholder)
                .insert_newline(InsertNewline::Never)
                .on_enter(on_enter)
                .auto_focus(auto_focus)
                .marquee_when_unfocused(marquee_when_unfocused)
                .marquee_end_hold_seconds(marquee_end_hold_seconds)
                .text_color(palette.text)
                .caret_color(palette.focus)
                .padding(Padding::from_vh(6.0, horizontal_padding))
                .background_color(palette.surface)
                .border(palette.border, 1.0)
                .corner_radius(Layout::RADIUS),
        )
    }
}

#[cfg(not(target_os = "android"))]
pub fn file_action_bar(state: &AppState) -> impl WidgetView<AppState> + use<> {
    let palette = state.palette();
    let has_selection = state.has_selection();
    let clipboard_selected = state.can_clipboard_selected();
    let read_only = !state.can_mutate_current_location();
    let pin_label = if state.selected_is_pinned() {
        "Unpin"
    } else {
        "Pin"
    };
    let normal = sized_box(portal(
        flex_row((
            toolbar_button("New folder", read_only, AppState::new_folder, palette),
            FlexSpacer::Fixed(4.px()),
            toolbar_button(
                "Cut",
                read_only || !clipboard_selected,
                AppState::cut_selected,
                palette,
            ),
            toolbar_button(
                "Copy",
                !clipboard_selected,
                AppState::copy_selected,
                palette,
            ),
            toolbar_button("Paste", !state.can_paste(), AppState::paste, palette),
            FlexSpacer::Fixed(4.px()),
            toolbar_button(
                "Rename",
                read_only || !has_selection,
                AppState::begin_rename,
                palette,
            ),
            toolbar_button(
                "Delete",
                read_only || !has_selection,
                AppState::delete_selected,
                palette,
            ),
            FlexSpacer::Fixed(4.px()),
            toolbar_button("Sort", false, AppState::open_sort_popup, palette),
            toolbar_button(
                pin_label,
                !has_selection,
                AppState::toggle_pin_selected,
                palette,
            ),
            toolbar_button("Share", !has_selection, AppState::share_selected, palette),
            FlexSpacer::Flex(1.0),
            icon_button(
                LucideIcon::Ellipsis,
                "More file actions",
                true,
                AppState::toggle_file_more_popup,
                palette,
            ),
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Center),
    ))
    .height(Layout::ACTION_HEIGHT.px())
    .expand_width()
    .padding(Padding::horizontal(10.0))
    .background_color(palette.chrome)
    .border(palette.border, 1.0);

    let context = sized_box(portal(
        flex_row((
            label("Selected item").text_size(12.0).color(palette.muted),
            FlexSpacer::Fixed(8.px()),
            toolbar_button("Open", !has_selection, AppState::activate_selected, palette),
            toolbar_button(
                "Cut",
                read_only || !clipboard_selected,
                AppState::cut_selected,
                palette,
            ),
            toolbar_button(
                "Copy",
                !clipboard_selected,
                AppState::copy_selected,
                palette,
            ),
            toolbar_button(
                "Rename",
                read_only || !has_selection,
                AppState::begin_rename,
                palette,
            ),
            toolbar_button(
                "Delete",
                read_only || !has_selection,
                AppState::delete_selected,
                palette,
            ),
            toolbar_button("Sort", false, AppState::open_sort_popup, palette),
            toolbar_button(
                pin_label,
                !has_selection,
                AppState::toggle_pin_selected,
                palette,
            ),
            toolbar_button("Share", !has_selection, AppState::share_selected, palette),
            FlexSpacer::Flex(1.0),
            toolbar_button("Close", false, AppState::close_context_actions, palette),
            FlexSpacer::Fixed(4.px()),
            icon_button(
                LucideIcon::Ellipsis,
                "More file actions",
                true,
                AppState::toggle_file_more_popup,
                palette,
            ),
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Center),
    ))
    .height(Layout::ACTION_HEIGHT.px())
    .expand_width()
    .padding(Padding::horizontal(10.0))
    .background_color(palette.accent_soft)
    .border(palette.border, 1.0);

    if state.context_actions_visible() {
        Either::A(context)
    } else {
        Either::B(normal)
    }
}

#[cfg(any(target_os = "android", test))]
#[derive(Clone, Copy)]
struct MobileActionVisibility {
    new_folder: bool,
    cut: bool,
    copy: bool,
    paste: bool,
    rename: bool,
    delete: bool,
    sort: bool,
    share: bool,
    pin: bool,
}

#[cfg(any(target_os = "android", test))]
impl MobileActionVisibility {
    fn has_overflow(self) -> bool {
        !(self.new_folder
            && self.cut
            && self.copy
            && self.paste
            && self.rename
            && self.delete
            && self.sort
            && self.share
            && self.pin)
    }
}

#[cfg(any(target_os = "android", test))]
fn mobile_action_visibility(capacity: usize) -> MobileActionVisibility {
    MobileActionVisibility {
        sort: capacity >= 1,
        copy: capacity >= 2,
        paste: capacity >= 3,
        delete: capacity >= 4,
        cut: capacity >= 5,
        new_folder: capacity >= 6,
        rename: capacity >= 7,
        share: capacity >= 8,
        pin: capacity >= 9,
    }
}

#[cfg(test)]
#[test]
fn mobile_action_visibility_progressively_overflows() {
    let full = mobile_action_visibility(9);
    assert!(!full.has_overflow());
    assert!(full.sort && full.share && full.pin);

    let normal_phone = mobile_action_visibility(8);
    assert!(normal_phone.sort && normal_phone.share);
    assert!(!normal_phone.pin);
    assert!(normal_phone.has_overflow());

    let medium = mobile_action_visibility(4);
    assert!(medium.copy && medium.paste && medium.delete && medium.sort);
    assert!(!medium.cut && !medium.new_folder && !medium.rename && !medium.share && !medium.pin);

    let sort_only = mobile_action_visibility(1);
    assert!(sort_only.sort);
    assert!(!sort_only.copy && !sort_only.paste && !sort_only.delete);
    assert!(sort_only.has_overflow());

    let tiny = mobile_action_visibility(0);
    assert!(tiny.has_overflow());
    assert!(!tiny.copy && !tiny.paste && !tiny.delete && !tiny.sort);
}

#[cfg(target_os = "android")]
pub fn file_action_bar(state: &AppState) -> impl WidgetView<AppState> + use<> {
    let palette = state.palette();
    let has_selection = state.has_selection();
    let clipboard_selected = state.can_clipboard_selected();
    let storage_blocked = !state.android_storage_access_granted() && !state.is_taildrive_current();
    let read_only = !state.can_mutate_current_location();
    let visible = mobile_action_visibility(state.mobile_primary_action_capacity());
    let mut actions: Vec<Box<AnyWidgetView<AppState>>> = Vec::new();

    if visible.new_folder {
        actions.push(
            file_action_icon_button(
                LucideIcon::FolderPlus,
                "New folder",
                storage_blocked || read_only,
                AppState::new_folder,
                palette,
            )
            .boxed(),
        );
    }
    if visible.cut {
        actions.push(
            file_action_icon_button(
                LucideIcon::Scissors,
                "Cut",
                read_only || !clipboard_selected,
                AppState::cut_selected,
                palette,
            )
            .boxed(),
        );
    }
    if visible.copy {
        actions.push(
            file_action_icon_button(
                LucideIcon::Copy,
                "Copy",
                !clipboard_selected,
                AppState::copy_selected,
                palette,
            )
            .boxed(),
        );
    }
    if visible.paste {
        actions.push(
            file_action_icon_button(
                LucideIcon::ClipboardPaste,
                "Paste",
                storage_blocked || !state.can_paste(),
                AppState::paste,
                palette,
            )
            .boxed(),
        );
    }
    if visible.rename {
        actions.push(
            file_action_icon_button(
                LucideIcon::Pencil,
                "Rename",
                read_only || !has_selection,
                AppState::begin_rename,
                palette,
            )
            .boxed(),
        );
    }
    if visible.delete {
        actions.push(
            file_action_icon_button(
                LucideIcon::Trash2,
                "Delete",
                read_only || !has_selection,
                AppState::delete_selected,
                palette,
            )
            .boxed(),
        );
    }
    if visible.sort {
        actions.push(
            file_action_icon_button(
                LucideIcon::ArrowUpDown,
                "Sort",
                false,
                AppState::open_sort_popup,
                palette,
            )
            .boxed(),
        );
    }
    if visible.share {
        actions.push(
            file_action_icon_button(
                LucideIcon::Share2,
                "Share",
                !has_selection,
                AppState::share_selected,
                palette,
            )
            .boxed(),
        );
    }
    if visible.pin {
        let pin_label = if state.selected_is_pinned() {
            "Unpin"
        } else {
            "Pin"
        };
        actions.push(
            file_action_icon_button(
                LucideIcon::Pin,
                pin_label,
                !has_selection,
                AppState::toggle_pin_selected,
                palette,
            )
            .boxed(),
        );
    }
    let primary_actions = flex_row(actions)
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Center);
    let actions = flex_row((
        primary_actions,
        FlexSpacer::Flex(1.0),
        file_action_icon_button(
            LucideIcon::Ellipsis,
            "More file actions",
            !visible.has_overflow(),
            AppState::toggle_file_more_popup,
            palette,
        ),
    ))
    .gap(0.px())
    .cross_axis_alignment(CrossAxisAlignment::Center);

    sized_box(actions)
        .height(Layout::TOOL_HEIGHT.px())
        .expand_width()
        .padding(Padding::horizontal(4.0))
        .background_color(palette.chrome)
        .border(palette.border, 1.0)
}

fn quick_access_items() -> Vec<(String, PathBuf)> {
    let mut items = Vec::new();
    if let Some(home) = crate::app::home_dir() {
        items.push(("Home".to_owned(), home.clone()));
        for name in ["Desktop", "Documents", "Downloads", "Pictures"] {
            let path = home.join(name);
            if path.is_dir() {
                items.push((name.to_owned(), path));
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        // GetLogicalDrives is a single in-memory OS query. Avoid probing A:\\..Z:\\
        // with is_dir(), which can synchronously stall on disconnected network or
        // removable drives while the UI thread is rendering.
        let drive_mask = unsafe { windows_sys::Win32::Storage::FileSystem::GetLogicalDrives() };
        for index in 0..26_u32 {
            if drive_mask & (1_u32 << index) == 0 {
                continue;
            }
            let letter = char::from(b'A' + index as u8);
            items.push((format!("{letter}:"), PathBuf::from(format!("{letter}:\\"))));
        }
    }
    #[cfg(not(target_os = "windows"))]
    items.push(("Root".to_owned(), PathBuf::from("/")));
    items
}

pub fn sidebar(state: &AppState) -> impl WidgetView<AppState> + use<> {
    let palette = state.palette();
    let current = state.active_tab().current_dir.clone();
    let mut items = quick_access_items();
    for path in state.pinned_paths() {
        if items.iter().any(|(_, existing)| existing == path) {
            continue;
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| crate::app::display_path(path));
        items.push((name, path.clone()));
    }
    items.push((
        "TailDrive".to_owned(),
        taildrive_path(&TaildriveLocation::Root),
    ));

    let buttons = items
        .into_iter()
        .map(|(name, path)| {
            let selected = current == path;
            quick_path_button(name, path, selected, palette)
        })
        .collect::<Vec<_>>();

    sized_box(
        flex_col((
            sized_box(label("Quick access").text_size(12.0).color(palette.muted))
                .padding(Padding::horizontal(8.0)),
            FlexSpacer::Fixed(4.px()),
            buttons,
        ))
        .gap(0.px())
        .main_axis_alignment(MainAxisAlignment::Start)
        .cross_axis_alignment(CrossAxisAlignment::Start),
    )
    .expand()
    .padding(Padding::from_vh(8.0, 8.0))
    .background_color(palette.sidebar)
}

#[cfg(not(target_os = "android"))]
pub fn file_area(state: &AppState) -> impl WidgetView<AppState> + use<> {
    let palette = state.palette();
    let selected_index = state.selected_entry_index();
    let item_count = state.active_tab().entries.len();
    let sort_field = state.sort_field();
    let sort_direction = state.sort_direction();
    let header = sized_box(column_row(
        label("").text_size(12.0),
        sort_column_button("Name", SortField::Name, sort_field, sort_direction, palette),
        sort_column_button("Type", SortField::Type, sort_field, sort_direction, palette),
        sort_column_button("Size", SortField::Size, sort_field, sort_direction, palette),
    ))
    .height(Layout::HEADER_HEIGHT.px())
    .expand_width()
    .padding(Padding::horizontal(8.0))
    .background_color(palette.header)
    .border(palette.border, 1.0);

    let list_content = if item_count == 0 {
        Either::A(empty_file_state(state, palette))
    } else {
        Either::B(
            virtual_scroll(0..item_count as i64, virtual_file_row)
                .reset_on_change(state.file_scroll_reset_key()),
        )
    };
    let shortcuts = file_list_shortcuts(
        list_content,
        item_count,
        selected_index,
        state.rename_active(),
    );
    let body = sized_box(shortcuts).expand();

    sized_box(flex_col((header, sized_box(body).expand().flex(1.0))).gap(0.px()))
        .expand()
        .background_color(palette.surface)
}

#[cfg(not(target_os = "android"))]
fn sort_column_button(
    text: &'static str,
    field: SortField,
    active_field: SortField,
    direction: SortDirection,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    let text = if active_field == field {
        format!("{text} {}", direction.symbol())
    } else {
        text.to_owned()
    };
    button(
        label(text).text_size(12.0).color(palette.muted),
        move |state: &mut AppState| state.activate_sort_field(field),
    )
    .padding(Padding::from_vh(4.0, 0.0))
    .background_color(palette.header)
    .active_background_color(palette.accent_soft)
    .border(palette.header, 0.0)
    .hovered_border_color(palette.border_strong)
    .corner_radius(0.0)
}

fn taildrive_loading_status(status: &str) -> bool {
    status.starts_with("Waiting for Tailscale")
        || status.starts_with("Connecting to TailDrive")
        || status.starts_with("Reconnecting to TailDrive")
        || status.starts_with("TailDrive worker is starting")
        || status.starts_with("Loading TailDrive")
        || status.starts_with("Scanning TailDrive")
}

#[cfg(target_os = "android")]
pub fn file_area(state: &AppState) -> impl WidgetView<AppState> + use<> {
    let palette = state.palette();
    if !state.android_storage_access_granted() && !state.is_taildrive_current() {
        return Either::A(android_storage_access_state(palette));
    }
    let selected_index = state.selected_entry_index();
    let item_count = state.active_tab().entries.len();

    let list_content = if item_count == 0 {
        Either::A(empty_file_state(state, palette))
    } else {
        // Keep a real scrollable tail below the last file row. The files page
        // draws behind Android's navigation/gesture area, so this spacer lets
        // the last tappable row move completely above that untappable inset.
        let virtual_item_count = item_count.saturating_add(1);
        Either::B(
            virtual_scroll(0..virtual_item_count as i64, android_virtual_file_row)
                .reset_on_change(state.file_scroll_reset_key()),
        )
    };
    let shortcuts = file_list_shortcuts(
        list_content,
        item_count,
        selected_index,
        state.rename_active(),
    );
    let status = state.active_tab().status.clone();
    let banner_text =
        (item_count > 0 && state.is_taildrive_current() && taildrive_loading_status(&status))
            .then_some(status);
    let loading_banner = if let Some(text) = banner_text {
        Either::A(
            sized_box(label(text).text_size(11.0).color(palette.muted))
                .expand_width()
                .padding(Padding::from_vh(6.0, 10.0))
                .background_color(palette.header),
        )
    } else {
        Either::B(sized_box(label("")).height(0.px()))
    };

    Either::B(
        sized_box(flex_col((loading_banner, sized_box(shortcuts).expand().flex(1.0))).gap(0.px()))
            .expand()
            .background_color(palette.surface),
    )
}

fn empty_file_state(state: &AppState, palette: ThemePalette) -> impl WidgetView<AppState> + use<> {
    #[cfg(target_os = "android")]
    if !state.android_storage_access_granted() && !state.is_taildrive_current() {
        return Either::A(android_storage_access_state(palette));
    }

    let tab = state.active_tab();
    let searching = !tab.search_input.trim().is_empty();
    let (title, detail) = if searching && tab.search_active {
        ("No results", "Try a shorter or different search term.")
    } else if searching {
        ("Search unavailable", tab.status.as_str())
    } else if state.is_taildrive_current() && taildrive_loading_status(&tab.status) {
        ("Loading TailDrive…", tab.status.as_str())
    } else if state.is_taildrive_current()
        && (tab.status.starts_with("TailDrive:") || tab.status.starts_with("TailDrive unavailable"))
    {
        ("TailDrive unavailable", tab.status.as_str())
    } else if state.is_archive_current()
        && (tab.status.starts_with("Loading archive")
            || tab.status.starts_with("Loading restored archive"))
    {
        ("Loading archive…", tab.status.as_str())
    } else if tab.status.starts_with("Loading folder")
        || tab.status.starts_with("Loading restored folder")
        || tab.status.starts_with("Folder worker is starting")
    {
        ("Loading folder…", tab.status.as_str())
    } else if tab.status.starts_with("Cannot read archive") {
        ("Can't open this archive", tab.status.as_str())
    } else if tab.status.starts_with("Cannot read directory") {
        ("Can't open this folder", tab.status.as_str())
    } else {
        ("This folder is empty", "There are no items to show here.")
    };
    let standard = sized_box(
        flex_col((
            FlexSpacer::Flex(1.0),
            label(title).text_size(16.0).color(palette.text),
            FlexSpacer::Fixed(6.px()),
            sized_box(
                prose(detail.to_owned())
                    .text_size(12.0)
                    .text_color(palette.muted)
                    .text_alignment(TextAlign::Center),
            )
            .expand_width()
            .padding(Padding::horizontal(24.0)),
            FlexSpacer::Flex(1.0),
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Center),
    )
    .expand()
    .background_color(palette.surface);

    #[cfg(target_os = "android")]
    return Either::B(standard);
    #[cfg(not(target_os = "android"))]
    standard
}

#[cfg(target_os = "android")]
fn android_storage_access_state(palette: ThemePalette) -> impl WidgetView<AppState> {
    sized_box(
        flex_col((
            FlexSpacer::Flex(1.0),
            label("File access required")
                .text_size(18.0)
                .color(palette.text),
            FlexSpacer::Fixed(8.px()),
            sized_box(
                prose("FastExplorer needs Android's all-files access to browse shared storage. You can revoke it later in system settings.")
                    .text_size(12.0)
                    .text_color(palette.muted)
                    .text_alignment(TextAlign::Center),
            )
            .expand_width()
            .padding(Padding::horizontal(28.0)),
            FlexSpacer::Fixed(14.px()),
            choice_button(
                "Grant file access",
                false,
                AppState::request_android_storage_access,
                palette,
            ),
            FlexSpacer::Flex(1.0),
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Center),
    )
    .expand()
    .background_color(palette.surface)
}

pub fn status_bar(state: &AppState) -> impl WidgetView<AppState> + use<> {
    let palette = state.palette();
    let tab = state.active_tab();
    let right = if tab.search_active {
        format!("{} search", state.search_mode_label())
    } else {
        crate::app::display_path(&tab.current_dir)
    };
    sized_box(
        flex_row((
            label(tab.status.clone())
                .text_size(12.0)
                .color(palette.muted),
            FlexSpacer::Flex(1.0),
            label(right).text_size(12.0).color(palette.muted),
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Center),
    )
    .height(Layout::STATUS_HEIGHT.px())
    .expand_width()
    .padding(Padding::horizontal(10.0))
    .background_color(palette.chrome)
    .border(palette.border, 1.0)
}

fn inline_rename_input(
    value: String,
    is_file: bool,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    let selection_end = if is_file {
        std::path::Path::new(&value)
            .extension()
            .and_then(|extension| extension.to_str())
            .map_or(value.len(), |extension| {
                value.len().saturating_sub(extension.len() + 1)
            })
    } else {
        value.len()
    };
    text_input(value, AppState::set_rename_input)
        .insert_newline(InsertNewline::Never)
        .on_enter(AppState::submit_rename)
        .clip(true)
        .text_color(palette.text)
        .auto_focus(true)
        .initial_selection(0, selection_end)
        .caret_color(palette.focus)
        .padding(Padding::from_vh(2.0, 4.0))
        .background_color(palette.surface)
        .border(palette.focus, 1.0)
        .corner_radius(Layout::RADIUS)
}

#[cfg(target_os = "android")]
fn android_virtual_file_row(state: &mut AppState, index: i64) -> impl WidgetView<AppState> + use<> {
    let item_count = state.active_tab().entries.len();
    if usize::try_from(index).ok() == Some(item_count) {
        let safe_tail = state.android_insets().bottom.max(1.0);
        Either::A(sized_box(label("")).height(safe_tail.px()).expand_width())
    } else {
        Either::B(virtual_file_row(state, index))
    }
}

fn virtual_file_row(state: &mut AppState, index: i64) -> impl WidgetView<AppState> + use<> {
    let palette = state.palette();
    let index = usize::try_from(index).unwrap_or(usize::MAX);
    let Some(entry) = state.active_tab().entries.get(index).cloned() else {
        return Either::A(
            sized_box(label(""))
                .height(Layout::ROW_HEIGHT.px())
                .expand_width(),
        );
    };
    let thumbnail = state.thumbnail_for_entry(&entry);
    let selected = state.active_tab().selected_path.as_ref() == Some(&entry.path);
    let rename = if selected {
        state.active_tab().rename_input.clone()
    } else {
        None
    };
    Either::B(file_row(entry, selected, rename, thumbnail, palette))
}

#[cfg(not(target_os = "android"))]
fn file_row(
    entry: FileEntry,
    selected: bool,
    rename: Option<String>,
    thumbnail: Option<ImageData>,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    let path = entry.path.clone();
    let name = entry.name.clone();
    let accessibility_label = name.clone();
    let kind = entry.kind_label();
    let size = entry.size_label();
    let is_file = entry.kind == EntryKind::File;
    let rename_active = rename.is_some();
    let name_cell = if let Some(value) = rename {
        Either::A(inline_rename_input(value, is_file, palette))
    } else {
        Either::B(
            prose(name)
                .line_break_mode(LineBreaking::Clip)
                .text_size(13.0)
                .text_color(palette.text),
        )
    };
    let row = column_row(
        file_icon(&entry, thumbnail, palette),
        name_cell,
        prose(kind)
            .line_break_mode(LineBreaking::Clip)
            .text_size(13.0)
            .text_color(palette.muted),
        label(size).text_size(13.0).color(palette.muted),
    );

    let content = sized_box(row)
        .expand_width()
        .padding(Padding::horizontal(7.0));
    if rename_active {
        Either::A(
            sized_box(content)
                .height(Layout::ROW_HEIGHT.px())
                .expand_width()
                .background_color(palette.accent_soft)
                .border(palette.accent, 1.0),
        )
    } else {
        Either::B(
            sized_box(file_row_button(
                content,
                path,
                accessibility_label,
                selected,
                palette,
            ))
            .height(Layout::ROW_HEIGHT.px())
            .expand_width(),
        )
    }
}

#[cfg(target_os = "android")]
fn file_row(
    entry: FileEntry,
    selected: bool,
    rename: Option<String>,
    thumbnail: Option<ImageData>,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    let path = entry.path.clone();
    let name = entry.name.clone();
    let accessibility_label = name.clone();
    let kind = entry.kind_label();
    let size = entry.size_label();
    let metadata = if size.is_empty() {
        kind.to_owned()
    } else {
        format!("{kind} · {size}")
    };
    let is_file = entry.kind == EntryKind::File;
    let rename_active = rename.is_some();
    let name_cell = if let Some(value) = rename {
        Either::A(inline_rename_input(value, is_file, palette))
    } else {
        Either::B(
            prose(name)
                .line_break_mode(LineBreaking::Clip)
                .text_size(14.0)
                .text_color(palette.text),
        )
    };
    let details = flex_col((
        name_cell,
        FlexSpacer::Fixed(3.px()),
        label(metadata).text_size(11.0).color(palette.muted),
    ))
    .gap(0.px())
    .cross_axis_alignment(CrossAxisAlignment::Start);
    let row = flex_row((
        file_icon(&entry, thumbnail, palette),
        FlexSpacer::Fixed(8.px()),
        sized_box(details).flex(1.0),
    ))
    .gap(0.px())
    .cross_axis_alignment(CrossAxisAlignment::Center);

    let content = sized_box(row)
        .expand_width()
        .padding(Padding::horizontal(10.0));
    if rename_active {
        Either::A(
            sized_box(content)
                .height(Layout::ROW_HEIGHT.px())
                .expand_width()
                .background_color(palette.accent_soft)
                .border(palette.accent, 1.0),
        )
    } else {
        Either::B(
            sized_box(file_row_button(
                content,
                path,
                accessibility_label,
                selected,
                palette,
            ))
            .height(Layout::ROW_HEIGHT.px())
            .expand_width(),
        )
    }
}

fn file_icon(
    entry: &FileEntry,
    thumbnail: Option<ImageData>,
    palette: ThemePalette,
) -> impl WidgetView<AppState> + use<> {
    if let Some(thumbnail) = thumbnail {
        let edge = if cfg!(target_os = "android") {
            32.0
        } else {
            24.0
        };
        Either::A(
            sized_box(image(thumbnail).fit(ObjectFit::Cover))
                .width(edge.px())
                .height(edge.px()),
        )
    } else {
        Either::B(platform_file_icon(entry, palette))
    }
}

#[cfg(target_os = "windows")]
fn platform_file_icon(
    entry: &FileEntry,
    palette: ThemePalette,
) -> impl WidgetView<AppState> + use<> {
    let virtual_network = matches!(
        entry.remote,
        Some(TaildriveLocation::Profile { .. } | TaildriveLocation::Device { .. })
    );
    if !virtual_network
        && let Some(native) = crate::windows_icons::shell_icon(&entry.path, &entry.name, entry.kind)
    {
        Either::A(
            sized_box(sized_box(image(native)).width(20.px()).height(20.px()))
                .width(Layout::ICON_WIDTH.px()),
        )
    } else {
        Either::B(fallback_file_icon(entry, palette))
    }
}

#[cfg(not(target_os = "windows"))]
fn platform_file_icon(
    entry: &FileEntry,
    palette: ThemePalette,
) -> impl WidgetView<AppState> + use<> {
    fallback_file_icon(entry, palette)
}

fn fallback_file_icon(
    entry: &FileEntry,
    palette: ThemePalette,
) -> impl WidgetView<AppState> + use<> {
    let (kind, color, label_text) = match entry.category() {
        FileCategory::Folder => (LucideIcon::FolderOpen, palette.icon_folder, "Folder"),
        FileCategory::Text => (LucideIcon::FileText, palette.icon_file, "Document"),
        FileCategory::Image => (LucideIcon::FileImage, palette.icon_file, "Image"),
        FileCategory::Video => (LucideIcon::Video, palette.icon_file, "Video"),
        FileCategory::Audio => (LucideIcon::Music, palette.icon_file, "Audio"),
        FileCategory::Archive => (LucideIcon::FileArchive, palette.icon_file, "Archive"),
        FileCategory::Code => (LucideIcon::FileCode, palette.icon_file, "Source file"),
        FileCategory::Spreadsheet => (
            LucideIcon::FileSpreadsheet,
            palette.icon_file,
            "Spreadsheet",
        ),
        FileCategory::Presentation => (LucideIcon::Presentation, palette.icon_file, "Presentation"),
        FileCategory::Json => (LucideIcon::Braces, palette.icon_file, "Structured data"),
        FileCategory::Network => (LucideIcon::Network, palette.icon_link, "Network location"),
        FileCategory::Symlink => (LucideIcon::ExternalLink, palette.icon_link, "Symbolic link"),
        FileCategory::Generic | FileCategory::Other => {
            (LucideIcon::File, palette.icon_file, "File")
        }
    };
    let icon_size = if cfg!(target_os = "android") {
        20.0
    } else {
        16.0
    };
    sized_box(icon(kind, color, icon_size, label_text)).width(Layout::ICON_WIDTH.px())
}

fn quick_path_button(
    name: String,
    path: PathBuf,
    selected: bool,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    let background = if selected {
        palette.accent_soft
    } else {
        palette.sidebar
    };
    let indicator = if selected { palette.accent } else { background };
    let quick_icon = if parse_taildrive_path(&path).is_some() {
        LucideIcon::Network
    } else if name.ends_with(':') {
        LucideIcon::HardDrive
    } else if name == "Home" {
        LucideIcon::House
    } else {
        LucideIcon::FolderOpen
    };
    let content = sized_box(
        flex_row((
            sized_box(label(""))
                .width(3.px())
                .height(18.px())
                .background_color(indicator)
                .corner_radius(Layout::RADIUS),
            FlexSpacer::Fixed(7.px()),
            icon(quick_icon, palette.icon_file, 15.0, "Quick access location"),
            FlexSpacer::Fixed(7.px()),
            label(name).text_size(13.0).color(palette.text),
            FlexSpacer::Flex(1.0),
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Center),
    )
    .expand_width();
    sized_box(
        button(content, move |state: &mut AppState| {
            state.navigate_to(path.clone())
        })
        .padding(Padding::from_vh(5.0, 8.0))
        .background_color(background)
        .active_background_color(palette.accent_soft)
        .border(background, 1.0)
        .hovered_border_color(palette.border_strong)
        .corner_radius(Layout::RADIUS),
    )
    .height(32.px())
    .expand_width()
}

struct TabItemSpec {
    index: usize,
    drag_index: usize,
    drop_targets: Vec<usize>,
    tab_id: u64,
    title: String,
    active: bool,
    width: f64,
    widths: Vec<f64>,
    scroll_leading: f64,
}

fn tab_item(spec: TabItemSpec, palette: ThemePalette) -> impl WidgetView<AppState> {
    let TabItemSpec {
        index,
        drag_index,
        drop_targets,
        tab_id,
        title,
        active,
        width: tab_width,
        widths: tab_widths,
        scroll_leading,
    } = spec;
    let background = if active {
        palette.tab_active
    } else {
        palette.tab_inactive
    };
    let accessibility_label = title.clone();
    let content = sized_box(
        flex_row((sized_box(
            prose(title)
                .line_break_mode(LineBreaking::Clip)
                .text_size(13.0)
                .text_color(palette.text),
        )
        .flex(1.0),))
        .gap(0.px()),
    )
    .expand_width()
    .padding(Padding::from_vh(5.0, 5.0));
    let close = tab_close_button(index, true, palette);
    let border_color = if active {
        palette.border_strong
    } else {
        palette.border
    };
    let surface = sized_box(
        flex_row((sized_box(content).flex(1.0), close))
            .gap(0.px())
            .cross_axis_alignment(CrossAxisAlignment::Center),
    )
    .expand()
    .background_color(background)
    .border(border_color, 1.0)
    .corner_radius(Layout::RADIUS);
    let drag = tab_drag_button(
        surface,
        TabDragConfig {
            tab_id,
            source_index: index,
            drag_index,
            tab_widths,
            drop_targets,
            scroll_leading,
            drag_handle_right_inset: Layout::TAB_CLOSE_WIDTH,
            accessibility_label,
            selected: active,
            background,
            border: border_color,
            text_color: palette.text,
        },
    );

    sized_box(sized_box(drag).padding(Padding::horizontal(2.0)))
        .width(tab_width.px())
        .height(Layout::TOOL_HEIGHT.px())
}

fn tab_close_button(
    index: usize,
    enabled: bool,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    let color = if enabled { palette.text } else { palette.muted };
    sized_box(
        button(
            icon(LucideIcon::X, color, 13.0, "Close tab"),
            move |state: &mut AppState| {
                if state.close_tab(index) {
                    crate::ipc::cleanup_owned_socket();
                    std::process::exit(0);
                }
            },
        )
        .disabled(!enabled)
        .padding(3.0)
        .background_color(Color::TRANSPARENT)
        .disabled_background_color(Color::TRANSPARENT)
        .active_background_color(palette.accent_soft)
        .border(Color::TRANSPARENT, 0.0)
        .hovered_border_color(palette.border_strong)
        .corner_radius(Layout::RADIUS),
    )
    .width(Layout::TAB_CLOSE_WIDTH.px())
    .height(Layout::TAB_CLOSE_WIDTH.px())
}

#[cfg(not(target_os = "android"))]
fn ui_font_picker(state: &AppState, palette: ThemePalette) -> impl WidgetView<AppState> + use<> {
    let selected = state.ui_font();
    flex_row((
        ui_font_button(UiFont::System, selected, palette).flex(1.0),
        FlexSpacer::Fixed(6.px()),
        ui_font_button(UiFont::Sans, selected, palette).flex(1.0),
        FlexSpacer::Fixed(6.px()),
        ui_font_button(UiFont::Serif, selected, palette).flex(1.0),
        FlexSpacer::Fixed(6.px()),
        ui_font_button(UiFont::Monospace, selected, palette).flex(1.0),
        FlexSpacer::Fixed(6.px()),
        ui_font_button(UiFont::Rounded, selected, palette).flex(1.0),
    ))
    .gap(0.px())
    .cross_axis_alignment(CrossAxisAlignment::Center)
}

#[cfg(target_os = "android")]
fn ui_font_picker(state: &AppState, palette: ThemePalette) -> impl WidgetView<AppState> + use<> {
    let selected = state.ui_font();
    flex_col((
        flex_row((
            ui_font_button(UiFont::System, selected, palette).flex(1.0),
            FlexSpacer::Fixed(6.px()),
            ui_font_button(UiFont::Sans, selected, palette).flex(1.0),
            FlexSpacer::Fixed(6.px()),
            ui_font_button(UiFont::Serif, selected, palette).flex(1.0),
        ))
        .gap(0.px()),
        FlexSpacer::Fixed(6.px()),
        flex_row((
            ui_font_button(UiFont::Monospace, selected, palette).flex(1.0),
            FlexSpacer::Fixed(6.px()),
            ui_font_button(UiFont::Rounded, selected, palette).flex(1.0),
        ))
        .gap(0.px()),
    ))
    .gap(0.px())
    .cross_axis_alignment(CrossAxisAlignment::Fill)
}

fn ui_font_button(
    font: UiFont,
    selected: UiFont,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    fill_choice_button(
        font.label(),
        font == selected,
        move |state| state.set_ui_font(font, true),
        palette,
    )
}

#[cfg(not(target_os = "android"))]
fn theme_color_picker(
    state: &AppState,
    palette: ThemePalette,
) -> impl WidgetView<AppState> + use<> {
    let selected = state.theme_color();
    flex_row((
        theme_color_button(ThemeColor::Blue, selected, palette).flex(1.0),
        FlexSpacer::Fixed(5.px()),
        theme_color_button(ThemeColor::Red, selected, palette).flex(1.0),
        FlexSpacer::Fixed(5.px()),
        theme_color_button(ThemeColor::Green, selected, palette).flex(1.0),
        FlexSpacer::Fixed(5.px()),
        theme_color_button(ThemeColor::Purple, selected, palette).flex(1.0),
        FlexSpacer::Fixed(5.px()),
        theme_color_button(ThemeColor::Orange, selected, palette).flex(1.0),
        FlexSpacer::Fixed(5.px()),
        theme_color_button(ThemeColor::Teal, selected, palette).flex(1.0),
        FlexSpacer::Fixed(5.px()),
        theme_color_button(ThemeColor::Pink, selected, palette).flex(1.0),
        FlexSpacer::Fixed(5.px()),
        theme_color_button(ThemeColor::Neutral, selected, palette).flex(1.0),
    ))
    .gap(0.px())
    .cross_axis_alignment(CrossAxisAlignment::Center)
}

#[cfg(target_os = "android")]
fn theme_color_picker(
    state: &AppState,
    palette: ThemePalette,
) -> impl WidgetView<AppState> + use<> {
    let selected = state.theme_color();
    let row = |a, b, c, d| {
        flex_row((
            theme_color_button(a, selected, palette).flex(1.0),
            FlexSpacer::Fixed(6.px()),
            theme_color_button(b, selected, palette).flex(1.0),
            FlexSpacer::Fixed(6.px()),
            theme_color_button(c, selected, palette).flex(1.0),
            FlexSpacer::Fixed(6.px()),
            theme_color_button(d, selected, palette).flex(1.0),
        ))
        .gap(0.px())
    };
    flex_col((
        row(
            ThemeColor::Blue,
            ThemeColor::Red,
            ThemeColor::Green,
            ThemeColor::Purple,
        ),
        FlexSpacer::Fixed(6.px()),
        row(
            ThemeColor::Orange,
            ThemeColor::Teal,
            ThemeColor::Pink,
            ThemeColor::Neutral,
        ),
    ))
    .gap(0.px())
    .cross_axis_alignment(CrossAxisAlignment::Start)
}

fn theme_color_button(
    color: ThemeColor,
    selected: ThemeColor,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    let swatch = sized_box(label(""))
        .width(12.px())
        .height(12.px())
        .background_color(color.seed().color())
        .border(palette.border_strong, 1.0)
        .corner_radius(3.0);
    let content = flex_row((
        swatch,
        FlexSpacer::Fixed(6.px()),
        label(color.label()).text_size(12.0).color(palette.text),
    ))
    .gap(0.px())
    .cross_axis_alignment(CrossAxisAlignment::Center);
    let is_selected = color == selected;
    sized_box(
        button(content, move |state: &mut AppState| {
            state.set_theme_color(color)
        })
        .padding(Padding::from_vh(5.0, 8.0))
        .background_color(if is_selected {
            palette.accent_soft
        } else {
            palette.surface
        })
        .active_background_color(palette.accent_soft)
        .border(
            if is_selected {
                palette.accent
            } else {
                palette.border
            },
            1.0,
        )
        .hovered_border_color(palette.border_strong)
        .corner_radius(Layout::RADIUS),
    )
    .height(Layout::TOOL_HEIGHT.px())
    .expand_width()
}

fn transfer_progress_button(
    state: &AppState,
    palette: ThemePalette,
) -> impl WidgetView<AppState> + use<> {
    let Some(transfer) = state.oldest_transfer_for_icon() else {
        // Always reserve the transfer slot. Otherwise starting the first transfer
        // inserts a new control into the navigation row and shifts the whole UI.
        return Either::A(
            sized_box(label(""))
                .width(Layout::NAV_WIDTH.px())
                .height(Layout::TOOL_HEIGHT.px()),
        );
    };
    let raw_fraction = transfer.fraction();
    let fraction = if !transfer.done && raw_fraction == 0.0 {
        0.08
    } else {
        raw_fraction
    };
    let progress_color = if transfer.error.is_some() {
        palette.muted
    } else {
        palette.accent
    };
    let ring = progress_ring(
        fraction,
        palette.border_strong,
        progress_color,
        18.0,
        "File transfers",
    );
    Either::B(
        sized_box(
            button(ring, AppState::toggle_transfer_popup)
                .padding(5.0)
                .background_color(palette.chrome)
                .active_background_color(palette.accent_soft)
                .border(palette.chrome, 1.0)
                .hovered_border_color(palette.border_strong)
                .corner_radius(Layout::RADIUS),
        )
        .width(Layout::NAV_WIDTH.px())
        .height(Layout::TOOL_HEIGHT.px()),
    )
}

fn sort_menu_choice_button(
    text: &'static str,
    selected: bool,
    callback: impl Fn(&mut AppState) + Send + Sync + 'static,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    let background = if selected {
        palette.accent_soft
    } else {
        palette.surface
    };
    let text_color = if selected {
        palette.text
    } else {
        palette.muted
    };
    sized_box(
        button(
            flex_row((
                label(text).text_size(13.0).color(text_color),
                FlexSpacer::Flex(1.0),
            ))
            .gap(0.px()),
            callback,
        )
        .padding(Padding::from_vh(6.0, 9.0))
        .background_color(background)
        .active_background_color(palette.accent_soft)
        .border(Color::TRANSPARENT, 0.0)
        .hovered_border_color(palette.border_strong)
        .corner_radius(Layout::RADIUS),
    )
    .height(34.px())
    .expand_width()
}

fn sort_field_controls(field: SortField, palette: ThemePalette) -> impl WidgetView<AppState> {
    flex_col((
        sort_menu_choice_button(
            "Name",
            field == SortField::Name,
            |state| state.set_sort_field(SortField::Name),
            palette,
        ),
        sort_menu_choice_button(
            "Date modified",
            field == SortField::DateModified,
            |state| state.set_sort_field(SortField::DateModified),
            palette,
        ),
        sort_menu_choice_button(
            "Type",
            field == SortField::Type,
            |state| state.set_sort_field(SortField::Type),
            palette,
        ),
        sort_menu_choice_button(
            "Size",
            field == SortField::Size,
            |state| state.set_sort_field(SortField::Size),
            palette,
        ),
    ))
    .gap(2.px())
    .cross_axis_alignment(CrossAxisAlignment::Fill)
}

fn sort_order_controls(
    direction: SortDirection,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    flex_col((
        sort_menu_choice_button(
            "Ascending",
            direction == SortDirection::Ascending,
            |state| state.set_sort_direction(SortDirection::Ascending),
            palette,
        ),
        sort_menu_choice_button(
            "Descending",
            direction == SortDirection::Descending,
            |state| state.set_sort_direction(SortDirection::Descending),
            palette,
        ),
    ))
    .gap(2.px())
    .cross_axis_alignment(CrossAxisAlignment::Fill)
}

#[cfg(target_os = "android")]
fn menu_item_button(
    kind: LucideIcon,
    text: &'static str,
    disabled: bool,
    callback: impl Fn(&mut AppState) + Send + Sync + 'static,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    let color = if disabled {
        palette.muted
    } else {
        palette.text
    };
    let content = flex_row((
        icon(kind, color, 15.0, text),
        FlexSpacer::Fixed(9.px()),
        label(text).text_size(13.0).color(color),
        FlexSpacer::Flex(1.0),
    ))
    .gap(0.px())
    .cross_axis_alignment(CrossAxisAlignment::Center);
    sized_box(
        button(content, callback)
            .disabled(disabled)
            .padding(Padding::from_vh(6.0, 9.0))
            .background_color(palette.surface)
            .disabled_background_color(palette.surface)
            .active_background_color(palette.accent_soft)
            .border(Color::TRANSPARENT, 0.0)
            .hovered_border_color(palette.border_strong)
            .corner_radius(Layout::RADIUS),
    )
    .height(36.px())
    .expand_width()
}

fn file_menu_top_offset() -> f64 {
    if cfg!(target_os = "android") {
        // Tab + navigation + address/search stack + file action row. The menu stays
        // attached to the right-side command area instead of appearing as a modal.
        224.0
    } else {
        Layout::TAB_HEIGHT + Layout::ADDRESS_HEIGHT + Layout::ACTION_HEIGHT + 4.0
    }
}

pub fn file_more_overlay(state: &AppState) -> impl WidgetView<AppState> + use<> {
    let palette = state.palette();
    #[cfg(target_os = "android")]
    let visible = mobile_action_visibility(state.mobile_primary_action_capacity());
    #[cfg(target_os = "android")]
    let has_overflow = visible.has_overflow();
    #[cfg(not(target_os = "android"))]
    let has_overflow = false;
    if !state.file_more_popup_open() || !has_overflow {
        return Either::A(sized_box(label("")).width(0.px()).height(0.px()));
    }

    #[cfg(target_os = "android")]
    let pin_label = if state.selected_is_pinned() {
        "Unpin"
    } else {
        "Pin"
    };
    #[cfg(target_os = "android")]
    let mut items: Vec<Box<AnyWidgetView<AppState>>> = Vec::new();
    #[cfg(not(target_os = "android"))]
    let items: Vec<Box<AnyWidgetView<AppState>>> = Vec::new();

    #[cfg(target_os = "android")]
    {
        let has_selection = state.has_selection();
        let clipboard_selected = state.can_clipboard_selected();
        let storage_blocked =
            !state.android_storage_access_granted() && !state.is_taildrive_current();
        let read_only = !state.can_mutate_current_location();
        if !visible.new_folder {
            items.push(
                menu_item_button(
                    LucideIcon::FolderPlus,
                    "New folder",
                    storage_blocked || read_only,
                    |state| {
                        state.close_file_more_popup();
                        state.new_folder();
                    },
                    palette,
                )
                .boxed(),
            );
        }
        if !visible.cut {
            items.push(
                menu_item_button(
                    LucideIcon::Scissors,
                    "Cut",
                    read_only || !clipboard_selected,
                    |state| {
                        state.close_file_more_popup();
                        state.cut_selected();
                    },
                    palette,
                )
                .boxed(),
            );
        }
        if !visible.copy {
            items.push(
                menu_item_button(
                    LucideIcon::Copy,
                    "Copy",
                    !clipboard_selected,
                    |state| {
                        state.close_file_more_popup();
                        state.copy_selected();
                    },
                    palette,
                )
                .boxed(),
            );
        }
        if !visible.paste {
            items.push(
                menu_item_button(
                    LucideIcon::ClipboardPaste,
                    "Paste",
                    storage_blocked || !state.can_paste(),
                    |state| {
                        state.close_file_more_popup();
                        state.paste();
                    },
                    palette,
                )
                .boxed(),
            );
        }
        if !visible.delete {
            items.push(
                menu_item_button(
                    LucideIcon::Trash2,
                    "Delete",
                    read_only || !has_selection,
                    |state| {
                        state.close_file_more_popup();
                        state.delete_selected();
                    },
                    palette,
                )
                .boxed(),
            );
        }
        if !visible.rename {
            items.push(
                menu_item_button(
                    LucideIcon::Pencil,
                    "Rename",
                    read_only || !has_selection,
                    |state| {
                        state.close_file_more_popup();
                        state.begin_rename();
                    },
                    palette,
                )
                .boxed(),
            );
        }
    }

    #[cfg(target_os = "android")]
    {
        if !visible.share {
            items.push(
                menu_item_button(
                    LucideIcon::Share2,
                    "Share",
                    !state.has_selection(),
                    |state| {
                        state.close_file_more_popup();
                        state.share_selected();
                    },
                    palette,
                )
                .boxed(),
            );
        }
        if !visible.pin {
            items.push(
                menu_item_button(
                    LucideIcon::Pin,
                    pin_label,
                    !state.has_selection(),
                    |state| {
                        state.close_file_more_popup();
                        state.toggle_pin_selected();
                    },
                    palette,
                )
                .boxed(),
            );
        }
    }

    let menu = sized_box(
        flex_col(items)
            .gap(2.px())
            .cross_axis_alignment(CrossAxisAlignment::Start),
    )
    .padding(6.0)
    .background_color(palette.surface)
    .border(palette.border_strong, 1.0)
    .corner_radius(8.0);
    #[cfg(target_os = "android")]
    let menu = sized_box(menu).width(state.mobile_overlay_width(260.0).px());
    #[cfg(not(target_os = "android"))]
    let menu = sized_box(menu).width(230.px());

    let backdrop = sized_box(
        button(label(""), AppState::close_file_more_popup)
            .background_color(Color::TRANSPARENT)
            .active_background_color(Color::TRANSPARENT)
            .border_color(Color::TRANSPARENT),
    )
    .expand();
    let anchored = sized_box(
        flex_col((FlexSpacer::Fixed(file_menu_top_offset().px()), menu))
            .gap(0.px())
            .cross_axis_alignment(CrossAxisAlignment::End),
    )
    .expand_width()
    .padding(Padding::horizontal(8.0));
    Either::B(sized_box(zstack((backdrop, anchored.alignment(UnitPoint::TOP_RIGHT)))).expand())
}

pub fn sort_overlay(state: &AppState) -> impl WidgetView<AppState> + use<> {
    let palette = state.palette();
    if !state.sort_popup_open() {
        return Either::A(sized_box(label("")).width(0.px()).height(0.px()));
    }
    let field = state.sort_field();
    let direction = state.sort_direction();
    let header = flex_row((
        icon_button(
            LucideIcon::X,
            "Close sort",
            false,
            AppState::close_sort_popup,
            palette,
        ),
        FlexSpacer::Fixed(6.px()),
        label("Sort").text_size(15.0).color(palette.text),
        FlexSpacer::Flex(1.0),
    ))
    .gap(0.px())
    .cross_axis_alignment(CrossAxisAlignment::Center);
    let card = sized_box(
        flex_col((
            header,
            FlexSpacer::Fixed(8.px()),
            sort_field_controls(field, palette),
            FlexSpacer::Fixed(10.px()),
            label("Order").text_size(11.0).color(palette.muted),
            FlexSpacer::Fixed(5.px()),
            sort_order_controls(direction, palette),
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Start),
    )
    .padding(8.0)
    .background_color(palette.surface)
    .border(palette.border_strong, 1.0)
    .corner_radius(8.0);
    #[cfg(target_os = "android")]
    let card = sized_box(card).width(state.mobile_overlay_width(260.0).px());
    #[cfg(not(target_os = "android"))]
    let card = sized_box(card).width(230.px());
    let backdrop = sized_box(
        button(label(""), AppState::close_sort_popup)
            .background_color(Color::TRANSPARENT)
            .active_background_color(Color::TRANSPARENT)
            .border_color(Color::TRANSPARENT),
    )
    .expand();
    let anchored = sized_box(
        flex_col((FlexSpacer::Fixed(file_menu_top_offset().px()), card))
            .gap(0.px())
            .cross_axis_alignment(CrossAxisAlignment::End),
    )
    .expand_width()
    .padding(Padding::horizontal(8.0));
    Either::B(sized_box(zstack((backdrop, anchored.alignment(UnitPoint::TOP_RIGHT)))).expand())
}

pub fn delete_confirmation_overlay(state: &AppState) -> impl WidgetView<AppState> + use<> {
    let palette = state.palette();
    let Some(name) = state.delete_confirmation_name().map(str::to_owned) else {
        return Either::A(sized_box(label("")).width(0.px()).height(0.px()));
    };
    let card = sized_box(
        flex_col((
            label("Delete permanently?")
                .text_size(17.0)
                .color(palette.text),
            FlexSpacer::Fixed(8.px()),
            prose(format!(
                "“{name}” will be permanently deleted. This cannot be undone."
            ))
            .text_size(12.0)
            .text_color(palette.muted),
            FlexSpacer::Fixed(14.px()),
            flex_row((
                toolbar_button(
                    "Cancel",
                    false,
                    AppState::cancel_delete_confirmation,
                    palette,
                ),
                FlexSpacer::Flex(1.0),
                toolbar_button("Delete", false, AppState::confirm_delete_once, palette),
            ))
            .gap(0.px())
            .cross_axis_alignment(CrossAxisAlignment::Center),
            FlexSpacer::Fixed(8.px()),
            toolbar_button(
                "Don't show again today",
                false,
                AppState::confirm_delete_for_today,
                palette,
            ),
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Start),
    )
    .padding(14.0)
    .background_color(palette.surface)
    .border(palette.border_strong, 1.0)
    .corner_radius(10.0);
    #[cfg(target_os = "android")]
    let card = sized_box(card)
        .expand_width()
        .padding(Padding::horizontal(12.0));
    #[cfg(not(target_os = "android"))]
    let card = sized_box(card).width(430.px());
    let backdrop = sized_box(
        button(label(""), AppState::cancel_delete_confirmation)
            .background_color(Color::from_rgba8(0, 0, 0, 72))
            .active_background_color(Color::from_rgba8(0, 0, 0, 82))
            .border_color(Color::TRANSPARENT),
    )
    .expand();
    Either::B(sized_box(zstack((backdrop, card.alignment(UnitPoint::CENTER)))).expand())
}

#[cfg(not(target_os = "android"))]
fn paste_conflict_actions(palette: ThemePalette) -> impl WidgetView<AppState> {
    flex_row((
        toolbar_button("Skip", false, AppState::cancel_paste_conflict, palette),
        FlexSpacer::Fixed(6.px()),
        toolbar_button(
            "Keep both",
            false,
            AppState::keep_both_paste_conflict,
            palette,
        ),
        FlexSpacer::Fixed(6.px()),
        toolbar_button("Replace", false, AppState::replace_paste_conflict, palette),
    ))
    .gap(0.px())
    .cross_axis_alignment(CrossAxisAlignment::Center)
}

#[cfg(target_os = "android")]
fn paste_conflict_actions(palette: ThemePalette) -> impl WidgetView<AppState> {
    flex_col((
        toolbar_button("Replace", false, AppState::replace_paste_conflict, palette),
        FlexSpacer::Fixed(6.px()),
        toolbar_button(
            "Keep both",
            false,
            AppState::keep_both_paste_conflict,
            palette,
        ),
        FlexSpacer::Fixed(6.px()),
        toolbar_button("Skip", false, AppState::cancel_paste_conflict, palette),
    ))
    .gap(0.px())
    .cross_axis_alignment(CrossAxisAlignment::Start)
}

pub fn paste_conflict_overlay(state: &AppState) -> impl WidgetView<AppState> + use<> {
    let palette = state.palette();
    let Some(name) = state.paste_conflict_name().map(str::to_owned) else {
        return Either::A(sized_box(label("")).width(0.px()).height(0.px()));
    };
    let destination = state.paste_conflict_destination().unwrap_or_default();
    let card = sized_box(
        flex_col((
            label("File already exists").text_size(17.0).color(palette.text),
            FlexSpacer::Fixed(8.px()),
            prose(format!("“{name}” already exists in {destination}."))
                .text_size(12.0)
                .text_color(palette.muted),
            FlexSpacer::Fixed(6.px()),
            prose("Choose the same action as Windows Explorer: replace it, keep both files, or skip this paste.")
                .line_break_mode(LineBreaking::WordWrap)
                .text_size(11.0)
                .text_color(palette.muted),
            FlexSpacer::Fixed(14.px()),
            paste_conflict_actions(palette),
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Start),
    )
    .padding(14.0)
    .background_color(palette.surface)
    .border(palette.border_strong, 1.0)
    .corner_radius(10.0);
    #[cfg(target_os = "android")]
    let card = sized_box(card)
        .expand_width()
        .padding(Padding::horizontal(12.0));
    #[cfg(not(target_os = "android"))]
    let card = sized_box(card).width(430.px());
    let backdrop = sized_box(
        button(label(""), AppState::cancel_paste_conflict)
            .background_color(Color::from_rgba8(0, 0, 0, 72))
            .active_background_color(Color::from_rgba8(0, 0, 0, 82))
            .border_color(Color::TRANSPARENT),
    )
    .expand();
    Either::B(sized_box(zstack((backdrop, card.alignment(UnitPoint::CENTER)))).expand())
}

pub fn transfer_popup(state: &AppState) -> impl WidgetView<AppState> + use<> {
    let palette = state.palette();
    if !state.transfer_popup_open() || state.file_transfers().is_empty() {
        return Either::A(sized_box(label("")).width(0.px()).height(0.px()));
    }

    let transfers = state.file_transfers().to_vec();
    let active_count = transfers.iter().filter(|transfer| !transfer.done).count();
    let has_finished = transfers.iter().any(|transfer| transfer.done);
    let rows = transfers
        .into_iter()
        .map(|transfer| transfer_popup_row(transfer, palette))
        .collect::<Vec<_>>();
    let summary = match active_count {
        0 => "No active transfers".to_owned(),
        1 => "1 active".to_owned(),
        count => format!("{count} active"),
    };
    let list_height = (state.file_transfers().len() as f64 * 56.0).clamp(56.0, 336.0);

    let header = sized_box(
        flex_row((
            label("Transfers").text_size(13.0).color(palette.text),
            FlexSpacer::Fixed(6.px()),
            label(summary).text_size(11.0).color(palette.muted),
            FlexSpacer::Flex(1.0),
            toolbar_button(
                "Clear finished",
                !has_finished,
                AppState::clear_finished_transfers,
                palette,
            ),
            FlexSpacer::Fixed(2.px()),
            compact_icon_button(
                LucideIcon::X,
                "Close transfers",
                false,
                AppState::close_transfer_popup,
                palette,
            ),
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Center),
    )
    .expand_width();

    let card = sized_box(
        flex_col((
            header,
            FlexSpacer::Fixed(4.px()),
            sized_box(portal(
                flex_col(rows)
                    .gap(2.px())
                    .cross_axis_alignment(CrossAxisAlignment::Start),
            ))
            .height(list_height.px())
            .expand_width(),
        ))
        .gap(0.px()),
    )
    .padding(8.0)
    .background_color(palette.surface)
    .border(palette.border_strong, 1.0)
    .corner_radius(8.0);
    #[cfg(target_os = "android")]
    let card = {
        let available = state.mobile_overlay_width(380.0);
        let width = (available - 20.0).max(260.0).min(available);
        sized_box(card).width(width.px())
    };
    #[cfg(not(target_os = "android"))]
    let card = sized_box(card).width(384.px());

    let top_offset = Layout::TAB_HEIGHT + 6.0;
    Either::B(
        sized_box(
            flex_col((FlexSpacer::Fixed(top_offset.px()), card))
                .gap(0.px())
                .cross_axis_alignment(CrossAxisAlignment::End),
        )
        .expand_width()
        .padding(Padding {
            left: 8.0,
            right: 8.0,
            top: 0.0,
            bottom: 0.0,
        }),
    )
}

fn compact_icon_button(
    kind: LucideIcon,
    accessible_label: &'static str,
    disabled: bool,
    callback: impl Fn(&mut AppState) + Send + Sync + 'static,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    let color = if disabled {
        palette.muted
    } else {
        palette.text
    };
    sized_box(
        button(icon(kind, color, 15.0, accessible_label), callback)
            .disabled(disabled)
            .padding(4.0)
            .background_color(palette.surface)
            .disabled_background_color(palette.surface)
            .active_background_color(palette.accent_soft)
            .border(Color::TRANSPARENT, 0.0)
            .hovered_border_color(palette.border_strong)
            .corner_radius(Layout::RADIUS),
    )
    .width(34.px())
    .height(34.px())
}

#[cfg(target_os = "android")]
fn transfer_row_controls(
    transfer: &crate::app::FileTransferProgress,
    palette: ThemePalette,
) -> impl WidgetView<AppState> + use<> {
    let id = transfer.transfer_id.clone();
    if transfer.done {
        let retryable = transfer.error.is_some() || transfer.cancelled;
        Either::A(compact_icon_button(
            LucideIcon::RefreshCw,
            "Retry transfer",
            !retryable,
            move |state| state.retry_transfer(&id),
            palette,
        ))
    } else {
        let pause_id = transfer.transfer_id.clone();
        let stop_id = transfer.transfer_id.clone();
        let pause = if transfer.paused {
            Either::A(compact_icon_button(
                LucideIcon::Play,
                "Resume transfer",
                transfer.cancelling,
                move |state| state.resume_transfer(&pause_id),
                palette,
            ))
        } else {
            Either::B(compact_icon_button(
                LucideIcon::Pause,
                "Pause transfer",
                transfer.cancelling,
                move |state| state.pause_transfer(&pause_id),
                palette,
            ))
        };
        Either::B(
            flex_row((
                pause,
                compact_icon_button(
                    LucideIcon::Square,
                    "Stop transfer",
                    transfer.cancelling,
                    move |state| state.cancel_transfer(&stop_id),
                    palette,
                ),
            ))
            .gap(0.px())
            .cross_axis_alignment(CrossAxisAlignment::Center),
        )
    }
}

#[cfg(not(target_os = "android"))]
fn transfer_row_controls(
    _transfer: &crate::app::FileTransferProgress,
    palette: ThemePalette,
) -> impl WidgetView<AppState> + use<> {
    sized_box(label("").color(palette.muted))
        .width(0.px())
        .height(0.px())
}

fn transfer_popup_row(
    transfer: crate::app::FileTransferProgress,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    let raw_fraction = transfer.fraction();
    let fraction = if !transfer.done && raw_fraction == 0.0 {
        0.08
    } else {
        raw_fraction
    };
    let detail = transfer.display_detail_text();
    let progress_color = if transfer.done || transfer.error.is_some() || transfer.cancelled {
        palette.muted
    } else {
        palette.accent
    };
    let controls = transfer_row_controls(&transfer, palette);
    let ring = progress_ring(
        fraction,
        palette.border,
        progress_color,
        16.0,
        "Transfer progress",
    );
    let text = flex_col((
        prose(transfer.label)
            .line_break_mode(LineBreaking::Clip)
            .text_size(12.5)
            .text_color(palette.text),
        FlexSpacer::Fixed(1.px()),
        prose(detail)
            .line_break_mode(LineBreaking::Clip)
            .text_size(10.5)
            .text_color(if transfer.error.is_some() {
                palette.text
            } else {
                palette.muted
            }),
    ))
    .gap(0.px())
    .cross_axis_alignment(CrossAxisAlignment::Start)
    .flex(1.0);

    sized_box(
        flex_row((
            ring,
            FlexSpacer::Fixed(8.px()),
            text,
            FlexSpacer::Fixed(4.px()),
            controls,
        ))
        .gap(0.px())
        .cross_axis_alignment(CrossAxisAlignment::Center),
    )
    .height(52.px())
    .expand_width()
    .padding(Padding::horizontal(6.0))
    .background_color(palette.surface)
    .border(Color::TRANSPARENT, 0.0)
    .corner_radius(4.0)
}

#[cfg(target_os = "android")]
fn file_action_icon_button(
    kind: LucideIcon,
    accessible_label: &'static str,
    disabled: bool,
    callback: impl Fn(&mut AppState) + Send + Sync + 'static,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    let color = if disabled {
        palette.muted
    } else {
        palette.text
    };
    sized_box(
        button(icon(kind, color, 17.0, accessible_label), callback)
            .disabled(disabled)
            .padding(5.0)
            .background_color(palette.chrome)
            .disabled_background_color(palette.chrome)
            .active_background_color(palette.accent_soft)
            .border(palette.chrome, 1.0)
            .hovered_border_color(palette.border_strong)
            .corner_radius(Layout::RADIUS),
    )
    .width(37.px())
    .height(Layout::TOOL_HEIGHT.px())
}

fn icon_button(
    kind: LucideIcon,
    accessible_label: &'static str,
    disabled: bool,
    callback: impl Fn(&mut AppState) + Send + Sync + 'static,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    let color = if disabled {
        palette.muted
    } else {
        palette.text
    };
    sized_box(
        button(icon(kind, color, 17.0, accessible_label), callback)
            .disabled(disabled)
            .padding(5.0)
            .background_color(palette.chrome)
            .disabled_background_color(palette.chrome)
            .active_background_color(palette.accent_soft)
            .border(palette.chrome, 1.0)
            .hovered_border_color(palette.border_strong)
            .corner_radius(Layout::RADIUS),
    )
    .width(Layout::NAV_WIDTH.px())
    .height(Layout::TOOL_HEIGHT.px())
}

fn icon_text_button(
    kind: LucideIcon,
    text: &'static str,
    disabled: bool,
    callback: impl Fn(&mut AppState) + Send + Sync + 'static,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    let color = if disabled {
        palette.muted
    } else {
        palette.text
    };
    let content = flex_row((
        icon(kind, color, 15.0, text),
        FlexSpacer::Fixed(5.px()),
        label(text).text_size(13.0).color(color),
    ))
    .gap(0.px())
    .cross_axis_alignment(CrossAxisAlignment::Center);
    sized_box(
        button(content, callback)
            .disabled(disabled)
            .padding(Padding::from_vh(5.0, 9.0))
            .background_color(palette.chrome)
            .disabled_background_color(palette.chrome)
            .active_background_color(palette.accent_soft)
            .border(palette.chrome, 1.0)
            .hovered_border_color(palette.border_strong)
            .corner_radius(Layout::RADIUS),
    )
    .height(Layout::TOOL_HEIGHT.px())
}

fn field_icon_button(
    kind: LucideIcon,
    accessible_label: &'static str,
    disabled: bool,
    callback: impl Fn(&mut AppState) + Send + Sync + 'static,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    let color = if disabled {
        palette.muted
    } else {
        palette.text
    };
    sized_box(
        button(icon(kind, color, 15.0, accessible_label), callback)
            .disabled(disabled)
            .padding(Padding::from_vh(4.0, 8.0))
            .background_color(palette.surface)
            .disabled_background_color(palette.surface)
            .active_background_color(palette.accent_soft)
            .border(palette.surface, 0.0)
            .hovered_border_color(palette.border_strong)
            .corner_radius(Layout::RADIUS),
    )
    .height(Layout::TOOL_HEIGHT.px())
}

fn accordion_button(
    text: String,
    expanded: bool,
    callback: impl Fn(&mut AppState) + Send + Sync + 'static,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    let icon_kind = if expanded {
        LucideIcon::Minus
    } else {
        LucideIcon::Plus
    };
    let content = flex_row((
        icon(icon_kind, palette.muted, 13.0, "Toggle offline devices"),
        FlexSpacer::Fixed(5.px()),
        label(text).text_size(12.0).color(palette.muted),
        FlexSpacer::Flex(1.0),
    ))
    .gap(0.px())
    .cross_axis_alignment(CrossAxisAlignment::Center);
    button(content, callback)
        .padding(Padding::from_vh(5.0, 7.0))
        .background_color(palette.surface)
        .active_background_color(palette.accent_soft)
        .border(palette.surface, 0.0)
        .hovered_border_color(palette.border_strong)
        .corner_radius(Layout::RADIUS)
}

fn toolbar_button(
    text: impl Into<String>,
    disabled: bool,
    callback: impl Fn(&mut AppState) + Send + Sync + 'static,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    let text = text.into();
    let compact = text.chars().count() == 1;
    let text_color = if disabled {
        palette.muted
    } else {
        palette.text
    };
    let button_padding = if cfg!(target_os = "android") {
        Padding::from_vh(7.0, 5.0)
    } else {
        Padding::from_vh(5.0, 9.0)
    };
    let button = button(label(text).text_size(13.0).color(text_color), callback)
        .disabled(disabled)
        .padding(button_padding)
        .background_color(palette.chrome)
        .disabled_background_color(palette.chrome)
        .active_background_color(palette.accent_soft)
        .border(palette.chrome, 1.0)
        .hovered_border_color(palette.border_strong)
        .corner_radius(Layout::RADIUS);

    let box_view = sized_box(button).height(Layout::TOOL_HEIGHT.px());
    if compact {
        box_view.width(Layout::NAV_WIDTH.px())
    } else {
        box_view
    }
}

fn fill_choice_button(
    text: &'static str,
    selected: bool,
    callback: impl Fn(&mut AppState) + Send + Sync + 'static,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    sized_box(choice_button(text, selected, callback, palette)).expand_width()
}

fn fill_choice_button_disabled(
    text: &'static str,
    selected: bool,
    disabled: bool,
    callback: impl Fn(&mut AppState) + Send + Sync + 'static,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    sized_box(choice_button_disabled(
        text, selected, disabled, callback, palette,
    ))
    .expand_width()
}

fn choice_button(
    text: &'static str,
    selected: bool,
    callback: impl Fn(&mut AppState) + Send + Sync + 'static,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    choice_button_disabled(text, selected, false, callback, palette)
}

fn choice_button_disabled(
    text: &'static str,
    selected: bool,
    disabled: bool,
    callback: impl Fn(&mut AppState) + Send + Sync + 'static,
    palette: ThemePalette,
) -> impl WidgetView<AppState> {
    let active = selected && !disabled;
    let background = if active {
        palette.accent
    } else {
        palette.surface
    };
    let text_color = if disabled {
        palette.muted
    } else if active {
        palette.accent_text
    } else {
        palette.text
    };
    sized_box(
        button(label(text).text_size(12.0).color(text_color), callback)
            .disabled(disabled)
            .padding(Padding::from_vh(5.0, 10.0))
            .background_color(background)
            .disabled_background_color(palette.surface)
            .active_background_color(if active {
                palette.accent_pressed
            } else {
                palette.accent_soft
            })
            .border(
                if active {
                    palette.accent
                } else {
                    palette.border
                },
                1.0,
            )
            .hovered_border_color(if active {
                palette.accent_hover
            } else {
                palette.border_strong
            })
            .corner_radius(Layout::RADIUS),
    )
    .height(Layout::TOOL_HEIGHT.px())
}

fn column_row<A, B, C, D>(icon: A, name: B, kind: C, size: D) -> impl WidgetView<AppState>
where
    A: WidgetView<AppState>,
    B: WidgetView<AppState>,
    C: WidgetView<AppState>,
    D: WidgetView<AppState>,
{
    flex_row((
        sized_box(icon).width(Layout::ICON_WIDTH.px()),
        sized_box(name).width((Layout::NAME_WIDTH - 12.0).px()),
        FlexSpacer::Fixed(12.px()),
        sized_box(kind).width(Layout::TYPE_WIDTH.px()),
        sized_box(size).width(Layout::SIZE_WIDTH.px()),
        FlexSpacer::Flex(1.0),
    ))
    .gap(0.px())
    .cross_axis_alignment(CrossAxisAlignment::Center)
}
