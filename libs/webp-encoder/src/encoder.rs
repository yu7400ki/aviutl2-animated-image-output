//! 全体の駆動

use crate::codec::{Codec, EncodedFrame};
use crate::delay::{Durations, Milliseconds};
use crate::error::Error;
use crate::frame::Canvas;
use crate::layout::{ColorType, Layout};
use crate::riff::{Frame, Riff};
use crate::{Config, Report};
use anim_core::{FrameDelay, Rect, has_transparency};
use std::io::{Seek, Write};

/// キャンバスを書き換えないフレームが載せる画素 (RGBA)
const FILLER_PIXEL: [u8; 4] = [0, 0, 0, 0];

/// 書き出しを待っているフレーム
struct Pending {
    /// キャンバス上の矩形
    rect: Rect,
    /// 符号化した画素
    encoded: EncodedFrame,
    /// 表示時間 (ms)
    duration: u64,
}

/// ANMFを流すアニメーション
///
/// フレームは1つ保留し、次のフレームの投入で書き出す。差分の無いフレームは
/// 保留中のフレームの表示時間へ併合する。
struct Animation<W: Write + Seek> {
    riff: Riff<W>,
    /// ミリ秒への累積の丸め
    milliseconds: Milliseconds,
    /// 次のフレームを待っているフレーム
    pending: Option<Pending>,
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
    /// 保留中のフレームを `pending` へ入れ替え、入れ替わったフレームを書き出す
    fn push(&mut self, pending: Pending, codec: &mut Codec) -> Result<(), Error> {
        let previous = self.pending.replace(pending);
        self.write(previous, codec)
    }

    /// 差分の無いフレームを、保留中のフレームの表示時間へ併合する
    fn merge(&mut self, duration: u64) {
        let pending = self
            .pending
            .as_mut()
            .expect("先頭フレームは全面の差分を持つ");
        pending.duration += duration;
        self.merged_frames += 1;
    }

    /// 保留中のフレームを書き出す
    fn flush(&mut self, codec: &mut Codec) -> Result<(), Error> {
        let pending = self.pending.take();
        self.write(pending, codec)
    }

    /// フレームをANMFへ載せる
    ///
    /// 表示時間が欄に収まらないぶんは、キャンバスを書き換えないフレームへ分ける。
    fn write(&mut self, pending: Option<Pending>, codec: &mut Codec) -> Result<(), Error> {
        let Some(pending) = pending else {
            return Ok(());
        };

        let mut durations = Durations::new(pending.duration);
        self.delay_clamped |= durations.raised;
        self.frames_have_alpha |= pending.encoded.has_alpha();

        let duration = durations.next().expect("表示時間は1つ以上に分かれる");
        self.riff.write_frame(&Frame {
            rect: pending.rect,
            duration,
            blend: false,
            dispose: false,
            alpha: pending.encoded.alpha(),
            image: pending.encoded.image(),
        })?;

        for duration in durations {
            let filler = filler(&mut self.filler, codec)?;
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
/// 符号化に失敗したとき [`Error::Encode`]。
fn filler<'a>(
    slot: &'a mut Option<EncodedFrame>,
    codec: &mut Codec,
) -> Result<&'a EncodedFrame, Error> {
    if slot.is_none() {
        let layout = Layout::new(1, 1, ColorType::Rgba8)?;
        *slot = Some(codec.encode(&FILLER_PIXEL, &layout, layout.whole())?);
    }
    Ok(slot.as_ref().expect("符号化した結果が入っている"))
}

/// 符号化したフレームの行き先
enum Sink<W: Write + Seek> {
    /// 単葉。`WebPEncode` の出力をそのまま書く
    Still(W),
    /// アニメーション
    Animation(Animation<W>),
}

/// WebPのエンコーダ
///
/// [`Encoder::new`] で寸法とフレーム数を宣言し、[`Encoder::add_frame`] で
/// フレームを投入し、[`Encoder::finish`] で閉じる。
///
/// フレーム数が2以上ならANMFチャンクを投入順に流し、RIFFのサイズと
/// VP8XのALPHAフラグを [`Encoder::finish`] が書き戻す。フレーム数が1なら
/// アニメーションにせず、単葉の .webp をそのまま書く。
pub struct Encoder<W: Write + Seek> {
    sink: Sink<W>,
    layout: Layout,
    codec: Codec,
    /// 前のフレームまでを描いたキャンバスと、正規化した入力
    canvas: Canvas,
    num_frames: u32,
    /// [`Self::add_frame`] が受け付けたフレーム数
    frames_accepted: u32,
    /// 書き出しに失敗し、チャンクの列が中断しているか
    poisoned: bool,
    /// 投入したフレームのいずれかに透過画素があったか
    material_has_alpha: bool,
}

impl<W: Write + Seek> Encoder<W> {
    /// `width` x `height` の `num_frames` フレームを `writer` へ書き出す
    ///
    /// フレーム数が2以上のとき、RIFFヘッダ・VP8X・ANIMをここで書く。
    ///
    /// # Errors
    /// 寸法が0か16383を超えるとき [`Error::InvalidDimensions`]。フレーム数が0の
    /// とき [`Error::InvalidFrameCount`]。設定が値域の外のとき [`Error::Encode`]。
    /// 書き出しに失敗したとき [`Error::Io`]。
    pub fn new(
        writer: W,
        width: u32,
        height: u32,
        num_frames: u32,
        config: Config,
    ) -> Result<Self, Error> {
        if num_frames == 0 {
            return Err(Error::InvalidFrameCount);
        }
        let layout = Layout::new(width, height, config.color_type)?;
        let codec = Codec::new(&config)?;

        let sink = if num_frames == 1 {
            Sink::Still(writer)
        } else {
            Sink::Animation(Animation {
                riff: Riff::new(writer, layout.width, layout.height, config.num_plays)?,
                milliseconds: Milliseconds::new(),
                pending: None,
                delay_clamped: false,
                frames_have_alpha: false,
                merged_frames: 0,
                filler: None,
            })
        };

        Ok(Encoder {
            sink,
            canvas: Canvas::new(&layout),
            layout,
            codec,
            num_frames,
            frames_accepted: 0,
            poisoned: false,
            material_has_alpha: false,
        })
    }

    /// フレームを1つ投入する
    ///
    /// `data` は [`Config::color_type`] の画素が左上から右下へ隙間なく
    /// 並んでいること。アニメーションではフレームを1つ保留するため、
    /// 書き出しは次の投入まで遅れる。
    ///
    /// # Errors
    /// バイト数が寸法と色種別から決まる長さと違うとき
    /// [`Error::FrameSizeMismatch`]。宣言したフレーム数を超えたとき
    /// [`Error::FrameCountMismatch`]。符号化に失敗したとき [`Error::Encode`]。
    /// ファイルがRIFFの上限を超えるとき [`Error::FileTooLarge`]。以前の投入が
    /// 書き出しに失敗しているとき [`Error::Poisoned`]。
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
    /// [`Error::Poisoned`]。保留中のフレームの符号化に失敗したとき
    /// [`Error::Encode`]。書き出しに失敗したとき [`Error::Io`]。
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
            sink,
            mut codec,
            material_has_alpha,
            ..
        } = self;

        let (writer, delay_clamped, merged_frames) = match sink {
            Sink::Still(mut writer) => {
                writer.flush()?;
                (writer, false, 0)
            }
            Sink::Animation(mut animation) => {
                animation.flush(&mut codec)?;
                let Animation {
                    riff,
                    delay_clamped,
                    frames_have_alpha,
                    merged_frames,
                    ..
                } = animation;
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
            has_alpha: material_has_alpha,
        };
        Ok((writer, report))
    }

    /// フレームを符号化して行き先へ渡す
    ///
    /// 符号化に渡す画素は、RGBAなら正規化した写し、RGBなら入力そのもの。
    fn write_frame(&mut self, data: &[u8], delay: FrameDelay) -> Result<(), Error> {
        if self.layout.color_type == ColorType::Rgba8 {
            self.material_has_alpha |= has_transparency(data);
        }
        self.canvas.stage(data, self.layout.color_type);
        let source = match self.layout.color_type {
            ColorType::Rgb8 => data,
            ColorType::Rgba8 => self.canvas.staged(),
        };

        let animation = match &mut self.sink {
            Sink::Still(writer) => {
                let rect = self.layout.whole();
                let encoded = self.codec.encode(source, &self.layout, rect)?;
                return Ok(writer.write_all(encoded.still())?);
            }
            Sink::Animation(animation) => animation,
        };

        let duration = animation.milliseconds.next(delay);
        let Some(rect) = self.canvas.dirty() else {
            animation.merge(duration);
            return Ok(());
        };

        let encoded = self.codec.encode(source, &self.layout, rect)?;
        self.canvas.commit();
        animation.push(
            Pending {
                rect,
                encoded,
                duration,
            },
            &mut self.codec,
        )
    }
}
