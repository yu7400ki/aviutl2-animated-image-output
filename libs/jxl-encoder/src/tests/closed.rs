//! 非可逆で、差分矩形を自分の出力を復号した画面から決める経路

use super::*;
use anim_core::tolerance;

/// 画面が入力から離れてよい量を測る品質。許容量は3になる
const QUALITY: f32 = 90.0;

/// 量子化の誤差が許容量を超える品質。許容量は12になる
const COARSE_QUALITY: f32 = 40.0;

/// 一様な背景の値。平坦な面は許容量の内で復号される
const FLAT: u8 = 100;

/// 背景から大きく離れた値
const PAINT: u8 = 200;

fn lossy(color_type: ColorType, quality: f32) -> Config {
    Config {
        quality,
        ..config(color_type)
    }
}

/// 一様な色で埋めた不透明なフレーム
fn flat(color_type: ColorType, value: u8) -> Vec<u8> {
    let mut frame = vec![value; (WIDTH * HEIGHT) as usize * color_type.bytes_per_pixel()];
    if color_type == ColorType::Rgba8 {
        for pixel in frame.chunks_exact_mut(4) {
            pixel[3] = 0xFF;
        }
    }
    frame
}

/// `block` の範囲の色を `value` で塗る
fn paint(frame: &mut [u8], color_type: ColorType, block: (u32, u32, u32, u32), value: u8) {
    let (x, y, width, height) = block;
    let bytes_per_pixel = color_type.bytes_per_pixel();
    for row in 0..height as usize {
        let start = ((y as usize + row) * WIDTH as usize + x as usize) * bytes_per_pixel;
        for pixel in
            frame[start..start + width as usize * bytes_per_pixel].chunks_exact_mut(bytes_per_pixel)
        {
            pixel[..3].fill(value);
        }
    }
}

/// `base` の上に `block` を `value` で塗ったフレーム
fn painted(base: &[u8], color_type: ColorType, block: (u32, u32, u32, u32), value: u8) -> Vec<u8> {
    let mut frame = base.to_vec();
    paint(&mut frame, color_type, block, value);
    frame
}

/// 全チャネルを `step` だけ持ち上げたフレーム
fn lifted(frame: &[u8], color_type: ColorType, step: u8) -> Vec<u8> {
    frame
        .chunks_exact(color_type.bytes_per_pixel())
        .flat_map(|pixel| {
            let mut pixel = pixel.to_vec();
            for channel in &mut pixel[..3] {
                *channel = channel.saturating_add(step);
            }
            pixel
        })
        .collect()
}

/// 2つの面で最も離れた画素の隔たり
fn max_gap(a: &[u8], b: &[u8]) -> u8 {
    a.iter()
        .zip(b)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0)
}

/// フレームを1枚ずつ投入し、表示フレームごとの画面と書き出したバイト列を集める
fn screens(config: Config, frames: &[Vec<u8>], durations: &[u32]) -> (Vec<Vec<u8>>, Vec<u8>) {
    let mut encoder = Encoder::new(Vec::new(), WIDTH, HEIGHT, frames.len() as u32, config).unwrap();
    let mut screens = Vec::new();
    for (frame, duration) in frames.iter().zip(durations) {
        encoder.add_frame(frame, *duration).unwrap();
        screens.push(encoder.delta().screen().expect("復号器が無い").to_vec());
    }
    encoder.close().unwrap();
    screens.push(encoder.delta().screen().expect("復号器が無い").to_vec());

    // 先頭フレームはまだ書き出されておらず、画面に出ていない
    screens.remove(0);
    (screens, encoder.finish().unwrap())
}

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

/// 入力が変わらなくても、画面が許容量を超えて離れていれば書き直す
///
/// 3枚目は2枚目と同じ入力で、2枚目が書いた矩形の外だけが画面と離れている。
/// 比べる相手が入力なら、その隔たりは見えず表示時間へ畳まれる。
#[test]
fn an_unchanged_input_is_rewritten_when_the_screen_drifts() {
    const BLOCK: (u32, u32, u32, u32) = (10, 6, 24, 18);

    let color_type = ColorType::Rgba8;
    let base = noise_rgba(WIDTH, HEIGHT);
    let moved = painted(&base, color_type, BLOCK, PAINT);
    let frames = vec![base, moved.clone(), moved];

    let encoded = encode_frames(
        lossy(color_type, COARSE_QUALITY),
        WIDTH,
        HEIGHT,
        &frames,
        &[3, 5, 7],
    );
    let decoded = decode(&encoded, color_type);

    assert_eq!(
        ticks(&decoded),
        [3, 5, 7],
        "同じ入力が表示時間へ畳まれていて、画面と比べていない"
    );
}

/// 許容量に収まる変化は矩形を立てず、表示時間へ畳まれる
///
/// 2枚目が書いた矩形の外は復号結果と、内は仮置きした入力と比べられる。
#[test]
fn a_change_within_the_tolerance_folds_into_the_duration() {
    const BLOCK: (u32, u32, u32, u32) = (10, 6, 24, 18);

    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let base = flat(color_type, FLAT);
        let block = painted(&base, color_type, BLOCK, PAINT);
        let lifted = lifted(&block, color_type, tolerance(QUALITY));
        let frames = vec![base, block, lifted];

        let encoded = encode_frames(
            lossy(color_type, QUALITY),
            WIDTH,
            HEIGHT,
            &frames,
            &[3, 5, 7],
        );
        let decoded = decode(&encoded, color_type);

        assert_eq!(
            rects(&decoded.headers),
            [WHOLE, BLOCK],
            "{color_type:?} で許容量に収まる変化が矩形を立てている"
        );
        assert_eq!(ticks(&decoded), [3, 12], "{color_type:?}");
    }
}

/// 保留中のフレームが書く矩形は、比べる相手の画面に仮置きされる
///
/// 仮置きが無いと、2枚目の矩形が3枚目でもう一度書き直される。
#[test]
fn the_pending_frame_is_placed_on_the_screen() {
    const BLOCK: (u32, u32, u32, u32) = (10, 6, 24, 18);

    let color_type = ColorType::Rgb8;
    let base = flat(color_type, FLAT);
    let block = painted(&base, color_type, BLOCK, PAINT);
    let frames = vec![base, block.clone(), block];

    let encoded = encode_frames(
        lossy(color_type, QUALITY),
        WIDTH,
        HEIGHT,
        &frames,
        &[3, 5, 7],
    );
    let decoded = decode(&encoded, color_type);

    assert_eq!(rects(&decoded.headers), [WHOLE, BLOCK]);
    assert_eq!(ticks(&decoded), [3, 12]);
}

/// 2つ前の画面へ戻したフレームは、その画面を比べる相手に残す
///
/// 戻した先を取り違えると、消えたはずの重なりが画面に残る。
#[test]
fn a_frame_restored_from_two_back_keeps_the_older_screen() {
    const POPUP: (u32, u32, u32, u32) = (10, 6, 24, 18);

    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let base = flat(color_type, FLAT);
        let covered = painted(&base, color_type, POPUP, PAINT);
        let frames = vec![base.clone(), covered, base.clone(), base];

        let encoded = encode_frames(
            lossy(color_type, QUALITY),
            WIDTH,
            HEIGHT,
            &frames,
            &[3, 5, 7, 11],
        );
        let decoded = decode(&encoded, color_type);
        let headers = &decoded.headers;

        assert_eq!(
            rects(headers),
            [WHOLE, POPUP, (0, 0, 1, 1)],
            "{color_type:?}"
        );
        // 最終フレームは置き先の欄を持たないので、土台を先頭フレームの置き先で読む
        assert_eq!(
            headers[2].blending_info.source, headers[0].save_as_reference,
            "{color_type:?} の3枚目が2つ前のキャンバスを土台にしていない"
        );
        assert_eq!(
            ticks(&decoded),
            [3, 5, 18],
            "{color_type:?} の4枚目が畳まれていない"
        );
    }
}

/// 許容量に収まる変化は、矩形を割る地図にも入らない
#[test]
fn a_change_within_the_tolerance_stays_out_of_the_cut() {
    const CORNERS: [(u32, u32, u32, u32); 2] = [(2, 2, 6, 6), (34, 20, 6, 6)];
    const MIDDLE: (u32, u32, u32, u32) = (20, 10, 6, 6);

    let color_type = ColorType::Rgb8;
    let base = flat(color_type, FLAT);
    let mut scattered = base.clone();
    for corner in CORNERS {
        paint(&mut scattered, color_type, corner, PAINT);
    }
    paint(
        &mut scattered,
        color_type,
        MIDDLE,
        FLAT + tolerance(QUALITY),
    );
    let frames = vec![base, scattered];

    let encoded = encode_frames(lossy(color_type, QUALITY), WIDTH, HEIGHT, &frames, &[3, 5]);
    let decoded = decode(&encoded, color_type);

    assert_eq!(rects(&decoded.headers), [WHOLE, CORNERS[0], CORNERS[1]]);
}

/// 先頭フレームは、続く変化がどれだけ小さくても全面で書かれる
#[test]
fn the_first_lossy_frame_covers_the_canvas() {
    const DOT: (u32, u32, u32, u32) = (1, 2, 1, 1);

    let color_type = ColorType::Rgb8;
    let base = flat(color_type, FLAT);
    let dotted = painted(&base, color_type, DOT, PAINT);
    let frames = vec![base, dotted];

    let encoded = encode_frames(lossy(color_type, QUALITY), WIDTH, HEIGHT, &frames, &[3, 5]);
    let decoded = decode(&encoded, color_type);

    assert_eq!(rects(&decoded.headers), [WHOLE, DOT]);
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
