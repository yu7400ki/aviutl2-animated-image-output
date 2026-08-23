//! APNGのストリーミング書き出し

use crate::chunk;
use crate::delay::FrameDelay;
use crate::diff::{self, Rect};
use crate::error::Error;
use crate::filter;
use crate::region;
use crate::spool::Spool;
use crate::zlib::Compressor;
use std::io::Write;
use std::ops::RangeInclusive;

/// 前のフレームを消さずに次のフレームを描画する
const DISPOSE_OP_NONE: u8 = 0;
/// フレームの内容で領域を置き換える
const BLEND_OP_SOURCE: u8 = 0;

/// 画素の色種別 (ビット深度8固定)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorType {
    /// 8bit/chのRGB
    Rgb8,
    /// 8bit/chのRGBA
    Rgba8,
}

impl ColorType {
    /// 1画素あたりのバイト数
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            ColorType::Rgb8 => 3,
            ColorType::Rgba8 => 4,
        }
    }

    /// IHDRのcolour type
    fn code(self) -> u8 {
        match self {
            ColorType::Rgb8 => 2,
            ColorType::Rgba8 => 6,
        }
    }
}

/// [`Config::compression_level`] に指定できる範囲
pub const COMPRESSION_LEVELS: RangeInclusive<u32> = 1..=9;

/// [`Config::max_spool_bytes`] の目安となる値
pub const DEFAULT_MAX_SPOOL_BYTES: usize = 512 << 20;

/// エンコード設定
#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// 入力フレームの色種別
    ///
    /// [`Encoder::add_frame`] に渡すバイト列の解釈を決める。出力の色種別は
    /// [`Config::reduce_color`] によってこれより小さくなることがある。
    pub color_type: ColorType,
    /// deflateの圧縮レベル ([`COMPRESSION_LEVELS`] の範囲)
    pub compression_level: u32,
    /// アニメーションの再生回数 (0で無限ループ)
    pub num_plays: u32,
    /// 出力の色種別を入力より小さいものへ落とすか
    ///
    /// PNGの色種別はファイル全体で1つなので、全フレームを見るまで落とせるか決まらない。
    /// 有効にすると、決まるまでのフレームをエンコーダ内部に溜める。
    pub reduce_color: bool,
    /// 溜めたフレームが抱えるメモリの上限バイト数 ([`DEFAULT_MAX_SPOOL_BYTES`] が目安)
    ///
    /// クロップ済みの画素データに、フレームごとの管理領域を加えた概算で数える。
    /// 超える場合は色種別を落とすのをやめ、入力の色種別のまま書き出す。
    pub max_spool_bytes: usize,
}

/// 既定は無限ループするRGB8で、圧縮レベルは6、色種別は落とさない
impl Default for Config {
    fn default() -> Self {
        Config {
            color_type: ColorType::Rgb8,
            compression_level: 6,
            num_plays: 0,
            reduce_color: false,
            max_spool_bytes: DEFAULT_MAX_SPOOL_BYTES,
        }
    }
}

/// APNGエンコーダ
///
/// [`Encoder::add_frame`] でフレームを1つずつ書き出し、[`Encoder::finish`] で終端する。
/// 通常はフレームを保持せず、その場で書き出す。[`Config::reduce_color`] が有効なときだけ、
/// 出力の色種別が決まるまでのフレームを内部に溜める。
pub struct Encoder<W: Write> {
    writer: W,
    width: u32,
    height: u32,
    num_frames: u32,
    num_plays: u32,
    /// [`Self::add_frame`] が受け付けたフレーム数
    frames_accepted: u32,
    /// 実際に書き出したフレーム数
    frames_emitted: u32,
    /// fcTLとfdATで共有する連番
    sequence: u32,
    /// 書き出しに失敗し、チャンク列が中断しているか
    poisoned: bool,
    /// 入力の1画素あたりのバイト数
    bytes_per_pixel: usize,
    /// 入力の1行のバイト数
    stride: usize,
    /// 入力の1フレームのバイト数
    frame_len: usize,
    /// 出力の色種別
    ///
    /// [`Self::pending`] が `Some` の間は暫定で入力の色種別が入り、
    /// [`Self::commit`] で確定してヘッダに載る。
    output_color_type: ColorType,
    /// 出力の色種別が決まるまでフレームを溜める領域
    pending: Option<Spool>,
    /// 溜めたバイト数の最大値
    peak_spool_bytes: usize,
    compressor: Compressor,
    /// 直前に書き出したフレーム
    ///
    /// dispose_op=NONE・blend_op=SOURCE のもとでは、合成後のキャンバスと一致する。
    previous: Vec<u8>,
    /// 差分矩形を切り出した連続バッファ
    region: Vec<u8>,
    /// 行ごとのフィルタ選択に使う作業領域
    scratch: filter::Scratch,
    filtered: Vec<u8>,
    compressed: Vec<u8>,
}

/// フレームを1つ受け付けたあとに取る行動
enum Step {
    /// そのまま書き出す
    Emit,
    /// 溜めたまま次のフレームを待つ
    Hold,
    /// 出力の色種別を確定し、溜めたぶんを書き出す
    Commit(ColorType),
    /// 入力の色種別で確定して溜めたぶんを流し、このフレームは書き出す
    Abandon,
}

impl<W: Write> Encoder<W> {
    /// `num_frames` フレームを受け付ける状態にする
    ///
    /// 出力の色種別が最初から決まっていれば、この時点でシグネチャとヘッダを書き出す。
    ///
    /// # Errors
    /// 幅・高さ・フレーム数が0のとき、1フレームのバイト数が `usize` で表現できないとき、
    /// または圧縮レベルが範囲外のとき。
    pub fn new(
        writer: W,
        width: u32,
        height: u32,
        num_frames: u32,
        config: Config,
    ) -> Result<Self, Error> {
        if width == 0 || height == 0 {
            return Err(Error::InvalidDimensions { width, height });
        }
        if num_frames == 0 {
            return Err(Error::InvalidFrameCount);
        }
        if !COMPRESSION_LEVELS.contains(&config.compression_level) {
            return Err(Error::InvalidCompressionLevel(config.compression_level));
        }

        let bytes_per_pixel = config.color_type.bytes_per_pixel();
        let stride = (width as usize)
            .checked_mul(bytes_per_pixel)
            .ok_or(Error::ImageTooLarge { width, height })?;
        let frame_len = stride
            .checked_mul(height as usize)
            .ok_or(Error::ImageTooLarge { width, height })?;

        // 落とせる余地があるのはアルファを持つ入力だけなので、それ以外は保留しない
        let deferred = config.reduce_color && config.color_type == ColorType::Rgba8;

        let mut encoder = Encoder {
            writer,
            width,
            height,
            num_frames,
            num_plays: config.num_plays,
            frames_accepted: 0,
            frames_emitted: 0,
            sequence: 0,
            poisoned: false,
            bytes_per_pixel,
            stride,
            frame_len,
            output_color_type: config.color_type,
            pending: deferred.then(|| Spool::new(config.max_spool_bytes)),
            peak_spool_bytes: 0,
            compressor: Compressor::new(config.compression_level),
            previous: Vec::new(),
            region: Vec::new(),
            scratch: filter::Scratch::new(),
            filtered: Vec::new(),
            compressed: Vec::new(),
        };
        if encoder.pending.is_none() {
            encoder.write_header()?;
        }
        Ok(encoder)
    }

    /// 溜めたフレームのバイト数の最大値
    pub fn peak_spool_bytes(&self) -> usize {
        self.peak_spool_bytes
    }

    fn write_header(&mut self) -> Result<(), Error> {
        self.writer.write_all(&chunk::SIGNATURE)?;

        let mut ihdr = [0u8; 13];
        ihdr[0..4].copy_from_slice(&self.width.to_be_bytes());
        ihdr[4..8].copy_from_slice(&self.height.to_be_bytes());
        ihdr[8] = 8;
        ihdr[9] = self.output_color_type.code();
        chunk::write(&mut self.writer, *b"IHDR", &ihdr)?;

        let mut actl = [0u8; 8];
        actl[0..4].copy_from_slice(&self.num_frames.to_be_bytes());
        actl[4..8].copy_from_slice(&self.num_plays.to_be_bytes());
        chunk::write(&mut self.writer, *b"acTL", &actl)?;

        Ok(())
    }

    /// フレームを1つ投入する
    ///
    /// `data` は上から下・左から右の順に並んだ `幅 * 高さ * 1画素のバイト数` バイトであること。
    ///
    /// # Errors
    /// `data` の長さが合わないとき、宣言したフレーム数を超えたとき、書き出しに失敗したとき、
    /// または過去の書き出し失敗でエンコーダが使用不能なとき。
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

        if data.len() != self.frame_len {
            return Err(Error::FrameSizeMismatch {
                expected: self.frame_len,
                actual: data.len(),
            });
        }

        let rect = self.frame_rect(data);

        // 途中で失敗するとfcTLだけが書かれた状態で残るため、以降の書き出しを拒否する
        self.take_step(data, rect, delay)
            .inspect_err(|_| self.poisoned = true)?;

        self.previous.clear();
        self.previous.extend_from_slice(data);
        self.frames_accepted += 1;
        Ok(())
    }

    /// フレームのうち実際に書き出す領域を決める
    ///
    /// 先頭フレームはIDATに入るためキャンバス全体とする。以降は直前のフレームとの
    /// 差分の外接矩形を使い、差分が無い場合はfcTLの個数を保つために1画素だけ書き直す。
    fn frame_rect(&self, data: &[u8]) -> Rect {
        const UNCHANGED: Rect = Rect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        };

        if self.frames_accepted == 0 {
            return Rect {
                x: 0,
                y: 0,
                width: self.width,
                height: self.height,
            };
        }

        diff::dirty_rect(&self.previous, data, self.stride, self.bytes_per_pixel)
            .unwrap_or(UNCHANGED)
    }

    /// フレームを溜めるか書き出すかを決め、決めたとおりに処理する
    fn take_step(&mut self, data: &[u8], rect: Rect, delay: FrameDelay) -> Result<(), Error> {
        let is_last = self.frames_accepted + 1 == self.num_frames;
        let input_color_type = self.output_color_type;

        let step = match &mut self.pending {
            None => Step::Emit,
            Some(spool) => {
                let region_len = rect.width as usize * rect.height as usize * self.bytes_per_pixel;
                if spool.can_hold(region_len) {
                    spool.push(data, rect, delay, self.stride, self.bytes_per_pixel);
                    self.peak_spool_bytes = self.peak_spool_bytes.max(spool.len());

                    if spool.transparent() {
                        Step::Commit(input_color_type)
                    } else if is_last {
                        Step::Commit(ColorType::Rgb8)
                    } else {
                        Step::Hold
                    }
                } else {
                    Step::Abandon
                }
            }
        };

        match step {
            Step::Emit => self.emit_frame(data, rect, delay),
            Step::Hold => Ok(()),
            Step::Commit(output) => self.commit(output),
            Step::Abandon => {
                self.commit(input_color_type)?;
                self.emit_frame(data, rect, delay)
            }
        }
    }

    /// 出力の色種別を確定し、ヘッダに続けて溜めたフレームを書き出す
    fn commit(&mut self, output: ColorType) -> Result<(), Error> {
        let Some(spool) = self.pending.take() else {
            return Ok(());
        };

        self.output_color_type = output;
        self.write_header()?;

        let in_bpp = self.bytes_per_pixel;
        let out_bpp = output.bytes_per_pixel();
        for frame in spool.frames() {
            let region_stride = frame.rect.width as usize * out_bpp;
            if in_bpp == out_bpp {
                self.compress(&frame.data, region_stride, out_bpp);
            } else {
                let mut converted = std::mem::take(&mut self.region);
                converted.clear();
                region::append_pixels(&frame.data, in_bpp, out_bpp, &mut converted);
                self.compress(&converted, region_stride, out_bpp);
                self.region = converted;
            }
            self.write_frame(frame.rect, frame.delay)?;
        }

        Ok(())
    }

    /// 出力の色種別が確定したフレームを1つ書き出す
    fn emit_frame(&mut self, data: &[u8], rect: Rect, delay: FrameDelay) -> Result<(), Error> {
        let in_bpp = self.bytes_per_pixel;
        let out_bpp = self.output_color_type.bytes_per_pixel();
        let region_stride = rect.width as usize * out_bpp;

        if in_bpp == out_bpp && rect.width as usize * in_bpp == self.stride {
            // 変換の要らない全幅の矩形は `data` 上で既に連続している
            let head = rect.y as usize * self.stride;
            let len = region_stride * rect.height as usize;
            self.compress(&data[head..head + len], region_stride, out_bpp);
        } else {
            let mut cropped = std::mem::take(&mut self.region);
            cropped.clear();
            region::crop(data, rect, self.stride, in_bpp, out_bpp, &mut cropped);
            self.compress(&cropped, region_stride, out_bpp);
            self.region = cropped;
        }

        self.write_frame(rect, delay)
    }

    /// 連続した領域をフィルタして圧縮し、[`Self::compressed`] へ格納する
    fn compress(&mut self, region: &[u8], region_stride: usize, bpp: usize) {
        self.filtered.clear();
        filter::filter_image(
            region,
            region_stride,
            bpp,
            &mut self.scratch,
            &mut self.filtered,
        );

        self.compressed.clear();
        self.compressor
            .compress_into(&self.filtered, &mut self.compressed);
    }

    fn write_frame(&mut self, rect: Rect, delay: FrameDelay) -> Result<(), Error> {
        self.write_fctl(rect, delay)?;

        // 先頭フレームはIDATに入り、以降はfdATに入る
        if self.frames_emitted == 0 {
            chunk::write(&mut self.writer, *b"IDAT", &self.compressed)?;
        } else {
            chunk::write_parts(
                &mut self.writer,
                *b"fdAT",
                &[&self.sequence.to_be_bytes(), &self.compressed],
            )?;
            self.sequence += 1;
        }

        self.frames_emitted += 1;
        Ok(())
    }

    fn write_fctl(&mut self, rect: Rect, delay: FrameDelay) -> Result<(), Error> {
        let (delay_num, delay_den) = delay.to_parts();

        let mut fctl = [0u8; 26];
        fctl[0..4].copy_from_slice(&self.sequence.to_be_bytes());
        fctl[4..8].copy_from_slice(&rect.width.to_be_bytes());
        fctl[8..12].copy_from_slice(&rect.height.to_be_bytes());
        fctl[12..16].copy_from_slice(&rect.x.to_be_bytes());
        fctl[16..20].copy_from_slice(&rect.y.to_be_bytes());
        fctl[20..22].copy_from_slice(&delay_num.to_be_bytes());
        fctl[22..24].copy_from_slice(&delay_den.to_be_bytes());
        fctl[24] = DISPOSE_OP_NONE;
        fctl[25] = BLEND_OP_SOURCE;
        chunk::write(&mut self.writer, *b"fcTL", &fctl)?;

        self.sequence += 1;
        Ok(())
    }

    /// 終端して書き出し先を返す
    ///
    /// # Errors
    /// 投入されたフレーム数が宣言したフレーム数に満たないとき、書き出しに失敗したとき、
    /// または過去の書き出し失敗でエンコーダが使用不能なとき。
    pub fn finish(mut self) -> Result<W, Error> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        if self.frames_accepted != self.num_frames {
            return Err(Error::FrameCountMismatch {
                expected: self.num_frames,
                actual: self.frames_accepted,
            });
        }

        chunk::write(&mut self.writer, *b"IEND", &[])?;
        Ok(self.writer)
    }
}
