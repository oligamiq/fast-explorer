use windows::Win32::{Foundation::FALSE, Graphics::Dwm::DwmGetColorizationColor};

pub fn get_accent_color() {
    let mut color = 0;
    let mut opaque = FALSE;

    unsafe { DwmGetColorizationColor(&mut color, &mut opaque) }.unwrap();

    // 0xAARRGGBB
    println!("Color: {:#X}", color);
}
