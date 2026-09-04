//! 書き出したバイト列から層を取り出す経路

use super::*;

/// 取り出した層が、そのフレームが書いた矩形で切り出した入力と一致する
#[test]
fn each_layer_is_the_input_cropped_to_its_rect() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        for sequence in sequences(color_type) {
            let encoded = encode_sequence(config(color_type), &sequence);
            let decoded = decode(&encoded, color_type);
            let at = format!("{} の {color_type:?}", sequence.name);
            assert_eq!(rects(&decoded.headers), sequence.rects, "{at}");

            let peeled = peel(color_type, [encoded.as_slice()], &sequence.rects);
            assert_eq!(peeled.len(), sequence.rects.len(), "{at} の層の枚数");
            let sources = sources(&decoded.headers);
            for (index, ((rect, layer), written)) in peeled.iter().zip(&sequence.rects).enumerate()
            {
                let at = format!("{at} の {} 枚目の層", index + 1);
                assert_eq!(*rect, rect_of(*written), "{at} と対になった矩形");
                assert_eq!(
                    layer,
                    &crop(&sequence.frames[sources[index]], color_type, *rect),
                    "{at}"
                );
            }
        }
    }
}
/// バイト列をどこで区切って流し込んでも、同じ層が同じ順で返る
#[test]
fn the_layers_do_not_depend_on_where_the_bytes_are_split() {
    let color_type = ColorType::Rgba8;
    for sequence in sequences(color_type) {
        let chunks = encode_into(
            Chunks::default(),
            config(color_type),
            WIDTH,
            HEIGHT,
            &sequence.frames,
            sequence.durations,
        )
        .0;
        let encoded: Vec<u8> = chunks.concat();
        let written = &sequence.rects;

        let whole = peel(color_type, [encoded.as_slice()], written);
        assert_eq!(whole.len(), written.len(), "{}", sequence.name);
        assert_eq!(
            peel(color_type, encoded.chunks(1), written),
            whole,
            "{} を1バイトずつ流し込んだ層",
            sequence.name
        );
        assert_eq!(
            peel(color_type, chunks.iter().map(Vec::as_slice), written),
            whole,
            "{} を書き出しの区切りで流し込んだ層",
            sequence.name
        );
    }
}

/// 層を貼り合わせると、表示フレームごとに投入フレームへ戻る
///
/// 全面の層はそのままキャンバスになり、矩形の層は土台の枠の写しへ貼る。置き先の枠が
/// 0 でなければ、合成後をその枠へ置く。
#[test]
fn pasting_the_layers_rebuilds_the_input_frames() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        for sequence in sequences(color_type) {
            let encoded = encode_sequence(config(color_type), &sequence);
            let decoded = decode(&encoded, color_type);
            let peeled = peel(color_type, [encoded.as_slice()], &sequence.rects);

            let empty = vec![0u8; (WIDTH * HEIGHT) as usize * color_type.bytes_per_pixel()];
            let mut slots = [empty.clone(), empty.clone(), empty.clone(), empty];
            let mut shown = Vec::new();
            for (header, (rect, layer)) in decoded.headers.iter().zip(&peeled) {
                let canvas = if *rect == rect_of(WHOLE) {
                    layer.clone()
                } else {
                    let mut canvas = slots[header.blending_info.source as usize].clone();
                    paste(&mut canvas, color_type, *rect, layer);
                    canvas
                };
                if header.save_as_reference != 0 {
                    slots[header.save_as_reference as usize] = canvas.clone();
                }
                if header.duration != 0 || header.is_last {
                    shown.push(canvas);
                }
            }

            assert_eq!(
                shown, sequence.frames,
                "{} の {color_type:?} を貼り合わせたキャンバス",
                sequence.name
            );
        }
    }
}

/// 書いた矩形より多く層が返ったら、そこで落ちる
#[test]
#[should_panic(expected = "枚目の層が返った")]
fn a_layer_beyond_the_written_rects_stops_the_decoding() {
    let color_type = ColorType::Rgb8;
    let sequence = &sequences(color_type)[0];
    let encoded = encode_sequence(config(color_type), sequence);

    let rects = &sequence.rects[..sequence.rects.len() - 1];
    peel(color_type, [encoded.as_slice()], rects);
}

/// 書いた矩形より層が少ないまま閉じたら、そこで落ちる
#[test]
#[should_panic(expected = "ストリームが閉じた時点で")]
fn a_stream_that_closes_short_of_the_written_rects_stops_the_decoding() {
    let color_type = ColorType::Rgb8;
    let sequence = &sequences(color_type)[0];
    let encoded = encode_sequence(config(color_type), sequence);

    let mut rects = sequence.rects.clone();
    rects.push(TRAILING_DOT);
    peel(color_type, [encoded.as_slice()], &rects);
}

/// 控えた矩形と大きさの合わない層が返ったら、そこで落ちる
#[test]
#[should_panic(expected = "1 枚目の層が")]
fn a_layer_that_does_not_fill_its_rect_stops_the_decoding() {
    let color_type = ColorType::Rgb8;
    let sequence = &sequences(color_type)[0];
    let encoded = encode_sequence(config(color_type), sequence);

    let mut rects = sequence.rects.clone();
    rects[0] = (0, 0, WIDTH - 1, HEIGHT);
    peel(color_type, [encoded.as_slice()], &rects);
}

/// 非可逆でも層は矩形のぶんだけ返り、αは入力のまま残る
#[test]
fn a_lossy_encoding_returns_a_layer_for_each_rect() {
    let color_type = ColorType::Rgba8;
    let config = Config {
        quality: 40.0,
        ..config(color_type)
    };
    for sequence in sequences(color_type) {
        let encoded = encode_sequence(config, &sequence);
        let decoded = decode(&encoded, color_type);
        let at = format!("{} の非可逆", sequence.name);
        assert_eq!(rects(&decoded.headers), sequence.rects, "{at}");

        let peeled = peel(color_type, [encoded.as_slice()], &sequence.rects);
        assert_eq!(peeled.len(), sequence.rects.len(), "{at} の層の枚数");
        let sources = sources(&decoded.headers);
        let mut moved = false;
        for (index, (rect, layer)) in peeled.iter().enumerate() {
            let source = crop(&sequence.frames[sources[index]], color_type, *rect);
            let at = format!("{at} の {} 枚目の層", index + 1);
            assert_eq!(layer.len(), source.len(), "{at} の大きさ");
            assert_eq!(alpha_channel(layer), alpha_channel(&source), "{at} のα");
            moved |= color_channels(layer) != color_channels(&source);
        }
        assert!(
            moved,
            "{at} のどの層の色も入力と一致していて、非可逆になっていない"
        );
    }
}
