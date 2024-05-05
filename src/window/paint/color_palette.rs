use palette::{rgb::Rgb, Hsv, IntoColor, Srgb};
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
    let color_hsv: Hsv = Srgb::new(color.r(), color.g(), color.b())
        .into_format()
        .into_color();
    let light = lighter(&color_hsv, &color_hsv);
    let lighter_ = lighter(&light, &color_hsv);
    let lightest = lighter(&lighter_, &color_hsv);
    let dark = darker(&color_hsv, &color_hsv);
    let darker_ = darker(&dark, &color_hsv);
    let darkest = darker(&darker_, &color_hsv);

    let light3: Srgb<u8> = IntoColor::<Rgb>::into_color(lightest).into_format();
    let light2: Srgb<u8> = IntoColor::<Rgb>::into_color(lighter_).into_format();
    let light1: Srgb<u8> = IntoColor::<Rgb>::into_color(light).into_format();
    let accent: Srgb<u8> = IntoColor::<Rgb>::into_color(color_hsv).into_format();
    let dark1: Srgb<u8> = IntoColor::<Rgb>::into_color(dark).into_format();
    let dark2: Srgb<u8> = IntoColor::<Rgb>::into_color(darker_).into_format();
    let dark3: Srgb<u8> = IntoColor::<Rgb>::into_color(darkest).into_format();

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

pub fn min(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}

pub fn max(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}

// https://github.com/res2k/Windows10Colors/blob/master/Windows10Colors/Windows10Colors.cpp#L396
fn lighter(prev: &Hsv, base: &Hsv) -> Hsv {
    let mut result = prev.clone();

    // https://learn.microsoft.com/windows-hardware/customize/desktop/unattend/microsoft-windows-shell-setup-themes-windowcolor
    // Shade: 25% of V
    // If V >= 70%, reduce sat to 75% rel
    let v_step = base.value / 4.;

    result.value = min(prev.value + v_step, 0x8000 as f32);
    if result.value >= 0x6000 as f32 {
        result.saturation = (((prev.saturation * 192.) as u32) >> 8) as f32;
    };

    result
}

// https://github.com/res2k/Windows10Colors/blob/master/Windows10Colors/Windows10Colors.cpp#L409
fn darker(prev: &Hsv, base: &Hsv) -> Hsv {
    let mut result = prev.clone();

    // Shade: 25% of V
    let v_step = base.value / 4.;

    result.value = max(prev.value - v_step, 0x0000 as f32);

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
