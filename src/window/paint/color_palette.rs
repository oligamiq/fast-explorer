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
    let color_hsv: Hsv = Rgb {
        red: color.r(),
        green: color.g(),
        blue: color.b(),
    } .into();
    let light = lighter(&color_hsv, &color_hsv);
    let lighter_ = lighter(&light, &color_hsv);
    let lightest = lighter(&lighter_, &color_hsv);
    let dark = darker(&color_hsv, &color_hsv);
    let darker_ = darker(&dark, &color_hsv);
    let darkest = darker(&darker_, &color_hsv);

    let light3: Rgb = lightest.into();
    let light2: Rgb = lighter_.into();
    let light1: Rgb = light.into();
    let dark1: Rgb = dark.into();
    let dark2: Rgb = darker_.into();
    let dark3: Rgb = darkest.into();

    SystemAccentColors {
        light3: Color::new(color.a(), light3.red, light3.green, light3.blue),
        light2: Color::new(color.a(), light2.red, light2.green, light2.blue),
        light1: Color::new(color.a(), light1.red, light1.green, light1.blue),
        accent: color,
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
fn lighter(prev: &Hsv, base: &Hsv) -> Hsv {
    let mut result = prev.clone();

    // https://learn.microsoft.com/windows-hardware/customize/desktop/unattend/microsoft-windows-shell-setup-themes-windowcolor
    // Shade: 25% of V
    // If V >= 70%, reduce sat to 75% rel
    let v_step = base.value / 4;

    result.value = min(prev.value + v_step, 0x8000);
    if result.value >= 0x6000 {
        result.saturation = (prev.saturation * 192) >> 8;
    };

    result
}

// https://github.com/res2k/Windows10Colors/blob/master/Windows10Colors/Windows10Colors.cpp#L409
fn darker(prev: &Hsv, base: &Hsv) -> Hsv {
    let mut result = prev.clone();

    // Shade: 25% of V
    let v_step = base.value / 4;

    result.value = max(prev.value - v_step, 0x0000);

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

    let colors = color_palette(Color::new(0x00, 0, 139, 0));
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
            (0, 48, 0), (0, 84, 0), (0, 115, 0), (0, 139, 0), (5, 181, 0), (63, 255, 33), (126, 255, 95)
        ]
    ];
}

#[derive(Debug, Clone, Copy)]
pub struct Hsv {
    pub hue: i32,
    pub saturation: i32,
    pub value: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct Rgb {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl Into<Hsv> for Rgb {
    fn into(self) -> Hsv {
        let mut result = Hsv {
            hue: 0,
            saturation: 0,
            value: 0,
        };

        // Compute color as HSV. Use range [0..0x8000]
        let r = self.red as i32 * 0x8000 / 255;
        let g = self.green as i32 * 0x8000 / 255;
        let b = self.blue as i32 * 0x8000 / 255;

        let max = max(r, max(g, b));
        let min = min(r, min(g, b));
        let diff = max - min;
        if diff != 0 {
            if max == r {
                result.hue = ((g - b) * 0x8000) / diff;
                if result.hue < 0 {
                    result.hue += 6 * 0x8000;
                }
            } else if max == g {
                result.hue = (((b - r) * 0x8000) / diff) + 2 * 0x8000;
            } else {
                result.hue = (((r - g) * 0x8000) / diff) + 4 * 0x8000;
            }
        }
        result.value = max;
        if result.value != 0 {
            result.saturation = (diff * 0x8000) / result.value
        }

        result
    }
}

impl Into<Rgb> for Hsv {
    fn into(self) -> Rgb {
        let r;
        let g;
        let b;
        let chroma = (self.value * self.saturation) / 0x8000;
        let second = chroma * (0x8000 - ((self.hue % (2 * 0x8000)) - 0x8000).abs()) / 0x8000;
        match self.hue / 0x8000 {
            0 => {
                r = chroma;
                g = second;
                b = 0;
            }
            1 => {
                r = second;
                g = chroma;
                b = 0;
            }
            2 => {
                r = 0;
                g = chroma;
                b = second;
            }
            3 => {
                r = 0;
                g = second;
                b = chroma;
            }
            4 => {
                r = second;
                g = 0;
                b = chroma;
            }
            5 => {
                r = chroma;
                g = 0;
                b = second;
            }
            _ => unreachable!(),
        }
        let min_ = self.value - chroma;
        return Rgb {
            red: min(((r + min_) * 255) / 0x8000, 255) as u8,
            green: min(((g + min_) * 255) / 0x8000, 255) as u8,
            blue: min(((b + min_) * 255) / 0x8000, 255) as u8,
        };
    }
}
