//! 領域のフィルタと圧縮、戦略の選択、およびバッファの使い回し

use crate::filter;
use crate::zlib::Compressor;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::{self, JoinHandle};

/// 相方に付ける名前
const MATE_NAME: &str = "apng-deflate";

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
/// 本体を包んで持ち回り、退けたときにバッファをプールへ返す口を分けて持つ。
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

    /// 書き出す候補として本体を取り出す
    pub(crate) fn into_body(self) -> Vec<u8> {
        self.body
    }

    /// 退けた候補としてバッファをプールへ返す
    pub(crate) fn discard(self, codec: &mut Codec) {
        codec.give(self.body);
    }
}

/// 相方へ渡す、フィルタ後のバイト列と書き込み先
struct Job {
    filtered: Vec<u8>,
    body: Vec<u8>,
}

/// 渡されたバイト列を圧縮して返し続ける
fn work(level: u32, jobs: &Receiver<Job>, done: &Sender<Job>) {
    let compressor = Compressor::new(level);
    while let Ok(mut job) = jobs.recv() {
        compressor.compress_into(&job.filtered, &mut job.body);
        if done.send(job).is_err() {
            return;
        }
    }
}

/// 戦略の対の片方を受け持つ相方
///
/// [`Mate::submit`] で渡したバイト列は、[`Mate::wait`] が書き込み先ごと返す。
struct Mate {
    jobs: Option<Sender<Job>>,
    done: Receiver<Job>,
    worker: Option<JoinHandle<()>>,
}

impl Mate {
    /// 圧縮レベル `level` (1..=9) の相方を、並列に走らせる先がある環境で起こす
    fn start(level: u32) -> Option<Self> {
        if thread::available_parallelism().ok()?.get() < 2 {
            return None;
        }

        let (jobs, requests) = channel();
        let (replies, done) = channel();
        let worker = thread::Builder::new()
            .name(MATE_NAME.to_owned())
            .spawn(move || work(level, &requests, &replies))
            .ok()?;
        Some(Mate {
            jobs: Some(jobs),
            done,
            worker: Some(worker),
        })
    }

    /// `filtered` の圧縮を渡す
    fn submit(&self, filtered: Vec<u8>, body: Vec<u8>) {
        let jobs = self.jobs.as_ref().expect("畳むまで相方は居る");
        jobs.send(Job { filtered, body })
            .expect("畳むまで相方は居る");
    }

    /// 渡した圧縮の結末を待つ
    fn wait(&self) -> (Vec<u8>, Vec<u8>) {
        let job = self.done.recv().expect("畳むまで相方は居る");
        (job.filtered, job.body)
    }
}

/// 投入口を落として相方を畳む
impl Drop for Mate {
    fn drop(&mut self) {
        self.jobs = None;
        if let Some(worker) = self.worker.take() {
            drop(worker.join());
        }
    }
}

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
    /// [`filter::Strategy::Unfiltered`] の圧縮を受け持つ相方
    ///
    /// 相方が居ない環境では、対を順に走らせる。
    mate: Option<Mate>,
}

impl Codec {
    /// 圧縮レベル `level` (1..=9) で圧縮する
    pub(crate) fn new(level: u32) -> Self {
        Codec {
            compressor: Compressor::new(level),
            scratch: filter::Scratch::new(),
            filtered: Vec::new(),
            pool: BufferPool::new(),
            mate: Mate::start(level),
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

    /// プールが抱えているバッファの数
    #[cfg(test)]
    pub(crate) fn pooled(&self) -> usize {
        self.pool.free.len()
    }

    /// 連続した領域をフィルタして圧縮する
    ///
    /// 両方の戦略で圧縮し、短い方を採る。同じ大きさなら
    /// [`filter::Strategy::Unfiltered`] を残す。
    pub(crate) fn compress(
        &mut self,
        region: &[u8],
        region_stride: usize,
        bpp: usize,
    ) -> Candidate {
        let mut rows = self.pool.take();
        filter::filter_image(
            region,
            region_stride,
            bpp,
            filter::Strategy::Unfiltered,
            &mut self.scratch,
            &mut rows,
        );

        let mut body = self.pool.take();
        let unfiltered = match self.mate.as_ref() {
            Some(mate) => {
                mate.submit(rows, body);
                None
            }
            None => {
                self.compressor.compress_into(&rows, &mut body);
                self.pool.give(rows);
                Some(body)
            }
        };

        let adaptive = self.compress_with(region, region_stride, bpp, filter::Strategy::Adaptive);
        let unfiltered = unfiltered.unwrap_or_else(|| {
            let (rows, body) = self.mate.as_ref().expect("圧縮を渡した相方").wait();
            self.pool.give(rows);
            body
        });
        Candidate {
            body: self.shorter(adaptive, unfiltered),
        }
    }

    /// フィルタ後のバイト列を圧縮する
    fn deflate(&mut self, filtered: &[u8]) -> Vec<u8> {
        let mut body = self.pool.take();
        self.compressor.compress_into(filtered, &mut body);
        body
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

        let filtered = std::mem::take(&mut self.filtered);
        let body = self.deflate(&filtered);
        self.filtered = filtered;
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
        let adaptive = codec.compress_with(&region, STRIDE, BPP, filter::Strategy::Adaptive);
        let unfiltered = codec.compress_with(&region, STRIDE, BPP, filter::Strategy::Unfiltered);
        assert_eq!(
            adaptive.len(),
            unfiltered.len(),
            "同じ大きさに並ぶ素材であること"
        );
        assert_ne!(adaptive, unfiltered, "戦略ごとに中身が違うこと");

        let candidate = codec.compress(&region, STRIDE, BPP);
        assert_eq!(candidate.body, unfiltered);
    }

    /// 対を並列に走らせても順に走らせても、書き出す本体は変わらない
    #[test]
    fn running_the_pair_in_parallel_keeps_the_same_body() {
        for rows in [1, 256] {
            let region = noise(rows * STRIDE, rows as u32);
            let mut parallel = Codec::new(6);
            let mut serial = Codec::new(6);
            serial.mate = None;
            assert_eq!(
                parallel.compress(&region, STRIDE, BPP).body,
                serial.compress(&region, STRIDE, BPP).body,
                "{rows} 行"
            );
        }
    }

    /// 退けた候補のバッファはプールへ返り、書き出す候補のバッファは出ていく
    #[test]
    fn a_discarded_candidate_returns_its_buffer_to_the_pool() {
        let region = noise(WIDTH * HEIGHT * BPP, 1);
        let mut codec = Codec::new(6);

        let candidate = codec.compress(&region, STRIDE, BPP);
        let held = codec.pooled();
        candidate.discard(&mut codec);
        assert_eq!(codec.pooled(), held + 1);

        let candidate = codec.compress(&region, STRIDE, BPP);
        let held = codec.pooled();
        let body = candidate.into_body();
        assert_eq!(codec.pooled(), held);
        codec.give(body);
        assert_eq!(codec.pooled(), held + 1);
    }
}
