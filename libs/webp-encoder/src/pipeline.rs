//! ワーカープールと、投入順に揃える結果の受け取り

use crate::codec::{Codec, EncodedFrame, Job};
use crate::error::Error;
use crate::layout::{ColorType, Layout};
use std::any::Any;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};

/// ワーカーに付ける名前
const WORKER_NAME: &str = "webp-encode";

/// 関数表を据えるために通す画素 (RGBA)
///
/// αを中間の値にして、透過を持つ入力が通る経路まで含める。
const WARM_UP_PIXEL: [u8; 4] = [0x7F, 0x7F, 0x7F, 0x80];

/// ジョブ1つの結末
enum Outcome {
    Encoded(EncodedFrame),
    Failed(Error),
    /// ワーカーが巻き戻した。駆動側で投げ直す
    Panicked(Box<dyn Any + Send>),
}

/// ワーカーが返す、ジョブ1つの結末と切り出しに使ったバッファ
struct Done {
    /// 投入したジョブの番号
    index: usize,
    outcome: Outcome,
    buffer: Vec<u8>,
}

/// 符号化を回すワーカーの群れ
struct Pool {
    /// ジョブの投入口。落とすとワーカーが順に抜ける
    jobs: Option<Sender<(usize, Job)>>,
    results: Receiver<Done>,
    workers: Vec<JoinHandle<()>>,
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.jobs = None;
        for worker in self.workers.drain(..) {
            drop(worker.join());
        }
    }
}

/// ジョブを1つずつ取り、符号化して結末を返す
///
/// 巻き戻しで抜けたワーカーは抱えていたジョブの結末を返さず、駆動はその番号を
/// 待ち続ける。符号化の巻き戻しは結末として持ち帰り、受け口の毒も取り出しだけは
/// 通して、この関数から巻き戻しの出口を無くす。
fn work(codec: &Codec, jobs: &Mutex<Receiver<(usize, Job)>>, results: &Sender<Done>) {
    loop {
        let received = jobs.lock().unwrap_or_else(PoisonError::into_inner).recv();
        let Ok((index, job)) = received else {
            return;
        };

        let outcome = match panic::catch_unwind(AssertUnwindSafe(|| codec.encode(&job))) {
            Ok(Ok(encoded)) => Outcome::Encoded(encoded),
            Ok(Err(error)) => Outcome::Failed(error),
            Err(payload) => Outcome::Panicked(payload),
        };
        let done = Done {
            index,
            outcome,
            buffer: job.into_pixels(),
        };
        if results.send(done).is_err() {
            return;
        }
    }
}

/// ワーカーを起こす
///
/// # Errors
/// スレッドを起こせないとき [`Error::Io`]。
fn spawn(codec: Codec, workers: NonZeroUsize) -> Result<Pool, Error> {
    let (sender, receiver) = channel();
    let (results, done) = channel();
    let jobs = Arc::new(Mutex::new(receiver));

    let mut pool = Pool {
        jobs: Some(sender),
        results: done,
        workers: Vec::with_capacity(workers.get()),
    };
    for _ in 0..workers.get() {
        let jobs = Arc::clone(&jobs);
        let results = results.clone();
        let worker = thread::Builder::new()
            .name(WORKER_NAME.to_owned())
            .spawn(move || work(&codec, &jobs, &results))?;
        pool.workers.push(worker);
    }
    Ok(pool)
}

/// libwebp の関数表を1回だけ据える
///
/// 関数表の初期化は最初の符号化のときに走り、Windows 版の見張りは同期を持たない。
/// ワーカーが同時に初回を踏まないよう、起こす前に1フレーム通しておく。
///
/// # Errors
/// 符号化に失敗したとき [`Error::Encode`]。
fn warm_up(codec: &Codec, color_type: ColorType) -> Result<(), Error> {
    let layout = Layout::new(1, 1, color_type)?;
    let pixel = &WARM_UP_PIXEL[..color_type.bytes_per_pixel()];
    codec.encode(&Job::crop(pixel, &layout, layout.whole(), None, Vec::new()))?;
    Ok(())
}

/// 符号化を回し、番号を指して結果を受け取るパイプライン
///
/// ワーカーが2つ以上あるときだけ群れを起こす。1つなら投入した場に符号化する。
/// 結果は届いた順に溜め、[`Pipeline::take`] が指した番号のものを返すので、
/// 受け取りの順は投入の順から独立している。
pub(crate) struct Pipeline {
    codec: Codec,
    pool: Option<Pool>,
    /// 次に投入するジョブの番号
    submitted: usize,
    /// 番号を指されるのを待っている結末
    ready: HashMap<usize, Outcome>,
    /// 使い回す切り出し先
    buffers: Vec<Vec<u8>>,
    /// 仕掛かりの上限
    capacity: usize,
}

impl Pipeline {
    /// `workers` 個のワーカーで `codec` を回すパイプラインを作る
    ///
    /// # Errors
    /// 関数表を据える符号化に失敗したとき [`Error::Encode`]。スレッドを
    /// 起こせないとき [`Error::Io`]。
    pub(crate) fn new(
        codec: Codec,
        color_type: ColorType,
        workers: NonZeroUsize,
    ) -> Result<Self, Error> {
        let pool = if workers.get() > 1 {
            warm_up(&codec, color_type)?;
            Some(spawn(codec, workers)?)
        } else {
            None
        };

        Ok(Pipeline {
            codec,
            pool,
            submitted: 0,
            ready: HashMap::new(),
            buffers: Vec::new(),
            capacity: workers.get() * 2,
        })
    }

    /// 符号化を待たせておけるフレーム数
    ///
    /// 仕掛かりはジョブ1つにつき切り出し済みのバッファ1つと符号化の結果1つを
    /// 抱えるので、この数が抱える画素の上限を決める。
    pub(crate) fn capacity(&self) -> usize {
        self.capacity
    }

    /// 設定を写した符号化器
    pub(crate) fn codec(&self) -> &Codec {
        &self.codec
    }

    /// 切り出し先に使うバッファ
    ///
    /// 結果を受け取ったジョブのバッファが戻ってくる。
    pub(crate) fn buffer(&mut self) -> Vec<u8> {
        self.buffers.pop().unwrap_or_default()
    }

    /// ジョブを投入し、結果を指すための番号を返す
    pub(crate) fn submit(&mut self, job: Job) -> usize {
        let index = self.submitted;
        self.submitted += 1;

        match &self.pool {
            Some(pool) => {
                let jobs = pool.jobs.as_ref().expect("投入口は畳むときだけ落とす");
                jobs.send((index, job)).expect("ワーカーは畳むまで受け取る");
            }
            None => {
                let outcome = match self.codec.encode(&job) {
                    Ok(encoded) => Outcome::Encoded(encoded),
                    Err(error) => Outcome::Failed(error),
                };
                self.buffers.push(job.into_pixels());
                self.ready.insert(index, outcome);
            }
        }
        index
    }

    /// `index` のジョブの結果を受け取る
    ///
    /// 届いていなければ届くまで待つ。先に届いた別の番号の結果は溜めておく。
    ///
    /// # Errors
    /// そのジョブの符号化に失敗したとき [`Error::Encode`]。結果のチャンク構成を
    /// 読み取れないとき [`Error::MalformedOutput`]。
    pub(crate) fn take(&mut self, index: usize) -> Result<EncodedFrame, Error> {
        loop {
            if let Some(outcome) = self.ready.remove(&index) {
                return match outcome {
                    Outcome::Encoded(encoded) => Ok(encoded),
                    Outcome::Failed(error) => Err(error),
                    Outcome::Panicked(payload) => panic::resume_unwind(payload),
                };
            }

            let pool = self.pool.as_ref().expect("投入した場で符号化が済んでいる");
            let done = pool
                .results
                .recv()
                .expect("ワーカーは結末を返してから抜ける");
            self.buffers.push(done.buffer);
            self.ready.insert(done.index, done.outcome);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    fn config(color_type: ColorType) -> Config {
        Config {
            color_type,
            lossless: true,
            quality: 75.0,
            method: 4,
            num_plays: 0,
        }
    }

    fn pipeline(color_type: ColorType, workers: usize) -> Pipeline {
        let codec = Codec::new(&config(color_type)).unwrap();
        Pipeline::new(codec, color_type, NonZeroUsize::new(workers).unwrap()).unwrap()
    }

    /// 画素ごとに値の違うRGBA
    fn ramp(width: u32, height: u32, seed: u32) -> Vec<u8> {
        (0..height)
            .flat_map(|y| {
                (0..width).flat_map(move |x| {
                    [
                        (x * 7 + seed) as u8,
                        (y * 11 + seed) as u8,
                        (x + y + seed) as u8,
                        0xFF,
                    ]
                })
            })
            .collect()
    }

    /// 投入した順と違う順で番号を指しても、指した番号の結果が返る
    #[test]
    fn a_result_is_taken_by_the_number_it_was_submitted_with() {
        let layout = Layout::new(16, 12, ColorType::Rgba8).unwrap();
        let frames: Vec<Vec<u8>> = (0..8).map(|seed| ramp(16, 12, seed)).collect();

        let mut expected = Vec::new();
        let mut sequential = pipeline(ColorType::Rgba8, 1);
        for data in &frames {
            let buffer = sequential.buffer();
            let index = sequential.submit(Job::crop(data, &layout, layout.whole(), None, buffer));
            expected.push(sequential.take(index).unwrap().still().to_vec());
        }

        let mut parallel = pipeline(ColorType::Rgba8, 4);
        let indices: Vec<usize> = frames
            .iter()
            .map(|data| {
                let buffer = parallel.buffer();
                parallel.submit(Job::crop(data, &layout, layout.whole(), None, buffer))
            })
            .collect();
        for (index, expected) in indices.into_iter().zip(&expected) {
            assert_eq!(&parallel.take(index).unwrap().still(), expected);
        }
    }

    /// 結果を受け取ったジョブのバッファは切り出し先へ戻る
    #[test]
    fn the_buffer_of_a_taken_job_comes_back() {
        let layout = Layout::new(16, 12, ColorType::Rgba8).unwrap();
        let data = ramp(16, 12, 0);

        let mut pipeline = pipeline(ColorType::Rgba8, 2);
        assert!(pipeline.buffer().is_empty());

        let index = pipeline.submit(Job::crop(&data, &layout, layout.whole(), None, Vec::new()));
        pipeline.take(index).unwrap();
        assert!(pipeline.buffer().capacity() >= layout.frame_len);
    }
}
