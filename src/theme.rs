use serde::{Deserialize, Serialize};
use xilem::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AppearanceMode {
    #[default]
    System,
    Light,
    Dark,
}

impl AppearanceMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "system" => Some(Self::System),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThemeColor {
    #[default]
    Blue,
    Red,
    Green,
    Purple,
    Orange,
    Teal,
    Pink,
    Neutral,
}

impl ThemeColor {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "blue" => Some(Self::Blue),
            "red" => Some(Self::Red),
            "green" => Some(Self::Green),
            "purple" => Some(Self::Purple),
            "orange" => Some(Self::Orange),
            "teal" => Some(Self::Teal),
            "pink" => Some(Self::Pink),
            "neutral" | "gray" | "grey" => Some(Self::Neutral),
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Blue => "Blue",
            Self::Red => "Red",
            Self::Green => "Green",
            Self::Purple => "Purple",
            Self::Orange => "Orange",
            Self::Teal => "Teal",
            Self::Pink => "Pink",
            Self::Neutral => "Neutral",
        }
    }

    pub const fn seed(self) -> Rgb {
        match self {
            Self::Blue => Rgb::new(0x00, 0x78, 0xd4),
            Self::Red => Rgb::new(0xd1, 0x34, 0x38),
            Self::Green => Rgb::new(0x10, 0x7c, 0x41),
            Self::Purple => Rgb::new(0x88, 0x17, 0x98),
            Self::Orange => Rgb::new(0xca, 0x50, 0x10),
            Self::Teal => Rgb::new(0x03, 0x83, 0x87),
            Self::Pink => Rgb::new(0xc2, 0x39, 0xb3),
            Self::Neutral => Rgb::new(0x60, 0x60, 0x60),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeSettings {
    #[serde(default)]
    pub appearance: AppearanceMode,
    #[serde(default)]
    pub color: ThemeColor,
    #[serde(default = "default_intensity")]
    pub intensity: u8,
}

pub const fn default_intensity() -> u8 {
    72
}

impl Default for ThemeSettings {
    fn default() -> Self {
        Self {
            appearance: AppearanceMode::System,
            color: ThemeColor::Blue,
            intensity: default_intensity(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct ThemePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appearance: Option<AppearanceMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ThemeColor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intensity: Option<u8>,
}

impl ThemePatch {
    pub fn apply(self, mut settings: ThemeSettings) -> ThemeSettings {
        if let Some(appearance) = self.appearance {
            settings.appearance = appearance;
        }
        if let Some(color) = self.color {
            settings.color = color;
        }
        if let Some(intensity) = self.intensity {
            settings.intensity = intensity.min(100);
        }
        settings
    }

    pub const fn is_empty(self) -> bool {
        self.appearance.is_none() && self.color.is_none() && self.intensity.is_none()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Rgb {
    r: f64,
    g: f64,
    b: f64,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self {
            r: r as f64 / 255.0,
            g: g as f64 / 255.0,
            b: b as f64 / 255.0,
        }
    }

    pub fn color(self) -> Color {
        Color::from_rgb8(to_u8(self.r), to_u8(self.g), to_u8(self.b))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ThemePalette {
    pub window: Color,
    pub chrome: Color,
    pub surface: Color,
    pub sidebar: Color,
    pub header: Color,
    pub border: Color,
    pub border_strong: Color,
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub accent_hover: Color,
    pub accent_pressed: Color,
    pub accent_soft: Color,
    pub accent_text: Color,
    pub focus: Color,
    pub tab_active: Color,
    pub tab_inactive: Color,
    pub icon_folder: Color,
    pub icon_file: Color,
    pub icon_link: Color,
}
impl ThemePalette {
    pub fn generate(settings: ThemeSettings, system_dark: bool) -> Self {
        let dark = match settings.appearance {
            AppearanceMode::System => system_dark,
            AppearanceMode::Light => false,
            AppearanceMode::Dark => true,
        };
        Self::from_seed(
            settings.color.seed(),
            dark,
            f64::from(settings.intensity.min(100)) / 100.0,
        )
    }

    pub fn from_seed(seed: Rgb, dark: bool, intensity: f64) -> Self {
        let strength = intensity.clamp(0.0, 1.0);
        let accent = if dark {
            mix_oklab(seed, Rgb::new(255, 255, 255), 0.08)
        } else {
            seed
        };
        let base = if dark {
            Rgb::new(30, 30, 30)
        } else {
            Rgb::new(255, 255, 255)
        };
        let window = tint(base, seed, strength * if dark { 0.12 } else { 0.10 });
        let chrome = tint(
            if dark {
                Rgb::new(36, 36, 36)
            } else {
                Rgb::new(247, 247, 247)
            },
            seed,
            strength * if dark { 0.20 } else { 0.16 },
        );
        let surface = tint(
            if dark {
                Rgb::new(28, 28, 28)
            } else {
                Rgb::new(255, 255, 255)
            },
            seed,
            strength * if dark { 0.09 } else { 0.065 },
        );
        let sidebar = tint(
            if dark {
                Rgb::new(33, 33, 33)
            } else {
                Rgb::new(248, 248, 248)
            },
            seed,
            strength * if dark { 0.30 } else { 0.24 },
        );
        let header = tint(
            if dark {
                Rgb::new(40, 40, 40)
            } else {
                Rgb::new(250, 250, 250)
            },
            seed,
            strength * if dark { 0.24 } else { 0.18 },
        );
        let text = if dark {
            Rgb::new(244, 244, 244)
        } else {
            Rgb::new(30, 30, 30)
        };
        let muted_base = if dark {
            Rgb::new(184, 184, 184)
        } else {
            Rgb::new(98, 98, 98)
        };
        let muted = mix_oklab(muted_base, seed, strength * 0.08);
        let border = tint(
            if dark {
                Rgb::new(72, 72, 72)
            } else {
                Rgb::new(222, 222, 222)
            },
            seed,
            strength * 0.18,
        );
        let border_strong = tint(
            if dark {
                Rgb::new(104, 104, 104)
            } else {
                Rgb::new(184, 184, 184)
            },
            seed,
            strength * 0.30,
        );
        let accent_soft = mix_oklab(
            base,
            accent,
            0.10 + strength * if dark { 0.38 } else { 0.30 },
        );
        let accent_hover = mix_oklab(
            accent,
            if dark {
                Rgb::new(255, 255, 255)
            } else {
                Rgb::new(0, 0, 0)
            },
            0.10,
        );
        let accent_pressed = mix_oklab(
            accent,
            if dark {
                Rgb::new(255, 255, 255)
            } else {
                Rgb::new(0, 0, 0)
            },
            0.20,
        );
        let accent_text = best_text_for(accent);
        let tab_active = tint(surface, seed, strength * if dark { 0.12 } else { 0.08 });
        let tab_inactive = tint(
            if dark {
                Rgb::new(43, 43, 43)
            } else {
                Rgb::new(238, 238, 238)
            },
            seed,
            strength * if dark { 0.32 } else { 0.28 },
        );
        Self {
            window: window.color(),
            chrome: chrome.color(),
            surface: surface.color(),
            sidebar: sidebar.color(),
            header: header.color(),
            border: border.color(),
            border_strong: border_strong.color(),
            text: text.color(),
            muted: muted.color(),
            accent: accent.color(),
            accent_hover: accent_hover.color(),
            accent_pressed: accent_pressed.color(),
            accent_soft: accent_soft.color(),
            accent_text: accent_text.color(),
            focus: accent.color(),
            tab_active: tab_active.color(),
            tab_inactive: tab_inactive.color(),
            icon_folder: mix_oklab(Rgb::new(243, 199, 79), seed, strength * 0.22).color(),
            icon_file: mix_oklab(
                if dark {
                    Rgb::new(145, 171, 195)
                } else {
                    Rgb::new(100, 130, 160)
                },
                seed,
                0.20 + strength * 0.36,
            )
            .color(),
            icon_link: accent.color(),
        }
    }
}
fn tint(base: Rgb, seed: Rgb, amount: f64) -> Rgb {
    mix_oklab(base, seed, amount)
}

fn mix_oklab(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let a = rgb_to_oklab(a);
    let b = rgb_to_oklab(b);
    oklab_to_rgb(Oklab {
        l: lerp(a.l, b.l, t),
        a: lerp(a.a, b.a, t),
        b: lerp(a.b, b.b, t),
    })
}

#[derive(Clone, Copy)]
struct Oklab {
    l: f64,
    a: f64,
    b: f64,
}

fn rgb_to_oklab(rgb: Rgb) -> Oklab {
    let r = srgb_to_linear(rgb.r);
    let g = srgb_to_linear(rgb.g);
    let b = srgb_to_linear(rgb.b);
    let l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
    let m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b;
    let s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;
    let l = l.cbrt();
    let m = m.cbrt();
    let s = s.cbrt();
    Oklab {
        l: 0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        a: 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        b: 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    }
}

fn oklab_to_rgb(lab: Oklab) -> Rgb {
    let l = (lab.l + 0.3963377774 * lab.a + 0.2158037573 * lab.b).powi(3);
    let m = (lab.l - 0.1055613458 * lab.a - 0.0638541728 * lab.b).powi(3);
    let s = (lab.l - 0.0894841775 * lab.a - 1.2914855480 * lab.b).powi(3);
    let r = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s;
    let g = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s;
    let b = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s;
    Rgb {
        r: linear_to_srgb(r).clamp(0.0, 1.0),
        g: linear_to_srgb(g).clamp(0.0, 1.0),
        b: linear_to_srgb(b).clamp(0.0, 1.0),
    }
}
fn best_text_for(background: Rgb) -> Rgb {
    let white = Rgb::new(255, 255, 255);
    let black = Rgb::new(0, 0, 0);
    if contrast_ratio(background, white) >= contrast_ratio(background, black) {
        white
    } else {
        black
    }
}

fn contrast_ratio(a: Rgb, b: Rgb) -> f64 {
    let la = relative_luminance(a);
    let lb = relative_luminance(b);
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

fn relative_luminance(rgb: Rgb) -> f64 {
    0.2126 * srgb_to_linear(rgb.r) + 0.7152 * srgb_to_linear(rgb.g) + 0.0722 * srgb_to_linear(rgb.b)
}

fn srgb_to_linear(v: f64) -> f64 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
fn linear_to_srgb(v: f64) -> f64 {
    if v <= 0.0031308 {
        12.92 * v
    } else {
        1.055 * v.max(0.0).powf(1.0 / 2.4) - 0.055
    }
}
fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t.clamp(0.0, 1.0)
}
fn to_u8(v: f64) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

pub struct Layout;
impl Layout {
    #[cfg(not(target_os = "android"))]
    pub const TAB_HEIGHT: f64 = 38.0;
    #[cfg(target_os = "android")]
    pub const TAB_HEIGHT: f64 = 48.0;
    pub const TAB_WIDTH: f64 = 170.0;
    #[cfg(not(target_os = "android"))]
    pub const TAB_LAYOUT_BUDGET: f64 = 270.0;
    #[cfg(target_os = "android")]
    pub const TAB_LAYOUT_BUDGET: f64 = 220.0;
    pub const TAB_STRIP_MAX: f64 = 300.0;
    #[cfg(not(target_os = "android"))]
    pub const TAB_CLOSE_WIDTH: f64 = 26.0;
    #[cfg(target_os = "android")]
    pub const TAB_CLOSE_WIDTH: f64 = 26.0;
    pub const CAPTION_BUTTON_WIDTH: f64 = 46.0;
    pub const RESIZE_HIT_SIZE: f64 = 5.0;
    pub const RESIZE_CORNER_SIZE: f64 = 10.0;
    pub const ADDRESS_HEIGHT: f64 = 44.0;
    pub const ACTION_HEIGHT: f64 = 38.0;
    #[cfg(not(target_os = "android"))]
    pub const LOCATION_FIELD_HEIGHT: f64 = 32.0;
    #[cfg(target_os = "android")]
    pub const LOCATION_FIELD_HEIGHT: f64 = 48.0;
    pub const SEARCH_GROUP_WIDTH: f64 = 300.0;
    pub const SETTINGS_CONTENT_WIDTH: f64 = 820.0;
    pub const STATUS_HEIGHT: f64 = 24.0;
    pub const SIDEBAR_MIN: f64 = 176.0;
    pub const CONTENT_MIN: f64 = 550.0;
    pub const SIDEBAR_FRACTION: f64 = 0.18;
    pub const HEADER_HEIGHT: f64 = 30.0;
    #[cfg(not(target_os = "android"))]
    pub const ROW_HEIGHT: f64 = 32.0;
    #[cfg(target_os = "android")]
    pub const ROW_HEIGHT: f64 = 56.0;
    #[cfg(not(target_os = "android"))]
    pub const ICON_WIDTH: f64 = 28.0;
    #[cfg(target_os = "android")]
    pub const ICON_WIDTH: f64 = 36.0;
    pub const NAME_WIDTH: f64 = 280.0;
    pub const TYPE_WIDTH: f64 = 150.0;
    pub const SIZE_WIDTH: f64 = 92.0;
    #[cfg(not(target_os = "android"))]
    pub const TOOL_HEIGHT: f64 = 30.0;
    #[cfg(target_os = "android")]
    pub const TOOL_HEIGHT: f64 = 48.0;
    #[cfg(not(target_os = "android"))]
    pub const NAV_WIDTH: f64 = 32.0;
    #[cfg(target_os = "android")]
    pub const NAV_WIDTH: f64 = 48.0;
    #[cfg(not(target_os = "android"))]
    pub const RADIUS: f64 = 4.0;
    #[cfg(target_os = "android")]
    pub const RADIUS: f64 = 8.0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_accent_text_meets_contrast_target() {
        for color in [
            ThemeColor::Blue,
            ThemeColor::Red,
            ThemeColor::Green,
            ThemeColor::Purple,
            ThemeColor::Orange,
            ThemeColor::Teal,
            ThemeColor::Pink,
            ThemeColor::Neutral,
        ] {
            for dark in [false, true] {
                let seed = color.seed();
                let accent = if dark {
                    mix_oklab(seed, Rgb::new(255, 255, 255), 0.08)
                } else {
                    seed
                };
                let text = best_text_for(accent);
                assert!(
                    contrast_ratio(accent, text) >= 4.5,
                    "{color:?} dark={dark} did not meet contrast target"
                );
            }
        }
    }

    #[test]
    fn theme_settings_support_named_colors_and_defaults() {
        let settings = ThemeSettings {
            appearance: AppearanceMode::Dark,
            color: ThemeColor::Red,
            intensity: 88,
        };
        let json = serde_json::to_string(&settings).expect("serialize theme settings");
        let restored: ThemeSettings =
            serde_json::from_str(&json).expect("deserialize theme settings");
        assert_eq!(restored, settings);
        let defaulted: ThemeSettings = serde_json::from_str("{}").expect("default fields");
        assert_eq!(defaulted, ThemeSettings::default());
        assert_eq!(defaulted.intensity, 72);
    }

    #[test]
    fn theme_patch_is_partial_and_clamps_intensity() {
        let base = ThemeSettings {
            appearance: AppearanceMode::Light,
            color: ThemeColor::Blue,
            intensity: 35,
        };
        let patched = ThemePatch {
            color: Some(ThemeColor::Purple),
            intensity: Some(255),
            ..ThemePatch::default()
        }
        .apply(base);
        assert_eq!(patched.appearance, AppearanceMode::Light);
        assert_eq!(patched.color, ThemeColor::Purple);
        assert_eq!(patched.intensity, 100);
    }
}
