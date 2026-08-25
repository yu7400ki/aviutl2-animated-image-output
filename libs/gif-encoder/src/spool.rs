//! グローバルカラーテーブルが決まるまでフレームを溜める領域

use crate::frame::bounding_rect;
use crate::layout::Layout;
use crate::quantize::Histogram;
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

/// `index` 番目に溜めたフレームがヒストグラムへ持つ重み
///
/// 描かれた画素が画面上に残りうるフレーム数の近似。
fn frame_weight(num_frames: u32, index: usize) -> u64 {
    u64::from(num_frames).saturating_sub(index as u64).max(1)
}

/// 溜めた区間から取り出した、色を決めるための材料
pub(crate) struct Settled {
    /// 投入された順のフレーム
    pub(crate) frames: Vec<Spooled>,
    /// 溜めた区間に現れた色の和集合
    pub(crate) colors: Colors,
    /// 6-6-6のヒストグラム
    ///
    /// 和集合が上限を超えたときだけ持つ。持っているなら量子化の経路になる。
    pub(crate) histogram: Option<Histogram>,
}

/// クロップ済みの領域を、グローバルカラーテーブルが決まるまで溜める
///
/// グローバルカラーテーブルは1枚目の画像データより前に書く必要があるため、
/// 色が決まるまで1フレームも書き出せない。
pub(crate) struct Spool {
    frames: Vec<Spooled>,
    /// 抱えている画素データの概算バイト数
    len: usize,
    /// 抱えられるメモリの上限バイト数
    limit: usize,
    /// 直前に溜めたフレームの正規化した入力
    ///
    /// 差分矩形を求めるためだけに持つ。溜めた領域には数えない。
    previous: Vec<u8>,
    /// 溜めた領域に現れた色の和集合
    colors: Colors,
    /// 量子化に使うヒストグラム。和集合が上限を超えた時点で確保する
    histogram: Option<Histogram>,
    /// 宣言されたフレーム数。ヒストグラムの重み付けに使う
    num_frames: u32,
}

impl Spool {
    pub(crate) fn new(limit: usize, num_frames: u32) -> Self {
        Spool {
            frames: Vec::new(),
            len: 0,
            limit,
            previous: Vec::new(),
            colors: Colors::new(),
            histogram: None,
            num_frames,
        }
    }

    /// 抱えている画素データの概算バイト数
    ///
    /// クロップ済みの領域に、フレームごとの管理領域を加えたもの。
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// `region_len` バイトの領域をもう1つ抱えても上限を超えないか
    ///
    /// カラーテーブルは1枚も溜めずには据えられないため、空のスプールは上限に
    /// 関わらず受け入れる。
    pub(crate) fn can_hold(&self, region_len: usize) -> bool {
        if self.frames.is_empty() {
            return true;
        }

        region_len
            .checked_add(FRAME_OVERHEAD)
            .and_then(|need| self.len.checked_add(need))
            .is_some_and(|total| total <= self.limit)
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
        self.accumulate(layout.bytes_per_pixel);

        self.previous.clear();
        self.previous.extend_from_slice(data);
    }

    /// 和集合が上限を超えていたら、直前に溜めたフレームをヒストグラムへ積む
    ///
    /// ヒストグラムが覆う区間は決着時のスプールと一致していなければならない。
    /// 確保するのは超えた時点なので、そこまでに溜めたフレームを遡って積んでから
    /// 以降のフレームを足す。超えた後のフレームだけを積むと、先頭区間の色が
    /// 分割に寄与しない。
    fn accumulate(&mut self, bytes_per_pixel: usize) {
        if !self.colors.exceeded() {
            return;
        }

        let last = self.frames.len() - 1;
        let num_frames = self.num_frames;
        match &mut self.histogram {
            Some(histogram) => histogram.observe(
                &self.frames[last].data,
                bytes_per_pixel,
                frame_weight(num_frames, last),
            ),
            None => {
                let mut histogram = Histogram::new();
                for (index, frame) in self.frames.iter().enumerate() {
                    histogram.observe(
                        &frame.data,
                        bytes_per_pixel,
                        frame_weight(num_frames, index),
                    );
                }
                self.histogram = Some(histogram);
            }
        }
    }

    /// 溜めたフレームを投入した順に、色を決める材料と合わせて取り出して空へ戻す
    pub(crate) fn drain(&mut self) -> Settled {
        let spool = std::mem::replace(self, Spool::new(self.limit, self.num_frames));
        Settled {
            frames: spool.frames,
            colors: spool.colors,
            histogram: spool.histogram,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::ColorType;

    const WIDTH: u32 = 4;
    const HEIGHT: u32 = 2;
    /// 重み付けを恒等にするフレーム数
    const FRAMES: u32 = 1;

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
        let mut spool = Spool::new(usize::MAX, FRAMES);
        assert_eq!(push(&mut spool, &frame(0x10)), layout().whole());
    }

    /// 2枚目以降は直前に溜めたフレームとの差分になる
    #[test]
    fn later_frames_are_cropped_against_the_previous_one() {
        let mut spool = Spool::new(usize::MAX, FRAMES);
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
        let mut spool = Spool::new(usize::MAX, FRAMES);
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
        let mut spool = Spool::new(usize::MAX, FRAMES);
        for value in [0x10u8, 0x20, 0x30] {
            push(&mut spool, &frame(value));
        }

        let settled = spool.drain();
        let heads: Vec<u8> = settled.frames.iter().map(|frame| frame.data[2]).collect();
        assert_eq!(heads, [0x10, 0x20, 0x30]);
        assert_eq!(settled.colors.count(), 3 * (WIDTH * HEIGHT) as u16);
    }

    /// 抱えているバイト数は、切り出した領域とフレームごとの管理領域の合計
    #[test]
    fn the_length_counts_the_regions_and_their_overhead() {
        let mut spool = Spool::new(usize::MAX, FRAMES);
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
        let mut spool = Spool::new(usize::MAX, FRAMES);
        push(&mut spool, &frame(0x10));
        spool.drain();

        assert_eq!(spool.len(), 0);
        assert_eq!(push(&mut spool, &frame(0x20)), layout().whole());
    }

    #[test]
    fn the_limit_is_reached_before_it_is_exceeded() {
        let region = (WIDTH * HEIGHT) as usize * 3;
        let mut spool = Spool::new(region + FRAME_OVERHEAD + 8 + FRAME_OVERHEAD, FRAMES);
        push(&mut spool, &frame(0x10));

        assert!(spool.can_hold(8));
        assert!(!spool.can_hold(9));
    }

    /// カラーテーブルを据えるには1枚が要るため、空のスプールは上限を見ない
    #[test]
    fn an_empty_spool_holds_the_first_frame_at_any_limit() {
        let mut spool = Spool::new(0, FRAMES);
        assert!(spool.can_hold(usize::MAX));

        push(&mut spool, &frame(0x10));
        assert!(!spool.can_hold(0));
    }

    /// 加算が溢れる大きさは、上限に収まらないものとして扱う
    #[test]
    fn an_overflowing_request_does_not_fit() {
        let mut spool = Spool::new(usize::MAX, FRAMES);
        push(&mut spool, &frame(0x10));
        assert!(!spool.can_hold(usize::MAX));
    }

    /// 和集合が上限を超えた時点で、そこまでに溜めたフレームを遡って積む
    ///
    /// 3フレームで和集合は384色になり、上限を超えるのは3枚目。超えた後の
    /// フレームだけを積むと、先頭2枚の色が分割に寄与しない。
    #[test]
    fn the_histogram_reaches_back_over_the_frames_already_spooled() {
        const WIDE: u32 = 128;
        const COUNT: u32 = 3;
        let layout = Layout::new(WIDE, 1, ColorType::Rgb8).unwrap();
        let mut spool = Spool::new(usize::MAX, COUNT);

        let shade = |index: u32| (index * 64) as u8;
        for index in 0..COUNT {
            let data: Vec<u8> = (0..WIDE).flat_map(|i| [i as u8, shade(index), 0]).collect();
            let rect = spool.rect_of(&layout, &data);
            spool.push(&layout, &data, rect, delay());
        }

        let histogram = spool
            .drain()
            .histogram
            .expect("和集合が上限を超えてもヒストグラムを持っていない");
        // フレーム k の重みは N - k。先に溜めたフレームほど重い
        for index in 0..COUNT {
            let color = u32::from_le_bytes([0, shade(index), 0, u8::MAX]);
            let expected = u64::from(COUNT - index) * 4;
            assert_eq!(
                histogram.weight_of(color),
                expected,
                "{index} フレーム目の色が積まれていない"
            );
        }
    }

    /// フレーム k の重みは残りのフレーム数で、最後のフレームでも1を下回らない
    #[test]
    fn the_weight_of_a_frame_is_the_number_of_frames_it_can_stay_on_screen() {
        assert_eq!(frame_weight(108, 0), 108);
        assert_eq!(frame_weight(108, 107), 1);
        // 宣言より多く溜まることは無いが、下限は式ではなく型で守る
        assert_eq!(frame_weight(108, 200), 1);
        assert_eq!(frame_weight(0, 0), 1);
    }

    /// 和集合が上限に収まっている間はヒストグラムを確保しない
    #[test]
    fn a_union_within_the_limit_leaves_the_histogram_unallocated() {
        let mut spool = Spool::new(usize::MAX, FRAMES);
        push(&mut spool, &frame(0x10));
        assert!(spool.drain().histogram.is_none());
    }

    /// 矩形の外にある色は数えない
    #[test]
    fn only_the_cropped_region_is_counted() {
        let mut spool = Spool::new(usize::MAX, FRAMES);
        let rect = Rect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        };
        spool.push(&layout(), &frame(0x10), rect, delay());
        assert_eq!(spool.drain().colors.count(), 1);
    }
}
