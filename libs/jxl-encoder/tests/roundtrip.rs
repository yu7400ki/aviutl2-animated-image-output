//! 符号化した .jxl を jxl-rs で読み直し、入力と設定に照らす

mod support;

use jxl::api;
use jxl::headers::frame_header::BlendingMode;
use jxl_encoder::{ColorType, Config};
use support::*;

#[test]
fn an_animation_carries_its_timing_and_loop_count() {
    let decoded = decode(
        &encode(config(ColorType::Rgba8), &DURATIONS),
        ColorType::Rgba8,
    );

    let animation = decoded
        .info
        .animation
        .expect("アニメーションになっていない");
    assert_eq!(animation.tps_numerator, TPS_NUMERATOR);
    assert_eq!(animation.tps_denominator, TPS_DENOMINATOR);
    assert_eq!(animation.num_loops, NUM_PLAYS);

    assert_eq!(decoded.frames.len(), DURATIONS.len());
    let ticks: Vec<u32> = decoded.frames.iter().map(|f| f.duration_ticks).collect();
    assert_eq!(ticks, DURATIONS);
}

/// 約した比が書かれる。そのままの比はヘッダの値域から外れて書けない
#[test]
fn a_reducible_tps_is_written_in_its_reduced_form() {
    let config = Config {
        tps_numerator: TPS_NUMERATOR * 2,
        tps_denominator: TPS_DENOMINATOR * 2,
        ..config(ColorType::Rgb8)
    };
    let decoded = decode(&encode(config, &DURATIONS), ColorType::Rgb8);

    let animation = decoded
        .info
        .animation
        .expect("アニメーションになっていない");
    assert_eq!(animation.tps_numerator, TPS_NUMERATOR);
    assert_eq!(animation.tps_denominator, TPS_DENOMINATOR);
    let ticks: Vec<u32> = decoded.frames.iter().map(|f| f.duration_ticks).collect();
    assert_eq!(ticks, DURATIONS, "約分が表示時間の意味を動かしている");
}

/// 最後のフレームで閉じた完全なストリームの主張
#[test]
fn the_last_frame_closes_the_stream() {
    let decoded = decode(
        &encode(config(ColorType::Rgba8), &DURATIONS),
        ColorType::Rgba8,
    );

    assert_eq!(decoded.frames.len(), DURATIONS.len());
    for frame in &decoded.frames[..DURATIONS.len() - 1] {
        assert!(!frame.is_last, "{} 枚目で閉じている", frame.index + 1);
    }
    assert!(decoded.frames[DURATIONS.len() - 1].is_last);
}

#[test]
fn a_lossless_animation_round_trips_byte_for_byte() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let decoded = decode(&encode(config(color_type), &DURATIONS), color_type);
        assert!(decoded.info.uses_original_profile, "{color_type:?}");
        assert_eq!(decoded.pixels.len(), DURATIONS.len());
        for (index, pixels) in decoded.pixels.iter().enumerate() {
            assert_eq!(
                pixels,
                &frame(color_type, index as u32 * 5),
                "{color_type:?} の {} 枚目",
                index + 1
            );
        }
    }
}

/// αの有無は入力の色種別が決める
#[test]
fn the_alpha_channel_follows_the_color_type() {
    let rgb = decode(
        &encode(config(ColorType::Rgb8), &DURATIONS),
        ColorType::Rgb8,
    );
    assert!(rgb.info.extra_channels.is_empty());

    let rgba = decode(
        &encode(config(ColorType::Rgba8), &DURATIONS),
        ColorType::Rgba8,
    );
    assert_eq!(rgba.info.extra_channels.len(), 1);
    assert!(!rgba.info.extra_channels[0].alpha_associated);
}

#[test]
fn a_lossy_encoding_decodes_at_the_declared_size() {
    let config = Config {
        quality: 80.0,
        ..config(ColorType::Rgba8)
    };
    let decoded = decode(&encode(config, &DURATIONS), ColorType::Rgba8);

    assert_eq!(decoded.info.size, (WIDTH as usize, HEIGHT as usize));
    assert!(!decoded.info.uses_original_profile);
    assert_eq!(decoded.pixels.len(), DURATIONS.len());
}

/// 色が動く設定でも、αだけは入力のまま残る
#[test]
fn a_lossy_encoding_keeps_the_alpha_exact() {
    let config = Config {
        quality: 40.0,
        ..config(ColorType::Rgba8)
    };
    let decoded = decode(&encode(config, &DURATIONS), ColorType::Rgba8);

    assert_eq!(decoded.pixels.len(), DURATIONS.len());
    for (index, pixels) in decoded.pixels.iter().enumerate() {
        let source = frame(ColorType::Rgba8, index as u32 * 5);
        assert_eq!(
            alpha_channel(pixels),
            alpha_channel(&source),
            "{} 枚目のαが動いている",
            index + 1
        );
        assert!(
            color_channels(pixels) != color_channels(&source),
            "{} 枚目の色が入力と一致していて、非可逆になっていない",
            index + 1
        );
    }
}

/// 品質は出力の大きさに現れる
#[test]
fn the_quality_reaches_the_output() {
    let at = |quality| {
        encode(
            Config {
                quality,
                ..config(ColorType::Rgb8)
            },
            &DURATIONS,
        )
        .len()
    };
    let (low, high) = (at(10.0), at(95.0));
    assert!(low < high, "品質10で{low}バイト、品質95で{high}バイト");
}

/// 均衡は出力の中身に現れる
///
/// 大きさは均衡に対して単調でないので、バイト列の違いで見る。
#[test]
fn the_effort_reaches_the_output() {
    let at = |effort| {
        encode(
            Config {
                effort,
                ..config(ColorType::Rgb8)
            },
            &DURATIONS,
        )
    };
    assert_ne!(at(1), at(9));
}

/// 暗黙の既定に依らず、書いた色の解釈とビット深度が読み戻せる
#[test]
fn the_color_encoding_and_the_bit_depth_are_read_back() {
    let decoded = decode(
        &encode(config(ColorType::Rgb8), &DURATIONS),
        ColorType::Rgb8,
    );

    assert_eq!(
        decoded.info.bit_depth,
        api::JxlBitDepth::Int { bits_per_sample: 8 }
    );
    assert!(
        decoded.profile == api::JxlColorProfile::Simple(api::JxlColorEncoding::srgb(false)),
        "sRGBが書かれていない"
    );
}

/// 排水の受け皿を何周も回して書いた出力が、欠けずに読み戻せる
#[test]
fn a_large_frame_round_trips_across_several_drains() {
    let config = Config {
        color_type: ColorType::Rgba8,
        ..config(ColorType::Rgba8)
    };
    let source = noise_rgba(LARGE_SIDE, LARGE_SIDE);
    let encoded = encode_frames(
        config,
        LARGE_SIDE,
        LARGE_SIDE,
        std::slice::from_ref(&source),
        &[1],
    );

    let decoded = decode(&encoded, ColorType::Rgba8);
    assert_eq!(
        decoded.info.size,
        (LARGE_SIDE as usize, LARGE_SIDE as usize)
    );
    assert_eq!(decoded.pixels[0], source);
}

/// 単葉はアニメーションを持たない静止画になる
#[test]
fn a_single_leaf_is_a_still_image() {
    let decoded = decode(&encode(config(ColorType::Rgb8), &[1]), ColorType::Rgb8);

    assert!(decoded.info.animation.is_none());
    assert_eq!(decoded.info.size, (WIDTH as usize, HEIGHT as usize));
    assert_eq!(decoded.frames.len(), 1);
    assert!(decoded.frames[0].is_last);
    assert_eq!(decoded.pixels[0], gradient_rgb(0));
}

/// 部分フレームを重ねた合成結果が、投入したフレームと一致する
#[test]
fn a_lossless_partial_frame_animation_round_trips_byte_for_byte() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let frames = block_frames(color_type);
        let decoded = decode(&encode_blocks(config(color_type)), color_type);

        assert_eq!(decoded.pixels.len(), frames.len(), "{color_type:?}");
        for (index, (pixels, source)) in decoded.pixels.iter().zip(&frames).enumerate() {
            assert_eq!(pixels, source, "{color_type:?} の {} 枚目", index + 1);
        }
    }
}

/// 矩形は投入した変化の位置と大きさで決まる
#[test]
fn each_partial_frame_carries_its_own_rect() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let decoded = decode(&encode_blocks(config(color_type)), color_type);
        assert_eq!(rects(&decoded.headers), block_rects(), "{color_type:?}");
    }
}

/// 先頭フレームは、続く変化がどれだけ小さくても全面で書かれる
#[test]
fn the_first_frame_covers_the_canvas() {
    const DOT: (u32, u32, u32, u32) = (1, 2, 1, 1);

    let color_type = ColorType::Rgb8;
    let base = frame(color_type, 0);
    let mut changed = base.clone();
    invert_block(&mut changed, color_type, DOT);

    let input = [base, changed];
    let encoded = encode_frames(config(color_type), WIDTH, HEIGHT, &input, &[3, 5]);
    let decoded = decode(&encoded, color_type);

    assert_eq!(rects(&decoded.headers), [WHOLE, DOT]);
    assert_eq!(decoded.pixels, input);
}

/// 部分フレームは、直前のフレームが合成後を置いたスロットを土台にする
#[test]
fn a_partial_frame_chains_through_a_reference_slot() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let decoded = decode(&encode_blocks(config(color_type)), color_type);
        let headers = &decoded.headers;
        assert_eq!(headers.len(), BLOCK_DURATIONS.len(), "{color_type:?}");

        for (index, header) in headers.iter().enumerate() {
            let at = format!("{color_type:?} の {} 枚目", index + 1);
            assert_eq!(header.blending_info.mode, BlendingMode::Replace, "{at}");

            // 全面REPLACEでは土台の欄が、最終フレームでは置き先の欄が書かれない
            if index > 0 {
                assert_eq!(
                    header.blending_info.source,
                    headers[index - 1].save_as_reference,
                    "{at} の土台が、直前のフレームの置き先と違う"
                );
                assert_ne!(header.blending_info.source, 0, "{at} の土台が空のスロット");
                if !header.is_last {
                    assert_ne!(
                        header.save_as_reference, header.blending_info.source,
                        "{at} が土台にした枠を上書きしている"
                    );
                }
            }

            if color_type == ColorType::Rgba8 {
                assert_eq!(header.ec_blending_info.len(), 1, "{at}");
                assert_eq!(
                    header.ec_blending_info[0].mode,
                    BlendingMode::Replace,
                    "{at} のαの重ね方が色と違う"
                );
                assert_eq!(
                    header.ec_blending_info[0].source, header.blending_info.source,
                    "{at} のαの土台が色と違う"
                );
            }
        }
    }
}

/// 色が動く設定でも、部分フレームの矩形の内と外でαが入力のまま残る
#[test]
fn a_lossy_partial_frame_keeps_the_alpha_exact() {
    let color_type = ColorType::Rgba8;
    let config = Config {
        quality: 40.0,
        ..config(color_type)
    };
    let frames = block_frames(color_type);
    let decoded = decode(&encode_blocks(config), color_type);

    assert_eq!(decoded.pixels.len(), frames.len());
    for (index, (pixels, source)) in decoded.pixels.iter().zip(&frames).enumerate() {
        assert_eq!(
            alpha_channel(pixels),
            alpha_channel(source),
            "{} 枚目のαが動いている",
            index + 1
        );
        assert!(
            color_channels(pixels) != color_channels(source),
            "{} 枚目の色が入力と一致していて、非可逆になっていない",
            index + 1
        );
    }
}

/// 差分の無いフレームは、書き出しを待っているフレームの表示時間へ畳まれる
///
/// 畳みは列の中間でも末尾でも起きる。
#[test]
fn identical_frames_fold_into_the_pending_duration() {
    let color_type = ColorType::Rgba8;
    let frames = block_frames(color_type);
    let input = [
        frames[0].clone(),
        frames[1].clone(),
        frames[1].clone(),
        frames[2].clone(),
        frames[2].clone(),
    ];
    let durations = [3, 5, 7, 11, 13];

    let encoded = encode_frames(config(color_type), WIDTH, HEIGHT, &input, &durations);
    let decoded = decode(&encoded, color_type);

    assert!(
        decoded.frames.len() < input.len(),
        "{} 枚が畳まれずに残っている",
        decoded.frames.len()
    );
    assert_eq!(ticks(&decoded), [3, 12, 24]);
    assert_eq!(
        ticks(&decoded).iter().sum::<u32>(),
        durations.iter().sum::<u32>(),
        "1周の総表示時間が動いている"
    );
    assert_eq!(rects(&decoded.headers), [WHOLE, BLOCKS[0], BLOCKS[1]]);
    assert_eq!(
        decoded.pixels,
        [frames[0].clone(), frames[1].clone(), frames[2].clone()]
    );
}

/// 離れた2箇所の変化は矩形へ割られ、近い2箇所は外接矩形のまま書かれる
#[test]
fn a_distant_pair_of_changes_is_cut_apart() {
    let color_type = ColorType::Rgb8;

    let distant = decode(&encode_scattered(config(color_type), &DISTANT), color_type);
    assert_eq!(
        rects(&distant.headers),
        [WHOLE, DISTANT[0], DISTANT[1], TRAILING_DOT]
    );

    let nearby = decode(&encode_scattered(config(color_type), &NEARBY), color_type);
    assert_eq!(rects(&nearby.headers), [WHOLE, NEARBY_BOUNDS, TRAILING_DOT]);
}

/// 割った表示フレームの合成結果が、投入したフレームと一致する
#[test]
fn a_cut_display_frame_round_trips_byte_for_byte() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let frames = scattered_frames(color_type, &DISTANT);
        let decoded = decode(&encode_scattered(config(color_type), &DISTANT), color_type);

        assert_eq!(
            decoded.headers.len(),
            frames.len() + 1,
            "{color_type:?} 副フレームが書かれていない"
        );
        assert_eq!(
            decoded.pixels.len(),
            frames.len(),
            "{color_type:?} 副フレームが表示フレームとして数えられている"
        );
        for (index, (pixels, source)) in decoded.pixels.iter().zip(&frames).enumerate() {
            assert_eq!(pixels, source, "{color_type:?} の {} 枚目", index + 1);
        }
    }
}

/// 割っても投入したフレーム数と表示時間の並びは動かない
#[test]
fn cutting_a_frame_keeps_the_visible_frames_and_their_ticks() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let decoded = decode(&encode_scattered(config(color_type), &DISTANT), color_type);

        assert_eq!(
            decoded.frames.len(),
            SCATTERED_DURATIONS.len(),
            "{color_type:?}"
        );
        assert_eq!(ticks(&decoded), SCATTERED_DURATIONS, "{color_type:?}");
        assert_eq!(
            ticks(&decoded).iter().sum::<u32>(),
            SCATTERED_DURATIONS.iter().sum::<u32>(),
            "{color_type:?} の1周の総表示時間が動いている"
        );
    }
}

/// 副フレームは表示時間を持たず、表示フレームと同じスロットへ連なる
///
/// 副フレームを重ねた結果は置き先の枠に入るので、表示フレームはそちらを土台にする。
#[test]
fn a_sub_frame_chains_through_the_slot_of_its_display_frame() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let decoded = decode(&encode_scattered(config(color_type), &DISTANT), color_type);
        let headers = &decoded.headers;
        assert_eq!(headers.len(), 4, "{color_type:?}");

        let (sub, display) = (&headers[1], &headers[2]);
        assert_eq!(sub.duration, 0, "{color_type:?} の副フレームが表示される");
        assert!(!sub.is_last, "{color_type:?} の副フレームで閉じている");
        assert_eq!(
            display.duration, SCATTERED_DURATIONS[1],
            "{color_type:?} の表示フレームの表示時間"
        );
        assert_ne!(
            sub.save_as_reference, 0,
            "{color_type:?} の副フレームが枠0へ入る"
        );
        assert_eq!(
            sub.save_as_reference, display.save_as_reference,
            "{color_type:?} の副フレームが表示フレームと違う枠へ置く"
        );
        assert_eq!(
            sub.blending_info.source, headers[0].save_as_reference,
            "{color_type:?} の副フレームの土台が、直前のフレームの置き先と違う"
        );
        assert_eq!(
            display.blending_info.source, sub.save_as_reference,
            "{color_type:?} の表示フレームが副フレームの置き先を土台にしていない"
        );
        assert_eq!(
            sub.blending_info.mode,
            BlendingMode::Replace,
            "{color_type:?} の副フレームの重ね方"
        );
    }
}

/// 一過性の重なりが消えたフレームは、2つ前のキャンバスを土台にする
#[test]
fn a_frame_that_undoes_a_transient_overlay_restores_the_older_canvas() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let frames = popup_frames(color_type);
        let decoded = decode(&encode_popup(config(color_type)), color_type);
        let headers = &decoded.headers;

        assert_eq!(
            rects(headers),
            [WHOLE, POPUP, UNCHANGED, TRAILING_DOT],
            "{color_type:?}"
        );
        assert_eq!(
            headers[2].blending_info.source, headers[2].save_as_reference,
            "{color_type:?} の3枚目が2つ前のキャンバスを土台にしていない"
        );
        assert_eq!(
            headers[3].blending_info.source, headers[2].save_as_reference,
            "{color_type:?} の4枚目の土台が、直前のフレームの置き先と違う"
        );
        assert_eq!(decoded.pixels, frames, "{color_type:?}");
    }
}

/// 合成後のキャンバスを置く枠は、表示フレームごとに入れ替わる
///
/// 入れ替えることで、直前のキャンバスと2つ前のキャンバスが同時に生きる。
#[test]
fn the_reference_slot_alternates_between_display_frames() {
    let color_type = ColorType::Rgb8;
    let decoded = decode(&encode_popup(config(color_type)), color_type);

    // 最終フレームは置き先の欄を持たない
    assert_eq!(saved_slots(&decoded.headers), [1, 2, 1, 0]);
}

/// 2枚目のフレームは、まだ書き込まれていない枠を土台にしない
#[test]
fn the_second_frame_leaves_the_unwritten_slot_alone() {
    let color_type = ColorType::Rgb8;
    let frames = lone_block_frames(color_type);
    let encoded = encode_frames(
        config(color_type),
        WIDTH,
        HEIGHT,
        &frames,
        &SCATTERED_DURATIONS,
    );
    let decoded = decode(&encoded, color_type);
    let headers = &decoded.headers;

    assert_eq!(
        rects(headers),
        [WHOLE, LONE_BOUNDS, TRAILING_DOT],
        "2枚目が空の枠との差分で書かれている"
    );
    assert_eq!(
        headers[1].blending_info.source, headers[0].save_as_reference,
        "2枚目の土台が、直前のフレームの置き先と違う"
    );
    assert_ne!(
        headers[1].blending_info.source, headers[1].save_as_reference,
        "2枚目が空の自分の置き先を土台にしている"
    );
    assert_eq!(decoded.pixels, frames);
}

/// 2つ前のキャンバスを土台にした表示フレームも、離れた変化なら矩形へ割られる
#[test]
fn a_restored_display_frame_is_cut_against_the_older_canvas() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let frames = flash_frames(color_type);
        let encoded = encode_frames(config(color_type), WIDTH, HEIGHT, &frames, &BLOCK_DURATIONS);
        let decoded = decode(&encoded, color_type);
        let headers = &decoded.headers;

        assert_eq!(
            rects(headers),
            [WHOLE, WHOLE, DISTANT[0], DISTANT[1], TRAILING_DOT],
            "{color_type:?}"
        );
        assert_eq!(saved_slots(headers), [1, 2, 1, 1, 0], "{color_type:?}");

        let (sub, display) = (&headers[2], &headers[3]);
        assert_eq!(sub.duration, 0, "{color_type:?} の副フレームが表示される");
        assert_eq!(
            sub.blending_info.source, sub.save_as_reference,
            "{color_type:?} の副フレームが2つ前のキャンバスを土台にしていない"
        );
        assert_eq!(
            display.blending_info.source, sub.save_as_reference,
            "{color_type:?} の表示フレームが副フレームの置き先を土台にしていない"
        );
        assert_eq!(decoded.pixels, frames, "{color_type:?}");
    }
}

/// `u32` に収まらない表示時間は、そこでフレームを分けて持つ
#[test]
fn a_duration_beyond_the_writable_range_splits_the_frame() {
    let color_type = ColorType::Rgb8;
    let source = frame(color_type, 0);
    let durations = [u32::MAX, 5];

    let input = [source.clone(), source];
    let encoded = encode_frames(config(color_type), WIDTH, HEIGHT, &input, &durations);
    let decoded = decode(&encoded, color_type);

    assert_eq!(ticks(&decoded), durations);
    assert_eq!(decoded.pixels, input);
}
