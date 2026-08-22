//! APNGのストリーミング書き出し

use crate::chunk;
use crate::delay::FrameDelay;
use crate::error::Error;
use crate::filter;
use crate::zlib::Compressor;
use std::io::Write;

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

/// エンコード設定
#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// 入力フレームの色種別
    pub color_type: ColorType,
    /// deflateの圧縮レベル (1..=9)
    pub compression_level: u32,
    /// アニメーションの再生回数 (0で無限ループ)
    pub num_plays: u32,
}

/// APNGエンコーダ
///
/// [`Encoder::new`] でヘッダを、[`Encoder::add_frame`] でフレームを1つずつ書き出し、
/// [`Encoder::finish`] で終端する。フレームは保持せず、その場で書き出す。
pub struct Encoder<W: Write> {
    writer: W,
    width: u32,
    height: u32,
    num_frames: u32,
    frames_written: u32,
    /// fcTLとfdATで共有する連番
    sequence: u32,
    bytes_per_pixel: usize,
    /// 1行のバイト数
    stride: usize,
    /// 1フレームのバイト数
    frame_len: usize,
    compressor: Compressor,
    filtered: Vec<u8>,
    compressed: Vec<u8>,
}

impl<W: Write> Encoder<W> {
    /// シグネチャとヘッダを書き出し、`num_frames` フレームを受け付ける状態にする
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
        if !(1..=9).contains(&config.compression_level) {
            return Err(Error::InvalidCompressionLevel(config.compression_level));
        }

        let bytes_per_pixel = config.color_type.bytes_per_pixel();
        let stride = (width as usize)
            .checked_mul(bytes_per_pixel)
            .ok_or(Error::ImageTooLarge { width, height })?;
        let frame_len = stride
            .checked_mul(height as usize)
            .ok_or(Error::ImageTooLarge { width, height })?;

        let mut encoder = Encoder {
            writer,
            width,
            height,
            num_frames,
            frames_written: 0,
            sequence: 0,
            bytes_per_pixel,
            stride,
            frame_len,
            compressor: Compressor::new(config.compression_level),
            filtered: Vec::new(),
            compressed: Vec::new(),
        };
        encoder.write_header(config)?;
        Ok(encoder)
    }

    fn write_header(&mut self, config: Config) -> Result<(), Error> {
        self.writer.write_all(&chunk::SIGNATURE)?;

        let mut ihdr = [0u8; 13];
        ihdr[0..4].copy_from_slice(&self.width.to_be_bytes());
        ihdr[4..8].copy_from_slice(&self.height.to_be_bytes());
        ihdr[8] = 8;
        ihdr[9] = config.color_type.code();
        chunk::write(&mut self.writer, *b"IHDR", &ihdr)?;

        let mut actl = [0u8; 8];
        actl[0..4].copy_from_slice(&self.num_frames.to_be_bytes());
        actl[4..8].copy_from_slice(&config.num_plays.to_be_bytes());
        chunk::write(&mut self.writer, *b"acTL", &actl)?;

        Ok(())
    }

    /// フレームを1つ書き出す
    ///
    /// `data` は上から下・左から右の順に並んだ `幅 * 高さ * 1画素のバイト数` バイトであること。
    ///
    /// # Errors
    /// `data` の長さが合わないとき、または宣言したフレーム数を超えたとき。
    pub fn add_frame(&mut self, data: &[u8], delay: FrameDelay) -> Result<(), Error> {
        if self.frames_written == self.num_frames {
            return Err(Error::FrameCountMismatch {
                expected: self.num_frames,
                actual: self.frames_written + 1,
            });
        }

        if data.len() != self.frame_len {
            return Err(Error::FrameSizeMismatch {
                expected: self.frame_len,
                actual: data.len(),
            });
        }

        self.filtered.clear();
        filter::filter_image(data, self.stride, self.bytes_per_pixel, &mut self.filtered);
        self.compressed.clear();
        self.compressor
            .compress_into(&self.filtered, &mut self.compressed);

        self.write_fctl(delay)?;

        // 先頭フレームはIDATに入り、以降はfdATに入る
        if self.frames_written == 0 {
            chunk::write(&mut self.writer, *b"IDAT", &self.compressed)?;
        } else {
            chunk::write_parts(
                &mut self.writer,
                *b"fdAT",
                &[&self.sequence.to_be_bytes(), &self.compressed],
            )?;
            self.sequence += 1;
        }

        self.frames_written += 1;
        Ok(())
    }

    fn write_fctl(&mut self, delay: FrameDelay) -> Result<(), Error> {
        let (delay_num, delay_den) = delay.to_parts();

        let mut fctl = [0u8; 26];
        fctl[0..4].copy_from_slice(&self.sequence.to_be_bytes());
        fctl[4..8].copy_from_slice(&self.width.to_be_bytes());
        fctl[8..12].copy_from_slice(&self.height.to_be_bytes());
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
    /// 投入されたフレーム数が宣言したフレーム数に満たないとき。
    pub fn finish(mut self) -> Result<W, Error> {
        if self.frames_written != self.num_frames {
            return Err(Error::FrameCountMismatch {
                expected: self.num_frames,
                actual: self.frames_written,
            });
        }

        chunk::write(&mut self.writer, *b"IEND", &[])?;
        Ok(self.writer)
    }
}
