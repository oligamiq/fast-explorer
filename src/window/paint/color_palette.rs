use std::cmp::{max, min};

use raqote::Color;

// https://learn.microsoft.com/ja-jp/windows/apps/design/style/color
// SystemAccentColorLight3
// SystemAccentColorLight2
// SystemAccentColorLight1
// SystemAccentColor
// SystemAccentColorDark1
// SystemAccentColorDark2
// SystemAccentColorDark3

// https://github.com/res2k/Windows10Colors/blob/master/Windows10Colors/Windows10Colors.cpp#L466
pub fn color_palette(color: Color) -> SystemAccentColors {
    let color_hsv: Hsl = Rgb {
        red: color.r(),
        green: color.g(),
        blue: color.b(),
    } .into();
    println!("{:#x?}", color_hsv);
    let light = lighter(&color_hsv, &color_hsv);
    let lighter_ = lighter(&light, &color_hsv);
    let lightest = lighter(&lighter_, &color_hsv);
    let accent = color_hsv;
    let dark = darker(&color_hsv, &color_hsv);
    let darker_ = darker(&dark, &color_hsv);
    let darkest = darker(&darker_, &color_hsv);

    let light3: Rgb = lightest.into();
    let light2: Rgb = lighter_.into();
    let light1: Rgb = light.into();
    let accent: Rgb = accent.into();
    let dark1: Rgb = dark.into();
    let dark2: Rgb = darker_.into();
    let dark3: Rgb = darkest.into();

    SystemAccentColors {
        light3: Color::new(color.a(), light3.red, light3.green, light3.blue),
        light2: Color::new(color.a(), light2.red, light2.green, light2.blue),
        light1: Color::new(color.a(), light1.red, light1.green, light1.blue),
        accent: Color::new(color.a(), accent.red, accent.green, accent.blue),
        dark1: Color::new(color.a(), dark1.red, dark1.green, dark1.blue),
        dark2: Color::new(color.a(), dark2.red, dark2.green, dark2.blue),
        dark3: Color::new(color.a(), dark3.red, dark3.green, dark3.blue),
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SystemAccentColors {
    pub light3: Color,
    pub light2: Color,
    pub light1: Color,
    pub accent: Color,
    pub dark1: Color,
    pub dark2: Color,
    pub dark3: Color,
}

// https://github.com/res2k/Windows10Colors/blob/master/Windows10Colors/Windows10Colors.cpp#L396
fn lighter(prev: &Hsl, base: &Hsl) -> Hsl {
    let mut result = prev.clone();

    // https://learn.microsoft.com/windows-hardware/customize/desktop/unattend/microsoft-windows-shell-setup-themes-windowcolor
    // Shade: 25% of V
    // If V >= 70%, reduce sat to 75% rel
    let v_step = base.lightness / 6;

    result.lightness = min(prev.lightness + v_step, 255);
    if base.lightness >= SL_RANGE * 75 / 100 {
        result.saturation = (result.saturation * 75) / 100;
    }

    result
}

// https://github.com/res2k/Windows10Colors/blob/master/Windows10Colors/Windows10Colors.cpp#L409
fn darker(prev: &Hsl, base: &Hsl) -> Hsl {
    let mut result = prev.clone();

    // Shade: 25% of V
    let v_step = base.lightness / 5;

    // 0x8B / 1.2 = 0x73
    // 0x8B / 6 * (6 - 1) = 0x73
    // 0x8B - (0x8B / 6) = 0x73

    // 0x73 / 1.36 = 0x54

    // 0x92 -> 0x7a -> 0x5b -> 0x38
    // 0x8B -> 0x73 -> 0x54 -> 0x30
    // 0x7f -> 0x66 -> 0x4c -> 0x28

    // 0x8B / 0x73 = 1.21
    // 0x73 / 0x8B = 0.83
    // 0x7f / 0x66 = 1.245
    // 0x66 / 0x7f = 0.80
    // 0x92 / 0x7a = 1.20
    // 0x7a / 0x92 = 0.835

    // 0x73 / 0x54 = 1.36
    // 0x54 / 0x73 = 0.73
    // 0x8B / 0x54 = 1.655
    // 0x54 / 0x8B = 0.60

    // 0x54 / 0x30 = 1.75
    // 0x30 / 0x54 = 0.57
    // 0x8B / 0x30 = 2.9
    // 0x30 / 0x8B = 0.345


    result.lightness = max(prev.lightness - v_step, 0x0000);

    result
}

// add8ff
// 80b9ee
// 559ce4
// 3379d9
// 235a9f
// 174276
// 092642

#[test]
pub fn color_palette_test() {
    let colors = color_palette(Color::new(0x00, 46, 118, 214));
    println!("{:#x?}", colors);

    let colors = color_palette(Color::new(0x00, 0, 0x8B, 0));
    println!("{:#x?}", colors);

    // to hsv and print
    // no rust fmt
    #[rustfmt::skip]
    let vec = vec![
        vec![
            (8, 23, 105), (23, 60, 148), (36, 92, 185), (46, 118, 214), (72, 140, 221), (140, 196, 238), (179, 229, 248)
        ],
        vec![
            (0, 63, 19), (0, 118, 53), (0, 178, 90), (0, 204, 106), (0, 231, 117), (38, 255, 142), (95, 255, 165)
        ],
        vec![
            (111, 3, 6), (158, 9, 18), (210, 14, 30), (232, 17, 35), (239, 39, 51), (244, 103, 98), (251, 157, 139)
        ],
        vec![
            (0, 0x38, 0), (0, 0x5b, 0), (0, 0x7a, 0), (0, 0x92, 0), (0x05, 0xaf, 0), (0x17, 0xfd, 0), (0x44, 0xff,0x2a)
        ],
        vec![
            (0, 0x30, 0), (0, 0x54, 0), (0, 0x73, 0), (0, 0x8B, 0), (0x05, 0xB5, 0), (0x3F, 0xFF, 0x21), (0x7E, 0xFF, 0x5F)
        ],
        vec![
            (0, 0x28, 0), (0, 0x4c, 0), (0, 0x66, 0), (0, 0x7f, 0), (0, 0x99, 0), (0, 0xb2, 0), (0, 0xd6, 0)
        ]
    ];
}

#[derive(Debug, Clone, Copy)]
pub struct Hsl {
    pub hue: i32,
    pub saturation: i32,
    pub lightness: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct Rgb {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

const H_RANGE: i32 = 360;
const SL_RANGE: i32 = 0x8000;

impl Into<Hsl> for Rgb {
    fn into(self) -> Hsl {
        let mut result = Hsl {
            hue: 0,
            saturation: 0,
            lightness: 0,
        };

        // Compute color as HSL. Use range [0..H_RANGE]
        let r = self.red as i32;
        let g = self.green as i32;
        let b = self.blue as i32;

        let max = max(r, max(g, b));
        let min = min(r, min(g, b));
        let diff = max - min;
        if diff != 0 {
            if max == r {
                result.hue = ((g - b) * (H_RANGE / 6)) / diff;
            } else if max == g {
                result.hue = (((b - r) * (H_RANGE / 6)) / diff) + 2 * (H_RANGE / 6);
            } else {
                result.hue = (((r - g) * (H_RANGE / 6)) / diff) + 4 * (H_RANGE / 6);
            }
        }
        if result.hue < 0 {
            result.hue += 6 * (H_RANGE / 6);
        }
        let cnt = (max + min) / 2;
        if cnt < 128 {
            result.saturation = diff * SL_RANGE / (max + min);
        } else {
            result.saturation = diff * SL_RANGE / (2 * 255 - max - min);
        }
        result.lightness = cnt;

        result
    }
}

impl Into<Rgb> for Hsl {
    fn into(self) -> Rgb {
        let r;
        let g;
        let b;
        let chroma = ((SL_RANGE - (2 * self.lightness - SL_RANGE).abs()) * self.saturation) / SL_RANGE;
        let h = self.hue / (H_RANGE / 6);
        match h {
            0 => {
                r = chroma;
                g = 0;
                b = 0;
            }
            1 => {
                r = chroma;
                g = chroma;
                b = 0;
            }
            2 => {
                r = 0;
                g = chroma;
                b = 0;
            }
            3 => {
                r = 0;
                g = chroma;
                b = chroma;
            }
            4 => {
                r = 0;
                g = 0;
                b = chroma;
            }
            5 => {
                r = chroma;
                g = 0;
                b = chroma;
            }
            _ => unreachable!(),
        }
        let min_ = self.lightness - chroma / 2;
        return Rgb {
            red: min(r + min_, 255) as u8,
            green: min(g + min_, 255) as u8,
            blue: min(b + min_, 255) as u8,
        };
    }
}
