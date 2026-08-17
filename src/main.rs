#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

#[cfg(target_os = "android")]
mod android_platform;
#[cfg(target_os = "android")]
mod android_transfer;
mod app;
mod archive;
mod cli;
mod ipc;
mod search;
mod settings;
mod supabase_sync;
mod system_clipboard;
mod tailscale;
mod theme;
mod thumbnails;
mod ui;
#[cfg(target_os = "windows")]
mod windows_icons;
#[cfg(target_os = "windows")]
mod windows_integration;

use app::AppState;
use cli::CliOptions;
use ui::root_view;
use xilem::{EventLoop, EventLoopBuilder, WindowOptions, Xilem};

#[cfg(not(target_os = "android"))]
fn window_options() -> WindowOptions<AppState> {
    WindowOptions::new("FastExplorer")
        .with_decorations(false)
        .with_initial_inner_size(xilem::dpi::LogicalSize::new(1180.0, 760.0))
        .with_min_inner_size(xilem::dpi::LogicalSize::new(760.0, 480.0))
        .on_close(|state: &mut AppState| state.shutdown())
}

#[cfg(target_os = "android")]
fn window_options() -> WindowOptions<AppState> {
    WindowOptions::new("FastExplorer").on_close(|state: &mut AppState| state.shutdown())
}

fn run_with_state(
    event_loop: EventLoopBuilder,
    options: CliOptions,
    state: AppState,
) -> Result<(), Box<dyn std::error::Error>> {
    let ipc_socket = options.ipc_enabled.then(|| {
        options
            .ipc_socket
            .clone()
            .unwrap_or_else(ipc::default_socket_path)
    });
    let window = window_options();
    let app = Xilem::new_simple(
        state,
        move |state| root_view(state, ipc_socket.clone()),
        window,
    );
    #[cfg(target_os = "android")]
    let app = if let Some(font) = android_platform::system_cjk_font() {
        app.with_font(font)
    } else {
        app
    };
    app.run_in(event_loop)?;
    Ok(())
}

#[expect(
    clippy::allow_attributes,
    reason = "shared desktop/Android entry source"
)]
#[allow(dead_code, reason = "used by the desktop binary target")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options =
        CliOptions::parse().map_err(|error| format!("{error}\n\n{}", CliOptions::HELP))?;
    if options.show_help {
        print!("{}", CliOptions::HELP);
        return Ok(());
    }
    tailscale::configure_state_dir(tailscale::desktop_state_dir());
    tailscale::configure_share_root(app::home_dir().unwrap_or_else(std::env::temp_dir));
    let mut state = AppState::new(options.theme_overrides, options.search_override);
    if let Some(path) = options.startup_path.clone() {
        state.navigate_to(path);
    }
    run_with_state(EventLoop::with_user_event(), options, state)
}

#[cfg(target_os = "android")]
#[expect(unsafe_code, reason = "required Android NativeActivity entry symbol")]
#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    use winit::platform::android::EventLoopBuilderExtAndroid;

    let home = android_platform::initialize(&app);
    tailscale::configure_share_root(home.clone());
    app::set_android_home(home);
    let private = app
        .internal_data_path()
        .expect("FastExplorer Android internal data directory is unavailable");
    app::set_android_state_dir(private.join("state"));
    tailscale::configure_state_dir(private.join("tailscale"));

    let options = CliOptions::default();
    let mut state = AppState::new(options.theme_overrides, options.search_override);
    state.attach_android_app(app.clone());

    let mut event_loop = EventLoop::with_user_event();
    event_loop.with_android_app(app);
    run_with_state(event_loop, options, state).expect("FastExplorer Android startup failed");
}
