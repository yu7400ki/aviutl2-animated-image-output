//! 投入と決定それぞれが見るフレームの追跡

use crate::chunk::DISPOSE_OP_NONE;
use std::sync::Arc;

/// 直前に投入されたフレームと、直前に決定したフレームとそのキャンバス
///
/// 投入と決定は別の時点で進む。投入は直前に投入されたフレームを進め、決定は
/// 直前に決定したフレームとそれを描く直前のキャンバスを進める。面は分け持ち、
/// どこからも指されなくなったところで落ちる。
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
}

impl Delta {
    pub(crate) fn new() -> Self {
        Delta {
            previous: Arc::default(),
            decided: Arc::default(),
            canvas: Arc::default(),
        }
    }

    /// 受け取った面を、直前に投入されたフレームとして覚える
    ///
    /// それまで直前に投入されていたフレームと、受け取ったフレームを返し、どちらの面も
    /// 投入されたフレームとして分け持つ。先頭フレームの直前は空の面になる。
    pub(crate) fn stage(&mut self, data: Vec<u8>) -> (Arc<Vec<u8>>, Arc<Vec<u8>>) {
        let previous = std::mem::replace(&mut self.previous, Arc::new(data));
        (previous, Arc::clone(&self.previous))
    }

    /// 決定を終えたフレームを覚え、保留中のフレームの `dispose` でキャンバスを進める
    pub(crate) fn advance(&mut self, data: Arc<Vec<u8>>, dispose: u8) {
        let decided = std::mem::replace(&mut self.decided, data);
        // 捨てない場合だけ、直前に決定したフレームがそのままキャンバスとして残る
        if dispose == DISPOSE_OP_NONE {
            self.canvas = decided;
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

    /// 直前に投入されたフレームの面
    #[cfg(test)]
    pub(crate) fn previous(&self) -> &Arc<Vec<u8>> {
        &self.previous
    }
}
