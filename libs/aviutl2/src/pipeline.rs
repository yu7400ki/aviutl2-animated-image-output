//! フレームの取り込みと符号化を別々のスレッドで重ねて走らせる

use crate::output::OutputInfo;
use crate::pixel::ColorFormat;
use std::sync::mpsc::{SyncSender, sync_channel};

/// 符号化側へ渡すフレームを溜めておける数
///
/// 取り込みのほうが遅いので定常状態では溜まらない。この深さは符号化側が
/// 一時的に遅れたときの緩衝でしかなく、大きくしても取り込みが先走って
/// メモリを食うだけになる。1920x1080 のRGBAで1フレーム約8.3MB、
/// 取り込み中・溜め・符号化中を合わせて同時に生きるのは約33MB。
const CHANNEL_DEPTH: usize = 2;

/// 取り込み側が1フレームについて得た結果
enum Fetched {
    /// フレームを取り込んだ
    Frame(Vec<u8>),
    /// ホストがフレームを返さなかった
    Missing,
    /// ホストが中断を報せた
    Aborted,
}

/// 取り込みと符号化を重ねている間に起きた失敗
#[derive(Debug, PartialEq, Eq)]
pub enum PipelineError<E> {
    /// ホストが中断を報せた
    Aborted,
    /// ホストがフレームを返さなかった
    FrameUnavailable(i32),
    /// 符号化側が返した失敗
    Encode(E),
}

impl<E: std::fmt::Display> std::fmt::Display for PipelineError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PipelineError::Aborted => write!(f, "処理が中断されました"),
            PipelineError::FrameUnavailable(frame) => {
                write!(f, "フレーム取得エラー: フレーム {}", frame)
            }
            PipelineError::Encode(e) => write!(f, "フレーム書き込みエラー: {}", e),
        }
    }
}

/// 取り込みを呼び出し元のスレッドに残したまま、符号化を別スレッドで進める
///
/// `fetch` は呼び出し元のスレッドで `frames` 回まで呼ばれ、`sink` は
/// 作業スレッドで取り込んだ順に呼ばれる。
///
/// `sink` が失敗すると取り込みも止まり、その失敗が返る。取り込み側が
/// 先に止まっていても符号化側の失敗を優先する (エンコーダは一度失敗すると
/// それ以降の投入に意味が無いため)。
fn run<F, S, E>(frames: i32, mut fetch: F, mut sink: S) -> Result<(), PipelineError<E>>
where
    F: FnMut(i32) -> Fetched,
    S: FnMut(Vec<u8>) -> Result<(), E> + Send,
    E: Send,
{
    let (sender, receiver) = sync_channel::<Vec<u8>>(CHANNEL_DEPTH);

    std::thread::scope(|scope| {
        let worker = scope.spawn(move || -> Result<(), E> {
            for frame in receiver {
                sink(frame)?;
            }
            Ok(())
        });

        let fetch_result = feed(frames, &mut fetch, sender);

        match worker.join() {
            Ok(Ok(())) => fetch_result,
            Ok(Err(e)) => Err(PipelineError::Encode(e)),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    })
}

/// 取り込んだフレームを作業スレッドへ順に送る
///
/// 送り口を返さずに落とすことで、受け手のループが終わり [`run`] の `join` が返る。
fn feed<F, E>(
    frames: i32,
    fetch: &mut F,
    sender: SyncSender<Vec<u8>>,
) -> Result<(), PipelineError<E>>
where
    F: FnMut(i32) -> Fetched,
{
    for frame in 0..frames {
        match fetch(frame) {
            Fetched::Frame(data) => {
                // 送れないのは受け手が失敗して落ちたときだけで、理由はjoinで受け取る
                if sender.send(data).is_err() {
                    return Ok(());
                }
            }
            Fetched::Missing => return Err(PipelineError::FrameUnavailable(frame)),
            Fetched::Aborted => return Err(PipelineError::Aborted),
        }
    }
    Ok(())
}

impl OutputInfo<'_> {
    /// 全フレームをホストから取り込み、`sink` へ順に渡す
    ///
    /// ホストの関数 (フレーム取得・中断確認・残り時間表示) は呼び出し元の
    /// スレッドに留まり、`sink` だけが作業スレッドで走る。取り込みと符号化が
    /// 重なるので、1フレームあたりの所要は両者の遅いほうに近づく。
    ///
    /// `sink` はフレームを取り込んだ順に、フレームごとに1回だけ呼ばれる。
    /// 中断されるか失敗すると、そこから先のフレームは渡らない。
    ///
    /// エンコーダは呼び出し元が持ったままなので、戻ったあとに終端処理や
    /// 統計の取り出しを行える。
    pub fn encode_frames<S, E>(&self, format: ColorFormat, sink: S) -> Result<(), PipelineError<E>>
    where
        S: FnMut(Vec<u8>) -> Result<(), E> + Send,
        E: Send,
    {
        let frames = self.num_frames();

        run(
            frames,
            |frame| {
                if self.is_abort() {
                    return Fetched::Aborted;
                }
                match self.get_video_frame(frame, format) {
                    Some(data) => {
                        self.rest_time_disp(frame, frames);
                        Fetched::Frame(data)
                    }
                    None => Fetched::Missing,
                }
            },
            sink,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::thread::ThreadId;

    /// 取り込み側が呼ばれたフレーム番号と、符号化側が受け取ったフレーム
    #[derive(Default)]
    struct Trace {
        fetched: Vec<i32>,
        encoded: Vec<Vec<u8>>,
        sink_threads: Vec<ThreadId>,
    }

    /// フレーム番号をそのまま中身にした1バイトのフレーム
    fn frame_bytes(frame: i32) -> Vec<u8> {
        vec![frame as u8]
    }

    /// 全フレームを取り込み、全フレームを符号化する
    fn run_all(frames: i32) -> (Result<(), PipelineError<String>>, Trace) {
        let trace = Mutex::new(Trace::default());
        let result = run(
            frames,
            |frame| {
                trace.lock().unwrap().fetched.push(frame);
                Fetched::Frame(frame_bytes(frame))
            },
            |data| {
                let mut trace = trace.lock().unwrap();
                trace.encoded.push(data);
                trace.sink_threads.push(std::thread::current().id());
                Ok(())
            },
        );
        (result, trace.into_inner().unwrap())
    }

    /// 取り込んだフレームが順序も中身も変わらずに符号化側へ届く
    #[test]
    fn every_frame_crosses_in_order_and_unchanged() {
        let (result, trace) = run_all(64);

        assert_eq!(result, Ok(()));
        assert_eq!(trace.fetched, (0..64).collect::<Vec<_>>());
        assert_eq!(
            trace.encoded,
            (0..64).map(frame_bytes).collect::<Vec<Vec<u8>>>()
        );
    }

    /// 1フレームも無い素材でも、符号化側を起こして畳むだけで済む
    #[test]
    fn a_material_without_frames_encodes_nothing() {
        let (result, trace) = run_all(0);

        assert_eq!(result, Ok(()));
        assert!(trace.fetched.is_empty());
        assert!(trace.encoded.is_empty());
    }

    /// 符号化は取り込みと別のスレッドで、しかも1つのスレッドで走る
    #[test]
    fn encoding_runs_on_one_thread_of_its_own() {
        let (_, trace) = run_all(16);

        let worker = trace.sink_threads[0];
        assert_ne!(worker, std::thread::current().id());
        assert!(trace.sink_threads.iter().all(|&id| id == worker));
    }

    /// 中断は取り込みを止め、そのフレームより後は符号化側へ渡らない
    #[test]
    fn an_abort_stops_the_fetching_and_the_encoding() {
        let trace = Mutex::new(Trace::default());
        let result: Result<(), PipelineError<String>> = run(
            64,
            |frame| {
                trace.lock().unwrap().fetched.push(frame);
                if frame == 8 {
                    Fetched::Aborted
                } else {
                    Fetched::Frame(frame_bytes(frame))
                }
            },
            |data| {
                trace.lock().unwrap().encoded.push(data);
                Ok(())
            },
        );

        assert_eq!(result, Err(PipelineError::Aborted));
        let trace = trace.into_inner().unwrap();
        assert_eq!(trace.fetched, (0..=8).collect::<Vec<_>>());
        assert_eq!(
            trace.encoded,
            (0..8).map(frame_bytes).collect::<Vec<Vec<u8>>>()
        );
    }

    /// ホストがフレームを返さなければ、そのフレーム番号を添えて止まる
    #[test]
    fn a_frame_the_host_does_not_return_names_itself() {
        let result: Result<(), PipelineError<String>> = run(
            64,
            |frame| {
                if frame == 5 {
                    Fetched::Missing
                } else {
                    Fetched::Frame(frame_bytes(frame))
                }
            },
            |_| Ok(()),
        );

        assert_eq!(result, Err(PipelineError::FrameUnavailable(5)));
    }

    /// 符号化側の失敗は握り潰されず、取り込みもそこで止まる
    #[test]
    fn an_encoding_failure_comes_back_and_stops_the_fetching() {
        let trace = Mutex::new(Trace::default());
        let result = run(
            4096,
            |frame| {
                trace.lock().unwrap().fetched.push(frame);
                Fetched::Frame(frame_bytes(frame))
            },
            |data| {
                trace.lock().unwrap().encoded.push(data);
                Err("書き出しに失敗".to_string())
            },
        );

        assert_eq!(result, Err(PipelineError::Encode("書き出しに失敗".into())));

        // 送り口が詰まるまでは先へ進めるので、止まる位置は溜められる数の分だけ動く
        let fetched = trace.into_inner().unwrap().fetched.len();
        assert!(fetched <= CHANNEL_DEPTH + 3, "{fetched}フレーム取り込んだ");
    }

    /// 中断と符号化の失敗が重なったら、符号化の失敗を返す
    ///
    /// エンコーダは一度失敗するとそれ以降の投入に意味が無いので、
    /// 中断で覆い隠すと原因が消える。
    #[test]
    fn an_encoding_failure_outranks_an_abort() {
        let result = run(
            4096,
            |frame| {
                if frame == 0 {
                    Fetched::Frame(frame_bytes(frame))
                } else {
                    Fetched::Aborted
                }
            },
            |_| Err("書き出しに失敗".to_string()),
        );

        assert_eq!(result, Err(PipelineError::Encode("書き出しに失敗".into())));
    }

    /// 溜められる数より多くのフレームが同時に生きない
    #[test]
    fn the_frames_in_flight_stay_within_the_channel_depth() {
        let in_flight = std::sync::atomic::AtomicUsize::new(0);
        let peak = std::sync::atomic::AtomicUsize::new(0);
        use std::sync::atomic::Ordering::SeqCst;

        let result: Result<(), PipelineError<String>> = run(
            256,
            |frame| {
                let now = in_flight.fetch_add(1, SeqCst) + 1;
                peak.fetch_max(now, SeqCst);
                Fetched::Frame(frame_bytes(frame))
            },
            |_| {
                in_flight.fetch_sub(1, SeqCst);
                Ok(())
            },
        );

        assert_eq!(result, Ok(()));
        // 溜めた分・取り込み中・符号化中
        assert!(peak.load(SeqCst) <= CHANNEL_DEPTH + 2, "{peak:?}");
    }

    /// 失敗の説明は、どこで何が起きたか分かる文面になる
    #[test]
    fn every_failure_explains_itself() {
        let aborted: PipelineError<String> = PipelineError::Aborted;
        assert_eq!(aborted.to_string(), "処理が中断されました");

        let missing: PipelineError<String> = PipelineError::FrameUnavailable(42);
        assert_eq!(missing.to_string(), "フレーム取得エラー: フレーム 42");

        let encode = PipelineError::Encode("書き出しに失敗".to_string());
        assert_eq!(encode.to_string(), "フレーム書き込みエラー: 書き出しに失敗");
    }
}
