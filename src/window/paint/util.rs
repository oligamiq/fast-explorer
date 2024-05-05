use raqote::Color;
use windows::Win32::{Foundation::FALSE, Graphics::Dwm::DwmGetColorizationColor};

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
