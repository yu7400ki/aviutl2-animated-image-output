//! 非可逆で、差分矩形が自分の出力を復号した画面から決まること

mod support;

use anim_core::tolerance;
use jxl_encoder::ColorType;
use support::*;

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
