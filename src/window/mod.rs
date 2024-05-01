mod window;
use std::mem;

use winapi::shared::windowsx::{GET_X_LPARAM, GET_Y_LPARAM};
pub use window::WindowWrapper;
use windows::Win32::UI::WindowsAndMessaging::{HTCLOSE, SC_MOVE, WM_CREATE};
use windows_sys::{
    core::HRESULT,
    Win32::{
        Foundation::{FALSE, HWND, LPARAM, LRESULT, RECT, S_OK, WPARAM},
        Graphics::{
            Dwm::{
                DwmDefWindowProc, DwmExtendFrameIntoClientArea, DwmGetWindowAttribute,
                DwmSetWindowAttribute, DWMWA_CAPTION_BUTTON_BOUNDS, DWMWCP_DONOTROUND,
                DWM_WINDOW_CORNER_PREFERENCE,
            },
            Gdi::{
                GetMonitorInfoW, MonitorFromRect, HMONITOR, MONITORINFO, MONITORINFOEXW,
                MONITOR_DEFAULTTONULL,
            },
        },
        UI::{
            Controls::MARGINS,
            Shell::DefSubclassProc,
            WindowsAndMessaging::{
                AdjustWindowRectEx, GetSystemMenu, GetWindowRect, IsZoomed, TrackPopupMenu,
                TrackPopupMenuEx, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTGROWBOX,
                HTLEFT, HTMAXBUTTON, HTNOWHERE, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, SC_CLOSE,
                SC_MAXIMIZE, SC_MINIMIZE, SC_RESTORE, SC_SIZE, TPM_LEFTALIGN, TPM_RETURNCMD,
                WM_CONTEXTMENU, WM_LBUTTONUP, WM_NCCALCSIZE, WM_NCHITTEST, WM_NCLBUTTONDBLCLK,
                WM_NCRBUTTONUP, WM_PAINT, WM_RBUTTONUP, WM_SYSCOMMAND, WS_CAPTION,
                WS_OVERLAPPEDWINDOW,
            },
        },
    },
};
use winit::monitor;

use crate::setting::window::{control_box::CaptionDirection, PinnedWindowSetting, WindowSetting};

const UIDSUBCLASS: usize = 0x1599764cf41046de;

unsafe extern "system" fn wrapper_subclass_prop(
    hwnd: HWND,
    umsg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    uidsubclass: usize,
    dwrefdata: usize,
) -> LRESULT {
    debug_assert_eq!(uidsubclass, UIDSUBCLASS);

    // println!("umsg: {}, wparam: {}, lparam: {}", umsg, wparam, lparam);

    let mut l_ret: LRESULT = 0;

    // 成功したか
    // let f_call_dwp = unsafe { DwmDefWindowProc(hwnd, umsg, wparam, lparam, &mut l_ret) == S_OK };
    // let f_call_dwp = true;

    // println!("f_call_dwp: {}", f_call_dwp);

    // Shift + F10
    if umsg == WM_CONTEXTMENU {
        show_context_menu(lparam, hwnd);
    }

    if umsg == WM_PAINT {
        // println!("WM_PAINT: {}", lparam as isize);

        // let margins = if IsZoomed(hwnd) != 0 {
        //     // 最大化されている
        //     MARGINS {
        //         cxLeftWidth: 0,
        //         cxRightWidth: 0,
        //         cyBottomHeight: 0,
        //         cyTopHeight: 20,
        //     }
        // } else {
        //     MARGINS {
        //         cxLeftWidth: LEFTEXTENDWIDTH,
        //         cxRightWidth: RIGHTEXTENDWIDTH,
        //         cyBottomHeight: BOTTOMEXTENDWIDTH,
        //         cyTopHeight: TOPEXTENDWIDTH,
        //     }
        // };

        // let hr = unsafe { DwmExtendFrameIntoClientArea(hwnd, &margins) };
        // if hr != 0 {
        //     println!("DwmExtendFrameIntoClientArea failed: {}", hr);
        // } else {
        //     println!("DwmExtendFrameIntoClientArea succeeded");
        // }

        // dbg!(hwnd);

        // return 0;
    }

    // if comment out and adjust closing box
    // why??
    if umsg == WM_NCCALCSIZE && wparam == 1 {
        let params = std::mem::transmute::<
            LPARAM,
            *mut windows_sys::Win32::UI::WindowsAndMessaging::NCCALCSIZE_PARAMS,
        >(lparam);

        // https://github.com/rust-windowing/winit/blob/337d50779c299240f6e0a67ef3e852f1c971cf16/src/platform_impl/windows/event_loop.rs#L1076

        if IsZoomed(hwnd) != 0 {
            let monitor = unsafe { MonitorFromRect(&(*params).rgrc[0], MONITOR_DEFAULTTONULL) };
            if let Ok(monitor_info) = get_monitor_info(monitor) {
                (*params).rgrc[0] = monitor_info.monitorInfo.rcWork;
            }
        }

        return 0;
        // const WVR_REDRAW: isize = 0x0300;
        // return WVR_REDRAW;
    }

    // 右クリックでコンテキストメニューを表示

    if umsg == WM_NCRBUTTONUP || umsg == WM_RBUTTONUP {
        let setting = dwrefdata as *const PinnedWindowSetting;
        let setting = &*setting;
        let setting = setting.setting();

        let pt_mouse = windows::Win32::Foundation::POINT {
            x: GET_X_LPARAM(lparam),
            y: GET_Y_LPARAM(lparam),
        };

        // Get the window rectangle.
        let mut rc_window = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        unsafe { GetWindowRect(hwnd, &mut rc_window) };

        if unsafe { IsZoomed(hwnd) } != 0 {
            let monitor = unsafe {
                MonitorFromRect(
                    &RECT {
                        left: 0,
                        top: 0,
                        right: 0,
                        bottom: 0,
                    },
                    MONITOR_DEFAULTTONULL,
                )
            };
            if let Ok(monitor_info) = get_monitor_info(monitor) {
                rc_window = monitor_info.monitorInfo.rcWork;
            }
        }

        let control_box_setting = setting.control_box_setting;
        let caption = check_caption(
            control_box_setting.caption_direction,
            rc_window,
            pt_mouse,
            control_box_setting.caption_wide,
        );

        if caption {
            show_context_menu(lparam, hwnd);
        }
    }

    // タップ動作の上書き
    if umsg == WM_NCHITTEST && l_ret == HTNOWHERE as isize {
        let setting = dwrefdata as *const PinnedWindowSetting;
        let setting = &*setting;
        let setting = setting.setting();

        l_ret = hit_test_nca(hwnd, wparam, lparam, setting);

        if l_ret == HTNOWHERE as isize {
            return DefSubclassProc(hwnd, umsg, wparam, lparam);
        }

        return l_ret;
    }

    // if f_call_dwp {
    //     return DefSubclassProc(hwnd, umsg, wparam, lparam);
    // } else {
    //     return l_ret;
    // }

    return DefSubclassProc(hwnd, umsg, wparam, lparam);
}

// https://github.com/rust-windowing/winit/blob/master/src/platform_impl/windows/monitor.rs#L135
pub(crate) fn get_monitor_info(hmonitor: HMONITOR) -> Result<MONITORINFOEXW, std::io::Error> {
    let mut monitor_info: MONITORINFOEXW = unsafe { mem::zeroed() };
    monitor_info.monitorInfo.cbSize = mem::size_of::<MONITORINFOEXW>() as u32;
    let status = unsafe {
        GetMonitorInfoW(
            hmonitor,
            &mut monitor_info as *mut MONITORINFOEXW as *mut MONITORINFO,
        )
    };
    if status == false.into() {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(monitor_info)
    }
}

// https://learn.microsoft.com/ja-jp/windows/win32/inputdev/wm-nchittest
// Hit test the frame for resizing and moving.
fn hit_test_nca(hwnd: HWND, _w_param: WPARAM, l_param: LPARAM, setting: &WindowSetting) -> LRESULT {
    // Get the point coordinates for the hit test.
    let pt_mouse = windows::Win32::Foundation::POINT {
        x: GET_X_LPARAM(l_param),
        y: GET_Y_LPARAM(l_param),
    };

    // Get the window rectangle.
    let mut rc_window = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe { GetWindowRect(hwnd, &mut rc_window) };

    if unsafe { IsZoomed(hwnd) } != 0 {
        let monitor = unsafe {
            MonitorFromRect(
                &RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                },
                MONITOR_DEFAULTTONULL,
            )
        };
        if let Ok(monitor_info) = get_monitor_info(monitor) {
            rc_window = monitor_info.monitorInfo.rcWork;
        }
    }

    // Get the frame rectangle, adjusted for the style without a caption.
    // let mut rc_frame = RECT {
    //     left: 0,
    //     top: 0,
    //     right: 0,
    //     bottom: 0,
    // };
    // unsafe { AdjustWindowRectEx(&mut rc_frame, WS_OVERLAPPEDWINDOW & !WS_CAPTION, FALSE, 0) };

    // Determine if the hit test is for resizing. Default middle (1,1).
    let mut u_row: usize = 1;
    let mut u_col: usize = 1;

    let top_ext_width = setting.top_frame_height;
    let bottom_ext_width = setting.bottom_frame_height;
    let left_ext_width = setting.left_frame_width;
    let right_ext_width = setting.right_frame_width;
    let control_box_setting = setting.control_box_setting;
    let caption_wide = control_box_setting.caption_wide;
    let caption_direction = control_box_setting.caption_direction;

    // println!(
    //     "top: {}, bottom: {}, left: {}, right: {}",
    //     top_ext_width, bottom_ext_width, left_ext_width, right_ext_width);

    // Determine if the point is at the top or bottom of the window.
    if (rc_window.top <= pt_mouse.y && pt_mouse.y < rc_window.top + top_ext_width) {
        u_row = 0;
    } else if (pt_mouse.y < rc_window.bottom && pt_mouse.y >= rc_window.bottom - bottom_ext_width) {
        u_row = 2;
    }

    // Determine if the point is at the left or right of the window.
    if (pt_mouse.x >= rc_window.left && pt_mouse.x < rc_window.left + left_ext_width) {
        u_col = 0; // left side
    } else if (pt_mouse.x < rc_window.right && pt_mouse.x >= rc_window.right - right_ext_width) {
        u_col = 2; // right side
    }

    // println!("left: {}, top: {}, right: {}, bottom: {}", rc_window.left, rc_window.top, rc_window.right, rc_window.bottom);

    let caption = check_caption(caption_direction, rc_window, pt_mouse, caption_wide);

    // println!("caption: {}", caption);

    // Hit test (HTTOPLEFT, ... HTBOTTOMRIGHT)
    let hit_tests: Vec<Vec<u32>> = vec![
        vec![HTTOPLEFT, HTTOP, HTTOPRIGHT],
        vec![HTLEFT, if caption { HTCAPTION } else { HTNOWHERE }, HTRIGHT],
        vec![HTBOTTOMLEFT, HTBOTTOM, HTBOTTOMRIGHT],
    ];

    return hit_tests[u_row][u_col] as isize;
}

pub fn check_caption(
    caption_direction: CaptionDirection,
    rc_window: RECT,
    pt_mouse: windows::Win32::Foundation::POINT,
    caption_wide: i32,
) -> bool {
    let caption = match caption_direction {
        CaptionDirection::Top => {
            if rc_window.top <= pt_mouse.y && pt_mouse.y < rc_window.top + caption_wide {
                true
            } else {
                false
            }
        }
        CaptionDirection::Bottom => {
            if pt_mouse.y < rc_window.bottom && pt_mouse.y >= rc_window.bottom - caption_wide {
                true
            } else {
                false
            }
        }
        CaptionDirection::Left => {
            if rc_window.left <= pt_mouse.x && pt_mouse.x < rc_window.left + caption_wide {
                true
            } else {
                false
            }
        }
        CaptionDirection::Right => {
            if pt_mouse.x < rc_window.right && pt_mouse.x >= rc_window.right - caption_wide {
                true
            } else {
                false
            }
        }
    };

    caption
}

pub fn show_context_menu(lparam: LPARAM, hwnd: HWND) -> LRESULT {
    println!("WM_CONTEXTMENU: {}", lparam as isize);

    // https://learn.microsoft.com/windows/win32/menurc/wm-contextmenu

    let x_pos = GET_X_LPARAM(lparam);
    let y_pos = GET_Y_LPARAM(lparam);

    // https://learn.microsoft.com/ja-jp/windows/win32/api/winuser/nf-winuser-trackpopupmenu
    let system_menu_handle = unsafe { GetSystemMenu(hwnd, false as i32) };
    println!("system_menu_handle: {:?}", system_menu_handle);

    // https://learn.microsoft.com/ja-jp/windows/win32/api/winuser/nf-winuser-trackpopupmenuex
    let wparam = unsafe {
        TrackPopupMenuEx(
            system_menu_handle,
            TPM_RETURNCMD,
            x_pos,
            y_pos,
            hwnd,
            std::ptr::null(),
        )
    };
    println!("w_param: {}", wparam);

    let umsg = WM_SYSCOMMAND;

    match wparam as u32 {
        SC_MAXIMIZE => {
            println!("SC_MAXIMIZE");
        }
        SC_RESTORE => {
            println!("SC_RESTORE");
        }
        SC_CLOSE => {
            println!("SC_CLOSE");
        }
        SC_MOVE => {
            println!("SC_MOVE");
        }
        SC_MINIMIZE => {
            println!("SC_MINIMIZE");
        }
        SC_SIZE => {
            println!("SC_SIZE");
        }
        _ => {}
    }

    return unsafe { DefSubclassProc(hwnd, umsg, wparam as usize, lparam) };
}

// https://learn.microsoft.com/ja-jp/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute

/// get DWMWA_CAPTION_BUTTON_BOUNDS
#[allow(dead_code)]
pub unsafe fn get_caption_button_rect(hwnd: HWND) -> RECT {
    let mut bounds = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let hr = DwmGetWindowAttribute(
        hwnd,
        DWMWA_CAPTION_BUTTON_BOUNDS as u32,
        &mut bounds as *mut _ as *mut std::ffi::c_void,
        std::mem::size_of::<RECT>() as u32,
    );
    if hr != 0 {
        println!("DwmGetWindowAttribute failed: {}", hr);
    } else {
        // println!("DwmGetWindowAttribute succeeded:\ntop: {}\nleft: {}\nright: {}\nbottom: {}", bounds.top, bounds.left, bounds.right, bounds.bottom);
        println!("DwmGetWindowAttribute succeeded");
    }

    bounds
}

/// set DWMWA_CAPTION_BUTTON_BOUNDS
#[allow(dead_code)]
pub unsafe fn set_caption_button_rect(hwnd: HWND, bounds: RECT) {
    let mut bounds = bounds.clone();
    let hr = DwmSetWindowAttribute(
        hwnd,
        DWMWA_CAPTION_BUTTON_BOUNDS as u32,
        &mut bounds as *mut _ as *mut std::ffi::c_void,
        std::mem::size_of::<RECT>() as u32,
    );
    if hr != 0 {
        println!("DwmSetWindowAttribute caption rect failed: 0x{:0x}", hr);
    } else {
        println!("DwmSetWindowAttribute caption rect succeeded:\ntop: {}\nleft: {}\nright: {}\nbottom: {}", bounds.top, bounds.left, bounds.right, bounds.bottom);
    }
}

/// get  DWMWA_EXTENDED_FRAME_BOUNDS
#[allow(dead_code)]
pub unsafe fn get_extended_frame_bounds(hwnd: HWND) -> RECT {
    let mut bounds = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let hr = DwmGetWindowAttribute(
        hwnd,
        windows_sys::Win32::Graphics::Dwm::DWMWA_EXTENDED_FRAME_BOUNDS as u32,
        &mut bounds as *mut _ as *mut std::ffi::c_void,
        std::mem::size_of::<RECT>() as u32,
    );
    if hr != 0 {
        println!("DwmGetWindowAttribute failed: {}", hr);
    } else {
        println!(
            "DwmGetWindowAttribute succeeded:\ntop: {}\nleft: {}\nright: {}\nbottom: {}",
            bounds.top, bounds.left, bounds.right, bounds.bottom
        );
    }

    bounds
}

/// get DWMWA_NCRENDERING_POLICY
#[allow(dead_code)]
pub unsafe fn get_nc_rendering_policy(hwnd: HWND) -> HRESULT {
    let mut policy = 0;
    let hr = DwmGetWindowAttribute(
        hwnd,
        windows_sys::Win32::Graphics::Dwm::DWMWA_NCRENDERING_POLICY as u32,
        &mut policy as *mut _ as *mut std::ffi::c_void,
        std::mem::size_of::<i32>() as u32,
    );
    if hr != 0 {
        println!("DwmGetWindowAttribute failed: {}", hr);
    } else {
        println!("DwmGetWindowAttribute succeeded: {}", policy);
    }

    policy
}

/// set DWMWA_NCRENDERING_POLICY
#[allow(dead_code)]
pub unsafe fn set_nc_rendering_policy(hwnd: HWND, policy: i32) {
    let hr = DwmSetWindowAttribute(
        hwnd,
        windows_sys::Win32::Graphics::Dwm::DWMWA_NCRENDERING_POLICY as u32,
        &policy as *const _ as *const std::ffi::c_void,
        std::mem::size_of::<i32>() as u32,
    );
    if hr != 0 {
        println!("DwmSetWindowAttribute failed: 0x{:0x}", hr);
    } else {
        println!("DwmSetWindowAttribute succeeded: {}", policy);
    }
}

/// get DWMWA_ALLOW_NCPAINT
#[allow(dead_code)]
pub unsafe fn set_allow_nc_paint(hwnd: HWND, allow: bool) {
    let allow = if allow { 1 } else { 0 };
    let hr = DwmSetWindowAttribute(
        hwnd,
        windows_sys::Win32::Graphics::Dwm::DWMWA_ALLOW_NCPAINT as u32,
        &allow as *const _ as *const std::ffi::c_void,
        std::mem::size_of::<i32>() as u32,
    );
    if hr != 0 {
        println!("DwmSetWindowAttribute failed: 0x{:0x}", hr);
    } else {
        println!("DwmSetWindowAttribute succeeded: {}", allow);
    }
}

/// set DWMWA_WINDOW_CORNER_PREFERENCE
#[allow(dead_code)]
pub unsafe fn set_window_corner_radius(hwnd: HWND, radius: DWM_WINDOW_CORNER_PREFERENCE) {
    let hr = DwmSetWindowAttribute(
        hwnd,
        windows_sys::Win32::Graphics::Dwm::DWMWA_WINDOW_CORNER_PREFERENCE as u32,
        &radius as *const _ as *const std::ffi::c_void,
        std::mem::size_of::<f32>() as u32,
    );
    if hr != 0 {
        println!("DwmSetWindowAttribute radius failed: 0x{:0x}", hr);
    } else {
        println!("DwmSetWindowAttribute radius succeeded: {}", radius);
    }
}

/// set DWMWA_TRANSITIONS_FORCEDISABLED
#[allow(dead_code)]
pub unsafe fn set_transitions_force_disabled(hwnd: HWND, disabled: bool) {
    let disabled = if disabled { 1 } else { 0 };
    let hr = DwmSetWindowAttribute(
        hwnd,
        windows_sys::Win32::Graphics::Dwm::DWMWA_TRANSITIONS_FORCEDISABLED as u32,
        &disabled as *const _ as *const std::ffi::c_void,
        std::mem::size_of::<i32>() as u32,
    );
    if hr != 0 {
        println!("DwmSetWindowAttribute disabled failed: 0x{:0x}", hr);
    } else {
        println!("DwmSetWindowAttribute disabled succeeded: {}", disabled);
    }
}
