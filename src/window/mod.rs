mod window;
use winapi::shared::windowsx::{GET_X_LPARAM, GET_Y_LPARAM};
pub use window::WindowWrapper;
use windows_sys::{
    core::HRESULT,
    Win32::{
        Foundation::{FALSE, HWND, LPARAM, LRESULT, RECT, S_OK, WPARAM},
        Graphics::Dwm::{DwmDefWindowProc, DwmExtendFrameIntoClientArea, DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWA_CAPTION_BUTTON_BOUNDS, DWMWCP_DONOTROUND, DWM_WINDOW_CORNER_PREFERENCE},
        UI::{
            Controls::MARGINS, Shell::DefSubclassProc, WindowsAndMessaging::{
                AdjustWindowRectEx, GetWindowRect, IsZoomed, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTLEFT, HTMAXBUTTON, HTNOWHERE, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, WM_NCCALCSIZE, WM_NCHITTEST, WM_PAINT, WS_CAPTION, WS_OVERLAPPEDWINDOW
            }
        },
    },
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

    let window = dwrefdata as *mut WindowWrapper;
    let window = &mut *window;

    let mut l_ret: LRESULT = 0;

    // 成功したか
    let f_call_dwp = unsafe { DwmDefWindowProc(hwnd, umsg, wparam, lparam, &mut l_ret) == S_OK };
    // let f_call_dwp = true;

    // println!("f_call_dwp: {}", f_call_dwp);

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

        dbg!(hwnd);

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

        (*params).rgrc[0].top += 0;
        (*params).rgrc[0].left += 0;
        (*params).rgrc[0].right += 0;
        (*params).rgrc[0].bottom += 0;
        println!("rgrc[0]: top: {}, left: {}, right: {}, bottom: {}", (*params).rgrc[0].top, (*params).rgrc[0].left, (*params).rgrc[0].right, (*params).rgrc[0].bottom);

        println!("WM_NCCALCSIZE: {}", lparam as isize);

        return 0;
        // const WVR_REDRAW: isize = 0x0300;
        // return WVR_REDRAW;
    }

    // タップ動作の上書き
    if umsg == WM_NCHITTEST && l_ret == 0 {
        if l_ret == HTNOWHERE as isize {
            l_ret = hit_test_nca(hwnd, wparam, lparam);

            return l_ret;
        }
    }

    if f_call_dwp {
        return DefSubclassProc(hwnd, umsg, wparam, lparam);
    } else {
        return l_ret;
    }
}

// Hit test the frame for resizing and moving.
fn hit_test_nca(hwnd: HWND, w_param: WPARAM, l_param: LPARAM) -> LRESULT {
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

    // Get the frame rectangle, adjusted for the style without a caption.
    let mut rc_frame = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe { AdjustWindowRectEx(&mut rc_frame, WS_OVERLAPPEDWINDOW & !WS_CAPTION, FALSE, 0) };

    // Determine if the hit test is for resizing. Default middle (1,1).
    let mut u_row: usize = 1;
    let mut u_col: usize = 1;
    let mut f_on_resize_border = false;

    let top_ext_width = 20;
    let bottom_ext_width = 5;
    let left_ext_width = 5;
    let right_ext_width = 5;

    // Determine if the point is at the top or bottom of the window.
    // if (pt_mouse.y >= rc_window.top && pt_mouse.y < rc_window.top + TOPEXTENDWIDTH) {
        if (pt_mouse.y >= rc_window.top && pt_mouse.y < rc_window.top + top_ext_width) {
        f_on_resize_border = (pt_mouse.y < (rc_window.top - rc_frame.top));
        u_row = 0;
    } else if (pt_mouse.y < rc_window.bottom && pt_mouse.y >= rc_window.bottom - bottom_ext_width)
    {
        u_row = 2;
    }

    // Determine if the point is at the left or right of the window.
    if (pt_mouse.x >= rc_window.left && pt_mouse.x < rc_window.left + left_ext_width) {
        u_col = 0; // left side
    } else if (pt_mouse.x < rc_window.right && pt_mouse.x >= rc_window.right - right_ext_width) {
        u_col = 2; // right side
    }

    // Hit test (HTTOPLEFT, ... HTBOTTOMRIGHT)
    let hit_tests: Vec<Vec<u32>> = vec![
        vec![
            HTTOPLEFT,
            if f_on_resize_border { HTTOP } else { HTCAPTION },
            HTTOPRIGHT,
        ],
        vec![HTLEFT, HTNOWHERE, HTRIGHT],
        vec![HTBOTTOMLEFT, HTBOTTOM, HTBOTTOMRIGHT],
    ];

    return hit_tests[u_row][u_col] as isize;
}

// これは余白
// この範囲がダブルクリックで拡大や、ドラッグで移動できる範囲
// const LEFTEXTENDWIDTH: i32 = 8;
// const RIGHTEXTENDWIDTH: i32 = 8;
// const BOTTOMEXTENDWIDTH: i32 = 20;
// const TOPEXTENDWIDTH: i32 = 27;
const LEFTEXTENDWIDTH: i32 = 0;
const RIGHTEXTENDWIDTH: i32 = 10;
const BOTTOMEXTENDWIDTH: i32 = 0;
const TOPEXTENDWIDTH: i32 = 40;
// pub const LEFTEXTENDWIDTH: i32 = 0;
// pub const RIGHTEXTENDWIDTH: i32 = 0;
// pub const BOTTOMEXTENDWIDTH: i32 = 0;
// pub const TOPEXTENDWIDTH: i32 = 0;

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
        println!("DwmGetWindowAttribute succeeded:\ntop: {}\nleft: {}\nright: {}\nbottom: {}", bounds.top, bounds.left, bounds.right, bounds.bottom);
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
