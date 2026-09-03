//! 書き出しを待っているフレームと、直前のフレームから決まる差分矩形

use crate::layout::Layout;
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
    /// `rect` が書き直す範囲
    ///
    /// キャンバス全体を覆う矩形は [`Region::Whole`] になる。
    fn of(layout: &Layout, rect: Rect) -> Self {
        if rect.width == layout.width && rect.height == layout.height {
            Region::Whole
        } else {
            Region::Part(rect)
        }
    }
}

/// 書き出しを待っているフレーム
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pending {
    /// 書き直す範囲
    pub(crate) region: Region,
    /// tick数の表示時間
    pub(crate) duration: u32,
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
        let Some(pending) = self.pending else {
            // 先頭フレームには土台にする直前のフレームが無い
            self.previous.clear();
            self.previous.extend_from_slice(data);
            self.pending = Some(Pending {
                region: Region::Whole,
                duration,
            });
            return None;
        };

        let region = match dirty_rect(&self.previous, data, layout.stride, layout.bytes_per_pixel) {
            Some(rect) => Region::of(layout, rect),
            None => match pending.duration.checked_add(duration) {
                Some(total) => {
                    self.pending = Some(Pending {
                        duration: total,
                        ..pending
                    });
                    return None;
                }
                None => Region::of(layout, UNCHANGED),
            },
        };

        match pending.region {
            // 全面フレームは面がそのまま書き直す画素になる
            Region::Whole => std::mem::swap(&mut self.previous, &mut self.staged),
            Region::Part(rect) => stage_part(&self.previous, rect, layout, &mut self.staged),
        }
        self.previous.clear();
        self.previous.extend_from_slice(data);
        self.pending = Some(Pending { region, duration });
        Some(pending)
    }

    /// 保留中のフレームを取り出し、直前のフレームを手放す
    ///
    /// 返したフレームが書き直す画素は [`Self::pixels`] にある。
    pub(crate) fn take(&mut self, layout: &Layout) -> Option<Pending> {
        let pending = self.pending.take()?;
        let previous = std::mem::take(&mut self.previous);
        match pending.region {
            Region::Whole => self.staged = previous,
            Region::Part(rect) => stage_part(&previous, rect, layout, &mut self.staged),
        }
        Some(pending)
    }

    /// 取り出したフレームが書き直す画素
    pub(crate) fn pixels(&self) -> &[u8] {
        &self.staged
    }
}

/// `previous` の `rect` を `staged` に置く
fn stage_part(previous: &[u8], rect: Rect, layout: &Layout, staged: &mut Vec<u8>) {
    staged.clear();
    crop(
        previous,
        rect,
        layout.stride,
        layout.bytes_per_pixel,
        layout.bytes_per_pixel,
        staged,
    );
}
