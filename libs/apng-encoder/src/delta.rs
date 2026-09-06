//! 投入と決定それぞれが見るフレームの追跡、およびそこから決まる差分矩形

use crate::chunk::DISPOSE_OP_NONE;
use crate::layout::Layout;
use anim_core::Rect;
use std::sync::Arc;

/// 直前に投入されたフレームと、直前に決定したフレームとそのキャンバス
///
/// 投入と決定は別の時点で進む。投入は直前に投入されたフレームを進め、決定は
/// 直前に決定したフレームとそれを描く直前のキャンバスを進める。面は分け持ち、
/// どこからも指されなくなったものを次の写し先へ回す。
pub(crate) struct Delta {
    /// 直前に投入されたフレーム
    previous: Arc<Vec<u8>>,
    /// 直前に決定したフレーム
    ///
    /// 書き出しは合成後が投入された内容と一致するように選ぶため、それを描いた後の
    /// キャンバスと一致する。
    decided: Arc<Vec<u8>>,
    /// [`Self::decided`] を描く直前のキャンバス
    ///
    /// 保留中のフレームをdispose_op=PREVIOUSで捨てると、この内容が復元される。
    /// 復元先は常に過去のいずれかのフレームそのものなので、1面あれば足りる。
    canvas: Arc<Vec<u8>>,
    /// 写し先へ配り直す面
    spare: Vec<Vec<u8>>,
}

impl Delta {
    pub(crate) fn new() -> Self {
        Delta {
            previous: Arc::default(),
            decided: Arc::default(),
            canvas: Arc::default(),
            spare: Vec::new(),
        }
    }

    /// フレームを写し取り、直前に投入されたフレームとして覚える
    ///
    /// 写し先は配り直された面を使う。返した面は投入されたフレームとして分け持つ。
    pub(crate) fn stage(&mut self, data: &[u8]) -> Arc<Vec<u8>> {
        let mut frame = self.spare.pop().unwrap_or_default();
        frame.clear();
        frame.extend_from_slice(data);

        self.previous = Arc::new(frame);
        Arc::clone(&self.previous)
    }

    /// 決定を終えたフレームを覚え、保留中のフレームの `dispose` でキャンバスを進める
    ///
    /// 指す先を失った面は写し先へ配り直す。
    pub(crate) fn advance(&mut self, data: Arc<Vec<u8>>, dispose: u8) {
        let decided = std::mem::replace(&mut self.decided, data);
        // 捨てない場合だけ、直前に決定したフレームがそのままキャンバスとして残る
        let released = if dispose == DISPOSE_OP_NONE {
            std::mem::replace(&mut self.canvas, decided)
        } else {
            decided
        };

        if let Ok(frame) = Arc::try_unwrap(released) {
            self.spare.push(frame);
        }
    }

    /// 保留中のフレームをdispose_op=PREVIOUSで捨てたときに復元されるキャンバス
    pub(crate) fn canvas(&self) -> Arc<Vec<u8>> {
        Arc::clone(&self.canvas)
    }

    /// 決定を終えたフレームを重ねるキャンバス
    ///
    /// 保留中のフレームを `dispose` で捨てると、キャンバスはそれを描く直前の内容へ戻る。
    pub(crate) fn base(&self, dispose: u8) -> &[u8] {
        if dispose == DISPOSE_OP_NONE {
            &self.decided
        } else {
            &self.canvas
        }
    }

    /// 配り直しを待っている面
    #[cfg(test)]
    pub(crate) fn spare(&self) -> &[Vec<u8>] {
        &self.spare
    }

    /// 保留中のフレームを捨てないときの、投入されたフレームの矩形
    ///
    /// `index` は投入された順の位置。先頭フレームはIDATに入るためキャンバス全体とし、
    /// 以降は直前に投入されたフレームとの差分の外接矩形を使う。
    pub(crate) fn kept_rect(&self, layout: &Layout, data: &[u8], index: u32) -> Rect {
        if index == 0 {
            return layout.whole();
        }

        layout.bounding_rect(&self.previous, data)
    }
}
