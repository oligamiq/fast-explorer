use std::{
    ffi::{c_void, OsStr, OsString},
    iter::once,
    os::windows::ffi::{OsStrExt as _, OsStringExt as _},
    pin::{pin, Pin},
    ptr::{null, null_mut},
};

use raqote::{
    DrawOptions, DrawTarget, LineCap, LineJoin, PathBuilder, SolidSource, Source, StrokeStyle,
};
use windows::{
    core::Interface,
    Win32::{
        Foundation::{HWND, TRUE},
        Graphics::Dwm::{DwmDefWindowProc, DwmIsCompositionEnabled},
        UI::WindowsAndMessaging::{
            AdjustWindowRectEx, GetSystemMenu, GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos,
            GWLP_HINSTANCE, GWL_STYLE, SWP_FRAMECHANGED, WS_SYSMENU,
        },
    },
    UI::WindowManagement::{AppWindow, AppWindowTitleBar},
};
use windows_sys::Win32::{
    Foundation::POINT,
    Graphics::{
        Dwm::{DwmExtendFrameIntoClientArea, DWMNCRP_ENABLED, DWMNCRP_USEWINDOWSTYLE},
        Gdi::{
            BeginPaint, BitBlt, CreateCompatibleDC, CreateDIBSection, CreateFontIndirectW,
            DeleteObject, EndPaint, SelectObject, TextOutW, BITMAPINFO, BITMAPINFOHEADER, DT_LEFT,
            DT_WORD_ELLIPSIS, LOGFONTW, PAINTSTRUCT, RGBQUAD, SRCCOPY,
        },
    },
    UI::{
        Controls::{
            CloseThemeData, DrawThemeTextEx, GetThemeSysFont, OpenThemeData, OpenThemeDataEx,
            DTTOPTS, DTT_COMPOSITED, DTT_GLOWSIZE, MARGINS, TMT_CAPTIONFONT,
        },
        Shell::SetWindowSubclass,
    },
};
use winit::{
    dpi::PhysicalSize,
    event_loop::ActiveEventLoop,
    platform::windows::{WindowAttributesExtWindows as _, WindowExtWindows as _},
    raw_window_handle::HasWindowHandle as _,
    window::{Window, WindowButtons},
};

use crate::{
    setting::{
        window::{
            control_box::{CaptionDirection, ControlBoxPositionAxis, ControlBoxSetting},
            PinnedWindowSetting, WindowSetting,
        },
        SettingContext,
    },
    window::{
        get_caption_button_rect, get_extended_frame_bounds, get_nc_rendering_policy,
        set_allow_nc_paint, set_nc_rendering_policy, set_transitions_force_disabled,
        set_window_corner_radius, wrapper_subclass_prop, UIDSUBCLASS,
    },
};

pub struct WindowWrapper {
    pub window: Window,
    pub setting: Pin<Box<PinnedWindowSetting>>,
}

impl WindowWrapper {
    pub fn new(event_loop: &ActiveEventLoop, setting: SettingContext) -> Self {
        let setting = {
            let window_setting_reader = setting.read();
            let setting = *window_setting_reader.window_setting();
            setting
        };

        let mut window = Window::default_attributes();
        window.title = "FastExplorer".into();
        // window = window.with_undecorated_shadow(true);
        // window = window.with_decorations(false);
        // window = window.with_enabled_buttons(WindowButtons::CLOSE);
        // let window = window.with_active(false);

        let mut window = event_loop.create_window(window).unwrap();

        window.focus_window();

        // let title = window.set_title("title");

        let hwnd: u64 = window.id().into();
        let hwnd: HWND = HWND(hwnd as isize);

        let position = window.outer_position().unwrap();
        let rect = window.outer_size();

        println!("position: {:?}", position);
        println!("rect: {:?}", rect);

        let caption_rect = unsafe { get_caption_button_rect(hwnd.0) };
        println!(
            "caption_rect -1: top: {}, left: {}, right: {}, bottom: {}",
            caption_rect.top, caption_rect.left, caption_rect.right, caption_rect.bottom
        );

        unsafe {
            let mut style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
            style &= !WS_SYSMENU.0;
            SetWindowLongPtrW(hwnd, GWL_STYLE, style as isize);
        }

        let pinned_setting = Box::pin(PinnedWindowSetting::new(setting));

        let ret = Self {
            window,
            setting: pinned_setting,
        };

        unsafe {
            SetWindowSubclass(
                hwnd.0,
                Some(wrapper_subclass_prop),
                UIDSUBCLASS,
                ret.setting.pointer() as *const PinnedWindowSetting as usize,
            )
        };

        let position = ret.window.outer_position().unwrap();
        let rect = ret.window.outer_size();

        println!("position: {:?}", position);
        println!("rect: {:?}", rect);

        // let caption_rect = unsafe { get_caption_button_rect(hwnd.0) };
        // println!(
        //     "caption_rect 0: top: {}, left: {}, right: {}, bottom: {}",
        //     caption_rect.top, caption_rect.left, caption_rect.right, caption_rect.bottom
        // );

        unsafe {
            SetWindowPos(
                hwnd,
                HWND::default(),
                position.x as i32,
                position.y as i32,
                rect.width as i32,
                rect.height as i32,
                // 500,
                // 500,
                SWP_FRAMECHANGED,
            )
        }
        .unwrap();

        // let caption_rect = unsafe { get_caption_button_rect(hwnd.0) };
        // println!(
        //     "caption_rect 1: top: {}, left: {}, right: {}, bottom: {}",
        //     caption_rect.top, caption_rect.left, caption_rect.right, caption_rect.bottom
        // );

        let mut margins = MARGINS {
            cxLeftWidth: 1,
            cxRightWidth: 1,
            cyTopHeight: 1,
            cyBottomHeight: 1,
        };
        let control_box_setting = setting.control_box_setting;
        match control_box_setting.caption_direction {
            CaptionDirection::Left => {
                margins.cxLeftWidth = control_box_setting.caption_wide;
            }
            CaptionDirection::Right => {
                margins.cxRightWidth = control_box_setting.caption_wide;
            }
            CaptionDirection::Top => {
                margins.cyTopHeight = control_box_setting.caption_wide;
            }
            CaptionDirection::Bottom => {
                margins.cyBottomHeight = control_box_setting.caption_wide;
            }
        }

        let hr = unsafe { DwmExtendFrameIntoClientArea(hwnd.0, &margins) };
        if hr != 0 {
            println!("DwmExtendFrameIntoClientArea failed: {}", hr);
        } else {
            println!("DwmExtendFrameIntoClientArea succeeded");
        }

        // let mut caption_rect = unsafe { get_caption_button_rect(hwnd.0) };
        // let window_rect = unsafe { get_extended_frame_bounds(hwnd.0) };
        // println!(
        //     "caption_rect 2: top: {}, left: {}, right: {}, bottom: {}",
        //     caption_rect.top, caption_rect.left, caption_rect.right, caption_rect.bottom
        // );
        // let window_width = window_rect.right - window_rect.left;
        // let diff = window_width - caption_rect.right;
        // dbg!(diff);
        // caption_rect.left += diff;
        // caption_rect.right += diff;
        // caption_rect.bottom = caption_rect.top + setting.get_window_setting().top_frame_height;

        // 一応
        // unsafe {
        //     set_allow_nc_paint(hwnd.0, true);
        // }

        unsafe {
            set_window_corner_radius(hwnd.0, windows_sys::Win32::Graphics::Dwm::DWMWCP_DONOTROUND)
        };

        // unsafe { set_transitions_force_disabled(hwnd.0, true) };

        // let policy = unsafe { get_nc_rendering_policy(hwnd.0) };
        // println!("policy: {:?}", policy);
        // dbg!(DWMNCRP_USEWINDOWSTYLE);
        // unsafe { set_nc_rendering_policy(hwnd.0, DWMNCRP_ENABLED) };

        // unsafe { crate::window::set_caption_button_rect(hwnd.0, caption_rect) };

        // println!("window_rect: top: {}, left: {}, right: {}, bottom: {}", window_rect.top, window_rect.left, window_rect.right, window_rect.bottom);
        // println!("window_width: {}", window_width);

        // unsafe {
        //     rewrite_title_bar(hwnd);
        // }

        ret
    }

    // https://learn.microsoft.com/en-us/windows/win32/dwm/customframe
    #[inline]
    pub fn check_dwm_is_composition(&self) -> bool {
        let hr = unsafe { DwmIsCompositionEnabled() };
        match hr {
            Ok(TRUE) => return true,
            _ => return false,
        }
    }

    pub fn paint(&self) {
        use raqote::*;

        // println!("paint start");

        let context = softbuffer::Context::new(&self.window).unwrap();

        let width = self.window.inner_size().width;
        let height = self.window.inner_size().height;
        println!("width: {}, height: {}", width, height);
        let mut dt = DrawTarget::new(width as i32, height as i32);

        let mut pb = PathBuilder::new();
        pb.move_to(0., 0.);
        pb.line_to(width as f32, height as f32);
        pb.line_to(width as f32, 0.);
        pb.close();
        let path = pb.finish();

        dt.fill(
            &path,
            &Source::Solid(SolidSource {
                r: 0x0,
                g: 0x0,
                b: 0x80,
                a: 0x80,
            }),
            &DrawOptions::new(),
        );

        self.paint_control_box(self.window.inner_size(), &mut dt);

        let mut surface = softbuffer::Surface::new(&context, &self.window).unwrap();
        surface
            .resize(width.try_into().unwrap(), height.try_into().unwrap())
            .unwrap();

        let mut buffer = surface.buffer_mut().unwrap();

        buffer.copy_from_slice(&dt.get_data_mut());

        buffer.present().unwrap();

        // println!("paint end");
    }

    pub fn paint_control_box(&self, size: PhysicalSize<u32>, mut paint_dt: &mut DrawTarget) {
        let setting = self.setting.as_ref();
        let control_box_setting = setting.setting().control_box_setting;

        let size_width = size.width as f32;
        let size_height = size.height as f32;

        let box_size: f32 = control_box_setting
            .box_height
            .min(control_box_setting.box_width) as f32;

        let count = (control_box_setting.maximize_button as u32
            + control_box_setting.minimize_button as u32
            + control_box_setting.close_button as u32) as f32;

        match control_box_setting.caption_direction {
            CaptionDirection::Left | CaptionDirection::Right => {
                let movement = match control_box_setting.position_y {
                    ControlBoxPositionAxis::Center { margin } => margin,
                    _ => 0,
                };

                let start_x: f32 = match control_box_setting.caption_direction {
                    CaptionDirection::Left => 0.,
                    CaptionDirection::Right => size_width - control_box_setting.caption_wide as f32,
                    _ => unreachable!(),
                };
                let start_y: f32 = match control_box_setting.position_y {
                    ControlBoxPositionAxis::First => 0.,
                    ControlBoxPositionAxis::Last => {
                        size_height - count * control_box_setting.box_height as f32
                    }
                    ControlBoxPositionAxis::Center { margin: _ } => {
                        (size_height - count * control_box_setting.box_height as f32) / 2.
                    }
                };

                let mut y = start_y + ((control_box_setting.box_height - box_size as i32) / 2) as f32;
                let start_x = start_x + ((control_box_setting.box_width - box_size as i32) / 2) as f32;

                if control_box_setting.minimize_button {
                    self.paint_control_box_minimize(
                        box_size,
                        &control_box_setting,
                        &mut paint_dt,
                        start_x,
                        y,
                    );
                    y += (control_box_setting.box_height + movement) as f32;
                    println!("y2: {}", y)
                }
                if control_box_setting.maximize_button {
                    self.paint_control_box_maximize(
                        box_size,
                        &control_box_setting,
                        &mut paint_dt,
                        start_x,
                        y,
                    );
                    y += (control_box_setting.box_height + movement) as f32;
                    println!("y: {}", y)
                }
                if control_box_setting.close_button {
                    self.paint_control_box_close(
                        box_size,
                        &control_box_setting,
                        &mut paint_dt,
                        start_x,
                        y,
                    );
                }
            },
            CaptionDirection::Top | CaptionDirection::Bottom => {
                let movement = match control_box_setting.position_x {
                    ControlBoxPositionAxis::Center { margin } => margin,
                    _ => 0,
                };

                let start_x: f32 = match control_box_setting.position_x {
                    ControlBoxPositionAxis::First => 0.,
                    ControlBoxPositionAxis::Last => {
                        size_width - count * control_box_setting.box_width as f32
                    }
                    ControlBoxPositionAxis::Center { margin: _ } => {
                        (size_width - count * control_box_setting.box_width as f32) / 2.
                    }
                };

                let mut x = start_x + ((control_box_setting.box_width - box_size as i32) / 2) as f32;
                let start_y = match control_box_setting.caption_direction {
                    CaptionDirection::Top => 0.,
                    CaptionDirection::Bottom => size_height - control_box_setting.caption_wide as f32,
                    _ => unreachable!(),
                };

                if control_box_setting.minimize_button {
                    self.paint_control_box_minimize(
                        box_size,
                        &control_box_setting,
                        &mut paint_dt,
                        x,
                        start_y,
                    );
                    x += (control_box_setting.box_width + movement) as f32;
                }
                if control_box_setting.maximize_button {
                    self.paint_control_box_maximize(
                        box_size,
                        &control_box_setting,
                        &mut paint_dt,
                        x,
                        start_y,
                    );
                    x += (control_box_setting.box_width + movement) as f32;
                }
                if control_box_setting.close_button {
                    self.paint_control_box_close(
                        box_size,
                        &control_box_setting,
                        &mut paint_dt,
                        x,
                        start_y,
                    );
                }
            },
        };
    }

    pub fn paint_control_box_close(
        &self,
        size: f32,
        setting: &ControlBoxSetting,
        dt: &mut DrawTarget,
        x: f32,
        y: f32,
    ) {
        let width = dt.width() as isize;
        let height = dt.height() as isize;

        let buff = dt.get_data_u8_mut();

        let size = size as f32;
        let diff = size / 3.;
        let diff = (diff / 2.).round() * 2.;
        let left = x + diff;
        let right = x + size - diff;
        let top = y + diff;

        let left = left as isize;
        let right = right as isize;
        let top = top as isize;

        // 対角線
        for i in 0..diff as isize {
            let x = left + i;
            let y = top + i;
            if x >= 0 && x < width && y >= 0 && y < height {
                let index = (y * width + x) as usize * 4;
                buff[index] = 0xff;
                buff[index + 1] = 0xff;
                buff[index + 2] = 0xff;
                buff[index + 3] = 0xff;
            }
            let x = right - i - 1;
            let y = top + i;
            if x >= 0 && x < width && y >= 0 && y < height {
                let index = (y * width + x) as usize * 4;
                buff[index] = 0xff;
                buff[index + 1] = 0xff;
                buff[index + 2] = 0xff;
                buff[index + 3] = 0xff;
            }
        }
    }

    pub fn paint_control_box_minimize(
        &self,
        size: f32,
        setting: &ControlBoxSetting,
        dt: &mut DrawTarget,
        x: f32,
        y: f32,
    ) {
        // 横線一つ。太さは1px。色は黒
        let diff = size / 3.;
        let diff = (diff / 2.).round() * 2.;
        let left = x + diff;
        let top = y + size / 2.;

        let width = dt.width() as isize;
        let height = dt.height() as isize;

        let buff = dt.get_data_u8_mut();

        let left = left as isize;
        let top = top as isize;

        // 横線
        for i in 0..diff as isize {
            let x = left + i;
            let y = top;
            if x >= 0 && x < width && y >= 0 && y < height {
                let index = (y * width + x) as usize * 4;
                buff[index] = 0xff;
                buff[index + 1] = 0xff;
                buff[index + 2] = 0xff;
                buff[index + 3] = 0xff;
            }
        }
    }

    pub fn paint_control_box_maximize(
        &self,
        size: f32,
        setting: &ControlBoxSetting,
        dt: &mut DrawTarget,
        x: f32,
        y: f32,
    ) {
        let mut pb = PathBuilder::new();

        // 四角形一つ。太さは1px。色は黒
        let diff = size / 3.;
        let left = x + diff;
        let top = y + diff;
        pb.rect(left, top, diff, diff);

        let path = pb.finish();

        dt.stroke(
            &path,
            &Source::Solid(SolidSource {
                r: 0xff,
                g: 0xff,
                b: 0xff,
                a: 0xff,
            }),
            &StrokeStyle {
                cap: LineCap::Square,
                join: LineJoin::Miter,
                width: 0.2,
                miter_limit: 10.,
                dash_array: vec![],
                dash_offset: 0.,
            },
            &DrawOptions::new(),
        );
    }

    // #[inline]
    // pub fn paint(
    //     &self,
    // ) {
    //     let hwnd: u64 = self.window.id().into();
    //     let hwnd: windows::Win32::Foundation::HWND =
    //         windows::Win32::Foundation::HWND(hwnd as isize);

    //     let mut caption_rect = unsafe { get_caption_button_rect(hwnd.0) };
    //     let window_rect = unsafe { get_extended_frame_bounds(hwnd.0) };
    //     println!("caption_rect: top: {}, left: {}, right: {}, bottom: {}", caption_rect.top, caption_rect.left, caption_rect.right, caption_rect.bottom);
    //     let window_width = window_rect.right - window_rect.left;
    //     let diff = window_width - caption_rect.right;
    //     dbg!(diff);
    //     caption_rect.left += diff;
    //     caption_rect.right += diff;
    //     caption_rect.bottom = caption_rect.top + TOPEXTENDWIDTH;
    //     unsafe { crate::window::set_caption_button_rect(hwnd.0, caption_rect) };

    //     let rect = self.window.outer_size();
    //     println!("rect: {:?}", rect);
    //     let inner_rect: winit::dpi::PhysicalSize<u32> = self.window.inner_size();
    //     println!("inner_rect: {:?}", inner_rect);
    //     let position = self.window.outer_position().unwrap();
    //     let rc_client = windows_sys::Win32::Foundation::RECT {
    //         left: position.x as i32,
    //         top: position.y as i32,
    //         right: position.x as i32 + rect.width as i32,
    //         bottom: position.y as i32 + rect.height as i32,
    //     };
    //     dbg!(hwnd);

    //     println!("rect: {:?}", rect);

    //     unsafe {
    //         // https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-paintstruct
    //         let mut ps = PAINTSTRUCT {
    //             hdc: Default::default(),
    //             fErase: 0,
    //             rcPaint: rc_client,
    //             fRestore: Default::default(),
    //             fIncUpdate: Default::default(),
    //             rgbReserved: Default::default(),
    //         };
    //         let hdc = BeginPaint(hwnd.0, &mut ps);
    //         let h_theme = OpenThemeData(0, encode_wide("CompositedWindow::Window").as_ptr());
    //         if h_theme == 0 {
    //             // Draw standard window frame.
    //             println!("Draw standard window frame");
    //         } else {
    //             // Draw themed window frame.
    //             println!("Draw themed window frame");
    //             let hdc_paint = CreateCompatibleDC(hdc);
    //             if hdc_paint != 0 {
    //                 println!("CreateCompatibleDC succeeded");
    //                 let width = rc_client.right - rc_client.left;
    //                 let height = rc_client.bottom - rc_client.top;

    //                 // Define the BITMAPINFO structure used to draw text.
    //                 // Note that biHeight is negative. This is done because
    //                 // DrawThemeTextEx() needs the bitmap to be in top-to-bottom
    //                 // order.
    //                 const BIT_COUNT: u16 = 32;
    //                 let dib = BITMAPINFO {
    //                     bmiHeader: BITMAPINFOHEADER {
    //                         biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
    //                         biWidth: width,
    //                         biHeight: -height,
    //                         biPlanes: 1,
    //                         biBitCount: BIT_COUNT,
    //                         biCompression: 0,
    //                         biSizeImage: 0,
    //                         biXPelsPerMeter: 0,
    //                         biYPelsPerMeter: 0,
    //                         biClrUsed: 0,
    //                         biClrImportant: 0,
    //                     },
    //                     bmiColors: [RGBQUAD {
    //                         rgbBlue: 100,
    //                         rgbGreen: 0,
    //                         rgbRed: 0,
    //                         rgbReserved: 0,
    //                     }; 1],
    //                 };

    //                 const DIB_RGB_COLORS: u32 = 0;
    //                 let hbm = CreateDIBSection(hdc, &dib, DIB_RGB_COLORS, null_mut(), 0, 0);
    //                 if hbm != 0 {
    //                     println!("CreateDIBSection succeeded");

    //                     let hbm_old = SelectObject(hdc_paint, hbm);

    //                     let dtt_opts = DTTOPTS {
    //                         dwSize: std::mem::size_of::<DTTOPTS>() as u32,
    //                         dwFlags: DTT_COMPOSITED | DTT_GLOWSIZE,
    //                         iGlowSize: 15,
    //                         crText: 20000,
    //                         crBorder: 10000,
    //                         crShadow: 1000,
    //                         iTextShadowType: 0,
    //                         ptShadowOffset: POINT { x: 0, y: 0 },
    //                         iBorderSize: 0,
    //                         iFontPropId: 0,
    //                         iColorPropId: 0,
    //                         iStateId: 0,
    //                         fApplyOverlay: 0,
    //                         pfnDrawTextCallback: None,
    //                         lParam: 0,
    //                     };

    //                     let mut lg_font = LOGFONTW {
    //                         lfHeight: 0,
    //                         lfWidth: 0,
    //                         lfEscapement: 0,
    //                         lfOrientation: 0,
    //                         lfWeight: 0,
    //                         lfItalic: 0,
    //                         lfUnderline: 0,
    //                         lfStrikeOut: 0,
    //                         lfCharSet: 0,
    //                         lfOutPrecision: 0,
    //                         lfClipPrecision: 0,
    //                         lfQuality: 0,
    //                         lfPitchAndFamily: 0,
    //                         lfFaceName: [0; 32],
    //                     };
    //                     let mut h_font_old = 0;
    //                     if GetThemeSysFont(h_theme, TMT_CAPTIONFONT as i32, &mut lg_font) != 0 {
    //                         println!("GetThemeSysFont failed");
    //                     } else {
    //                         println!("CreateFontIndirectW succeeded");
    //                         println!("lfFaceName: {:?}", decode_wide(&lg_font.lfFaceName));
    //                         let h_font = CreateFontIndirectW(&lg_font);
    //                         h_font_old = SelectObject(hdc_paint, h_font);
    //                     }

    //                     let mut rc_paint = rc_client;
    //                     // rc_paint.top += 8;
    //                     // rc_paint.left += 8;
    //                     // rc_paint.right -= 125;
    //                     // rc_paint.bottom = 50;
    //                     if DrawThemeTextEx(
    //                         h_theme,
    //                         hdc_paint,
    //                         0,
    //                         0,
    //                         encode_wide("Title !!").as_ptr(),
    //                         -1,
    //                         // DT_LEFT | DT_WORD_ELLIPSIS,
    //                         DT_LEFT,
    //                         &mut rc_paint,
    //                         &dtt_opts,
    //                     ) != 0
    //                     {
    //                         println!("DrawThemeTextEx failed");
    //                     } else {
    //                         println!("DrawThemeTextEx succeeded");
    //                     }

    //                     // if DrawThemeTextEx(
    //                     //     h_theme,
    //                     //     hdc,
    //                     //     0,
    //                     //     20,
    //                     //     encode_wide("Hello, World").as_ptr(),
    //                     //     -1,
    //                     //     DT_LEFT | DT_WORD_ELLIPSIS,
    //                     //     &mut rc_paint,
    //                     //     &dtt_opts,
    //                     // ) != 0
    //                     // {
    //                     //     println!("DrawThemeTextEx failed");
    //                     // }

    //                     TextOutW(hdc_paint, 0, 0, encode_wide("Hello, World").as_ptr(), 13);
    //                     if BitBlt(hdc, 0, 0, width, height, hdc_paint, 0, 0, SRCCOPY) == 0 {
    //                         println!("BitBlt failed");
    //                     }

    //                     TextOutW(hdc, 100, 0, encode_wide("ハローワールド！！").as_ptr(), 10);

    //                     SelectObject(hdc_paint, hbm_old);
    //                     if h_font_old != 0 {
    //                         SelectObject(hdc_paint, h_font_old);
    //                     }
    //                     DeleteObject(hbm);

    //                     println!("DrawThemeTextEx succeeded")
    //                 } else {
    //                     println!("CreateDIBSection failed");
    //                 }
    //                 DeleteObject(hdc_paint);
    //             }
    //             CloseThemeData(h_theme);
    //         }
    //         // paint_func(hwnd, hdc, rect);
    //         EndPaint(hwnd.0, &ps);
    //     }
    // }
}

// Win32_UI_WindowsAndMessaging
// GetWindowLongPtrW

// UI_WindowManagement_Preview
// AppWindowTitleBar

// Win32_Foundation

pub fn encode_wide(string: impl AsRef<OsStr>) -> Vec<u16> {
    string.as_ref().encode_wide().chain(once(0)).collect()
}

pub fn decode_wide(mut wide_c_string: &[u16]) -> OsString {
    if let Some(null_pos) = wide_c_string.iter().position(|c| *c == 0) {
        wide_c_string = &wide_c_string[..null_pos];
    }

    OsString::from_wide(wide_c_string)
}
