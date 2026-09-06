//! ワーカープールと、番号を指して受け取る圧縮の結果

use crate::codec::{BufferPool, Candidate, Codec};
use crate::error::Error;
use crate::layout::Layout;
use anim_core::{Rect, crop};
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

/// 圧縮を待つ仕事1つ
struct Job {
    /// 隙間なく並んだ矩形の中の画素を置くバッファ
    region: Vec<u8>,
    /// 圧縮した本体を組み立てるバッファ
    body: Vec<u8>,
    /// 領域の埋め方
    source: Source,
}

/// ジョブの領域を何から埋めるか
enum Source {
    /// 切り出し済み
    ///
    /// 領域は投入の時点でフレームから独立していて、圧縮はそのバッファだけを読む。
    Ready {
        /// 領域の1行のバイト数
        region_stride: usize,
    },
    /// フレームから `rect` を切り出して埋める
    Crop {
        /// 切り出す元のフレーム
        data: Arc<Vec<u8>>,
        /// 切り出す矩形
        rect: Rect,
    },
    /// キャンバスとフレームの差分の外接矩形を求め、そこを切り出して埋める
    ///
    /// 矩形の面積が `kept_area` に満たないときだけ切り出して圧縮する。それ以外は
    /// 候補にしない — 圧縮すれば小さくなることはあるが、それを測る圧縮の方が高くつく。
    Restored {
        /// 差分を採るキャンバス
        canvas: Arc<Vec<u8>>,
        /// 切り出す元のフレーム
        data: Arc<Vec<u8>>,
        /// 比べる相手の矩形の面積
        kept_area: u64,
    },
}

/// ジョブ1つの結末
enum Outcome {
    Compressed(Candidate),
    /// 走査で求めた矩形と、そこを圧縮した候補
    Restored {
        rect: Rect,
        candidate: Candidate,
    },
    /// 走査した矩形が広く、候補にしなかった。本体のバッファは使っていない
    Rejected {
        body: Vec<u8>,
    },
    /// ワーカーが巻き戻した。駆動側で投げ直す
    Panicked(Box<dyn Any + Send>),
}

/// ワーカーが返す、ジョブ1つの結末と領域のバッファ
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

/// ジョブ1つを片付け、結末と領域のバッファを返す
///
/// フレームから埋めるジョブは走査と切り出しを済ませてから圧縮する。そこまでの
/// 巻き戻しは結末として持ち帰るので、領域のバッファは巻き戻しても返る。
///
/// 返る時点でフレームを手放している。
fn run(codec: &mut Codec, layout: &Layout, job: Job) -> (Outcome, Vec<u8>) {
    let Job {
        mut region,
        body,
        source,
    } = job;
    let bpp = layout.bytes_per_pixel;

    let compress = || match source {
        Source::Ready { region_stride } => {
            Outcome::Compressed(codec.compress(&region, region_stride, bpp, body))
        }
        Source::Crop { data, rect } => {
            crop(&data, rect, layout.stride, bpp, bpp, &mut region);
            let region_stride = rect.width as usize * bpp;
            Outcome::Compressed(codec.compress(&region, region_stride, bpp, body))
        }
        Source::Restored {
            canvas,
            data,
            kept_area,
        } => {
            let rect = layout.bounding_rect(&canvas, &data);
            if rect.area() >= kept_area {
                return Outcome::Rejected { body };
            }

            crop(&data, rect, layout.stride, bpp, bpp, &mut region);
            let region_stride = rect.width as usize * bpp;
            Outcome::Restored {
                rect,
                candidate: codec.compress(&region, region_stride, bpp, body),
            }
        }
    };
    let outcome = match panic::catch_unwind(AssertUnwindSafe(compress)) {
        Ok(outcome) => outcome,
        Err(payload) => Outcome::Panicked(payload),
    };
    (outcome, region)
}

/// ジョブを1つずつ取り、圧縮して結末を返す
///
/// 巻き戻しで抜けたワーカーは抱えていたジョブの結末を返さず、駆動はその番号を
/// 待ち続ける。ジョブの巻き戻しは結末として持ち帰り、受け口の毒も取り出しだけは
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

        let (outcome, region) = run(codec, layout, job);
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
/// 結末は届いた順に溜め、指された番号のものを返すので、受け取りの順は投入の順から
/// 独立している。
///
/// 領域と本体に使うバッファはここが配り、結果と一緒に受け取って配り直す。
pub(crate) struct Pipeline {
    /// ワーカーが1つのときに、投入した場で回す圧縮器
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

    /// 切り出し済みの領域の圧縮を投入し、結果を指すための番号を返す
    ///
    /// `region` はパイプラインが引き取り、[`Pipeline::buffer`] から配り直す。
    pub(crate) fn submit_region(&mut self, region: Vec<u8>, region_stride: usize) -> usize {
        let body = self.buffer();
        self.submit(Job {
            region,
            body,
            source: Source::Ready { region_stride },
        })
    }

    /// フレームから `rect` を切り出す圧縮を投入し、結果を指すための番号を返す
    ///
    /// 切り出し先は [`Pipeline::buffer`] から借り、結果と一緒に配り直す。
    pub(crate) fn submit_crop(&mut self, data: Arc<Vec<u8>>, rect: Rect) -> usize {
        let region = self.buffer();
        let body = self.buffer();
        self.submit(Job {
            region,
            body,
            source: Source::Crop { data, rect },
        })
    }

    /// [`Source::Restored`] の圧縮を投入し、結果を指すための番号を返す
    ///
    /// 領域と本体は [`Pipeline::buffer`] から借り、どちらの結末でも配り直す。
    pub(crate) fn submit_restored(
        &mut self,
        canvas: Arc<Vec<u8>>,
        data: Arc<Vec<u8>>,
        kept_area: u64,
    ) -> usize {
        let region = self.buffer();
        let body = self.buffer();
        self.submit(Job {
            region,
            body,
            source: Source::Restored {
                canvas,
                data,
                kept_area,
            },
        })
    }

    /// ジョブを投入し、結果を指すための番号を返す
    ///
    /// ワーカーが1つなら、投入した場で片付けて結末を溜める。
    fn submit(&mut self, job: Job) -> usize {
        let index = self.submitted;
        self.submitted += 1;

        match &self.pool {
            Some(pool) => {
                let jobs = pool.jobs.as_ref().expect("投入口は畳むときだけ落とす");
                jobs.send((index, job)).expect("ワーカーは畳むまで受け取る");
            }
            None => {
                let (outcome, region) = run(&mut self.codec, &self.layout, job);
                self.buffers.give(region);
                match outcome {
                    Outcome::Panicked(payload) => panic::resume_unwind(payload),
                    outcome => {
                        self.ready.insert(index, outcome);
                    }
                }
            }
        }
        index
    }

    /// `index` の領域を圧縮するジョブの結果を受け取る
    pub(crate) fn take(&mut self, index: usize) -> Candidate {
        match self.wait(index) {
            Outcome::Compressed(candidate) => candidate,
            Outcome::Panicked(payload) => panic::resume_unwind(payload),
            Outcome::Restored { .. } | Outcome::Rejected { .. } => {
                panic!("領域を圧縮するジョブの番号を指している")
            }
        }
    }

    /// `index` の差分を走査するジョブの結果を受け取る
    ///
    /// 候補にしなかったときは本体のバッファを配り直して `None` を返す。
    pub(crate) fn take_restored(&mut self, index: usize) -> Option<(Rect, Candidate)> {
        match self.wait(index) {
            Outcome::Restored { rect, candidate } => Some((rect, candidate)),
            Outcome::Rejected { body } => {
                self.buffers.give(body);
                None
            }
            Outcome::Panicked(payload) => panic::resume_unwind(payload),
            Outcome::Compressed(_) => panic!("差分を走査するジョブの番号を指している"),
        }
    }

    /// `index` のジョブの結末を受け取る
    ///
    /// 届いていなければ届くまで待つ。先に届いた別の番号の結末は溜めておく。
    fn wait(&mut self, index: usize) -> Outcome {
        loop {
            if let Some(outcome) = self.ready.remove(&index) {
                return outcome;
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
                let index = sequential.submit_region(region.clone(), STRIDE);
                sequential.take(index).into_body()
            })
            .collect();

        let mut parallel = pipeline(4);
        let indices: Vec<usize> = regions
            .iter()
            .map(|region| parallel.submit_region(region.clone(), STRIDE))
            .collect();
        for (index, expected) in indices.into_iter().zip(&expected).rev() {
            assert_eq!(&parallel.take(index).into_body(), expected);
        }
    }

    /// 切り出しのジョブは、同じ矩形を切り出した領域のジョブと同じ候補を返す
    #[test]
    fn a_crop_job_compresses_what_the_rect_cuts_out() {
        const RECT: Rect = Rect {
            x: 3,
            y: 2,
            width: 9,
            height: 7,
        };

        let frame = Arc::new(region(5));
        let mut cut = Vec::new();
        crop(&frame, RECT, STRIDE, BPP, BPP, &mut cut);

        for workers in [1, 2] {
            let mut pipeline = pipeline(workers);
            let index = pipeline.submit_region(cut.clone(), RECT.width as usize * BPP);
            let expected = pipeline.take(index).into_body();

            let index = pipeline.submit_crop(Arc::clone(&frame), RECT);
            assert_eq!(
                pipeline.take(index).into_body(),
                expected,
                "ワーカー{workers}個"
            );
        }
    }

    /// 復元のジョブは、走査した矩形が比べる相手の面積に満たないときだけ候補になる
    ///
    /// 面積が並ぶ相手には候補を立てず、領域と本体のバッファをそのまま返す。1つ広い
    /// 相手には切り出して圧縮し、走査した矩形と、同じ矩形を切り出したジョブと同じ
    /// 本体を返す。
    #[test]
    fn a_restored_job_is_a_candidate_only_below_the_area_it_is_compared_with() {
        const RECT: Rect = Rect {
            x: 4,
            y: 3,
            width: 6,
            height: 5,
        };

        let canvas = Arc::new(region(11));
        let mut frame = canvas.as_ref().clone();
        for y in RECT.y as usize..(RECT.y + RECT.height) as usize {
            for x in RECT.x as usize..(RECT.x + RECT.width) as usize {
                for byte in &mut frame[(y * WIDTH + x) * BPP..(y * WIDTH + x + 1) * BPP] {
                    *byte ^= 0xFF;
                }
            }
        }
        let frame = Arc::new(frame);

        for workers in [1, 2] {
            let mut pipeline = pipeline(workers);
            let index =
                pipeline.submit_restored(Arc::clone(&canvas), Arc::clone(&frame), RECT.area());
            assert!(
                pipeline.take_restored(index).is_none(),
                "ワーカー{workers}個: 面積が並ぶ矩形の候補"
            );
            assert_eq!(
                pipeline.pooled(),
                2,
                "ワーカー{workers}個: 領域と本体の戻り"
            );

            let index = pipeline.submit_crop(Arc::clone(&frame), RECT);
            let expected = pipeline.take(index).into_body();

            let index =
                pipeline.submit_restored(Arc::clone(&canvas), Arc::clone(&frame), RECT.area() + 1);
            let (rect, candidate) = pipeline
                .take_restored(index)
                .expect("面積で落ちない矩形の候補");
            assert_eq!(rect, RECT, "ワーカー{workers}個: 走査した矩形");
            assert_eq!(
                candidate.into_body(),
                expected,
                "ワーカー{workers}個: 圧縮した本体"
            );
        }
    }

    /// 結果を受け取ったジョブの領域と、返した本体は配り直す先へ戻る
    #[test]
    fn the_buffer_of_a_taken_job_comes_back() {
        let region = region(0);
        let mut pipeline = pipeline(2);
        assert_eq!(pipeline.pooled(), 0, "配る前から抱えている");

        let index = pipeline.submit_region(region.clone(), STRIDE);
        let body = pipeline.take(index).into_body();
        assert_eq!(pipeline.pooled(), 1, "領域のバッファが戻っていない");

        pipeline.recycle(body);
        assert_eq!(pipeline.pooled(), 2, "返したバッファが戻っていない");

        let buffer = pipeline.buffer();
        assert_eq!(pipeline.pooled(), 1, "配ったバッファが残っている");
        assert!(buffer.is_empty(), "配ったバッファに中身がある");
    }

    /// 切り出しのジョブを受け取ると、切り出し先が戻り、フレームを手放している
    ///
    /// ワーカーは結末を返す前にフレームを手放すので、受け取った側はそのフレームを
    /// 次の写し先へ回せる。
    #[test]
    fn a_taken_crop_job_gives_back_its_region_and_frame() {
        const RECT: Rect = Rect {
            x: 1,
            y: 1,
            width: 8,
            height: 6,
        };

        for workers in [2, 4] {
            let mut pipeline = pipeline(workers);
            let frame = Arc::new(region(9));

            let index = pipeline.submit_crop(Arc::clone(&frame), RECT);
            drop(pipeline.take(index));

            assert_eq!(
                pipeline.pooled(),
                1,
                "ワーカー{workers}個: 切り出し先の戻り"
            );
            assert_eq!(
                Arc::strong_count(&frame),
                1,
                "ワーカー{workers}個: フレームを指している数"
            );
        }
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
                body: Vec::new(),
                source: Source::Ready {
                    region_stride: STRIDE,
                },
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
