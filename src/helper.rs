use crate::params::WatermarkParams;

/// 自动判断背景色，适合使用true黑色字体还是false白色
pub fn auto_color(watermark_params: &WatermarkParams) -> bool {
    let [r, g, b] = watermark_params.background;
    watermark_params.solid_background && !is_dark_color(r, g, b)
}

// 判断是否是暗色
fn is_dark_color(r: u8, g: u8, b: u8) -> bool {
    let r = r as f64 / 255.0;
    let g = g as f64 / 255.0;
    let b = b as f64 / 255.0;

    let r = if r <= 0.04045 {
        r / 12.92
    } else {
        ((r + 0.055) / 1.055).powf(2.4)
    };

    let g = if g <= 0.04045 {
        g / 12.92
    } else {
        ((g + 0.055) / 1.055).powf(2.4)
    };

    let b = if b <= 0.04045 {
        b / 12.92
    } else {
        ((b + 0.055) / 1.055).powf(2.4)
    };

    let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;

    luminance < 0.5
}

/// 罗马数字转换
pub fn to_roman(mut num: i32) -> String {
    let values = [1000, 900, 500, 400, 100, 90, 50, 40, 10, 9, 5, 4, 1];
    let symbols = [
        "M", "CM", "D", "CD", "C", "XC", "L", "XL", "X", "IX", "V", "IV", "I",
    ];
    let mut result = String::new();

    for (i, &value) in values.iter().enumerate() {
        while num >= value {
            result.push_str(symbols[i]);
            num -= value;
        }
    }
    result
}

// svg 替换xml符号
pub fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
