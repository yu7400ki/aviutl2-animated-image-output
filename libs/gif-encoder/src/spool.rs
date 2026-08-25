//! グローバルカラーテーブルが決まるまでフレームを溜める領域

use crate::frame::bounding_rect;
use crate::layout::Layout;
use anim_core::{Colors, FrameDelay, Rect, crop};

/// 溜めたフレーム1つ
pub(crate) struct Spooled {
    /// 直前のフレームとの差分の外接矩形
    pub(crate) rect: Rect,
    pub(crate) delay: FrameDelay,
    /// 正規化した入力のままの、クロップ済み領域
    pub(crate) data: Vec<u8>,
}

/// フレーム1つを溜めるのに、画素データとは別にかかるバイト数
const FRAME_OVERHEAD: usize = size_of::<Spooled>();

/// クロップ済みの領域を、グローバルカラーテーブルが決まるまで溜める
///
/// グローバルカラーテーブルは1枚目の画像データより前に書く必要があるため、
/// 色が決まるまで1フレームも書き出せない。
pub(crate) struct Spool {
    frames: Vec<Spooled>,
    /// 抱えている画素データの概算バイト数
    len: usize,
    /// 直前に溜めたフレームの正規化した入力
    ///
    /// 差分矩形を求めるためだけに持つ。溜めた領域には数えない。
    previous: Vec<u8>,
    /// 溜めた領域に現れた色の和集合
    colors: Colors,
}

impl Spool {
    pub(crate) fn new() -> Self {
        Spool {
            frames: Vec::new(),
            len: 0,
            previous: Vec::new(),
            colors: Colors::new(),
        }
    }

    /// 抱えている画素データの概算バイト数
    ///
    /// クロップ済みの領域に、フレームごとの管理領域を加えたもの。
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// 投入されたフレームを溜めるときの矩形
    ///
    /// 先頭フレームは論理画面全体、以降は直前に溜めたフレームとの差分の
    /// 外接矩形。まだ添字が無いため、差分は入力の画素で求める。
    pub(crate) fn rect_of(&self, layout: &Layout, data: &[u8]) -> Rect {
        if self.previous.is_empty() {
            return layout.whole();
        }

        bounding_rect(layout, &self.previous, data)
    }

    /// `data` から `rect` を切り出して溜め、その領域の色を数える
    ///
    /// 矩形の外は直前のフレームから変わっていないため、走査は矩形の中だけで足りる。
    pub(crate) fn push(&mut self, layout: &Layout, data: &[u8], rect: Rect, delay: FrameDelay) {
        let mut region = Vec::new();
        crop(
            data,
            rect,
            layout.stride,
            layout.bytes_per_pixel,
            layout.bytes_per_pixel,
            &mut region,
        );
        self.colors.observe(&region, layout.bytes_per_pixel);

        self.len += region.len() + FRAME_OVERHEAD;
        self.frames.push(Spooled {
            rect,
            delay,
            data: region,
        });

        self.previous.clear();
        self.previous.extend_from_slice(data);
    }

    /// 溜めたフレームを投入した順に、色の和集合と合わせて取り出して空へ戻す
    pub(crate) fn drain(&mut self) -> (Vec<Spooled>, Colors) {
        let spool = std::mem::replace(self, Spool::new());
        (spool.frames, spool.colors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::ColorType;

    const WIDTH: u32 = 4;
    const HEIGHT: u32 = 2;

    fn layout() -> Layout {
        Layout::new(WIDTH, HEIGHT, ColorType::Rgb8).unwrap()
    }

    fn frame(value: u8) -> Vec<u8> {
        (0..WIDTH * HEIGHT)
            .flat_map(|i| [i as u8, 0x20, value])
            .collect()
    }

    fn delay() -> FrameDelay {
        FrameDelay::new(1, 30).unwrap()
    }

    fn push(spool: &mut Spool, data: &[u8]) -> Rect {
        let rect = spool.rect_of(&layout(), data);
        spool.push(&layout(), data, rect, delay());
        rect
    }

    #[test]
    fn the_first_frame_covers_the_logical_screen() {
        let mut spool = Spool::new();
        assert_eq!(push(&mut spool, &frame(0x10)), layout().whole());
    }

    /// 2枚目以降は直前に溜めたフレームとの差分になる
    #[test]
    fn later_frames_are_cropped_against_the_previous_one() {
        let mut spool = Spool::new();
        push(&mut spool, &frame(0x10));

        let mut next = frame(0x10);
        next[(WIDTH as usize + 2) * 3] = 0xFF;
        assert_eq!(
            push(&mut spool, &next),
            Rect {
                x: 2,
                y: 1,
                width: 1,
                height: 1
            }
        );
    }

    /// 差分の無いフレームは1画素の矩形になる
    #[test]
    fn an_identical_frame_becomes_a_unit_rect() {
        let mut spool = Spool::new();
        push(&mut spool, &frame(0x10));
        assert_eq!(
            push(&mut spool, &frame(0x10)),
            Rect {
                x: 0,
                y: 0,
                width: 1,
                height: 1
            }
        );
    }

    #[test]
    fn frames_come_back_in_the_order_they_were_pushed() {
        let mut spool = Spool::new();
        for value in [0x10u8, 0x20, 0x30] {
            push(&mut spool, &frame(value));
        }

        let (frames, colors) = spool.drain();
        let heads: Vec<u8> = frames.iter().map(|frame| frame.data[2]).collect();
        assert_eq!(heads, [0x10, 0x20, 0x30]);
        assert_eq!(colors.count(), 3 * (WIDTH * HEIGHT) as u16);
    }

    /// 抱えているバイト数は、切り出した領域とフレームごとの管理領域の合計
    #[test]
    fn the_length_counts_the_regions_and_their_overhead() {
        let mut spool = Spool::new();
        push(&mut spool, &frame(0x10));
        assert_eq!(
            spool.len(),
            (WIDTH * HEIGHT) as usize * 3 + FRAME_OVERHEAD,
            "先頭フレームは全画面"
        );

        push(&mut spool, &frame(0x20));
        assert_eq!(
            spool.len(),
            (WIDTH * HEIGHT) as usize * 3
                + FRAME_OVERHEAD
                + (WIDTH * HEIGHT) as usize * 3
                + FRAME_OVERHEAD
        );
    }

    /// 取り出した後は先頭フレームを迎える前の状態へ戻る
    #[test]
    fn draining_leaves_the_spool_empty() {
        let mut spool = Spool::new();
        push(&mut spool, &frame(0x10));
        spool.drain();

        assert_eq!(spool.len(), 0);
        assert_eq!(push(&mut spool, &frame(0x20)), layout().whole());
    }

    /// 矩形の外にある色は数えない
    #[test]
    fn only_the_cropped_region_is_counted() {
        let mut spool = Spool::new();
        let rect = Rect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        };
        spool.push(&layout(), &frame(0x10), rect, delay());
        assert_eq!(spool.drain().1.count(), 1);
    }
}
