//! 出力の色種別が決まるまでフレームを溜めておく領域

use crate::alpha;
use crate::delay::FrameDelay;
use crate::diff::Rect;
use crate::palette::Colors;
use crate::region;

/// 溜めたフレーム1つ
pub(crate) struct Spooled {
    pub(crate) rect: Rect,
    pub(crate) delay: FrameDelay,
    /// 入力の画素表現のままのクロップ済み領域
    pub(crate) data: Vec<u8>,
}

/// フレーム1つを溜めるのに、画素データとは別にかかるバイト数
const FRAME_OVERHEAD: usize = size_of::<Spooled>();

/// クロップ済みの領域を、出力の色種別が決まるまで溜める
pub(crate) struct Spool {
    frames: Vec<Spooled>,
    /// 抱えているメモリの概算バイト数
    len: usize,
    /// 抱えられるメモリの上限バイト数
    limit: usize,
    /// 不透明でない画素を見つけたか
    transparent: bool,
    /// 溜めた領域に現れた色の和集合
    colors: Colors,
}

impl Spool {
    pub(crate) fn new(limit: usize) -> Self {
        Spool {
            frames: Vec::new(),
            len: 0,
            limit,
            transparent: false,
            colors: Colors::new(),
        }
    }

    /// 抱えているメモリの概算バイト数
    ///
    /// 画素データに、フレームごとの管理領域を加えたもの。
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// `region_len` バイトの領域をもう1つ抱えても上限を超えないか
    pub(crate) fn can_hold(&self, region_len: usize) -> bool {
        region_len
            .checked_add(FRAME_OVERHEAD)
            .and_then(|need| self.len.checked_add(need))
            .is_some_and(|total| total <= self.limit)
    }

    /// 不透明でない画素をこれまでに見つけたか
    ///
    /// 一度真になったら戻らないため、以降の走査は要らない。
    pub(crate) fn transparent(&self) -> bool {
        self.transparent
    }

    /// 色の和集合がパレットに収まる数を超えたか
    ///
    /// 一度真になったら戻らないため、以降の走査は要らない。
    pub(crate) fn colors_exceeded(&self) -> bool {
        self.colors.exceeded()
    }

    /// 溜めたフレームを投入した順に見る
    pub(crate) fn frames(&self) -> &[Spooled] {
        &self.frames
    }

    /// 溜めたフレームを投入した順に、色の和集合と合わせて取り出す
    fn into_parts(self) -> (Vec<Spooled>, Colors) {
        (self.frames, self.colors)
    }

    /// 溜めた内容を [`Self::into_parts`] と同じ形で取り出し、空へ戻す
    pub(crate) fn drain(&mut self) -> (Vec<Spooled>, Colors) {
        std::mem::replace(self, Spool::new(self.limit)).into_parts()
    }

    /// `data` から `rect` を切り出して溜め、その領域のアルファと色を調べる
    ///
    /// 矩形の外は直前のフレームから変わっていないため、走査は矩形の中だけで足りる。
    pub(crate) fn push(
        &mut self,
        data: &[u8],
        rect: Rect,
        delay: FrameDelay,
        stride: usize,
        bpp: usize,
    ) {
        let mut region = Vec::new();
        region::crop(data, rect, stride, bpp, bpp, &mut region);

        if bpp == 4 && !self.transparent {
            self.transparent = alpha::has_transparency(&region);
        }
        self.colors.observe(&region, bpp);

        self.len += region.len() + FRAME_OVERHEAD;
        self.frames.push(Spooled {
            rect,
            delay,
            data: region,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: u32 = 4;
    const STRIDE: usize = WIDTH as usize * 4;

    fn frame(alpha: u8) -> Vec<u8> {
        (0..WIDTH * 2)
            .flat_map(|i| [i as u8, 0x20, 0x30, alpha])
            .collect()
    }

    fn whole() -> Rect {
        Rect {
            x: 0,
            y: 0,
            width: WIDTH,
            height: 2,
        }
    }

    fn delay() -> FrameDelay {
        FrameDelay::new(1, 30).unwrap()
    }

    #[test]
    fn frames_come_back_in_the_order_they_were_pushed() {
        let mut spool = Spool::new(usize::MAX);
        for value in [0x10u8, 0x20, 0x30] {
            let mut data = frame(0xFF);
            data[0] = value;
            spool.push(&data, whole(), delay(), STRIDE, 4);
        }

        let heads: Vec<u8> = spool.into_parts().0.iter().map(|f| f.data[0]).collect();
        assert_eq!(heads, [0x10, 0x20, 0x30]);
    }

    /// 抱えているバイト数は、切り出した領域とフレームごとの管理領域の合計
    #[test]
    fn the_length_counts_the_regions_and_their_overhead() {
        let mut spool = Spool::new(usize::MAX);
        let rect = Rect {
            x: 1,
            y: 0,
            width: 2,
            height: 2,
        };
        spool.push(&frame(0xFF), rect, delay(), STRIDE, 4);

        assert_eq!(spool.len(), 2 * 2 * 4 + FRAME_OVERHEAD);
        assert_eq!(spool.into_parts().0[0].rect, rect);
    }

    #[test]
    fn the_limit_is_reached_before_it_is_exceeded() {
        let spool = Spool::new(8 + FRAME_OVERHEAD);
        assert!(spool.can_hold(8));
        assert!(!spool.can_hold(9));
    }

    /// 加算が溢れる大きさは、上限に収まらないものとして扱う
    #[test]
    fn an_overflowing_request_does_not_fit() {
        assert!(!Spool::new(usize::MAX).can_hold(usize::MAX));
    }

    #[test]
    fn transparency_latches_once_it_is_seen() {
        let mut spool = Spool::new(usize::MAX);
        spool.push(&frame(0xFF), whole(), delay(), STRIDE, 4);
        assert!(!spool.transparent());

        spool.push(&frame(0x80), whole(), delay(), STRIDE, 4);
        assert!(spool.transparent());

        spool.push(&frame(0xFF), whole(), delay(), STRIDE, 4);
        assert!(spool.transparent());
    }

    /// 矩形の外にあるアルファは走査されない
    #[test]
    fn only_the_cropped_region_is_scanned() {
        let mut data = frame(0xFF);
        data[3] = 0x40;
        let rect = Rect {
            x: 1,
            y: 0,
            width: 3,
            height: 2,
        };

        let mut spool = Spool::new(usize::MAX);
        spool.push(&data, rect, delay(), STRIDE, 4);
        assert!(!spool.transparent());
    }
}
