//! GIFのストリーミング書き出し

use crate::block::{self, DISPOSAL_DO_NOT_DISPOSE};
use crate::error::Error;
use crate::frame::{self, Canvas};
use crate::layout::{ColorType, Layout};
use crate::normalize;
use crate::spool::{Spool, Spooled};
use crate::table::Palette;
use anim_core::{FrameDelay, Rect, paste};
use std::borrow::Cow;
use std::io::Write;

/// 遅延時間の下限 (1/100秒)
///
/// 0と1は多くのデコーダが10へ引き上げるため、それを避ける下限を置く。
const MIN_DELAY: u64 = 2;

/// [`Config::max_spool_bytes`] の目安となる値
///
/// グローバルカラーテーブルは1枚目の画像データより前に書く必要があるため、
/// 色が決まるまでのフレームをエンコーダが抱えることになり、その量は素材の
/// 大きさとフレーム数に比例する。
///
/// この512MiBは、1920x1080のRGBA8 (1フレーム約8.29MB) が全画面差分で続く場合の
/// 64フレーム、30fpsで約2.1秒に相当する。
pub const DEFAULT_MAX_SPOOL_BYTES: usize = 512 << 20;

/// エンコード設定
#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// 入力フレームの色種別
    ///
    /// [`Encoder::add_frame`] に渡すバイト列の解釈を決める。
    pub color_type: ColorType,
    /// アニメーションの再生回数 (0で無限ループ)
    pub num_plays: u32,
    /// 溜めたフレームが抱えるメモリの上限バイト数
    /// ([`DEFAULT_MAX_SPOOL_BYTES`] が目安)
    ///
    /// クロップ済みの画素データに、フレームごとの管理領域を加えた概算で数える。
    /// 超える場合はそこまでの色でカラーテーブルを据え、以降のフレームは
    /// 1フレーム遅れで書き出す。カラーテーブルは1枚も溜めずには据えられない
    /// ため、先頭フレームだけは上限に関わらず溜める。
    pub max_spool_bytes: usize,
}

/// 既定は無限ループするRGB8
impl Default for Config {
    fn default() -> Self {
        Config {
            color_type: ColorType::Rgb8,
            num_plays: 0,
            max_spool_bytes: DEFAULT_MAX_SPOOL_BYTES,
        }
    }
}

/// グローバルカラーテーブルの据え方
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteKind {
    /// 全フレームの色がそのまま載った (可逆)
    Exact {
        /// 色の和集合の大きさ
        colors: u16,
    },
    /// 溜めきれず、先頭区間の色を据えた
    ExactFromPrefix {
        /// 据えた区間の色の和集合の大きさ
        colors: u16,
    },
}

/// 色とタイミングの決定の結果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    /// グローバルカラーテーブルの据え方
    pub palette: PaletteKind,
    /// 閾値未満のアルファを完全透過へ潰した画素数
    pub binarized_pixels: u64,
    /// 遅延を下限で切り上げたか
    pub delay_clamped: bool,
    /// 溜めたフレームが抱えたバイト数の最大値
    pub peak_spool_bytes: usize,
}

/// 書き出しを待っているフレーム
///
/// 廃棄方法は次のフレームを見るまで決まらず、グラフィック制御拡張は画像記述子の
/// 前に置く必要があるため、書けるようになるまで1つぶんを保持する。
struct Pending {
    rect: Rect,
    /// 1/100秒へ丸めた遅延
    delay: u16,
    /// LZWで圧縮する添字の並び
    indices: Vec<u8>,
}

/// エンコーダが進む段階
///
/// グローバルカラーテーブルは1枚目の画像データより前に書く必要があるため、
/// 色が決まるまで1フレームも書き出せない。決まった時点で [`Stage::Deciding`] は
/// [`Stage::Streaming`] へ移り、後戻りしない。
enum Stage {
    /// 色が決まるまでフレームを溜めている
    Deciding(Spool),
    /// ヘッダとカラーテーブルを書き終え、1フレーム遅れで書き出している
    Streaming(Streaming),
}

/// 書き出しの段階が持つ状態
struct Streaming {
    /// 据えたグローバルカラーテーブル
    palette: Palette,
    /// 描画後の色の面
    canvas: Canvas,
    /// 書き出しを待っているフレーム
    pending: Option<Pending>,
}

/// GIFのエンコーダ
///
/// [`Encoder::new`] で寸法とフレーム数を宣言し、[`Encoder::add_frame`] で
/// フレームを投入し、[`Encoder::finish`] で閉じる。
///
/// グローバルカラーテーブルを全フレームの色の和集合から据えるため、投入された
/// フレームは色が決まるまでエンコーダ内部に溜まる。決まった後は、廃棄方法が
/// 決まる次の投入まで1フレームぶんを保持する。
pub struct Encoder<W: Write> {
    writer: W,
    layout: Layout,
    stage: Stage,
    num_frames: u32,
    num_plays: u32,
    /// [`Self::add_frame`] が受け付けたフレーム数
    frames_accepted: u32,
    /// 書き出しに失敗し、ブロックの列が中断しているか
    poisoned: bool,
    /// グローバルカラーテーブルの据え方。決まるまでは `None`
    palette_kind: Option<PaletteKind>,
    /// 完全透過へ潰した画素数
    binarized_pixels: u64,
    /// 遅延を下限で切り上げたか
    delay_clamped: bool,
    /// 溜めたバイト数の最大値
    peak_spool_bytes: usize,
}

impl<W: Write> Encoder<W> {
    /// `width` x `height` の `num_frames` フレームを `writer` へ書き出す
    ///
    /// # Errors
    /// 寸法が0か65535を超えるとき [`Error::InvalidDimensions`]。フレーム数が0の
    /// とき [`Error::InvalidFrameCount`]。1フレームのバイト数が `usize` で
    /// 表現できないとき [`Error::ImageTooLarge`]。
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

        Ok(Encoder {
            writer,
            layout: Layout::new(width, height, config.color_type)?,
            stage: Stage::Deciding(Spool::new(config.max_spool_bytes)),
            num_frames,
            num_plays: config.num_plays,
            frames_accepted: 0,
            poisoned: false,
            palette_kind: None,
            binarized_pixels: 0,
            delay_clamped: false,
            peak_spool_bytes: 0,
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
    /// [`Error::FrameCountMismatch`]。全フレームの色の和集合がカラーテーブルに
    /// 収まらないとき [`Error::TooManyColors`]。不透明な画素が透過になる遷移が
    /// あるとき [`Error::UnsupportedTransparency`]。
    pub fn add_frame(&mut self, data: &[u8], delay: FrameDelay) -> Result<(), Error> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        if data.len() != self.layout.frame_len {
            return Err(Error::FrameSizeMismatch {
                expected: self.layout.frame_len,
                actual: data.len(),
            });
        }
        if self.frames_accepted == self.num_frames {
            return Err(Error::FrameCountMismatch {
                expected: self.num_frames,
                actual: self.frames_accepted + 1,
            });
        }

        let pixels = match self.layout.color_type {
            ColorType::Rgb8 => Cow::Borrowed(data),
            ColorType::Rgba8 => {
                let mut pixels = data.to_vec();
                self.binarized_pixels += normalize::binarize(&mut pixels);
                Cow::Owned(pixels)
            }
        };

        // 途中で失敗するとブロックの列が中断した状態で残るため、以降の投入を拒否する
        self.accept(&pixels, delay)
            .inspect_err(|_| self.poisoned = true)?;

        self.frames_accepted += 1;
        Ok(())
    }

    /// 書き出しを終え、終端を書いて `writer` と結果を返す
    ///
    /// # Errors
    /// 投入されたフレーム数が宣言したフレーム数に満たないとき
    /// [`Error::FrameCountMismatch`]。
    pub fn finish(mut self) -> Result<(W, Report), Error> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        if self.frames_accepted != self.num_frames {
            return Err(Error::FrameCountMismatch {
                expected: self.num_frames,
                actual: self.frames_accepted,
            });
        }

        let (stage, mut parts) = self.split();
        if let Stage::Streaming(streaming) = stage
            && let Some(pending) = streaming.pending.take()
        {
            // 次のフレームが無く、廃棄方法が変えられるキャンバスの続きも無い
            parts.write_pending(&streaming.palette, pending, DISPOSAL_DO_NOT_DISPOSE)?;
        }

        block::trailer(&mut self.writer)?;
        self.writer.flush()?;

        let report = Report {
            palette: self
                .palette_kind
                .expect("全フレームを投入した時点で色は決まっている"),
            binarized_pixels: self.binarized_pixels,
            delay_clamped: self.delay_clamped,
            peak_spool_bytes: self.peak_spool_bytes,
        };
        Ok((self.writer, report))
    }

    /// 段階と、段階に依らない部品に分けて借りる
    ///
    /// 段階ごとの値を取り出したまま部品を触れるようにする。
    fn split(&mut self) -> (&mut Stage, Parts<'_, W>) {
        let Encoder {
            writer,
            layout,
            stage,
            num_frames,
            num_plays,
            frames_accepted,
            poisoned: _,
            palette_kind,
            binarized_pixels: _,
            delay_clamped,
            peak_spool_bytes,
        } = self;

        (
            stage,
            Parts {
                writer,
                layout,
                palette_kind,
                delay_clamped,
                peak_spool_bytes,
                num_frames: *num_frames,
                num_plays: *num_plays,
                frames_accepted: *frames_accepted,
            },
        )
    }

    /// 正規化したフレームを段階に応じて処理し、段階が移ったらそれを覚える
    fn accept(&mut self, pixels: &[u8], delay: FrameDelay) -> Result<(), Error> {
        let (stage, mut parts) = self.split();

        let next = match stage {
            Stage::Deciding(spool) => parts.spool_frame(spool, pixels, delay)?,
            Stage::Streaming(streaming) => {
                parts.write_frame(streaming, pixels, delay)?;
                None
            }
        };

        if let Some(next) = next {
            *stage = next;
        }
        Ok(())
    }
}

/// [`Encoder`] から [`Stage`] 以外を借りたもの
///
/// 段階ごとの値は引数で受け取る。フレーム1つを処理する判断と書き出しを担う。
struct Parts<'a, W: Write> {
    writer: &'a mut W,
    layout: &'a Layout,
    palette_kind: &'a mut Option<PaletteKind>,
    delay_clamped: &'a mut bool,
    peak_spool_bytes: &'a mut usize,
    num_frames: u32,
    num_plays: u32,
    frames_accepted: u32,
}

impl<W: Write> Parts<'_, W> {
    /// 溜めているフレームへ1つ加え、色が決まったら溜めたぶんを流す
    fn spool_frame(
        &mut self,
        spool: &mut Spool,
        pixels: &[u8],
        delay: FrameDelay,
    ) -> Result<Option<Stage>, Error> {
        let rect = spool.rect_of(self.layout, pixels);
        let region_len = rect.area() as usize * self.layout.bytes_per_pixel;

        // 抱えきれない大きさが来たら、そこまでの色で据えて溜めたぶんを流し、
        // 投入されたフレームは以降と同じ逐次の経路へ通す
        if !spool.can_hold(region_len) {
            let mut streaming = self.commit(spool, true)?;
            self.write_frame(&mut streaming, pixels, delay)?;
            return Ok(Some(Stage::Streaming(streaming)));
        }

        spool.push(self.layout, pixels, rect, delay);
        *self.peak_spool_bytes = (*self.peak_spool_bytes).max(spool.len());

        // 全フレームの色を見終えるまでカラーテーブルは据えられない
        if self.frames_accepted + 1 != self.num_frames {
            return Ok(None);
        }

        let streaming = self.commit(spool, false)?;
        Ok(Some(Stage::Streaming(streaming)))
    }

    /// 色を決めてヘッダからカラーテーブルまでを書き、溜めたフレームを流す
    ///
    /// `from_prefix` は溜めきれずに決着したことを表す。据えた色は溜めた区間の
    /// ものでしかないため、以降のフレームの色を覆っているとは限らない。覆って
    /// いないことは、その色を写す時点で分かる。
    fn commit(&mut self, spool: &mut Spool, from_prefix: bool) -> Result<Streaming, Error> {
        let (frames, colors) = spool.drain();
        if colors.exceeded() {
            return Err(Error::TooManyColors);
        }

        let colors_count = colors.count();
        let palette = Palette::from_colors(colors);
        *self.palette_kind = Some(if from_prefix {
            PaletteKind::ExactFromPrefix {
                colors: colors_count,
            }
        } else {
            PaletteKind::Exact {
                colors: colors_count,
            }
        });
        self.write_head(&palette)?;

        let mut streaming = Streaming {
            palette,
            canvas: Canvas::new(*self.layout),
            pending: None,
        };
        self.replay(&frames, &mut streaming)?;
        Ok(streaming)
    }

    /// ヘッダ・論理画面記述子・グローバルカラーテーブル・ループ回数を書く
    fn write_head(&mut self, palette: &Palette) -> Result<(), Error> {
        let table = palette.table();
        block::header(self.writer)?;
        block::logical_screen_descriptor(
            self.writer,
            self.layout.width,
            self.layout.height,
            table.size_field(),
        )?;
        block::color_table(self.writer, table.bytes())?;
        block::netscape(self.writer, self.num_plays)?;
        Ok(())
    }

    /// 溜めたフレームをキャンバスへ貼り直し、書き出しの経路へ通す
    ///
    /// 溜めた矩形は直前のフレームとの差分なので、投入された順に貼れば入力の
    /// フレームがそのまま戻る。戻したフレームを流せば、溜めなかった場合と同じ
    /// 判定で廃棄方法が決まる。
    ///
    /// 最後のフレームは保留のまま残す。続きを見ずに書き出すと廃棄方法を選べない。
    fn replay(&mut self, frames: &[Spooled], streaming: &mut Streaming) -> Result<(), Error> {
        let mut rebuilt = vec![0; self.layout.frame_len];
        for frame in frames {
            paste(
                &mut rebuilt,
                &frame.data,
                frame.rect,
                self.layout.stride,
                self.layout.bytes_per_pixel,
            );
            self.write_frame(streaming, &rebuilt, frame.delay)?;
        }
        Ok(())
    }

    /// 保留中のフレームを書き出し、投入されたフレームを保留にする
    fn write_frame(
        &mut self,
        streaming: &mut Streaming,
        pixels: &[u8],
        delay: FrameDelay,
    ) -> Result<(), Error> {
        // 廃棄方法は保留中のフレームのもので、投入されたフレームが載るキャンバスを
        // 決める。決めてからそのキャンバスで矩形と添字を求める
        if let Some(pending) = streaming.pending.take() {
            let disposal = frame::choose_disposal(&streaming.canvas, pixels)
                .ok_or(Error::UnsupportedTransparency)?;
            self.write_pending(&streaming.palette, pending, disposal)?;
        }

        let rect = streaming.canvas.rect_of(pixels);
        let mut indices = Vec::new();
        streaming
            .canvas
            .append_indices(pixels, rect, &streaming.palette, &mut indices)?;

        streaming.pending = Some(Pending {
            rect,
            delay: self.hundredths(delay),
            indices,
        });
        streaming.canvas.advance(pixels, rect);
        Ok(())
    }

    /// 保留していたフレームを `disposal` で書き出す
    fn write_pending(
        &mut self,
        palette: &Palette,
        pending: Pending,
        disposal: u8,
    ) -> Result<(), Error> {
        block::graphic_control(self.writer, disposal, pending.delay, palette.transparent())?;
        block::image_descriptor(
            self.writer,
            pending.rect.x as u16,
            pending.rect.y as u16,
            pending.rect.width as u16,
            pending.rect.height as u16,
        )?;
        block::image_data(
            self.writer,
            palette.table().min_code_size(),
            &pending.indices,
        )?;
        Ok(())
    }

    /// フレーム遅延を1/100秒へ丸め、下限で切り上げたことを覚える
    fn hundredths(&mut self, delay: FrameDelay) -> u16 {
        let (rounded, clamped) = hundredths(delay);
        *self.delay_clamped |= clamped;
        rounded
    }
}

/// フレーム遅延を1/100秒へ丸め、下限で切り上げたかどうかを添える
///
/// 上限での飽和は、素材のレートで再生できないことを表さないため数えない。
fn hundredths(delay: FrameDelay) -> (u16, bool) {
    let numerator = u64::from(delay.numerator()) * 100;
    let denominator = u64::from(delay.denominator());
    let rounded = (numerator + denominator / 2) / denominator;
    (
        rounded.clamp(MIN_DELAY, u64::from(u16::MAX)) as u16,
        rounded < MIN_DELAY,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_delay_is_rounded_to_hundredths_of_a_second() {
        for (numerator, denominator, expected) in [
            (1, 30, 3),
            (2, 30, 7),
            (1, 1, 100),
            (1001, 30000, 3),
            (1, 10, 10),
            (3, 40, 8),
        ] {
            let delay = FrameDelay::new(numerator, denominator).unwrap();
            assert_eq!(
                hundredths(delay),
                (expected, false),
                "{numerator}/{denominator}"
            );
        }
    }

    /// 0と1へ丸まる遅延だけが下限まで切り上げられる
    #[test]
    fn a_delay_below_the_lower_bound_is_raised_and_reported() {
        for (numerator, denominator) in [(0, 30), (1, 1000), (1, 100)] {
            let delay = FrameDelay::new(numerator, denominator).unwrap();
            assert_eq!(
                hundredths(delay),
                (MIN_DELAY as u16, true),
                "{numerator}/{denominator}"
            );
        }

        // 下限そのものへ丸まる遅延は切り上げていない
        let delay = FrameDelay::new(1, 60).unwrap();
        assert_eq!(hundredths(delay), (MIN_DELAY as u16, false));
    }

    /// 上限を超える遅延は飽和させるが、切り上げとしては数えない
    #[test]
    fn a_long_delay_saturates_at_the_field_width() {
        let delay = FrameDelay::new(1000, 1).unwrap();
        assert_eq!(hundredths(delay), (u16::MAX, false));
    }
}
