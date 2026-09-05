//! 領域のフィルタと圧縮、戦略の選択、およびバッファの使い回し

use crate::filter;
use crate::zlib::Compressor;

/// 使い終わったバッファを溜めて配り直す領域
///
/// 圧縮した本体と切り出した領域はフレームごとに同じ大きさへ落ち着くため、
/// 一度確保した容量をそのまま次のフレームへ回す。
struct BufferPool {
    free: Vec<Vec<u8>>,
}

impl BufferPool {
    fn new() -> Self {
        BufferPool { free: Vec::new() }
    }

    /// 空のバッファを1つ借りる
    fn take(&mut self) -> Vec<u8> {
        let mut buffer = self.free.pop().unwrap_or_default();
        buffer.clear();
        buffer
    }

    /// 借りたバッファを返す
    fn give(&mut self, buffer: Vec<u8>) {
        self.free.push(buffer);
    }
}

/// 圧縮した候補1つ
///
/// 本体を包んで持ち回る。
#[must_use]
pub(crate) struct Candidate {
    /// フィルタして圧縮した本体
    body: Vec<u8>,
}

impl Candidate {
    /// 圧縮した本体のバイト数
    pub(crate) fn len(&self) -> usize {
        self.body.len()
    }

    /// 本体を取り出す
    pub(crate) fn into_body(self) -> Vec<u8> {
        self.body
    }
}

/// 領域をフィルタして圧縮する
///
/// 対のもう一方に使うバッファはプールから借り、退けた側を返す。
pub(crate) struct Codec {
    compressor: Compressor,
    /// 行ごとのフィルタ選択に使う作業領域
    scratch: filter::Scratch,
    /// フィルタ後のバイト列を組み立てる領域
    filtered: Vec<u8>,
    /// 対のもう一方の本体を回すバッファ
    pool: BufferPool,
}

impl Codec {
    /// 圧縮レベル `level` (1..=9) で圧縮する
    pub(crate) fn new(level: u32) -> Self {
        Codec {
            compressor: Compressor::new(level),
            scratch: filter::Scratch::new(),
            filtered: Vec::new(),
            pool: BufferPool::new(),
        }
    }

    /// プールが抱えているバッファの数
    #[cfg(test)]
    pub(crate) fn pooled(&self) -> usize {
        self.pool.free.len()
    }

    /// 連続した領域をフィルタし、`body` へ圧縮する
    ///
    /// 両方の戦略で圧縮し、短い方を採る。同じ大きさなら
    /// [`filter::Strategy::Unfiltered`] を残す。もう一方に使ったバッファは
    /// プールへ残る。
    pub(crate) fn compress(
        &mut self,
        region: &[u8],
        region_stride: usize,
        bpp: usize,
        body: Vec<u8>,
    ) -> Candidate {
        let unfiltered = self.compress_with(
            region,
            region_stride,
            bpp,
            filter::Strategy::Unfiltered,
            body,
        );
        let spare = self.pool.take();
        let adaptive = self.compress_with(
            region,
            region_stride,
            bpp,
            filter::Strategy::Adaptive,
            spare,
        );
        Candidate {
            body: self.shorter(adaptive, unfiltered),
        }
    }

    /// 短い方を残し、もう一方のバッファをプールへ返す
    fn shorter(&mut self, adaptive: Vec<u8>, unfiltered: Vec<u8>) -> Vec<u8> {
        if adaptive.len() < unfiltered.len() {
            self.pool.give(unfiltered);
            adaptive
        } else {
            self.pool.give(adaptive);
            unfiltered
        }
    }

    /// `strategy` でフィルタし、`body` へ圧縮する
    fn compress_with(
        &mut self,
        region: &[u8],
        region_stride: usize,
        bpp: usize,
        strategy: filter::Strategy,
        mut body: Vec<u8>,
    ) -> Vec<u8> {
        self.filtered.clear();
        filter::filter_image(
            region,
            region_stride,
            bpp,
            strategy,
            &mut self.scratch,
            &mut self.filtered,
        );

        body.clear();
        self.compressor.compress_into(&self.filtered, &mut body);
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::noise;

    const WIDTH: usize = 8;
    const HEIGHT: usize = 8;
    const BPP: usize = 4;
    const STRIDE: usize = WIDTH * BPP;

    /// 両方の戦略が同じ大きさなら、フィルタを掛けない側の本体を残す
    ///
    /// 擬似乱数の領域はどちらの戦略でも縮まず、圧縮後の大きさが並ぶ。大きさが
    /// 同じでも中身は違うため、どちらを残したかがそのまま書き出すバイト列になる。
    #[test]
    fn a_tie_keeps_the_unfiltered_body() {
        let region = noise(WIDTH * HEIGHT * BPP, 0);
        let mut codec = Codec::new(6);
        let adaptive =
            codec.compress_with(&region, STRIDE, BPP, filter::Strategy::Adaptive, Vec::new());
        let unfiltered = codec.compress_with(
            &region,
            STRIDE,
            BPP,
            filter::Strategy::Unfiltered,
            Vec::new(),
        );
        assert_eq!(
            adaptive.len(),
            unfiltered.len(),
            "同じ大きさに並ぶ素材であること"
        );
        assert_ne!(adaptive, unfiltered, "戦略ごとに中身が違うこと");

        let candidate = codec.compress(&region, STRIDE, BPP, Vec::new());
        assert_eq!(candidate.body, unfiltered);
    }

    /// 退けた側のバッファはプールへ残り、圧縮のたびに配り直される
    ///
    /// 出ていくのは採った本体だけなので、渡すバッファが1つでも対を圧縮できる。
    #[test]
    fn the_losing_strategy_leaves_its_buffer_in_the_pool() {
        let region = noise(WIDTH * HEIGHT * BPP, 1);
        let mut codec = Codec::new(6);
        assert_eq!(codec.pooled(), 0);

        let mut body = codec.compress(&region, STRIDE, BPP, Vec::new()).into_body();
        assert_eq!(codec.pooled(), 1);

        body = codec.compress(&region, STRIDE, BPP, body).into_body();
        assert_eq!(codec.pooled(), 1);
        assert!(!body.is_empty());
    }
}
