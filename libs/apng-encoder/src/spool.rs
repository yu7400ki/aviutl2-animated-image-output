//! 出力の色種別が決まるまでフレームを溜めておく領域

use crate::alpha;
use crate::delay::FrameDelay;
use crate::diff::Rect;
use crate::region;

/// 溜めたフレーム1つぶんの位置と属性
pub(crate) struct Spooled {
    pub(crate) rect: Rect,
    pub(crate) delay: FrameDelay,
    /// [`Spool::data`] 上でこのフレームの領域が終わる位置
    end: usize,
}

/// クロップ済みのRGBA8領域を、出力の色種別が決まるまで溜める
pub(crate) struct Spool {
    /// クロップ済み領域を入力の画素表現のまま連結したもの
    data: Vec<u8>,
    frames: Vec<Spooled>,
    /// [`Self::data`] に溜められるバイト数の上限
    limit: usize,
    /// 不透明でない画素を見つけたか
    transparent: bool,
}

impl Spool {
    pub(crate) fn new(limit: usize) -> Self {
        Spool {
            data: Vec::new(),
            frames: Vec::new(),
            limit,
            transparent: false,
        }
    }

    /// 溜めているバイト数
    pub(crate) fn len(&self) -> usize {
        self.data.len()
    }

    /// `len` バイトを追加しても上限を超えないか
    pub(crate) fn can_hold(&self, len: usize) -> bool {
        self.data.len() + len <= self.limit
    }

    /// 不透明でない画素をこれまでに見つけたか
    ///
    /// 一度真になったら戻らないため、以降の走査は要らない。
    pub(crate) fn transparent(&self) -> bool {
        self.transparent
    }

    /// `data` から `rect` を切り出して溜め、その領域のアルファを調べる
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
        let start = self.data.len();
        region::crop(data, rect, stride, bpp, bpp, &mut self.data);

        if !self.transparent {
            self.transparent = alpha::has_transparency(&self.data[start..]);
        }

        self.frames.push(Spooled {
            rect,
            delay,
            end: self.data.len(),
        });
    }

    /// 溜めたフレームを、投入した順に領域と属性の組で返す
    pub(crate) fn frames(&self) -> impl Iterator<Item = (&[u8], &Spooled)> {
        let mut start = 0;
        self.frames.iter().map(move |frame| {
            let region = &self.data[start..frame.end];
            start = frame.end;
            (region, frame)
        })
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
        for _ in 0..3 {
            spool.push(&frame(0xFF), whole(), delay(), STRIDE, 4);
        }

        let regions: Vec<&[u8]> = spool.frames().map(|(region, _)| region).collect();
        assert_eq!(regions.len(), 3);
        for region in regions {
            assert_eq!(region, frame(0xFF));
        }
    }

    /// 溜めたバイト数は切り出した領域の合計
    #[test]
    fn the_length_counts_the_cropped_regions() {
        let mut spool = Spool::new(usize::MAX);
        let rect = Rect {
            x: 1,
            y: 0,
            width: 2,
            height: 2,
        };
        spool.push(&frame(0xFF), rect, delay(), STRIDE, 4);

        assert_eq!(spool.len(), 2 * 2 * 4);
    }

    #[test]
    fn the_limit_is_reached_before_it_is_exceeded() {
        let spool = Spool::new(8);
        assert!(spool.can_hold(8));
        assert!(!spool.can_hold(9));
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
