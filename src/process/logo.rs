use libvips::{VipsImage, ops};

use crate::params::WatermarkParams;

pub fn find_make_logo(make: &str, watermark_params: &WatermarkParams) -> Option<VipsImage> {
    let make = make.replace("CORPORATION", "").trim().to_lowercase();
    let [r, g, b] = watermark_params.background;
    if watermark_params.solid_background && !is_dark_color(r, g, b) {
        ops::svgload(&format!("./static/logo/{}-b.svg", make)).ok()
    } else {
        ops::svgload(&format!("./static/logo/{}-w.svg", make)).ok()
    }
}

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
