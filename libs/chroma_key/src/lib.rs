use palette::{Hsl, IntoColor, Srgb};

/// # パラメータ
/// - `image_data`: RGBA画像データ
/// - `key_color`: キー色のRGB値 [R, G, B]
/// - `hue_tolerance`: 色相の許容範囲の「幅」(度数法)
/// - `saturation_tolerance`: 彩度の許容度 (0.0-100.0)
#[inline(always)]
pub fn apply_chroma_key(
    image_data: &mut [u8],
    key_color: [u8; 3],
    hue_tolerance: f32,
    saturation_tolerance: f32,
) {
    let key_rgb = Srgb::new(
        key_color[0] as f32 / 255.0,
        key_color[1] as f32 / 255.0,
        key_color[2] as f32 / 255.0,
    );
    let key_hsl: Hsl = key_rgb.into_color();
    let key_hue = key_hsl.hue.into_positive_degrees();

    let min_saturation = 1.0 - (saturation_tolerance / 100.0).clamp(0.0, 1.0);

    for chunk in image_data.chunks_exact_mut(4) {
        if chunk[0] == key_color[0] && chunk[1] == key_color[1] && chunk[2] == key_color[2] {
            // ピクセルがキーカラーそのものなら、確実にキーイングする
            chunk[3] = 0;
            continue;
        }

        let r = chunk[0] as f32 / 255.0;
        let g = chunk[1] as f32 / 255.0;
        let b = chunk[2] as f32 / 255.0;

        let pixel_rgb = Srgb::new(r, g, b);
        let pixel_hsl: Hsl = pixel_rgb.into_color();

        // 1. 色相が範囲内にあるか判定
        let hue_diff = {
            let diff = (pixel_hsl.hue.into_positive_degrees() - key_hue).abs();
            diff.min(360.0 - diff)
        };
        let is_hue_match = hue_diff <= hue_tolerance / 2.0;

        // 2. 彩度が下限値以上か判定
        let is_saturation_match = pixel_hsl.saturation >= min_saturation;

        if is_hue_match && is_saturation_match {
            chunk[3] = 0;
        }
    }
}
