pub mod window;
use std::mem;

use winapi::shared::windowsx::{GET_X_LPARAM, GET_Y_LPARAM};
use windows::Win32::UI::WindowsAndMessaging::{HTCLOSE, HTSIZE, SC_MOVE};
use windows_sys::{
    core::HRESULT,
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::{
            Dwm::{
                DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWA_CAPTION_BUTTON_BOUNDS,
                DWM_WINDOW_CORNER_PREFERENCE,
            },
            Gdi::{
                GetMonitorInfoW, MonitorFromRect, HMONITOR, MONITORINFO, MONITORINFOEXW,
                MONITOR_DEFAULTTONULL,
            },
        },
        UI::{
            Shell::DefSubclassProc,
            WindowsAndMessaging::{
                GetSystemMenu, GetWindowRect, IsZoomed, TrackPopupMenuEx, HTBOTTOM, HTBOTTOMLEFT,
                HTBOTTOMRIGHT, HTCAPTION, HTGROWBOX, HTHELP, HTLEFT, HTMAXBUTTON, HTMINBUTTON,
                HTNOWHERE, HTREDUCE, HTRIGHT, HTSYSMENU, HTTOP, HTTOPLEFT, HTTOPRIGHT, HTZOOM,
                SC_CLOSE, SC_MAXIMIZE, SC_MINIMIZE, SC_RESTORE, SC_SIZE, TPM_RETURNCMD,
                WM_CONTEXTMENU, WM_NCCALCSIZE, WM_NCHITTEST, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP,
                WM_NCRBUTTONUP, WM_PAINT, WM_RBUTTONUP, WM_SYSCOMMAND,
            },
        },
    },
};

use crate::setting::window::{
    control_box::{CaptionDirection, ControlBoxPositionAxis},
    PinnedWindowSetting, WindowSetting,
};

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
        } else {
            // expand accent color frame
            // 何か伸ばさないと影やフレームがなくなる
            (*params).rgrc[0].top += 1;
            // (*params).rgrc[0].left += 1;
            // (*params).rgrc[0].right += 1;
            // (*params).rgrc[0].bottom += 1;
        }

        return 0;
        // const WVR_REDRAW: isize = 0x0300;
        // return WVR_REDRAW;
    }

    if umsg == WM_NCLBUTTONDOWN {
        // https://stackoverflow.com/a/22013757
        // https://learn.microsoft.com/ja-jp/windows/win32/inputdev/wm-nchittest
        let vec = vec![
            HTCLOSE,
            HTMAXBUTTON,
            HTMINBUTTON,
            HTGROWBOX,
            HTSIZE,
            HTHELP,
            HTREDUCE,
            HTSYSMENU,
            HTZOOM,
        ];

        if vec.contains(&(wparam as u32)) {
            return 0;
        }
    }

    if umsg == WM_NCLBUTTONUP {
        // https://chokuto.ifdef.jp/urawaza/message/WM_SYSCOMMAND.html
        match wparam as u32 {
            HTCLOSE => {
                let umsg = WM_SYSCOMMAND;
                let wparam = SC_CLOSE as usize;
                return DefSubclassProc(hwnd, umsg, wparam, lparam);
            }
            // HTMINBUTTON | HTREDUCE => {
            HTMINBUTTON => {
                let umsg = WM_SYSCOMMAND;
                let wparam = SC_MINIMIZE as usize;
                return DefSubclassProc(hwnd, umsg, wparam, lparam);
            }
            // HTMAXBUTTON | HTZOOM => {
            HTMAXBUTTON => {
                let umsg = WM_SYSCOMMAND;
                let wparam = if unsafe { IsZoomed(hwnd) } != 0 {
                    SC_RESTORE as usize
                } else {
                    SC_MAXIMIZE as usize
                };
                return DefSubclassProc(hwnd, umsg, wparam, lparam);
            }
            _ => {}
        }
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
        let mut rc_window: RECT = std::mem::zeroed();
        unsafe { GetWindowRect(hwnd, &mut rc_window) };

        if unsafe { IsZoomed(hwnd) } != 0 {
            let monitor = unsafe {
                MonitorFromRect(
                    &std::mem::zeroed(),
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
        // println!("tap");

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
    let mut rc_window: RECT = unsafe { std::mem::zeroed() };
    unsafe { GetWindowRect(hwnd, &mut rc_window) };

    let is_zoomed = unsafe { IsZoomed(hwnd) } != 0;
    if is_zoomed {
        let monitor = unsafe {
            MonitorFromRect(
                &std::mem::zeroed(),
                MONITOR_DEFAULTTONULL,
            )
        };
        if let Ok(monitor_info) = get_monitor_info(monitor) {
            rc_window = monitor_info.monitorInfo.rcWork;
        }
    }

    // check control box
    {
        let control_box_setting = setting.control_box_setting;

        let overlay_caption_frame_width = setting.overlay_caption_frame_width;

        let size_width = rc_window.right - rc_window.left;
        let size_height = rc_window.bottom - rc_window.top;

        let count = control_box_setting.maximize_button as i32
            + control_box_setting.minimize_button as i32
            + control_box_setting.close_button as i32;

        match control_box_setting.caption_direction {
            CaptionDirection::Left | CaptionDirection::Right => {
                let movement = match control_box_setting.position_y {
                    ControlBoxPositionAxis::Center { margin } => margin,
                    _ => 0,
                };

                let (start_x, outer_x) = match control_box_setting.caption_direction {
                    CaptionDirection::Left => (
                        overlay_caption_frame_width,
                        rc_window.left + control_box_setting.box_width,
                    ),
                    CaptionDirection::Right => (
                        size_width - control_box_setting.box_width,
                        rc_window.right - overlay_caption_frame_width,
                    ),
                    _ => unreachable!(),
                };
                let start_y = match control_box_setting.position_y {
                    ControlBoxPositionAxis::First => 0,
                    ControlBoxPositionAxis::Last => {
                        size_height - count * control_box_setting.box_height
                    }
                    ControlBoxPositionAxis::Center { margin } => {
                        (size_height
                            - count * control_box_setting.box_height
                            - (count - 1) * margin)
                            / 2
                    }
                };
                let first_overlay_frame_y = rc_window.top + overlay_caption_frame_width;
                let last_overlay_frame_y = rc_window.bottom - overlay_caption_frame_width;

                let mut y = start_y + rc_window.top;
                let start_x = start_x + rc_window.left;

                if control_box_setting.minimize_button {
                    if start_x <= pt_mouse.x
                        && pt_mouse.x < outer_x
                        && y <= pt_mouse.y
                        && pt_mouse.y < y + control_box_setting.box_height
                        && first_overlay_frame_y <= pt_mouse.y
                        && pt_mouse.y < last_overlay_frame_y
                    {
                        return HTMINBUTTON as isize;
                    }
                    y += control_box_setting.box_height + movement;
                }
                if control_box_setting.maximize_button {
                    if start_x <= pt_mouse.x
                        && pt_mouse.x < outer_x
                        && pt_mouse.y - control_box_setting.box_height < y
                        && y <= pt_mouse.y
                        && first_overlay_frame_y <= pt_mouse.y
                        && pt_mouse.y < last_overlay_frame_y
                    {
                        return HTMAXBUTTON as isize;
                    }
                    y += control_box_setting.box_height + movement;
                }
                if control_box_setting.close_button {
                    if start_x <= pt_mouse.x
                        && pt_mouse.x < outer_x
                        && pt_mouse.y - control_box_setting.box_height < y
                        && y <= pt_mouse.y
                        && first_overlay_frame_y <= pt_mouse.y
                        && pt_mouse.y < last_overlay_frame_y
                    {
                        return HTCLOSE as isize;
                    }
                }
            }
            CaptionDirection::Top | CaptionDirection::Bottom => {
                let movement = match control_box_setting.position_x {
                    ControlBoxPositionAxis::Center { margin } => margin,
                    _ => 0,
                };

                let start_x = match control_box_setting.position_x {
                    ControlBoxPositionAxis::First => 0,
                    ControlBoxPositionAxis::Last => {
                        size_width - count * control_box_setting.box_width
                    }
                    ControlBoxPositionAxis::Center { margin } => {
                        (size_width - count * control_box_setting.box_width - (count - 1) * margin)
                            / 2
                    }
                };
                let (start_y, outer_y) = match control_box_setting.caption_direction {
                    CaptionDirection::Top => (
                        overlay_caption_frame_width,
                        rc_window.top + control_box_setting.box_height,
                    ),
                    CaptionDirection::Bottom => (
                        size_height - control_box_setting.caption_wide,
                        rc_window.top + size_height - control_box_setting.box_height,
                    ),
                    _ => unreachable!(),
                };

                let first_overlay_frame_x = rc_window.left + overlay_caption_frame_width;
                let last_overlay_frame_x = rc_window.right - overlay_caption_frame_width;

                let mut x = start_x + rc_window.left;
                let start_y = start_y + rc_window.top;

                if control_box_setting.minimize_button {
                    if x <= pt_mouse.x
                        && pt_mouse.x < x + control_box_setting.box_width
                        && start_y <= pt_mouse.y
                        && pt_mouse.y < outer_y
                        && first_overlay_frame_x <= pt_mouse.x
                        && pt_mouse.x < last_overlay_frame_x
                    {
                        // println!("minimize_button");
                        return HTMINBUTTON as isize;
                    }
                    x += control_box_setting.box_width + movement;
                }
                if control_box_setting.maximize_button {
                    if x <= pt_mouse.x
                        && pt_mouse.x < x + control_box_setting.box_width
                        && start_y <= pt_mouse.y
                        && pt_mouse.y < outer_y
                        && first_overlay_frame_x <= pt_mouse.x
                        && pt_mouse.x < last_overlay_frame_x
                    {
                        // println!("maximize_button");
                        return HTMAXBUTTON as isize;
                    }
                    x += control_box_setting.box_width + movement;
                }
                if control_box_setting.close_button {
                    if x <= pt_mouse.x
                        && pt_mouse.x < x + control_box_setting.box_width
                        && start_y <= pt_mouse.y
                        && pt_mouse.y < outer_y
                        && first_overlay_frame_x <= pt_mouse.x
                        && pt_mouse.x < last_overlay_frame_x
                    {
                        // println!("close_button");
                        return HTCLOSE as isize;
                    }
                }
            }
        }
    }

    if !is_zoomed {
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

        // Determine if the point is at the top or bottom of the window.
        if rc_window.top <= pt_mouse.y && pt_mouse.y < rc_window.top + top_ext_width {
            u_row = 0;
        } else if rc_window.bottom - bottom_ext_width <= pt_mouse.y && pt_mouse.y < rc_window.bottom
        {
            u_row = 2;
        }

        // Determine if the point is at the left or right of the window.
        if rc_window.left <= pt_mouse.x && pt_mouse.x < rc_window.left + left_ext_width {
            u_col = 0; // left side
        } else if rc_window.right - right_ext_width <= pt_mouse.x && pt_mouse.x < rc_window.right {
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
    } else {
        let control_box_setting = setting.control_box_setting;
        let caption_wide = control_box_setting.caption_wide;
        let caption_direction = control_box_setting.caption_direction;

        let caption = check_caption(caption_direction, rc_window, pt_mouse, caption_wide);

        return if caption { HTCAPTION } else { HTNOWHERE } as isize;
    }
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
    let mut bounds: RECT = std::mem::zeroed();
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
    let mut bounds: RECT = std::mem::zeroed();
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
