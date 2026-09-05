//! 全体の駆動

use crate::codec::{Codec, EncodedFrame, Job};
use crate::delay::{Durations, MAX_DURATION, Milliseconds};
use crate::error::Error;
use crate::frame::Canvas;
use crate::layout::Layout;
use crate::pipeline::Pipeline;
use crate::riff::{Frame, Riff};
use crate::{Config, Report};
use anim_core::{ColorType, FrameDelay, Rect};
use std::collections::VecDeque;
use std::io::{Seek, Write};
use std::num::NonZeroUsize;
use std::thread::available_parallelism;

/// キャンバスを書き換えないフレームが載せる画素 (RGBA)
const FILLER_PIXEL: [u8; 4] = [0, 0, 0, 0];

/// 書き出しを待っているフレーム
struct Pending {
    /// パイプラインが預かっているジョブの番号
    job: usize,
    /// キャンバス上の矩形
    rect: Rect,
    /// 透過画素を下のキャンバスへ重ねるか
    blend: bool,
    /// 表示した後に矩形を抜くか。次のフレームの決定で決まる
    dispose: bool,
    /// 表示時間 (ms)
    duration: u64,
}

/// ANMFを流すアニメーション
///
/// 決定の済んだフレームを列で保留し、投入した順にANMFへ載せる。末尾のフレームは
/// 廃棄方法と表示時間がまだ動くので、次のフレームの投入を待つ。差分の無い
/// フレームは末尾の表示時間へ併合する。
struct Animation<W: Write + Seek> {
    riff: Riff<W>,
    /// ミリ秒への累積の丸め
    milliseconds: Milliseconds,
    /// 書き出しを待っているフレーム。投入した順に並ぶ
    pending: VecDeque<Pending>,
    /// 表示時間を下限で切り上げたか
    delay_clamped: bool,
    /// 書いたフレームのいずれかがαを持ったか
    frames_have_alpha: bool,
    /// 表示時間の延長に併合したフレーム数
    merged_frames: u32,
    /// 透明1画素のフレーム。欄に収まらない表示時間を載せる
    filler: Option<EncodedFrame>,
}

impl<W: Write + Seek> Animation<W> {
    /// 末尾のフレームが、表示した後に矩形を抜く廃棄方法を載せられるか
    fn disposable(&self) -> bool {
        self.pending
            .back()
            .is_some_and(|pending| pending.duration <= u64::from(MAX_DURATION))
    }

    /// フレームを列の末尾へ足し、仕掛かりが上限を超えたぶんを書き出す
    ///
    /// `dispose` はそれまで末尾にいたフレームの廃棄方法。
    fn push(
        &mut self,
        pending: Pending,
        dispose: bool,
        pipeline: &mut Pipeline,
    ) -> Result<(), Error> {
        if let Some(previous) = self.pending.back_mut() {
            previous.dispose = dispose;
        }
        self.pending.push_back(pending);

        let held = pipeline.capacity();
        debug_assert!(
            held >= 2,
            "排出する先頭は、廃棄方法の決まっていない末尾と別のフレームであること"
        );
        while self.pending.len() > held {
            self.write(pipeline)?;
        }
        Ok(())
    }

    /// 差分の無いフレームを、末尾のフレームの表示時間へ併合する
    fn merge(&mut self, duration: u64) {
        let pending = self
            .pending
            .back_mut()
            .expect("先頭フレームは全面の差分を持つ");
        pending.duration += duration;
        self.merged_frames += 1;
    }

    /// 残っているフレームをすべて書き出す
    ///
    /// 末尾のフレームは次が無いので、廃棄方法は載せないまま書く。
    fn flush(&mut self, pipeline: &mut Pipeline) -> Result<(), Error> {
        while !self.pending.is_empty() {
            self.write(pipeline)?;
        }
        Ok(())
    }

    /// 列の先頭のフレームをANMFへ載せる
    ///
    /// 表示時間が欄に収まらないぶんは、キャンバスを書き換えないフレームへ分ける。
    fn write(&mut self, pipeline: &mut Pipeline) -> Result<(), Error> {
        let Some(pending) = self.pending.pop_front() else {
            return Ok(());
        };
        let encoded = pipeline.take(pending.job)?;
        let dispose = pending.dispose;

        let mut durations = Durations::new(pending.duration);
        self.delay_clamped |= durations.raised;
        self.frames_have_alpha |= encoded.has_alpha();

        let duration = durations.next().expect("表示時間は1つ以上に分かれる");
        self.riff.write_frame(&Frame {
            rect: pending.rect,
            duration,
            blend: pending.blend,
            dispose,
            alpha: encoded.alpha(),
            image: encoded.image(),
        })?;

        for duration in durations {
            debug_assert!(!dispose, "抜いた跡が分けた先の表示に見えてしまう");
            let filler = filler(&mut self.filler, pipeline.codec())?;
            self.frames_have_alpha |= filler.has_alpha();
            self.riff.write_frame(&Frame {
                rect: Rect {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                duration,
                blend: true,
                dispose: false,
                alpha: filler.alpha(),
                image: filler.image(),
            })?;
        }
        Ok(())
    }
}

/// 透明1画素のフレームを、まだ無ければ符号化して返す
///
/// # Errors
/// 符号化に失敗したとき [`Error::Encode`]。結果のチャンク構成を読み取れない
/// とき [`Error::MalformedOutput`]。
fn filler<'a>(
    slot: &'a mut Option<EncodedFrame>,
    codec: &Codec,
) -> Result<&'a EncodedFrame, Error> {
    if slot.is_none() {
        let layout = Layout::new(1, 1, ColorType::Rgba8)?;
        let job = Job::crop(&FILLER_PIXEL, &layout, layout.whole(), None, Vec::new());
        *slot = Some(codec.encode(&job)?);
    }
    Ok(slot.as_ref().expect("符号化した結果が入っている"))
}

/// 符号化へ渡す画素
///
/// RGBAは正規化した写し、RGBは投入されたフレームそのもの。
fn source<'a>(layout: &Layout, canvas: &'a Canvas, data: &'a [u8]) -> &'a [u8] {
    match layout.color_type {
        ColorType::Rgb8 => data,
        ColorType::Rgba8 => canvas.staged(),
    }
}

/// 符号化したフレームの行き先
enum Sink<W: Write + Seek> {
    /// 単葉。`WebPEncode` の出力をそのまま書く
    Still(W),
    /// アニメーション。仕掛かりを抱えるので、単葉の行き先を太らせないよう間接に置く
    Animation(Box<Animation<W>>),
}

/// WebPのエンコーダ
///
/// [`Encoder::new`] で寸法とフレーム数を宣言し、[`Encoder::add_frame`] で
/// フレームを投入し、[`Encoder::finish`] で閉じる。
///
/// フレーム数が2以上ならANMFチャンクを投入順に流し、RIFFのサイズと
/// VP8XのALPHAフラグを [`Encoder::finish`] が書き戻す。フレーム数が1なら
/// アニメーションにせず、単葉の .webp をそのまま書く。
///
/// フレームの載せ方の決定は投入した場で済ませ、符号化はワーカーへ回す。
/// 落としたエンコーダはワーカーを畳んでから返る。
pub struct Encoder<W: Write + Seek> {
    sink: Sink<W>,
    layout: Layout,
    /// 符号化の投入口と、投入順に揃える結果の受け取り
    pipeline: Pipeline,
    /// 前のフレームまでを描いたキャンバスと、正規化した入力
    canvas: Canvas,
    num_frames: u32,
    /// [`Self::add_frame`] が受け付けたフレーム数
    frames_accepted: u32,
    /// 書き出しに失敗し、チャンクの列が中断しているか
    poisoned: bool,
}

impl<W: Write + Seek> Encoder<W> {
    /// `width` x `height` の `num_frames` フレームを `writer` へ書き出す
    ///
    /// フレーム数が2以上のとき、RIFFヘッダ・VP8X・ANIMをここで書く。
    ///
    /// 符号化を回すワーカー数は機械の並列度になる。実際に起こした数は
    /// [`Encoder::workers`] が返す。
    ///
    /// # Errors
    /// フレーム数が0のとき [`Error::InvalidFrameCount`]。寸法が0か16383を超える
    /// とき [`Error::InvalidDimensions`]。設定が値域の外のとき [`Error::Encode`]。
    /// 書き出しに失敗したとき [`Error::Io`]。
    pub fn new(
        writer: W,
        width: u32,
        height: u32,
        num_frames: u32,
        config: Config,
    ) -> Result<Self, Error> {
        let workers = available_parallelism().unwrap_or(NonZeroUsize::MIN);
        Self::with_workers(writer, width, height, num_frames, config, workers)
    }

    /// ワーカー数を指してエンコーダを作る
    ///
    /// 渡した数をそのまま起こす。ワーカーが1つなら群れを起こさず、投入した場で
    /// 符号化する。決定も書き出しの順序もワーカー数に依らないので、出力は
    /// どちらでも同じになる。
    ///
    /// # Errors
    /// [`Encoder::new`] と同じ。加えてスレッドを起こせないとき [`Error::Io`]。
    pub fn with_workers(
        writer: W,
        width: u32,
        height: u32,
        num_frames: u32,
        config: Config,
        workers: NonZeroUsize,
    ) -> Result<Self, Error> {
        if num_frames == 0 {
            return Err(Error::InvalidFrameCount);
        }
        let layout = Layout::new(width, height, config.color_type)?;
        let codec = Codec::new(&config)?;

        let sink = if num_frames == 1 {
            Sink::Still(writer)
        } else {
            Sink::Animation(Box::new(Animation {
                riff: Riff::new(writer, layout.width, layout.height, config.num_plays)?,
                milliseconds: Milliseconds::new(),
                pending: VecDeque::new(),
                delay_clamped: false,
                frames_have_alpha: false,
                merged_frames: 0,
                filler: None,
            }))
        };

        // 単葉は仕掛かりを持たないので、群れを起こす相手がいない
        let workers = if num_frames == 1 {
            NonZeroUsize::MIN
        } else {
            workers
        };
        // 書き出し先がヘッダを受け取ってから起こす
        let pipeline = Pipeline::new(codec, config.color_type, workers)?;

        Ok(Encoder {
            sink,
            canvas: Canvas::new(&layout, &config),
            layout,
            pipeline,
            num_frames,
            frames_accepted: 0,
            poisoned: false,
        })
    }

    /// 符号化を回すワーカー数
    ///
    /// 単葉の書き出しでは群れを起こさないので1になる。
    pub fn workers(&self) -> NonZeroUsize {
        self.pipeline.workers()
    }

    /// フレームを1つ投入する
    ///
    /// `data` は [`Config::color_type`] の画素が左上から右下へ隙間なく
    /// 並んでいること。アニメーションでは決定の済んだフレームを保留して
    /// 符号化を並列に回すため、書き出しは仕掛かりが上限を超えるまで遅れる。
    ///
    /// # Errors
    /// バイト数が寸法と色種別から決まる長さと違うとき
    /// [`Error::FrameSizeMismatch`]。宣言したフレーム数を超えたとき
    /// [`Error::FrameCountMismatch`]。符号化に失敗したとき [`Error::Encode`]。
    /// 符号化した内容のチャンク構成を読み取れないとき
    /// [`Error::MalformedOutput`]。ファイルがRIFFの上限を超えるとき
    /// [`Error::FileTooLarge`]。書き出しに失敗したとき [`Error::Io`]。
    /// 以前の投入が書き出しに失敗しているとき [`Error::Poisoned`]。
    pub fn add_frame(&mut self, data: &[u8], delay: FrameDelay) -> Result<(), Error> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        if self.frames_accepted == self.num_frames {
            return Err(Error::FrameCountMismatch {
                expected: self.num_frames,
                actual: self.frames_accepted + 1,
            });
        }
        self.layout.check_frame(data)?;

        // 途中で失敗するとチャンクの列が中断した状態で残るため、以降の投入を拒否する
        self.write_frame(data, delay)
            .inspect_err(|_| self.poisoned = true)?;

        self.frames_accepted += 1;
        Ok(())
    }

    /// 書き出しを終え、`writer` と結果を返す
    ///
    /// アニメーションなら保留中のフレームを書き出し、RIFFのサイズと
    /// VP8XのALPHAフラグを書き戻す。
    ///
    /// # Errors
    /// 投入されたフレーム数が宣言したフレーム数に満たないとき
    /// [`Error::FrameCountMismatch`]。以前の投入が書き出しに失敗しているとき
    /// [`Error::Poisoned`]。表示時間を分けるフレームの符号化に失敗したとき
    /// [`Error::Encode`] か [`Error::MalformedOutput`]。ファイルがRIFFの上限を
    /// 超えるとき [`Error::FileTooLarge`]。書き出しに失敗したとき [`Error::Io`]。
    pub fn finish(self) -> Result<(W, Report), Error> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        if self.frames_accepted != self.num_frames {
            return Err(Error::FrameCountMismatch {
                expected: self.num_frames,
                actual: self.frames_accepted,
            });
        }

        let Encoder {
            sink, mut pipeline, ..
        } = self;

        let (writer, delay_clamped, merged_frames) = match sink {
            Sink::Still(mut writer) => {
                writer.flush()?;
                (writer, false, 0)
            }
            Sink::Animation(mut animation) => {
                animation.flush(&mut pipeline)?;
                let Animation {
                    riff,
                    delay_clamped,
                    frames_have_alpha,
                    merged_frames,
                    ..
                } = *animation;
                (
                    riff.finish(frames_have_alpha)?,
                    delay_clamped,
                    merged_frames,
                )
            }
        };

        let report = Report {
            merged_frames,
            delay_clamped,
        };
        Ok((writer, report))
    }

    /// フレームを符号化して行き先へ渡す
    fn write_frame(&mut self, data: &[u8], delay: FrameDelay) -> Result<(), Error> {
        // 写した画素を読むのは、差分を取るときと、RGBAを符号化へ渡すとき
        if self.layout.color_type == ColorType::Rgba8 || !matches!(self.sink, Sink::Still(_)) {
            self.canvas.stage(data, self.layout.color_type);
        }

        let animation = match &mut self.sink {
            Sink::Still(writer) => {
                let source = source(&self.layout, &self.canvas, data);
                let job = Job::crop(source, &self.layout, self.layout.whole(), None, Vec::new());
                let encoded = self.pipeline.codec().encode(&job)?;
                return Ok(writer.write_all(encoded.still())?);
            }
            Sink::Animation(animation) => animation,
        };

        let duration = animation.milliseconds.next(delay);
        let Some(placement) = self.canvas.place(animation.disposable()) else {
            animation.merge(duration);
            return Ok(());
        };

        let source = source(&self.layout, &self.canvas, data);
        let base = (placement.blend && self.pipeline.codec().substitutes_transparency())
            .then(|| self.canvas.base(placement.dispose));
        // 切り出しはここで閉じる。以降の符号化はキャンバスを読まない
        let job = Job::crop(
            source,
            &self.layout,
            placement.rect,
            base,
            self.pipeline.buffer(),
        );

        let index = self.pipeline.submit(job);
        self.canvas.commit(placement);

        animation.push(
            Pending {
                job: index,
                rect: placement.rect,
                blend: placement.blend,
                dispose: false,
                duration,
            },
            placement.dispose,
            &mut self.pipeline,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anim_core::has_transparency;
    use std::io::Cursor;

    /// 動く四角の一辺の長さ
    const SQUARE: u32 = 8;

    /// 四角を塗る色
    const SQUARE_COLOR: [u8; 4] = [0x20, 0x40, 0x60, 0xFF];

    /// 決定的な擬似乱数列
    fn noise(len: usize, seed: u32) -> Vec<u8> {
        let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state >> 16) as u8
            })
            .collect()
    }

    /// 位置が重なりながら動く四角の左上
    fn at(index: usize, width: u32, height: u32) -> (u32, u32) {
        (
            (index as u32 * 3) % (width - SQUARE),
            (index as u32 * 2) % (height - SQUARE),
        )
    }

    /// `frame` の `at` に四角を塗る
    fn draw(frame: &mut [u8], width: u32, at: (u32, u32)) {
        for y in at.1..at.1 + SQUARE {
            let head = (y * width + at.0) as usize * 4;
            for pixel in frame[head..head + SQUARE as usize * 4].chunks_exact_mut(4) {
                pixel.copy_from_slice(&SQUARE_COLOR);
            }
        }
    }

    /// 先頭が重く、以降が軽いRGBAのフレーム列
    ///
    /// 先頭は全面の雑音で符号化に時間がかかり、以降は四角が動くだけなので
    /// 矩形が小さく速い。先頭の符号化が続く間に後続が投入されるため、結果が
    /// 届く順は投入の順から外れる。表示時間の併合も混ぜる。不透明な面なので
    /// 透過置換の経路も通る。
    fn skewed_frames(width: u32, height: u32, count: usize) -> Vec<Vec<u8>> {
        let mut base = noise((width * height * 4) as usize, 0x5EED);
        for pixel in base.chunks_exact_mut(4) {
            pixel[3] = 0xFF;
        }

        (0..count)
            .map(|index| {
                let mut frame = base.clone();
                if index > 0 {
                    // 5フレームに1枚は前と同じ位置に置き、表示時間の併合を混ぜる
                    let position = if index % 5 == 0 { index - 1 } else { index };
                    draw(&mut frame, width, at(position, width, height));
                }
                frame
            })
            .collect()
    }

    /// 透過の面を不透明な四角が1つ動くRGBAのフレーム列
    ///
    /// 前のフレームの矩形を抜いた方が矩形が狭くなるので、廃棄方法の両候補が
    /// 現れる。
    fn sprite_frames(width: u32, height: u32, count: usize) -> Vec<Vec<u8>> {
        (0..count)
            .map(|index| {
                let mut frame = vec![0u8; (width * height * 4) as usize];
                draw(&mut frame, width, at(index, width, height));
                frame
            })
            .collect()
    }

    fn config(lossless: bool) -> Config {
        Config {
            color_type: ColorType::Rgba8,
            lossless,
            quality: 75.0,
            method: 4,
            num_plays: 0,
        }
    }

    /// `workers` 個のワーカーでフレーム列を符号化する
    fn encode(
        width: u32,
        height: u32,
        frames: &[Vec<u8>],
        workers: usize,
        config: Config,
    ) -> (Vec<u8>, Report) {
        let mut encoder = Encoder::with_workers(
            Cursor::new(Vec::new()),
            width,
            height,
            frames.len() as u32,
            config,
            NonZeroUsize::new(workers).unwrap(),
        )
        .unwrap();
        for (index, frame) in frames.iter().enumerate() {
            let delay = FrameDelay::new(index as u32 * 7 + 20, 1000).unwrap();
            encoder.add_frame(frame, delay).unwrap();
            if let Sink::Animation(a) = &encoder.sink {
                let q: Vec<String> = a
                    .pending
                    .iter()
                    .map(|p| format!("{:?} b{} d{}", p.rect, p.blend, p.dispose))
                    .collect();
                println!("after {index}: {q:?}");
            }
        }
        let (writer, report) = encoder.finish().unwrap();
        (writer.into_inner(), report)
    }

    /// 並列に符号化しても、逐次に符号化した出力とバイト一致する
    ///
    /// 決定は入力の純関数で、書き出しは投入順に揃うので、ワーカー数は出力に
    /// 現れない。結果が届く順に書けば、重い先頭を持つ素材でここが割れる。
    ///
    /// 非可逆も回す。libwebp が最初の符号化で据える関数表はこちらの方が広く、
    /// プラグインの既定でもある。
    ///
    /// フレーム数に並ぶワーカー数と、それを超える数も回す。仕掛かりの上限が
    /// フレーム数へ届くと投入の途中で書き出さなくなるので、書き出しの起きる
    /// 位置が変わる。
    #[test]
    fn the_output_does_not_depend_on_the_number_of_workers() {
        let (width, height) = (160, 120);
        for lossless in [true, false] {
            let config = config(lossless);
            for frames in [
                skewed_frames(width, height, 24),
                sprite_frames(width, height, 24),
            ] {
                let (expected, report) = encode(width, height, &frames, 1, config);
                for workers in [2, 3, 4, 8, 12, 32] {
                    let (bytes, parallel) = encode(width, height, &frames, workers, config);
                    assert_eq!(bytes, expected, "可逆{lossless} ワーカー{workers}個の出力");
                    assert_eq!(parallel, report, "可逆{lossless} ワーカー{workers}個の結果");
                }
            }
        }
    }

    /// 素材が決定の経路を踏んでいることを、逐次の結果で確かめる
    ///
    /// 併合も透過も現れない素材では、ワーカー数の比較が薄いところしか通らない。
    #[test]
    fn the_compared_material_exercises_the_decisions() {
        let (width, height) = (160, 120);

        let skewed_material = skewed_frames(width, height, 24);
        assert!(
            !skewed_material.iter().any(|frame| has_transparency(frame)),
            "不透明な素材のはずが透過を持っている"
        );
        let (_, skewed) = encode(width, height, &skewed_material, 1, config(true));
        assert!(skewed.merged_frames > 0, "併合が現れていない");

        let sprite_material = sprite_frames(width, height, 24);
        assert!(
            sprite_material.iter().any(|frame| has_transparency(frame)),
            "透過の面が透過を持っていない"
        );
        let (_, sprite) = encode(width, height, &sprite_material, 1, config(true));
        assert_eq!(sprite.merged_frames, 0, "動く四角が併合されている");
    }

    /// 直前の投入で変わった画素を持ち越すのは非可逆だけ
    #[test]
    fn only_a_lossy_encoder_carries_the_previous_change() {
        for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
            for lossless in [true, false] {
                let config = Config {
                    color_type,
                    ..config(lossless)
                };
                let encoder = Encoder::new(Cursor::new(Vec::new()), 16, 16, 2, config).unwrap();
                assert_eq!(
                    encoder.canvas.carries_the_previous_change(),
                    !lossless,
                    "{color_type:?} 可逆{lossless}"
                );
            }
        }
    }

    /// 半透明の平らな背景に、離れた2つの不透明な四角を置いたRGBA
    fn panel_frames(width: u32, height: u32, count: usize) -> Vec<Vec<u8>> {
        let corners = [(2, 2), (width - SQUARE - 2, height - SQUARE - 2)];
        (0..count)
            .map(|index| {
                let mut frame = Vec::with_capacity((width * height * 4) as usize);
                for y in 0..height {
                    for x in 0..width {
                        let inside = corners.iter().any(|at: &(u32, u32)| {
                            x.wrapping_sub(at.0) < SQUARE && y.wrapping_sub(at.1) < SQUARE
                        });
                        frame.extend_from_slice(&if inside {
                            [(index * 32) as u8, 0x30, 0xF0, 0xFF]
                        } else {
                            [0x40, 0x80, 0xC0, 0x80]
                        });
                    }
                }
                frame
            })
            .collect()
    }

    /// フレームを1枚ずつ投入し、そのつどキャンバスを控える
    fn encode_watching_the_canvas(
        width: u32,
        height: u32,
        frames: &[Vec<u8>],
        config: Config,
    ) -> (Vec<u8>, Vec<Vec<u8>>) {
        let mut encoder = Encoder::with_workers(
            Cursor::new(Vec::new()),
            width,
            height,
            frames.len() as u32,
            config,
            NonZeroUsize::MIN,
        )
        .unwrap();

        let mut canvases = Vec::new();
        for (index, frame) in frames.iter().enumerate() {
            let delay = FrameDelay::new(index as u32 * 7 + 20, 1000).unwrap();
            encoder.add_frame(frame, delay).unwrap();
            canvases.push(encoder.canvas.base(false).to_vec());
        }
        let (writer, _) = encoder.finish().unwrap();
        (writer.into_inner(), canvases)
    }

    /// αの並び
    fn alpha_of(rgba: &[u8]) -> Vec<u8> {
        rgba.iter().skip(3).step_by(4).copied().collect()
    }

    /// 独立したデコーダで合成する
    fn compose(bytes: &[u8], frames: usize) -> Vec<Vec<u8>> {
        let mut decoder =
            image_webp::WebPDecoder::new(Cursor::new(bytes)).expect("image-webp が読めない");
        decoder
            .set_background_color([0, 0, 0, 0])
            .expect("背景色を透明にする");
        assert_eq!(decoder.num_frames() as usize, frames, "書いたフレーム数");

        let size = decoder.output_buffer_size().expect("出力の大きさ");
        (0..frames)
            .map(|_| {
                let mut composed = vec![0u8; size];
                decoder
                    .read_frame(&mut composed)
                    .expect("image-webp のデコード");
                composed
            })
            .collect()
    }

    /// 非可逆の出力は、独立したデコーダで合成すると入力へ戻る
    ///
    /// αは可逆で格納されるので、合成後も入力と1も違わない。これが重ねる形を
    /// 入力のキャンバスで判定してよい根拠になる。RGBは非可逆の量子化ぶん離れる。
    ///
    /// 素材は半透明の背景を上書きで載せるので、`image-webp` の重ねる合成の
    /// 逸脱にも廃棄の逸脱にも当たらない。矩形がずれるか重ねる向きが逆になれば
    /// 桁で外れる。
    #[test]
    fn a_lossy_output_decodes_back_to_the_input() {
        /// 画素ごとの差の平均の上限。非可逆の量子化とデコーダ間の変換の違いを見込む
        const LIMIT: f64 = 3.0;

        let (width, height) = (48, 36);
        let frames = panel_frames(width, height, 8);
        let (bytes, _) = encode_watching_the_canvas(width, height, &frames, config(false));

        for (index, (frame, composed)) in
            frames.iter().zip(compose(&bytes, frames.len())).enumerate()
        {
            let mut expected = frame.clone();
            crate::normalize::normalize(&mut expected);
            assert_eq!(
                alpha_of(&composed),
                alpha_of(&expected),
                "フレーム{index}のα"
            );

            let error: f64 = expected
                .iter()
                .zip(&composed)
                .map(|(a, b)| f64::from(a.abs_diff(*b)))
                .sum::<f64>()
                / expected.len() as f64;
            assert!(error < LIMIT, "フレーム{index}の平均絶対誤差 {error}");
        }
    }
}
