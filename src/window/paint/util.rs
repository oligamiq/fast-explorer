use raqote::Color;
use windows::{Win32::{Foundation::FALSE, Graphics::Dwm::DwmGetColorizationColor}, UI::ViewManagement::{UIColorType, UISettings}};

// https://learn.microsoft.com/ja-jp/windows/win32/api/shlwapi/nf-shlwapi-shcreateshellpalette
// https://wisdom.sakura.ne.jp/system/winapi/win32/win127.html

/// Get the dwm color of the system.
pub fn get_dwm_color() -> Color {
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

#[derive(Debug, Clone, Copy)]
pub struct SystemAccentColors {
    pub background: Color,
    pub foreground: Color,
    pub accent_dark3: Color,
    pub accent_dark2: Color,
    pub accent_dark1: Color,
    pub accent: Color,
    pub accent_light1: Color,
    pub accent_light2: Color,
    pub accent_light3: Color,
    pub complement: Color,
}

pub fn get_accent_colors() -> SystemAccentColors {
    let ui_settings = UISettings::new().unwrap();
    SystemAccentColors {
        background: ui_settings.GetColorValue(UIColorType::Background).unwrap_or_default().to_raqote_color(),
        foreground: ui_settings.GetColorValue(UIColorType::Foreground).unwrap_or_default().to_raqote_color(),
        accent_dark3: ui_settings.GetColorValue(UIColorType::AccentDark3).unwrap_or_default().to_raqote_color(),
        accent_dark2: ui_settings.GetColorValue(UIColorType::AccentDark2).unwrap_or_default().to_raqote_color(),
        accent_dark1: ui_settings.GetColorValue(UIColorType::AccentDark1).unwrap_or_default().to_raqote_color(),
        accent: ui_settings.GetColorValue(UIColorType::Accent).unwrap_or_default().to_raqote_color(),
        accent_light1: ui_settings.GetColorValue(UIColorType::AccentLight1).unwrap_or_default().to_raqote_color(),
        accent_light2: ui_settings.GetColorValue(UIColorType::AccentLight2).unwrap_or_default().to_raqote_color(),
        accent_light3: ui_settings.GetColorValue(UIColorType::AccentLight3).unwrap_or_default().to_raqote_color(),
        complement: ui_settings.GetColorValue(UIColorType::Complement).unwrap_or_default().to_raqote_color(),
    }
}

trait ToRaqoteColor {
    fn to_raqote_color(&self) -> raqote::Color;
}

impl ToRaqoteColor for windows::UI::Color {
    fn to_raqote_color(&self) -> raqote::Color {
        raqote::Color::new(self.A, self.R, self.G, self.B)
    }
}
