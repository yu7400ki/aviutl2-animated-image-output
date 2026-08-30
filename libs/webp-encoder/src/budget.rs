//! ワーカー数に置く上限

use crate::Config;
use crate::layout::Layout;
use std::num::NonZeroUsize;

/// エンコーダ1つが抱えてよい量 (バイト)
const BUDGET: u64 = 1 << 30;

/// キャンバスが抱える、ワーカー数に依らない取り分 (画素あたりバイト)
///
/// 前のフレームを描いた画素・その矩形を抜いた画素・正規化して写した入力の
/// 3面がRGBAで並び、伸ばす途中の写しがそこへ重なる。3面ぶんの倍を超える
/// 実測があるので、そこから切り上げた値を採る。
const CANVAS_PER_PIXEL: u64 = 32;

/// 非可逆の符号化がワーカー1つあたりに使う、面積に依らない作業領域 (バイト)
const LOSSY_BASE: u64 = 2 << 20;

/// 非可逆の符号化がワーカー1つあたりに使う作業領域 (画素あたりバイト)
const LOSSY_PER_PIXEL: u64 = 20;

/// 可逆の符号化がワーカー1つあたりに使う、面積に依らない作業領域 (バイト)
const LOSSLESS_BASE: u64 = 48 << 20;

/// 可逆の符号化がワーカー1つあたりに使う作業領域 (画素あたりバイト)
const LOSSLESS_PER_PIXEL: u64 = 36;

/// ワーカー1つが抱える量の見積り (バイト)
///
/// 仕掛かりの上限がワーカー数の2倍なので、切り出し先も2つぶん数える。
/// 符号化の結果は大きさが投入前に読めないので、作業領域の係数がまとめて覆う。
/// 作業領域は画素の並びと、品質・メソッドの上げ方で太る。係数は最も重い
/// 動作点で採り、素材ごとの振れを覆う余裕を載せてある。
fn share(layout: &Layout, config: &Config) -> u64 {
    let pixels = u64::from(layout.width) * u64::from(layout.height);
    let in_flight = 2 * pixels * layout.bytes_per_pixel as u64;
    let workspace = if config.lossless {
        LOSSLESS_BASE + pixels * LOSSLESS_PER_PIXEL
    } else {
        LOSSY_BASE + pixels * LOSSY_PER_PIXEL
    };
    in_flight + workspace
}

/// 予算の内側に収まるワーカー数
///
/// `available` を超えることはない。1ワーカーでも予算に収まらない大きさでは、
/// 収まる数が無いので1を返す。
pub(crate) fn workers(layout: &Layout, config: &Config, available: NonZeroUsize) -> NonZeroUsize {
    let pixels = u64::from(layout.width) * u64::from(layout.height);
    let left = BUDGET.saturating_sub(pixels * CANVAS_PER_PIXEL);
    let affordable = usize::try_from(left / share(layout, config)).unwrap_or(usize::MAX);
    NonZeroUsize::new(affordable.min(available.get())).unwrap_or(NonZeroUsize::MIN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anim_core::ColorType;

    const AVAILABLE: NonZeroUsize = NonZeroUsize::new(16).unwrap();

    fn config(lossless: bool) -> Config {
        Config {
            color_type: ColorType::Rgba8,
            lossless,
            quality: 100.0,
            method: 6,
            num_plays: 0,
        }
    }

    fn workers_for(width: u32, height: u32, lossless: bool) -> usize {
        let config = config(lossless);
        let layout = Layout::new(width, height, config.color_type).unwrap();
        workers(&layout, &config, AVAILABLE).get()
    }

    /// 抱える量の見積りが予算に収まる。1つまで絞ってなお超える大きさは除く
    fn fits(width: u32, height: u32, lossless: bool) -> bool {
        let config = config(lossless);
        let layout = Layout::new(width, height, config.color_type).unwrap();
        let pixels = u64::from(width) * u64::from(height);
        let workers = workers(&layout, &config, AVAILABLE).get();
        let held = pixels * CANVAS_PER_PIXEL + share(&layout, &config) * workers as u64;
        held <= BUDGET || workers == 1
    }

    /// 小さいキャンバスでは機械の並列度をそのまま使う
    #[test]
    fn a_small_canvas_keeps_every_worker() {
        assert_eq!(workers_for(320, 240, true), AVAILABLE.get());
        assert_eq!(workers_for(320, 240, false), AVAILABLE.get());
    }

    /// 大きいキャンバスでは予算がワーカー数を決める
    ///
    /// 1920x1080の可逆は、実測のピークが 747 MiB / 7ワーカーで収まる点。
    /// 見積りの係数を緩めるとこの数が動く。
    #[test]
    fn a_large_canvas_is_bounded_by_the_budget() {
        assert_eq!(workers_for(1920, 1080, true), 7);
        assert!(fits(1920, 1080, true));
    }

    /// 非可逆でも、寸法が大きくなれば予算がワーカー数を決める
    ///
    /// 見積りの3つの項 — キャンバス・仕掛かり・作業領域 — がどれも効く点を
    /// 固定する。小さい方は面積に依らない作業領域だけが残る点。
    #[test]
    fn the_lossy_estimate_bounds_the_workers() {
        assert_eq!(workers_for(3840, 2160, false), 3);

        let config = config(false);
        let layout = Layout::new(16, 16, config.color_type).unwrap();
        let plenty = NonZeroUsize::new(1024).unwrap();
        assert_eq!(workers(&layout, &config, plenty).get(), 510);
    }

    /// 可逆の下駄は、小さいキャンバスでも起こす数を縛る
    ///
    /// 面積に依らない作業領域が予算を割るので、いくら並列度があっても
    /// 64x64 で 21 を超えては起こさない。
    #[test]
    fn the_lossless_workspace_bounds_even_a_small_canvas() {
        let config = config(true);
        let layout = Layout::new(64, 64, config.color_type).unwrap();
        let plenty = NonZeroUsize::new(1024).unwrap();
        assert_eq!(workers(&layout, &config, plenty).get(), 21);
    }

    /// 予算に収まらない大きさでも1つは起こす
    #[test]
    fn an_oversized_canvas_still_gets_one_worker() {
        assert_eq!(workers_for(16383, 16383, true), 1);
    }

    /// キャンバスが大きくなるほどワーカー数は増えない
    #[test]
    fn a_wider_canvas_never_gets_more_workers() {
        for lossless in [true, false] {
            let mut previous = usize::MAX;
            for height in [240, 480, 720, 1080, 1440, 2160, 4320] {
                let workers = workers_for(height * 16 / 9, height, lossless);
                assert!(
                    workers <= previous,
                    "{height}p で増えている: {previous} → {workers}"
                );
                assert!(fits(height * 16 / 9, height, lossless));
                previous = workers;
            }
        }
    }

    /// 可逆は非可逆より作業領域が大きく、起こす数も多くならない
    #[test]
    fn lossless_never_gets_more_workers_than_lossy() {
        for height in [240, 480, 720, 1080, 1440, 2160] {
            let width = height * 16 / 9;
            assert!(workers_for(width, height, true) <= workers_for(width, height, false));
        }
    }

    /// 色種別が太いほど仕掛かりが太り、起こす数も多くならない
    #[test]
    fn a_wider_pixel_never_gets_more_workers() {
        for lossless in [true, false] {
            let mut config = config(lossless);
            let (width, height) = (3840, 2160);

            config.color_type = ColorType::Rgb8;
            let narrow = Layout::new(width, height, config.color_type).unwrap();
            let narrow = workers(&narrow, &config, AVAILABLE);

            config.color_type = ColorType::Rgba8;
            let wide = Layout::new(width, height, config.color_type).unwrap();
            let wide = workers(&wide, &config, AVAILABLE);

            assert!(wide <= narrow, "可逆{lossless}: {narrow} → {wide}");
        }
    }
}
