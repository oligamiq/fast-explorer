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
        // let window = window.with_enabled_buttons(WindowButtons::empty());

        // window;


        let window = event_loop
                    .create_window(window)
                    .unwrap();

        window.focus_window();

        let hwnd: u64 = window.id().into();
        let title = window.title();

        let hwnd_ptr = hwnd as *mut c_void;
        let app_window = unsafe { windows::UI::WindowManagement::AppWindow::from_raw_borrowed(&hwnd_ptr) };

        // println!("title: {:?}", app_window.unwrap().Title());

        let hwnd: HWND = HWND(hwnd as isize);

        // println!("app_window: {:?}", app_window);

        // let title = app_window.Title();
        // println!("{:?}", title);

        let app_window_ptr = unsafe {
            GetWindowLongPtrW(hwnd, windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA)
        };
        // AppWindow::
        // let app_window_ptr = app_window_ptr as *mut c_void;
        // let app_window2 = unsafe { windows::UI::WindowManagement::AppWindow::from_raw_borrowed(&app_window_ptr) };
        // let title = app_window2.unwrap().Title();

        // println!("title: {:?}", title);

        // let userdata_ptr = get_app_window_from_hwnd(hwnd).unwrap();

        dbg!(hwnd);

        // unsafe {
            // AppWindowTitleBar::query(&self, iid, interface)
        // }

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
