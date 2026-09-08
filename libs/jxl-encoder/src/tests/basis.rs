//! 差分矩形を決めるとき比べる相手の選び分け

use super::*;

/// 直前の投入で変わった画素を持ち越すのは非可逆だけ
#[test]
fn only_a_lossy_encoder_carries_the_previous_change() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let lossless = Encoder::new(Vec::new(), WIDTH, HEIGHT, 1, config(color_type)).unwrap();
        assert!(
            !lossless.delta().carries_the_previous_change(),
            "{color_type:?} の可逆が直前の変化を持ち越している"
        );

        let lossy = Encoder::new(Vec::new(), WIDTH, HEIGHT, 1, lossy(color_type, QUALITY)).unwrap();
        assert!(
            lossy.delta().carries_the_previous_change(),
            "{color_type:?} の非可逆が直前の変化を持ち越していない"
        );
    }
}
