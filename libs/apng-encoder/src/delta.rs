//! 直前のフレームとキャンバスの追跡、およびそこから決まる差分矩形

use crate::chunk::DISPOSE_OP_NONE;
use crate::diff::{self, Rect};
use crate::layout::Layout;

/// 直前のフレームと、それを描く直前のキャンバス
pub(crate) struct Delta {
    /// 直前に投入されたフレーム
    ///
    /// 書き出しは合成後が投入された内容と一致するように選ぶため、それを描いた後の
    /// キャンバスと一致する。
    pub(crate) previous: Vec<u8>,
    /// [`Self::previous`] を描く直前のキャンバス
    ///
    /// 保留中のフレームをdispose_op=PREVIOUSで捨てると、この内容が復元される。
    /// 復元先は常に過去のいずれかのフレームそのものなので、1面あれば足りる。
    pub(crate) canvas: Vec<u8>,
}

impl Delta {
    pub(crate) fn new() -> Self {
        Delta {
            previous: Vec::new(),
            canvas: Vec::new(),
        }
    }

    /// 先頭フレームを迎える前の状態へ戻す
    ///
    /// 確保済みの容量はそのまま残す。
    pub(crate) fn reset(&mut self) {
        self.previous.clear();
        self.canvas.clear();
    }

    /// 投入されたフレームを直前のフレームとして覚え、キャンバスを進める
    pub(crate) fn advance(&mut self, data: &[u8], dispose: u8) {
        // 捨てない場合だけ、直前のフレームがそのままキャンバスとして残る
        if dispose == DISPOSE_OP_NONE {
            std::mem::swap(&mut self.canvas, &mut self.previous);
        }
        self.previous.clear();
        self.previous.extend_from_slice(data);
    }

    /// 保留中のフレームを捨てないときの、投入されたフレームの矩形
    ///
    /// 先頭フレームはIDATに入るためキャンバス全体とする。以降は保留中のフレームとの
    /// 差分の外接矩形を使う。
    pub(crate) fn kept_rect(&self, layout: &Layout, data: &[u8], frames_accepted: u32) -> Rect {
        if frames_accepted == 0 {
            return layout.whole();
        }

        bounding_rect(layout, &self.previous, data)
    }

    /// 保留中のフレームをdispose_op=PREVIOUSで捨てるときの、投入されたフレームの矩形
    ///
    /// `disposable` は捨てられる保留中のフレームがあることを表す。次の場合は
    /// 捨てても割に合わないため、候補にせず `None` を返す。
    /// - 書き出しを待っているフレームが無いとき。捨てる先が無く、
    ///   [`Self::canvas`] もまだ埋まっていない
    /// - 保留中のフレームが先頭フレームのとき。先頭のfcTLの
    ///   dispose_op=PREVIOUSはBACKGROUNDとして扱われてキャンバスが復元されず、
    ///   [`Self::canvas`] もまだ埋まっていない
    /// - 矩形が捨てない場合より小さくならないとき。圧縮すれば小さくなることは
    ///   あるが、それを測る圧縮の方が高くつく
    pub(crate) fn restored_rect(
        &self,
        layout: &Layout,
        data: &[u8],
        kept: Rect,
        frames_accepted: u32,
        disposable: bool,
    ) -> Option<Rect> {
        if !disposable || frames_accepted < 2 {
            return None;
        }

        let rect = bounding_rect(layout, &self.canvas, data);
        (rect.area() < kept.area()).then_some(rect)
    }
}

/// `base` と `data` の差分の外接矩形
///
/// 差分が無い場合はfcTLの個数を保つために1画素だけ書き直す。
fn bounding_rect(layout: &Layout, base: &[u8], data: &[u8]) -> Rect {
    const UNCHANGED: Rect = Rect {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    };

    diff::dirty_rect(base, data, layout.stride, layout.bytes_per_pixel).unwrap_or(UNCHANGED)
}
