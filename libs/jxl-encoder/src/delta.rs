//! 書き出しを待っているフレームと、直前のフレームから決まる差分矩形

use crate::layout::Layout;
use crate::split::{THRESHOLD, cut};
use anim_core::{Rect, crop, dirty_rect};

/// 差分が空のまま書き出すときの矩形
const UNCHANGED: Rect = Rect {
    x: 0,
    y: 0,
    width: 1,
    height: 1,
};

/// フレームが書き直すキャンバスの範囲
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Region {
    /// キャンバス全体
    Whole,
    /// キャンバスの一部
    Part(Rect),
}

impl Region {
    /// 書き直す画素のバイト数
    pub(crate) fn byte_len(self, layout: &Layout) -> usize {
        match self {
            Region::Whole => layout.frame_len,
            Region::Part(rect) => rect.area() as usize * layout.bytes_per_pixel,
        }
    }
}

/// 1枚の表示フレームを書く形
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// キャンバス全体を1枚で書く
    Whole,
    /// 矩形1枚で書く
    One(Rect),
    /// 矩形2枚で書く。先の1枚は表示時間0の副フレームになる
    Two(Rect, Rect),
}

impl Shape {
    /// `rect` を書き直すフレームの書き方
    ///
    /// キャンバス全体を覆う矩形は1枚の全面フレームになる。それ以外は、書かずに
    /// 済む画素が固定費に見合うなら2枚へ割る。
    fn of(layout: &Layout, previous: &[u8], frame: &[u8], rect: Rect) -> Self {
        if rect.width == layout.width && rect.height == layout.height {
            Shape::Whole
        } else if let Some((head, tail)) = cut(previous, frame, layout, rect, THRESHOLD) {
            Shape::Two(head, tail)
        } else {
            Shape::One(rect)
        }
    }

    /// 書き直す矩形を書き出す順に返す
    fn rects(self) -> impl Iterator<Item = Rect> {
        let (head, tail) = match self {
            Shape::Whole => (None, None),
            Shape::One(rect) => (None, Some(rect)),
            Shape::Two(head, tail) => (Some(head), Some(tail)),
        };
        head.into_iter().chain(tail)
    }
}

/// 書き出しを待っているフレーム
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pending {
    /// 書き直す形
    shape: Shape,
    /// tick数の表示時間
    duration: u32,
}

impl Pending {
    /// 書き出す順の、フレーム1枚が書き直す範囲と表示時間
    ///
    /// 最後の1枚が表示時間を持ち、手前の矩形は表示時間0で最後の1枚へ畳まれる。
    pub(crate) fn frames(self) -> impl Iterator<Item = (Region, u32)> {
        let (leading, last) = match self.shape {
            Shape::Whole => (None, Region::Whole),
            Shape::One(rect) => (None, Region::Part(rect)),
            Shape::Two(head, tail) => (Some(Region::Part(head)), Region::Part(tail)),
        };
        leading
            .map(|region| (region, 0))
            .into_iter()
            .chain(std::iter::once((last, self.duration)))
    }
}

/// 直前のフレームの追跡と、書き出しを待っているフレーム
///
/// [`Self::previous`] が埋まっているのは、書き出しを待っているフレームがある間だけ。
pub(crate) struct Delta {
    /// 直前に投入されたフレーム
    ///
    /// 書き出しは合成後が投入された内容と一致するように選ぶため、それを描いた後の
    /// キャンバスと一致する。
    previous: Vec<u8>,
    pending: Option<Pending>,
    /// 取り出したフレームが書き直す画素
    staged: Vec<u8>,
}

impl Delta {
    pub(crate) fn new() -> Self {
        Delta {
            previous: Vec::new(),
            pending: None,
            staged: Vec::new(),
        }
    }

    /// 投入されたフレームを保留し、押し出された保留中のフレームを返す
    ///
    /// 投入されたフレームが直前のフレームと一致するときは、保留中のフレームの
    /// 表示時間へ畳んで `None` を返す。畳んだ表示時間が `u32` に収まらないときは、
    /// 保留中のフレームを押し出して投入されたフレームを保留し直す。
    ///
    /// 返したフレームが書き直す画素は [`Self::pixels`] にある。
    pub(crate) fn advance(
        &mut self,
        layout: &Layout,
        data: &[u8],
        duration: u32,
    ) -> Option<Pending> {
        let Some(mut pending) = self.pending else {
            // 先頭フレームには土台にする直前のフレームが無い
            self.previous.clear();
            self.previous.extend_from_slice(data);
            self.pending = Some(Pending {
                shape: Shape::Whole,
                duration,
            });
            return None;
        };

        let shape = match dirty_rect(&self.previous, data, layout.stride, layout.bytes_per_pixel) {
            Some(rect) => Shape::of(layout, &self.previous, data, rect),
            None => match pending.duration.checked_add(duration) {
                Some(total) => {
                    pending.duration = total;
                    self.pending = Some(pending);
                    return None;
                }
                None => Shape::of(layout, &self.previous, data, UNCHANGED),
            },
        };

        self.stage(layout, pending.shape);
        self.previous.clear();
        self.previous.extend_from_slice(data);
        self.pending = Some(Pending { shape, duration });
        Some(pending)
    }

    /// 保留中のフレームを取り出し、直前のフレームを手放す
    ///
    /// 返したフレームが書き直す画素は [`Self::pixels`] にある。
    pub(crate) fn take(&mut self, layout: &Layout) -> Option<Pending> {
        let pending = self.pending.take()?;
        self.stage(layout, pending.shape);
        self.previous = Vec::new();
        Some(pending)
    }

    /// 取り出したフレームが書き直す画素
    ///
    /// [`Pending::frames`] が返す順に、矩形の画素が隙間なく並ぶ。
    pub(crate) fn pixels(&self) -> &[u8] {
        &self.staged
    }

    /// 直前のフレームのうち `shape` が書き直す画素を [`Self::staged`] へ移す
    fn stage(&mut self, layout: &Layout, shape: Shape) {
        // 全面フレームは面がそのまま書き直す画素になる
        if shape == Shape::Whole {
            std::mem::swap(&mut self.previous, &mut self.staged);
            return;
        }
        self.staged.clear();
        for rect in shape.rects() {
            crop(
                &self.previous,
                rect,
                layout.stride,
                layout.bytes_per_pixel,
                layout.bytes_per_pixel,
                &mut self.staged,
            );
        }
    }
}
