//! 非可逆で、差分矩形が直前の投入の変化も覆うこと

mod support;

use jxl_encoder::ColorType;
use support::*;

/// 直前の投入で変わった画素は、次のフレームの矩形へ入る
///
/// 3枚目は2枚目の塊の中を1だけ動かす。矩形はその1画素ではなく、2枚目が書いた
/// 塊まで戻る。
#[test]
fn the_previous_change_widens_the_next_rect() {
    const BLOCK: (u32, u32, u32, u32) = (10, 6, 24, 18);

    /// `BLOCK` の内側で1刻み動く塊 (x, y, 幅, 高さ)
    const STEP: (u32, u32, u32, u32) = (14, 10, 8, 6);

    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let base = flat(color_type, FLAT);
        let block = painted(&base, color_type, BLOCK, PAINT);
        let stepped = painted(&block, color_type, STEP, PAINT + 1);
        let frames = vec![base, block, stepped];

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
            [WHOLE, BLOCK, BLOCK],
            "{color_type:?} の3枚目が2枚目の変化を覆っていない"
        );
        assert_eq!(ticks(&decoded), [3, 5, 7], "{color_type:?}");
    }
}

/// 変化が止まった次のフレームも、直前の変化を書き直す
///
/// 3枚目は2枚目と同じ入力で、変化そのものは空になる。それでも2枚目が書いた跡が
/// もう一度書かれる。
#[test]
fn a_stopped_change_is_still_rewritten() {
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
        rects(&decoded.headers),
        [WHOLE, BLOCK, BLOCK],
        "3枚目が2枚目の跡を書き直していない"
    );
    assert_eq!(ticks(&decoded), [3, 5, 7], "3枚目が表示時間へ畳まれている");
}

/// 変化も持ち越しも空なら、表示時間へ畳む
///
/// 4枚目は3枚目と同じ入力で、3枚目の変化も空になっている。
#[test]
fn a_frame_after_a_still_one_is_merged() {
    const BLOCK: (u32, u32, u32, u32) = (10, 6, 24, 18);

    let color_type = ColorType::Rgb8;
    let base = flat(color_type, FLAT);
    let block = painted(&base, color_type, BLOCK, PAINT);
    let frames = vec![base, block.clone(), block.clone(), block];

    let encoded = encode_frames(
        lossy(color_type, QUALITY),
        WIDTH,
        HEIGHT,
        &frames,
        &[3, 5, 7, 11],
    );
    let decoded = decode(&encoded, color_type);

    assert_eq!(rects(&decoded.headers), [WHOLE, BLOCK, BLOCK]);
    assert_eq!(ticks(&decoded), [3, 5, 18]);
}

/// 2つ前のキャンバスへ戻す土台は、投入の並びから採る
///
/// 3枚目は先頭フレームと同じ入力で、2つ前へ戻せば1画素で書ける。戻す側の候補まで
/// 直前の変化で膨らませると、この1枚が塊のまま書かれる。
#[test]
fn a_frame_equal_to_the_older_canvas_is_restored_from_two_back() {
    const POPUP: (u32, u32, u32, u32) = (10, 6, 24, 18);

    /// `POPUP` と `TRAILING_DOT` をどちらも含む矩形 (x, y, 幅, 高さ)
    const SPANNING: (u32, u32, u32, u32) = (1, 1, 33, 23);

    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let base = flat(color_type, FLAT);
        let covered = painted(&base, color_type, POPUP, PAINT);
        let dotted = painted(&base, color_type, TRAILING_DOT, PAINT);
        let frames = vec![base.clone(), covered, base, dotted];

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
            [WHOLE, POPUP, UNCHANGED, SPANNING],
            "{color_type:?}"
        );
        assert_eq!(
            headers[2].blending_info.source, headers[0].save_as_reference,
            "{color_type:?} の3枚目が2つ前のキャンバスを土台にしていない"
        );
        assert_eq!(ticks(&decoded), [3, 5, 7, 11], "{color_type:?}");
    }
}

/// 1刻みの変化も、矩形を割る地図に入る
///
/// 中央の塊が地図に入ると、割った2枚目がそこまで広がる。
#[test]
fn a_single_step_of_change_enters_the_cut() {
    const CORNERS: [(u32, u32, u32, u32); 2] = [(2, 2, 6, 6), (34, 20, 6, 6)];
    const MIDDLE: (u32, u32, u32, u32) = (20, 10, 6, 6);

    /// 中央の塊まで届いた、割った2枚目 (x, y, 幅, 高さ)
    const REACHING: (u32, u32, u32, u32) = (20, 10, 20, 16);

    let color_type = ColorType::Rgb8;
    let base = flat(color_type, FLAT);
    let mut scattered = base.clone();
    for corner in CORNERS {
        paint(&mut scattered, color_type, corner, PAINT);
    }
    paint(&mut scattered, color_type, MIDDLE, FLAT + 1);
    let frames = vec![base, scattered];

    let encoded = encode_frames(lossy(color_type, QUALITY), WIDTH, HEIGHT, &frames, &[3, 5]);
    let decoded = decode(&encoded, color_type);

    assert_eq!(rects(&decoded.headers), [WHOLE, CORNERS[0], REACHING]);
}

/// 直前の投入で変わった画素も、矩形を割る地図に入る
///
/// 2枚目が書いた中央の塊は3枚目の変化に含まれないが、割った2枚目はそこまで広がる。
#[test]
fn the_previous_change_enters_the_cut() {
    const CORNERS: [(u32, u32, u32, u32); 2] = [(2, 2, 6, 6), (34, 20, 6, 6)];
    const MIDDLE: (u32, u32, u32, u32) = (20, 10, 6, 6);

    /// 中央の塊まで届いた、割った2枚目 (x, y, 幅, 高さ)
    const REACHING: (u32, u32, u32, u32) = (20, 10, 20, 16);

    let color_type = ColorType::Rgb8;
    let base = flat(color_type, FLAT);
    let middle = painted(&base, color_type, MIDDLE, FLAT + 1);
    let mut scattered = middle.clone();
    for corner in CORNERS {
        paint(&mut scattered, color_type, corner, PAINT);
    }
    let frames = vec![base, middle, scattered];

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
        [WHOLE, MIDDLE, CORNERS[0], REACHING]
    );
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

/// 粗い品質でも、平坦な背景の列は2つ前のキャンバスを土台にする
#[test]
fn a_flat_background_restores_the_older_canvas_at_a_coarse_quality() {
    /// `POPUP` と `TRAILING_DOT` をどちらも含む矩形 (x, y, 幅, 高さ)
    const SPANNING: (u32, u32, u32, u32) = (1, 1, 33, 23);

    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let sequence = sequences(color_type)
            .into_iter()
            .find(|sequence| sequence.name == "restored")
            .expect("2つ前へ戻す列が無い");
        let encoded = encode_sequence(lossy(color_type, COARSE_QUALITY), &sequence);
        let decoded = decode(&encoded, color_type);

        assert_eq!(
            rects(&decoded.headers),
            [WHOLE, POPUP, UNCHANGED, SPANNING],
            "{color_type:?}"
        );
    }
}
