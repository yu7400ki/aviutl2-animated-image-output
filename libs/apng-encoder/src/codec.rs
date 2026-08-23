//! 領域のフィルタと圧縮、およびフィルタ戦略の決定

use crate::filter;
use crate::zlib::Compressor;

/// フィルタ戦略を決めるまでに両方の戦略で圧縮するフレーム数
///
/// 先頭フレームはキャンバス全体を書くため、差分矩形を書く以降のフレームとは
/// 中身の性質が違う。差分矩形のフレームも何枚か見てから決めるだけの回数を取る。
pub(crate) const PROBE_FRAMES: u32 = 4;

/// フィルタ戦略の決定
///
/// 先頭の [`PROBE_FRAMES`] フレームは両方の戦略で圧縮して小さい方を採り、
/// 圧縮後のバイト数を戦略ごとに積む。プローブを終えた時点で合計の小さい戦略へ
/// 固定し、以降のフレームはその戦略だけを実行する。
pub(crate) struct FilterChoice {
    /// 残りのプローブ回数
    remaining: u32,
    /// プローブで [`filter::Strategy::Adaptive`] が出した圧縮後バイト数の合計
    pub(crate) adaptive_bytes: u64,
    /// プローブで [`filter::Strategy::Unfiltered`] が出した圧縮後バイト数の合計
    pub(crate) unfiltered_bytes: u64,
    /// 固定した戦略。プローブが残っていれば `None`
    pub(crate) fixed: Option<filter::Strategy>,
}

/// プローブ1回ぶんの、戦略ごとの圧縮後バイト数
#[derive(Debug, Clone, Copy)]
pub(crate) struct Probe {
    pub(crate) adaptive: usize,
    pub(crate) unfiltered: usize,
}

impl FilterChoice {
    pub(crate) fn new() -> Self {
        FilterChoice {
            remaining: PROBE_FRAMES,
            adaptive_bytes: 0,
            unfiltered_bytes: 0,
            fixed: None,
        }
    }

    /// プローブ1回ぶんの圧縮後バイト数を記録し、残りが尽きたら戦略を固定する
    pub(crate) fn record(&mut self, probe: Probe) {
        debug_assert!(self.fixed.is_none());

        self.adaptive_bytes += probe.adaptive as u64;
        self.unfiltered_bytes += probe.unfiltered as u64;
        self.remaining -= 1;

        if self.remaining == 0 {
            self.fixed = Some(if self.adaptive_bytes < self.unfiltered_bytes {
                filter::Strategy::Adaptive
            } else {
                filter::Strategy::Unfiltered
            });
        }
    }
}

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

/// 圧縮した本体と、両方の戦略を試したときのバイト数
///
/// 採らなかった候補のプローブを記録できないよう、本体と対で受け渡す。
pub(crate) type Candidate = (Vec<u8>, Option<Probe>);

/// 領域をフィルタして圧縮する
///
/// 圧縮に使うバッファはプールから借り、使い終わったら返す。
pub(crate) struct Codec {
    compressor: Compressor,
    /// 行ごとのフィルタ選択に使う作業領域
    scratch: filter::Scratch,
    /// フィルタ後のバイト列を組み立てる領域
    filtered: Vec<u8>,
    /// 切り出した領域と圧縮した本体を回すバッファ
    pool: BufferPool,
    /// フィルタ戦略の決定
    pub(crate) choice: FilterChoice,
}

impl Codec {
    /// 圧縮レベル `level` (1..=9) で圧縮する
    pub(crate) fn new(level: u32) -> Self {
        Codec {
            compressor: Compressor::new(level),
            scratch: filter::Scratch::new(),
            filtered: Vec::new(),
            pool: BufferPool::new(),
            choice: FilterChoice::new(),
        }
    }

    /// 空のバッファを1つ借りる
    pub(crate) fn take(&mut self) -> Vec<u8> {
        self.pool.take()
    }

    /// 借りたバッファを返す
    pub(crate) fn give(&mut self, buffer: Vec<u8>) {
        self.pool.give(buffer);
    }

    /// 書き出すフレーム1つぶんのプローブを記録する
    ///
    /// dispose_opとblend_opの候補を選ぶための圧縮は、採らなかった側を二重に
    /// 数えないよう記録しない。
    pub(crate) fn record(&mut self, probe: Option<Probe>) {
        if let Some(probe) = probe {
            self.choice.record(probe);
        }
    }

    /// 連続した領域をフィルタして圧縮する
    ///
    /// フィルタ戦略が固まるまでは両方を試し、それ以降は固めた戦略だけを使う。
    /// 両方を試した場合は、そのバイト数を戦略ごとに添えて返す。
    pub(crate) fn compress(
        &mut self,
        region: &[u8],
        region_stride: usize,
        bpp: usize,
    ) -> Candidate {
        match self.choice.fixed {
            Some(strategy) => (
                self.compress_with(region, region_stride, bpp, strategy),
                None,
            ),
            None => {
                let (body, probe) = self.probe(region, region_stride, bpp);
                (body, Some(probe))
            }
        }
    }

    /// `strategy` でフィルタして圧縮する
    fn compress_with(
        &mut self,
        region: &[u8],
        region_stride: usize,
        bpp: usize,
        strategy: filter::Strategy,
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

        let mut body = self.pool.take();
        self.compressor.compress_into(&self.filtered, &mut body);
        body
    }

    /// 両方の戦略で圧縮し、小さい方を結果と合わせて返す
    ///
    /// 同じ大きさなら [`filter::Strategy::Unfiltered`] を残す。
    fn probe(&mut self, region: &[u8], region_stride: usize, bpp: usize) -> (Vec<u8>, Probe) {
        let adaptive_body =
            self.compress_with(region, region_stride, bpp, filter::Strategy::Adaptive);
        let unfiltered_body =
            self.compress_with(region, region_stride, bpp, filter::Strategy::Unfiltered);

        let (adaptive, unfiltered) = (adaptive_body.len(), unfiltered_body.len());
        let body = if adaptive < unfiltered {
            self.pool.give(unfiltered_body);
            adaptive_body
        } else {
            self.pool.give(adaptive_body);
            unfiltered_body
        };
        (
            body,
            Probe {
                adaptive,
                unfiltered,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(adaptive: usize, unfiltered: usize) -> Probe {
        Probe {
            adaptive,
            unfiltered,
        }
    }

    /// プローブは候補ごとの圧縮後バイト数を積み、合計の小さい方へ固める
    #[test]
    fn the_probe_fixes_the_candidate_with_the_smaller_total() {
        let mut choice = FilterChoice::new();
        for _ in 0..PROBE_FRAMES - 1 {
            choice.record(probe(100, 120));
            assert_eq!(choice.fixed, None);
        }
        choice.record(probe(100, 1));
        assert_eq!(choice.fixed, Some(filter::Strategy::Unfiltered));

        let mut choice = FilterChoice::new();
        for _ in 0..PROBE_FRAMES {
            choice.record(probe(100, 120));
        }
        assert_eq!(choice.fixed, Some(filter::Strategy::Adaptive));
    }

    /// 合計が同じならフィルタを掛けない方へ固める
    #[test]
    fn a_tie_settles_on_the_unfiltered_strategy() {
        let mut choice = FilterChoice::new();
        for _ in 0..PROBE_FRAMES {
            choice.record(probe(64, 64));
        }
        assert_eq!(choice.fixed, Some(filter::Strategy::Unfiltered));
    }
}
