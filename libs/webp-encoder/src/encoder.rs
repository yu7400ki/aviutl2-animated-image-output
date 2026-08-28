//! 全体の駆動

use crate::codec::{Codec, EncodedFrame};
use crate::delay::Milliseconds;
use crate::error::Error;
use crate::layout::{ColorType, Layout};
use crate::riff::{Frame, Riff};
use crate::{Config, Report};
use anim_core::{FrameDelay, Rect, has_transparency};
use std::io::{Seek, Write};

/// ANMFを流すアニメーション
struct Animation<W: Write + Seek> {
    riff: Riff<W>,
    /// ミリ秒への累積の丸め
    milliseconds: Milliseconds,
    /// 遅延を下限で切り上げたか
    delay_clamped: bool,
}

impl<W: Write + Seek> Animation<W> {
    fn write_frame(
        &mut self,
        rect: Rect,
        encoded: &EncodedFrame,
        delay: FrameDelay,
    ) -> Result<(), Error> {
        let (duration, clamped) = self.milliseconds.next(delay);
        self.delay_clamped |= clamped;

        self.riff.write_frame(&Frame {
            rect,
            duration,
            blend: false,
            dispose: false,
            alpha: encoded.alpha(),
            image: encoded.image(),
        })
    }
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
    num_frames: u32,
    /// [`Self::add_frame`] が受け付けたフレーム数
    frames_accepted: u32,
    /// 書き出しに失敗し、チャンクの列が中断しているか
    poisoned: bool,
    /// 素材に透過画素があったか
    has_alpha: bool,
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
                delay_clamped: false,
            })
        };

        Ok(Encoder {
            sink,
            layout,
            codec,
            num_frames,
            frames_accepted: 0,
            poisoned: false,
            has_alpha: false,
        })
    }

    /// フレームを1つ投入する
    ///
    /// `data` は [`Config::color_type`] の画素が左上から右下へ隙間なく
    /// 並んでいること。
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
        self.layout.check_frame(data)?;
        if self.frames_accepted == self.num_frames {
            return Err(Error::FrameCountMismatch {
                expected: self.num_frames,
                actual: self.frames_accepted + 1,
            });
        }

        // 途中で失敗するとチャンクの列が中断した状態で残るため、以降の投入を拒否する
        self.write_frame(data, delay)
            .inspect_err(|_| self.poisoned = true)?;

        self.frames_accepted += 1;
        Ok(())
    }

    /// 書き出しを終え、`writer` と結果を返す
    ///
    /// アニメーションならRIFFのサイズとVP8XのALPHAフラグをここで書き戻す。
    ///
    /// # Errors
    /// 投入されたフレーム数が宣言したフレーム数に満たないとき
    /// [`Error::FrameCountMismatch`]。以前の投入が書き出しに失敗しているとき
    /// [`Error::Poisoned`]。書き出しに失敗したとき [`Error::Io`]。
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

        let (writer, delay_clamped) = match self.sink {
            Sink::Still(mut writer) => {
                writer.flush()?;
                (writer, false)
            }
            Sink::Animation(animation) => (
                animation.riff.finish(self.has_alpha)?,
                animation.delay_clamped,
            ),
        };

        let report = Report {
            merged_frames: 0,
            delay_clamped,
            has_alpha: self.has_alpha,
        };
        Ok((writer, report))
    }

    /// フレームを符号化して行き先へ渡す
    fn write_frame(&mut self, data: &[u8], delay: FrameDelay) -> Result<(), Error> {
        let rect = self.layout.whole();
        let encoded = self.codec.encode(data, &self.layout, rect)?;

        if self.layout.color_type == ColorType::Rgba8 {
            self.has_alpha |= has_transparency(data);
        }

        match &mut self.sink {
            Sink::Still(writer) => Ok(writer.write_all(encoded.still())?),
            Sink::Animation(animation) => animation.write_frame(rect, &encoded, delay),
        }
    }
}
