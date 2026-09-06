//! ワーカープールと、番号を指して受け取る圧縮の結果

use crate::codec::{BufferPool, Candidate, Codec};
use crate::error::Error;
use crate::layout::Layout;
use std::any::Any;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};

/// ワーカーに付ける名前
const WORKER_NAME: &str = "apng-compress";

/// 圧縮を待つ、切り出し済みの領域1つ
///
/// 切り出した時点でフレームから独立し、圧縮はこのバッファだけを読む。
struct Job {
    /// 隙間なく並んだ矩形の中の画素
    region: Vec<u8>,
    /// 領域の1行のバイト数
    region_stride: usize,
    /// 圧縮した本体を組み立てるバッファ
    body: Vec<u8>,
}

/// ジョブ1つの結末
enum Outcome {
    Compressed(Candidate),
    /// ワーカーが巻き戻した。駆動側で投げ直す
    Panicked(Box<dyn Any + Send>),
}

/// ワーカーが返す、ジョブ1つの結末と切り出しに使ったバッファ
struct Done {
    /// 投入したジョブの番号
    index: usize,
    outcome: Outcome,
    region: Vec<u8>,
}

/// 圧縮を回すワーカーの群れ
struct Pool {
    /// ジョブの投入口。落とすとワーカーが順に抜ける
    jobs: Option<Sender<(usize, Job)>>,
    /// 投入済みのジョブを圧縮せずに捨てるか
    abandoned: Arc<AtomicBool>,
    results: Receiver<Done>,
    workers: Vec<JoinHandle<()>>,
}

/// 群れを畳む
///
/// 結末を待つ相手はもういないので、投入済みのジョブは圧縮せずに捨てる。
/// 待つのは圧縮に入っているぶんだけで、仕掛かりの上限まで溜まったジョブを
/// 端から焼き切ることはない。
impl Drop for Pool {
    fn drop(&mut self) {
        self.abandoned.store(true, Ordering::Relaxed);
        self.jobs = None;
        for worker in self.workers.drain(..) {
            drop(worker.join());
        }
    }
}

/// ジョブを1つずつ取り、圧縮して結末を返す
///
/// 巻き戻しで抜けたワーカーは抱えていたジョブの結末を返さず、駆動はその番号を
/// 待ち続ける。圧縮の巻き戻しは結末として持ち帰り、受け口の毒も取り出しだけは
/// 通して、この関数から巻き戻しの出口を無くす。
///
/// `abandoned` が立った後に取り出したジョブは捨てて抜ける。立てるのは群れを
/// 畳むときだけで、そこから先は結末を指す相手がいない。
fn work(
    codec: &mut Codec,
    layout: &Layout,
    jobs: &Mutex<Receiver<(usize, Job)>>,
    results: &Sender<Done>,
    abandoned: &AtomicBool,
) {
    loop {
        let received = jobs.lock().unwrap_or_else(PoisonError::into_inner).recv();
        let Ok((index, job)) = received else {
            return;
        };
        if abandoned.load(Ordering::Relaxed) {
            return;
        }

        let Job {
            region,
            region_stride,
            body,
        } = job;
        let compress = || codec.compress(&region, region_stride, layout.bytes_per_pixel, body);
        let outcome = match panic::catch_unwind(AssertUnwindSafe(compress)) {
            Ok(candidate) => Outcome::Compressed(candidate),
            Err(payload) => Outcome::Panicked(payload),
        };
        let done = Done {
            index,
            outcome,
            region,
        };
        if results.send(done).is_err() {
            return;
        }
    }
}

/// ワーカーを起こす
///
/// 圧縮器はワーカーの中で作る。
///
/// # Errors
/// スレッドを起こせないとき [`Error::Io`]。
fn spawn(level: u32, workers: NonZeroUsize, layout: Layout) -> Result<Pool, Error> {
    let (sender, receiver) = channel();
    let (results, done) = channel();
    let jobs = Arc::new(Mutex::new(receiver));

    let mut pool = Pool {
        jobs: Some(sender),
        abandoned: Arc::new(AtomicBool::new(false)),
        results: done,
        workers: Vec::with_capacity(workers.get()),
    };
    for _ in 0..workers.get() {
        let jobs = Arc::clone(&jobs);
        let results = results.clone();
        let abandoned = Arc::clone(&pool.abandoned);
        let worker = thread::Builder::new()
            .name(WORKER_NAME.to_owned())
            .spawn(move || work(&mut Codec::new(level), &layout, &jobs, &results, &abandoned))?;
        pool.workers.push(worker);
    }
    Ok(pool)
}

/// 圧縮を回し、番号を指して結果を受け取るパイプライン
///
/// ワーカーが2つ以上あるときだけ群れを起こす。1つなら投入した場で圧縮する。
/// 結果は届いた順に溜め、[`Pipeline::take`] が指した番号のものを返すので、
/// 受け取りの順は投入の順から独立している。
///
/// 領域と本体に使うバッファはここが配り、結果と一緒に受け取って配り直す。
pub(crate) struct Pipeline {
    /// 駆動スレッドで回す圧縮器
    codec: Codec,
    pool: Option<Pool>,
    /// 次に投入するジョブの番号
    submitted: usize,
    /// 番号を指されるのを待っている結末
    ready: HashMap<usize, Outcome>,
    /// 配り直すバッファ
    buffers: BufferPool,
    /// 圧縮を回すワーカー数
    workers: NonZeroUsize,
    /// キャンバスの大きさと入力フレームのバイト並び
    layout: Layout,
}

impl Pipeline {
    /// `layout` に並ぶフレームを、圧縮レベル `level` (1..=9) で圧縮する
    /// `workers` 個のワーカーを持つパイプラインを作る
    ///
    /// # Errors
    /// スレッドを起こせないとき [`Error::Io`]。
    pub(crate) fn new(level: u32, workers: NonZeroUsize, layout: Layout) -> Result<Self, Error> {
        let pool = match workers.get() {
            1 => None,
            _ => Some(spawn(level, workers, layout)?),
        };

        Ok(Pipeline {
            codec: Codec::new(level),
            pool,
            submitted: 0,
            ready: HashMap::new(),
            buffers: BufferPool::new(),
            workers,
            layout,
        })
    }

    /// 圧縮を回すワーカー数
    pub(crate) fn workers(&self) -> NonZeroUsize {
        self.workers
    }

    /// 空のバッファを1つ借りる
    pub(crate) fn buffer(&mut self) -> Vec<u8> {
        self.buffers.take()
    }

    /// 借りたバッファを返す
    pub(crate) fn recycle(&mut self, buffer: Vec<u8>) {
        self.buffers.give(buffer);
    }

    /// 配り直せるバッファの数
    #[cfg(test)]
    pub(crate) fn pooled(&self) -> usize {
        self.buffers.len()
    }

    /// 駆動スレッドで領域を圧縮する
    pub(crate) fn compress(&mut self, region: &[u8], region_stride: usize) -> Candidate {
        let body = self.buffer();
        self.codec
            .compress(region, region_stride, self.layout.bytes_per_pixel, body)
    }

    /// 領域の圧縮を投入し、結果を指すための番号を返す
    ///
    /// `region` はパイプラインが引き取り、[`Pipeline::buffer`] から配り直す。
    pub(crate) fn submit(&mut self, region: Vec<u8>, region_stride: usize) -> usize {
        let index = self.submitted;
        self.submitted += 1;
        let job = Job {
            region,
            region_stride,
            body: self.buffer(),
        };

        match &self.pool {
            Some(pool) => {
                let jobs = pool.jobs.as_ref().expect("投入口は畳むときだけ落とす");
                jobs.send((index, job)).expect("ワーカーは畳むまで受け取る");
            }
            None => {
                let Job {
                    region,
                    region_stride,
                    body,
                } = job;
                let candidate =
                    self.codec
                        .compress(&region, region_stride, self.layout.bytes_per_pixel, body);
                self.buffers.give(region);
                self.ready.insert(index, Outcome::Compressed(candidate));
            }
        }
        index
    }

    /// `index` のジョブの結果を受け取る
    ///
    /// 届いていなければ届くまで待つ。先に届いた別の番号の結果は溜めておく。
    pub(crate) fn take(&mut self, index: usize) -> Candidate {
        loop {
            if let Some(outcome) = self.ready.remove(&index) {
                return match outcome {
                    Outcome::Compressed(candidate) => candidate,
                    Outcome::Panicked(payload) => panic::resume_unwind(payload),
                };
            }

            let pool = self.pool.as_ref().expect("投入した場で圧縮が済んでいる");
            let done = pool
                .results
                .recv()
                .expect("ワーカーは結末を返してから抜ける");
            self.buffers.give(done.region);
            self.ready.insert(done.index, done.outcome);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::noise;
    use anim_core::ColorType;

    const LEVEL: u32 = 6;
    const BPP: usize = 4;
    const WIDTH: usize = 16;
    const HEIGHT: usize = 12;
    const STRIDE: usize = WIDTH * BPP;

    fn layout() -> Layout {
        Layout::new(WIDTH as u32, HEIGHT as u32, ColorType::Rgba8).unwrap()
    }

    fn pipeline(workers: usize) -> Pipeline {
        Pipeline::new(LEVEL, NonZeroUsize::new(workers).unwrap(), layout()).unwrap()
    }

    fn region(seed: u32) -> Vec<u8> {
        noise(STRIDE * HEIGHT, seed)
    }

    /// 投入した順と違う順で番号を指しても、指した番号の結果が返る
    #[test]
    fn a_result_is_taken_by_the_number_it_was_submitted_with() {
        let regions: Vec<Vec<u8>> = (0..8).map(region).collect();

        let mut sequential = pipeline(1);
        let expected: Vec<Vec<u8>> = regions
            .iter()
            .map(|region| {
                let index = sequential.submit(region.clone(), STRIDE);
                sequential.take(index).into_body()
            })
            .collect();

        let mut parallel = pipeline(4);
        let indices: Vec<usize> = regions
            .iter()
            .map(|region| parallel.submit(region.clone(), STRIDE))
            .collect();
        for (index, expected) in indices.into_iter().zip(&expected).rev() {
            assert_eq!(&parallel.take(index).into_body(), expected);
        }
    }

    /// 結果を受け取ったジョブの領域と、返した本体は配り直す先へ戻る
    #[test]
    fn the_buffer_of_a_taken_job_comes_back() {
        let region = region(0);
        let mut pipeline = pipeline(2);
        assert_eq!(pipeline.pooled(), 0, "配る前から抱えている");

        let index = pipeline.submit(region.clone(), STRIDE);
        let body = pipeline.take(index).into_body();
        assert_eq!(pipeline.pooled(), 1, "領域のバッファが戻っていない");

        pipeline.recycle(body);
        assert_eq!(pipeline.pooled(), 2, "返したバッファが戻っていない");

        let buffer = pipeline.buffer();
        assert_eq!(pipeline.pooled(), 1, "配ったバッファが残っている");
        assert!(buffer.is_empty(), "配ったバッファに中身がある");
    }

    /// 群れを畳むと、抜ける前に旗が立つ
    ///
    /// 旗を読む側と立てる側を別々に問う。立てるのは畳むときだけなので、
    /// 起こした直後は倒れている。
    #[test]
    fn dropping_the_pool_raises_the_flag() {
        let pool = spawn(LEVEL, NonZeroUsize::new(2).unwrap(), layout()).unwrap();
        let abandoned = Arc::clone(&pool.abandoned);

        assert!(
            !abandoned.load(Ordering::Relaxed),
            "起こした直後に立っている"
        );
        drop(pool);
        assert!(abandoned.load(Ordering::Relaxed), "畳んでも立っていない");
    }

    /// 畳んだ後に取り出したジョブは、圧縮せずに捨てる
    ///
    /// 結末を返さないので、捨てたぶんの番号は誰にも指されない。
    #[test]
    fn a_job_taken_from_an_abandoned_pool_is_discarded() {
        for abandoned in [false, true] {
            let (sender, receiver) = channel();
            let (results, done) = channel();
            let job = Job {
                region: region(0),
                region_stride: STRIDE,
                body: Vec::new(),
            };
            sender.send((0, job)).unwrap();
            drop(sender);

            work(
                &mut Codec::new(LEVEL),
                &layout(),
                &Mutex::new(receiver),
                &results,
                &AtomicBool::new(abandoned),
            );
            drop(results);

            assert_eq!(done.try_recv().is_ok(), !abandoned, "捨てる={abandoned}");
        }
    }
}
