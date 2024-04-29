use std::ffi::c_void;

use windows::{core::Interface, Win32::{Foundation::HWND, UI::WindowsAndMessaging::GetWindowLongPtrW}, UI::WindowManagement::{AppWindow, AppWindowTitleBar}};
use winit::{event_loop::ActiveEventLoop, window::{Window, WindowButtons}};

pub struct WindowWrapper {
    pub window: Window,
}

impl WindowWrapper {
    pub fn new(event_loop: &ActiveEventLoop) -> Self {
        let mut window = Window::default_attributes();
        window.title = "FastExplorer".into();
        // let window = window.with_active(false);

        let window = event_loop
                    .create_window(window)
                    .unwrap();

        window.focus_window();

        let hwnd: u64 = window.id().into();
        let hwnd: HWND = HWND(hwnd as isize);


        Self { window }
    }
}

fn get_app_window_from_hwnd(hwnd: HWND) -> Option<*mut u8> {
    let userdata_ptr = unsafe {
        GetWindowLongPtrW(hwnd, windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA)
    };
    if userdata_ptr != 0 {
        Some(userdata_ptr as *mut u8)
    } else {
        None
    }
}

// Win32_UI_WindowsAndMessaging
// GetWindowLongPtrW

// UI_WindowManagement_Preview
// AppWindowTitleBar

// Win32_Foundation
