mod components;
mod file_row;
mod file_shortcuts;
mod font;
mod icons;
mod tab_drag;
mod window_chrome;

use std::path::PathBuf;

use xilem::WidgetView;
use xilem::core::{fork, one_of::Either};
#[cfg(not(target_os = "android"))]
use xilem::masonry::core::ResizeDirection;
use xilem::masonry::properties::types::{AsUnit, UnitPoint};
#[cfg(target_os = "android")]
use xilem::style::Padding;
use xilem::style::Style as _;
#[cfg(not(target_os = "android"))]
use xilem::view::split;
#[cfg(target_os = "android")]
use xilem::view::worker;
use xilem::view::{FlexExt as _, ZStackExt as _, flex_col, sized_box, zstack};

use crate::app::{AppPage, AppState};
#[cfg(not(target_os = "android"))]
use crate::theme::Layout;
use components::{
    address_bar, delete_confirmation_overlay, file_action_bar, file_area, file_more_overlay,
    paste_conflict_overlay, restore_warning_banner, settings_page, sort_overlay, tab_bar,
    transfer_popup,
};
#[cfg(not(target_os = "android"))]
use components::{sidebar, status_bar};
use file_shortcuts::browser_shortcuts;
#[cfg(not(target_os = "android"))]
use window_chrome::resize_region;

pub fn root_view(
    state: &mut AppState,
    ipc_socket: Option<PathBuf>,
) -> impl WidgetView<AppState> + use<> {
    font::set_current(state.ui_font());
    let palette = state.palette();

    #[cfg(not(target_os = "android"))]
    let body = split(sidebar(state), file_area(state))
        .split_point(Layout::SIDEBAR_FRACTION)
        .min_size(Layout::SIDEBAR_MIN.px(), Layout::CONTENT_MIN.px())
        .bar_size(1.px())
        .min_bar_area(3.px())
        .solid_bar(true)
        .draggable(true);
    #[cfg(target_os = "android")]
    let body = file_area(state);

    #[cfg(not(target_os = "android"))]
    let browser_content = sized_box(
        flex_col((
            address_bar(state),
            file_action_bar(state),
            restore_warning_banner(state),
            body.flex(1.0),
            status_bar(state),
        ))
        .gap(0.px()),
    )
    .expand()
    .background_color(palette.window);

    #[cfg(target_os = "android")]
    let browser_content = sized_box(
        flex_col((
            address_bar(state),
            file_action_bar(state),
            restore_warning_banner(state),
            body.flex(1.0),
        ))
        .gap(0.px()),
    )
    .expand()
    .background_color(palette.window);
    let browser = browser_shortcuts(browser_content, state.rename_active());

    let page = match state.page() {
        AppPage::Files => Either::A(browser),
        AppPage::Settings => Either::B(settings_page(state)),
    };

    let app_content = sized_box(flex_col((tab_bar(state), sized_box(page).flex(1.0))).gap(0.px()))
        .expand()
        .background_color(palette.window);
    let content = zstack((
        app_content,
        transfer_popup(state).alignment(UnitPoint::TOP_RIGHT),
        sort_overlay(state).alignment(UnitPoint::TOP_RIGHT),
        file_more_overlay(state).alignment(UnitPoint::TOP_RIGHT),
        paste_conflict_overlay(state).alignment(UnitPoint::CENTER),
        delete_confirmation_overlay(state).alignment(UnitPoint::CENTER),
    ));

    #[cfg(not(target_os = "android"))]
    let window_content = zstack((
        content,
        resize_region(ResizeDirection::North).alignment(UnitPoint::TOP),
        resize_region(ResizeDirection::South).alignment(UnitPoint::BOTTOM),
        resize_region(ResizeDirection::West).alignment(UnitPoint::LEFT),
        resize_region(ResizeDirection::East).alignment(UnitPoint::RIGHT),
        resize_region(ResizeDirection::NorthWest).alignment(UnitPoint::TOP_LEFT),
        resize_region(ResizeDirection::NorthEast).alignment(UnitPoint::TOP_RIGHT),
        resize_region(ResizeDirection::SouthWest).alignment(UnitPoint::BOTTOM_LEFT),
        resize_region(ResizeDirection::SouthEast).alignment(UnitPoint::BOTTOM_RIGHT),
    ));
    #[cfg(target_os = "android")]
    let window_content = {
        let insets = state.android_insets();
        // File content intentionally extends behind Android's navigation/gesture
        // region. The virtual file list adds an equivalent scrollable end spacer,
        // so every tappable row can still be moved fully above that untappable area.
        // Settings remains fully inset because its controls are not part of that list.
        let bottom_inset = match state.page() {
            AppPage::Files => 0.0,
            AppPage::Settings => insets.bottom,
        };
        sized_box(content)
            .expand()
            .padding(Padding {
                left: insets.left,
                right: insets.right,
                top: insets.top,
                bottom: bottom_inset,
            })
            .background_color(palette.window)
    };

    #[cfg(not(target_os = "android"))]
    return fork(
        window_content,
        (
            crate::ipc::control_task(ipc_socket),
            crate::tailscale::network_task(),
            xilem::view::worker(
                |proxy, mut rx: xilem::tokio::sync::mpsc::UnboundedReceiver<()>| async move {
                    if rx.recv().await.is_some() {
                        let _ = xilem::tokio::task::spawn_blocking(
                            crate::app::preload_taildrive_directory_cache,
                        )
                        .await;
                        let _ = proxy.message(());
                    }
                },
                |_state: &mut AppState, sender| {
                    let _ = sender.send(());
                },
                |state: &mut AppState, ()| state.taildrive_directory_cache_ready(),
            ),
            xilem::view::worker(
                |_proxy, mut rx: xilem::tokio::sync::mpsc::UnboundedReceiver<crate::app::PersistCommand>| async move {
                    while let Some(command) = rx.recv().await {
                        let mut pending = std::collections::BTreeMap::new();
                        pending.insert(command.path.clone(), command);
                        while let Ok(newer) = rx.try_recv() {
                            pending.insert(newer.path.clone(), newer);
                        }
                        for command in pending.into_values() {
                            if let Err(error) = xilem::tokio::task::spawn_blocking(move || {
                                crate::app::perform_persist_command(&command)
                            })
                            .await
                            .unwrap_or_else(|error| Err(format!("Persistence worker failed: {error}")))
                            {
                                eprintln!("FastExplorer: persistence failed: {error}");
                            }
                        }
                    }
                },
                |state: &mut AppState, sender| state.set_persistence_sender(sender),
                |_state: &mut AppState, ()| {},
            ),
            xilem::view::worker(
                |proxy, mut rx: xilem::tokio::sync::mpsc::UnboundedReceiver<crate::app::RemotePrepareRequest>| async move {
                    while let Some(request) = rx.recv().await {
                        let worker_request = request.clone();
                        let result = xilem::tokio::task::spawn_blocking(move || {
                            crate::app::perform_remote_prepare(&worker_request)
                        })
                        .await
                        .unwrap_or_else(|error| Err(format!("Remote file worker failed: {error}")));
                        let _ = proxy.message(crate::app::RemotePrepareEvent { request, result });
                    }
                },
                |state: &mut AppState, sender| state.set_remote_prepare_sender(sender),
                |state: &mut AppState, event| state.apply_remote_prepare_event(event),
            ),
            xilem::view::worker(
                |proxy, mut rx: xilem::tokio::sync::mpsc::UnboundedReceiver<crate::app::CacheCommand>| async move {
                    while let Some(command) = rx.recv().await {
                        let usage_refresh_id = match &command {
                            crate::app::CacheCommand::Maintain {
                                usage_refresh_id, ..
                            }
                            | crate::app::CacheCommand::Clear {
                                usage_refresh_id, ..
                            } => Some(*usage_refresh_id),
                            _ => None,
                        };
                        let worker_command = command.clone();
                        let result = xilem::tokio::task::spawn_blocking(move || {
                            crate::app::perform_cache_command(&worker_command)
                        })
                        .await
                        .unwrap_or_else(|error| Err(format!("Cache worker failed: {error}")));
                        let _ = proxy.message(crate::app::CacheEvent {
                            result,
                            usage_refresh_id,
                        });
                    }
                },
                |state: &mut AppState, sender| state.set_cache_sender(sender),
                |state: &mut AppState, event| state.apply_cache_event(event),
            ),
            xilem::view::worker(
                |proxy, mut rx: xilem::tokio::sync::mpsc::UnboundedReceiver<crate::app::LocalFileCommand>| async move {
                    while let Some(command) = rx.recv().await {
                        let worker_command = command.clone();
                        let result = xilem::tokio::task::spawn_blocking(move || {
                            crate::app::perform_local_file_command(&worker_command)
                        })
                        .await
                        .unwrap_or_else(|error| Err(format!("File operation worker failed: {error}")));
                        let _ = proxy.message(crate::app::LocalFileEvent { command, result });
                    }
                },
                |state: &mut AppState, sender| state.set_local_file_sender(sender),
                |state: &mut AppState, event| state.apply_local_file_event(event),
            ),
            xilem::view::worker(
                |proxy, rx: xilem::tokio::sync::mpsc::UnboundedReceiver<crate::archive::Command>| async move {
                    crate::archive::run_worker(proxy, rx).await;
                },
                |state: &mut AppState, sender| state.set_archive_sender(sender),
                |state: &mut AppState, event| state.apply_archive_event(event),
            ),
            xilem::view::worker(
                |proxy,
                 mut rx: xilem::tokio::sync::mpsc::UnboundedReceiver<crate::app::DirectoryRequest>| async move {
                    while let Some(request) = rx.recv().await {
                        let mut request = request;
                        while let Ok(newer) = rx.try_recv() {
                            request = newer;
                        }
                        let (generation, dir, show_hidden) = request;
                        let event_dir = dir.clone();
                        let result = xilem::tokio::task::spawn_blocking(move || {
                            crate::app::scan_directory(&dir, show_hidden)
                        })
                        .await
                        .unwrap_or_else(|error| Err(format!("Folder worker failed: {error}")));
                        let _ = proxy.message((generation, event_dir, result));
                    }
                },
                |state: &mut AppState, sender| state.set_directory_sender(sender),
                |state: &mut AppState, (generation, dir, result)| {
                    state.apply_directory_result(generation, dir, result);
                },
            ),
            xilem::view::worker(
                |proxy,
                 mut rx: xilem::tokio::sync::mpsc::UnboundedReceiver<crate::app::ThumbnailRequest>| async move {
                    while let Some(path) = rx.recv().await {
                        let event_path = path.clone();
                        let result = xilem::tokio::task::spawn_blocking(move || {
                            crate::thumbnails::load(&path)
                        })
                        .await
                        .unwrap_or_else(|error| Err(format!("Thumbnail worker failed: {error}")));
                        let _ = proxy.message((event_path, result));
                    }
                },
                |state: &mut AppState, sender| state.set_thumbnail_sender(sender),
                |state: &mut AppState, (path, result)| {
                    state.apply_thumbnail_result(path, result);
                },
            ),
            xilem::view::worker(
                |proxy,
                 mut rx: xilem::tokio::sync::mpsc::UnboundedReceiver<crate::app::SearchRequest>| async move {
                    while let Some(request) = rx.recv().await {
                        let mut request = request;
                        while let Ok(Some(newer)) = xilem::tokio::time::timeout(
                            std::time::Duration::from_millis(180),
                            rx.recv(),
                        )
                        .await
                        {
                            request = newer;
                        }
                        let (generation, dir, query, mode, show_hidden) = request;
                        let event_dir = dir.clone();
                        let event_query = query.clone();
                        let results = xilem::tokio::task::spawn_blocking(move || {
                            crate::search::search(mode, &dir, &query, show_hidden)
                        })
                        .await
                        .unwrap_or_else(|error| Err(format!("Search worker failed: {error}")));
                        let _ = proxy.message((generation, event_dir, event_query, results));
                    }
                },
                |state: &mut AppState, sender| {
                    state.set_search_sender(sender);
                },
                |state: &mut AppState, (generation, dir, query, results)| {
                    if !state.accepts_search_result(generation, &dir, &query) {
                        return;
                    }
                    let tab = state.active_tab_mut();
                    match results {
                        Ok(entries) => {
                            tab.entries = entries;
                            tab.apply_sort();
                            tab.status = format!("Found {} items", tab.entries.len());
                            if !tab.entries.is_empty() {
                                tab.selected_path = Some(tab.entries[0].path.clone());
                            }
                        }
                        Err(error) => {
                            tab.status = error;
                        }
                    }
                },
            ),
        ),
    );

    #[cfg(target_os = "android")]
    fork(
        window_content,
        (
            crate::ipc::control_task(ipc_socket),
            worker(
                |proxy, _rx: xilem::tokio::sync::mpsc::UnboundedReceiver<()>| async move {
                    let mut last_transfer_revision = 0;
                    loop {
                        xilem::tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        if !crate::android_platform::is_activity_resumed() {
                            continue;
                        }
                        let transfer_revision = crate::android_transfer::ui_revision();
                        let back_pending = crate::android_platform::has_back_request();
                        if !back_pending && transfer_revision == last_transfer_revision {
                            continue;
                        }
                        last_transfer_revision = transfer_revision;
                        if proxy.message(()).is_err() {
                            break;
                        }
                    }
                },
                |_state: &mut AppState, _sender| {},
                |state: &mut AppState, ()| {
                    state.poll_android_back();
                    state.poll_android_transfers();
                },
            ),
            worker(
                |proxy, _rx: xilem::tokio::sync::mpsc::UnboundedReceiver<()>| async move {
                    loop {
                        xilem::tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        if !crate::android_platform::is_activity_resumed() {
                            continue;
                        }
                        if proxy.message(()).is_err() {
                            break;
                        }
                    }
                },
                |_state: &mut AppState, _sender| {},
                |state: &mut AppState, ()| state.poll_android_platform_state(),
            ),
            crate::tailscale::network_task(),
            worker(
                |proxy, mut rx: xilem::tokio::sync::mpsc::UnboundedReceiver<()>| async move {
                    if rx.recv().await.is_some() {
                        let _ = xilem::tokio::task::spawn_blocking(
                            crate::app::preload_taildrive_directory_cache,
                        )
                        .await;
                        let _ = proxy.message(());
                    }
                },
                |_state: &mut AppState, sender| {
                    let _ = sender.send(());
                },
                |state: &mut AppState, ()| state.taildrive_directory_cache_ready(),
            ),
            worker(
                |_proxy, mut rx: xilem::tokio::sync::mpsc::UnboundedReceiver<crate::app::PersistCommand>| async move {
                    while let Some(command) = rx.recv().await {
                        let mut pending = std::collections::BTreeMap::new();
                        pending.insert(command.path.clone(), command);
                        while let Ok(newer) = rx.try_recv() {
                            pending.insert(newer.path.clone(), newer);
                        }
                        for command in pending.into_values() {
                            if let Err(error) = xilem::tokio::task::spawn_blocking(move || {
                                crate::app::perform_persist_command(&command)
                            })
                            .await
                            .unwrap_or_else(|error| Err(format!("Persistence worker failed: {error}")))
                            {
                                eprintln!("FastExplorer: persistence failed: {error}");
                            }
                        }
                    }
                },
                |state: &mut AppState, sender| state.set_persistence_sender(sender),
                |_state: &mut AppState, ()| {},
            ),
            worker(
                |proxy, mut rx: xilem::tokio::sync::mpsc::UnboundedReceiver<crate::app::RemotePrepareRequest>| async move {
                    while let Some(request) = rx.recv().await {
                        let worker_request = request.clone();
                        let result = xilem::tokio::task::spawn_blocking(move || {
                            crate::app::perform_remote_prepare(&worker_request)
                        })
                        .await
                        .unwrap_or_else(|error| Err(format!("Remote file worker failed: {error}")));
                        let _ = proxy.message(crate::app::RemotePrepareEvent { request, result });
                    }
                },
                |state: &mut AppState, sender| state.set_remote_prepare_sender(sender),
                |state: &mut AppState, event| state.apply_remote_prepare_event(event),
            ),
            worker(
                |proxy, mut rx: xilem::tokio::sync::mpsc::UnboundedReceiver<crate::app::CacheCommand>| async move {
                    while let Some(command) = rx.recv().await {
                        let usage_refresh_id = match &command {
                            crate::app::CacheCommand::Maintain {
                                usage_refresh_id, ..
                            }
                            | crate::app::CacheCommand::Clear {
                                usage_refresh_id, ..
                            } => Some(*usage_refresh_id),
                            _ => None,
                        };
                        let worker_command = command.clone();
                        let result = xilem::tokio::task::spawn_blocking(move || {
                            crate::app::perform_cache_command(&worker_command)
                        })
                        .await
                        .unwrap_or_else(|error| Err(format!("Cache worker failed: {error}")));
                        let _ = proxy.message(crate::app::CacheEvent {
                            result,
                            usage_refresh_id,
                        });
                    }
                },
                |state: &mut AppState, sender| state.set_cache_sender(sender),
                |state: &mut AppState, event| state.apply_cache_event(event),
            ),
            worker(
                |proxy, mut rx: xilem::tokio::sync::mpsc::UnboundedReceiver<crate::app::LocalFileCommand>| async move {
                    while let Some(command) = rx.recv().await {
                        if matches!(command, crate::app::LocalFileCommand::CopyMove { .. }) {
                            crate::android_transfer::submit_local(command);
                            continue;
                        }
                        let worker_command = command.clone();
                        let result = xilem::tokio::task::spawn_blocking(move || {
                            crate::app::perform_local_file_command(&worker_command)
                        })
                        .await
                        .unwrap_or_else(|error| Err(format!("File operation worker failed: {error}")));
                        let _ = proxy.message(crate::app::LocalFileEvent { command, result });
                    }
                },
                |state: &mut AppState, sender| state.set_local_file_sender(sender),
                |state: &mut AppState, event| state.apply_local_file_event(event),
            ),
            worker(
                |proxy, rx: xilem::tokio::sync::mpsc::UnboundedReceiver<crate::archive::Command>| async move {
                    crate::archive::run_worker(proxy, rx).await;
                },
                |state: &mut AppState, sender| state.set_archive_sender(sender),
                |state: &mut AppState, event| state.apply_archive_event(event),
            ),
            worker(
                |proxy,
                 mut rx: xilem::tokio::sync::mpsc::UnboundedReceiver<crate::app::DirectoryRequest>| async move {
                    while let Some(request) = rx.recv().await {
                        let mut request = request;
                        while let Ok(newer) = rx.try_recv() {
                            request = newer;
                        }
                        let (generation, dir, show_hidden) = request;
                        let event_dir = dir.clone();
                        let result = xilem::tokio::task::spawn_blocking(move || {
                            crate::app::scan_directory(&dir, show_hidden)
                        })
                        .await
                        .unwrap_or_else(|error| Err(format!("Folder worker failed: {error}")));
                        let _ = proxy.message((generation, event_dir, result));
                    }
                },
                |state: &mut AppState, sender| state.set_directory_sender(sender),
                |state: &mut AppState, (generation, dir, result)| {
                    state.apply_directory_result(generation, dir, result);
                },
            ),
            worker(
                |proxy,
                 mut rx: xilem::tokio::sync::mpsc::UnboundedReceiver<crate::app::ThumbnailRequest>| async move {
                    while let Some(path) = rx.recv().await {
                        let event_path = path.clone();
                        let result = xilem::tokio::task::spawn_blocking(move || {
                            crate::thumbnails::load(&path)
                        })
                        .await
                        .unwrap_or_else(|error| Err(format!("Thumbnail worker failed: {error}")));
                        let _ = proxy.message((event_path, result));
                    }
                },
                |state: &mut AppState, sender| state.set_thumbnail_sender(sender),
                |state: &mut AppState, (path, result)| {
                    state.apply_thumbnail_result(path, result);
                },
            ),
            worker(
                |proxy,
                 mut rx: xilem::tokio::sync::mpsc::UnboundedReceiver<crate::app::SearchRequest>| async move {
                    while let Some(request) = rx.recv().await {
                        let mut request = request;
                        while let Ok(Some(newer)) = xilem::tokio::time::timeout(
                            std::time::Duration::from_millis(180),
                            rx.recv(),
                        )
                        .await
                        {
                            request = newer;
                        }
                        let (generation, dir, query, mode, show_hidden) = request;
                        let event_dir = dir.clone();
                        let event_query = query.clone();
                        let results = xilem::tokio::task::spawn_blocking(move || {
                            crate::search::search(mode, &dir, &query, show_hidden)
                        })
                        .await
                        .unwrap_or_else(|error| Err(format!("Search worker failed: {error}")));
                        let _ = proxy.message((generation, event_dir, event_query, results));
                    }
                },
                |state: &mut AppState, sender| {
                    state.set_search_sender(sender);
                },
                |state: &mut AppState, (generation, dir, query, results)| {
                    if !state.accepts_search_result(generation, &dir, &query) {
                        return;
                    }
                    let tab = state.active_tab_mut();
                    match results {
                        Ok(entries) => {
                            tab.entries = entries;
                            tab.apply_sort();
                            tab.status = format!("Found {} items", tab.entries.len());
                            if !tab.entries.is_empty() {
                                tab.selected_path = Some(tab.entries[0].path.clone());
                            }
                        }
                        Err(error) => {
                            tab.status = error;
                        }
                    }
                },
            ),
        ),
    )
}
