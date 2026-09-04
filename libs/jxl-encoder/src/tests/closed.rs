//! 自分の出力を復号して組み上げた画面そのものを見る

use super::*;

/// 復号器を組み立てるのは非可逆だけ
#[test]
fn a_lossless_encoder_builds_no_decoder() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let lossless = Encoder::new(Vec::new(), WIDTH, HEIGHT, 1, config(color_type)).unwrap();
        assert!(
            lossless.delta().decoder().is_none(),
            "{color_type:?} の可逆が復号器を組み立てている"
        );

        let lossy = Encoder::new(Vec::new(), WIDTH, HEIGHT, 1, lossy(color_type, QUALITY)).unwrap();
        assert!(
            lossy.delta().decoder().is_some(),
            "{color_type:?} の非可逆が復号器を組み立てていない"
        );
    }
}

/// 自前で組んだ画面が、復号器の合成と画素あたり1以内で一致する
#[test]
fn the_screen_follows_the_decoder_within_one_step() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        for sequence in sequences(color_type) {
            let config = lossy(color_type, COARSE_QUALITY);
            let (screens, encoded) = screens(config, &sequence.frames, sequence.durations);
            let decoded = decode(&encoded, color_type);
            let at = format!("{} の {color_type:?}", sequence.name);

            assert_eq!(
                decoded.pixels.len(),
                sequence.frames.len(),
                "{at} で表示フレームが畳まれている"
            );
            assert_eq!(screens.len(), decoded.pixels.len(), "{at} の画面の枚数");
            for (index, (screen, composed)) in screens.iter().zip(&decoded.pixels).enumerate() {
                assert!(
                    max_gap(screen, composed) <= 1,
                    "{at} の {} 枚目の画面が復号器の合成から {} 離れている",
                    index + 1,
                    max_gap(screen, composed)
                );
            }
        }
    }
}

/// 書いた矩形の層は、次のフレームを投入するまでに揃って返る
#[test]
fn every_written_layer_returns_before_the_next_frame() {
    let color_type = ColorType::Rgba8;
    let config = lossy(color_type, COARSE_QUALITY);
    for sequence in sequences(color_type) {
        let encoded = encode_sequence(config, &sequence);
        let decoded = decode(&encoded, color_type);
        let at = &sequence.name;
        assert_eq!(
            decoded.pixels.len(),
            sequence.frames.len(),
            "{at} で表示フレームが畳まれている"
        );

        // 表示フレームごとに書かれた矩形の枚数を、投入した順に積む
        let sources = sources(&decoded.headers);
        let returned: Vec<u64> = (0..sequence.frames.len())
            .map(|shown| sources.iter().filter(|source| **source < shown).count() as u64)
            .collect();

        let mut encoder = Encoder::new(
            Vec::new(),
            WIDTH,
            HEIGHT,
            sequence.frames.len() as u32,
            config,
        )
        .unwrap();
        for (index, (frame, duration)) in sequence.frames.iter().zip(sequence.durations).enumerate()
        {
            encoder.add_frame(frame, *duration).unwrap();
            let layers = encoder.delta().decoder().expect("復号器が無い");
            assert_eq!(
                layers.awaiting(),
                0,
                "{at} の {} 枚目までに層を待っている矩形が残っている",
                index + 1
            );
            assert_eq!(
                layers.returned(),
                returned[index],
                "{at} の {} 枚目までに返った層の枚数",
                index + 1
            );
        }
        encoder.close().unwrap();
        let layers = encoder.delta().decoder().expect("復号器が無い");
        assert_eq!(layers.awaiting(), 0, "{at} の最後の層が返っていない");
        assert_eq!(
            layers.returned(),
            sources.len() as u64,
            "{at} の書いた矩形の枚数"
        );
    }
}
