use palette::{rgb::Rgb, Hsl, Hsv, Hwb, IntoColor as _, Lab, Lch, LinSrgb, Srgb, Xyz};
use raqote::Color;
use windows::Win32::{Foundation::FALSE, Graphics::Dwm::DwmGetColorizationColor};

use crate::window::paint::color_palette::color_palette;

// https://learn.microsoft.com/ja-jp/windows/win32/api/shlwapi/nf-shlwapi-shcreateshellpalette
// https://wisdom.sakura.ne.jp/system/winapi/win32/win127.html

/// Get the accent color of the system.
pub fn get_accent_color() -> Color {
    let mut color = 0;
    let mut opaque = FALSE;

    unsafe { DwmGetColorizationColor(&mut color, &mut opaque) }.unwrap();

    // println!("opaque: {:?}", opaque);

    // 0xAARRGGBB
    return Color::new(
        ((color >> 24) & 0xFF) as u8,
        ((color >> 16) & 0xFF) as u8,
        ((color >> 8) & 0xFF) as u8,
        (color & 0xFF) as u8,
    );
}
