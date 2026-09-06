use chrono::Local;
use nom_exif::ExifDateTime;

use lumen_frame::{
    photo::{ExifInfo, Rational},
    process::text::render_exif_template,
};

#[test]
fn test_render_exif_template() {
    let template = "{拍摄日期} {等效焦距}mm f/{光圈} {快门}s ISO{ISO} {曝光补偿}EV";
    let exif_info = ExifInfo {
        created_time: Some(ExifDateTime::Aware(Local::now().into())),
        focal_length_in35mm_film: Some(50),
        exposure_time: Some(Rational::Fraction(1, 250)),
        f_number: Some(Rational::Fraction(18, 10)),
        exposure_bias_value: Some(Rational::Fraction(-1, 3)),
        focal_length: Some(Rational::Fraction(500, 10)),
        isospeed_ratings: Some(100),
        ..Default::default()
    };
    let text = render_exif_template(template, &exif_info, "%Y/%m/%d %H:%M:%S");
    dbg!(text);
}
