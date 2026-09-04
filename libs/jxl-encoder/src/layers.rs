//! `JxlDecoder` と並列実行器のRAII、および書き出したバイト列からの層の取り出し

use crate::error::{DecodingError, Error};
use crate::layout::ColorType;
use anim_core::Rect;
use jxl_sys::{
    JXL_DEC_ERROR, JXL_DEC_FULL_IMAGE, JXL_DEC_NEED_IMAGE_OUT_BUFFER, JXL_DEC_NEED_MORE_INPUT,
    JXL_DEC_SUCCESS, JXL_FALSE, JXL_NATIVE_ENDIAN, JXL_TYPE_UINT8, JxlDecoder, JxlDecoderCreate,
    JxlDecoderDestroy, JxlDecoderImageOutBufferSize, JxlDecoderProcessInput,
    JxlDecoderReleaseInput, JxlDecoderSetCoalescing, JxlDecoderSetImageOutBuffer,
    JxlDecoderSetInput, JxlDecoderSetParallelRunner, JxlDecoderSubscribeEvents, JxlPixelFormat,
    JxlThreadParallelRunner, JxlThreadParallelRunnerCreate, JxlThreadParallelRunnerDestroy,
};
use std::collections::VecDeque;
use std::ffi::{c_int, c_void};
use std::ptr;

/// 失敗した `JxlDecoderStatus` を写す
fn check(status: c_int) -> Result<(), Error> {
    if status == JXL_DEC_SUCCESS {
        return Ok(());
    }
    Err(Error::Decode(DecodingError::new(status)))
}

/// 確保に失敗したときのエラー
fn allocation_failed() -> Error {
    Error::Decode(DecodingError::new(JXL_DEC_ERROR))
}

/// `JxlDecoder` と、それが参照する並列実行器のRAII
///
/// 実行器は符号化器のものとは別に持つ。
struct Raw {
    dec: *mut JxlDecoder,
    runner: *mut c_void,
}

impl Raw {
    /// `max_threads` の作業スレッドを持つ復号器を作る
    fn new(max_threads: u32) -> Result<Self, Error> {
        let runner = unsafe { JxlThreadParallelRunnerCreate(ptr::null(), max_threads as usize) };
        if runner.is_null() {
            return Err(allocation_failed());
        }
        let dec = unsafe { JxlDecoderCreate(ptr::null()) };
        if dec.is_null() {
            unsafe { JxlThreadParallelRunnerDestroy(runner) };
            return Err(allocation_failed());
        }

        let raw = Raw { dec, runner };
        check(unsafe {
            JxlDecoderSetParallelRunner(raw.dec, Some(JxlThreadParallelRunner), raw.runner)
        })?;
        // 合成せずに、フレームが書いた層をそのまま返させる
        check(unsafe { JxlDecoderSetCoalescing(raw.dec, JXL_FALSE) })?;
        // 層の画素が復号されるのは `JXL_DEC_FULL_IMAGE` を購読したときで、受け皿の
        // 要求はそれに伴って返る
        check(unsafe { JxlDecoderSubscribeEvents(raw.dec, JXL_DEC_FULL_IMAGE) })?;
        Ok(raw)
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        unsafe {
            JxlDecoderDestroy(self.dec);
            JxlThreadParallelRunnerDestroy(self.runner);
        }
    }
}

/// 書き出したバイト列から、フレームが書いた層を取り出す
///
/// [`Layers::wrote`] で書いた矩形を控え、[`Layers::feed`] で書き出したバイト列を
/// 継ぎ足す。揃った層は控えた矩形と対になって、書いた順に呼び戻しへ渡る。
///
/// 層1枚ぶんの画素と、渡されたバイト列のうち読み進めていない分を抱える。層は
/// キャンバス全面まで大きくなる (1920x1080のRGBA8で約8.3MB)。
pub struct Layers {
    raw: Raw,
    format: JxlPixelFormat,
    /// 層の1画素あたりのバイト数
    bytes_per_pixel: usize,
    /// 復号器へ渡すバイト列。読み残しの後ろへ継ぎ足す
    input: Vec<u8>,
    /// 層1枚の受け皿
    layer: Vec<u8>,
    /// 層を待っている矩形
    pending: VecDeque<Rect>,
    /// 返した層の枚数
    returned: u64,
}

impl Layers {
    /// `color_type` の画素を返す復号器を、`max_threads` の作業スレッドで組み立てる
    ///
    /// # Errors
    /// 復号器を組み立てられないとき [`Error::Decode`]。
    pub fn new(color_type: ColorType, max_threads: u32) -> Result<Self, Error> {
        Ok(Layers {
            raw: Raw::new(max_threads)?,
            format: JxlPixelFormat {
                num_channels: color_type.num_channels(),
                data_type: JXL_TYPE_UINT8,
                endianness: JXL_NATIVE_ENDIAN,
                align: 0,
            },
            bytes_per_pixel: color_type.bytes_per_pixel(),
            input: Vec::new(),
            layer: Vec::new(),
            pending: VecDeque::new(),
            returned: 0,
        })
    }

    /// 書いた矩形を控える
    ///
    /// 返る層は控えた順にこの矩形と対になり、画素は矩形の面積のぶんだけ並ぶ。
    pub fn wrote(&mut self, rect: Rect) {
        self.pending.push_back(rect);
    }

    /// バイト列を継ぎ足し、揃った層を書いた矩形と対にして `each` へ渡す
    ///
    /// 渡すのは [`Encoder`](crate::Encoder) が書き出したバイト列を、書き出した順に
    /// 切れ目なく並べたもの。読み残したバイトは次の呼び出しへ持ち越す。
    ///
    /// # Errors
    /// libjxlの復号が失敗したとき [`Error::Decode`]。
    ///
    /// # Panics
    /// 返った層が、[`Layers::wrote`] で控えた矩形と枚数か大きさで食い違ったとき。
    pub fn feed(&mut self, bytes: &[u8], mut each: impl FnMut(Rect, &[u8])) -> Result<(), Error> {
        if bytes.is_empty() {
            return Ok(());
        }
        self.input.extend_from_slice(bytes);

        // 掴んだ入力は、読み進みがどう終わっても手放す
        let status = self.pull(&mut each);
        let unprocessed = unsafe { JxlDecoderReleaseInput(self.raw.dec) };
        self.input.drain(..self.input.len() - unprocessed);

        match status? {
            JXL_DEC_NEED_MORE_INPUT => Ok(()),
            JXL_DEC_SUCCESS => {
                assert!(
                    self.pending.is_empty(),
                    "ストリームが閉じた時点で、書いた矩形 {} 枚に対して層が {} 枚返っている",
                    self.returned as usize + self.pending.len(),
                    self.returned
                );
                Ok(())
            }
            status => Err(Error::Decode(DecodingError::new(status))),
        }
    }

    /// 入力を掴んで読み進め、止まった `JxlDecoderStatus` を返す
    fn pull(&mut self, each: &mut impl FnMut(Rect, &[u8])) -> Result<c_int, Error> {
        check(unsafe { JxlDecoderSetInput(self.raw.dec, self.input.as_ptr(), self.input.len()) })?;
        loop {
            match unsafe { JxlDecoderProcessInput(self.raw.dec) } {
                JXL_DEC_NEED_IMAGE_OUT_BUFFER => self.hold_layer()?,
                JXL_DEC_FULL_IMAGE => self.deliver(each),
                status => return Ok(status),
            }
        }
    }

    /// 揃った層を、控えた矩形と対にして渡す
    fn deliver(&mut self, each: &mut impl FnMut(Rect, &[u8])) {
        let Some(rect) = self.pending.pop_front() else {
            panic!(
                "書いた矩形は {} 枚なのに、{} 枚目の層が返った",
                self.returned,
                self.returned + 1
            )
        };
        let expected = rect.area() as usize * self.bytes_per_pixel;
        assert!(
            self.layer.len() == expected,
            "{} 枚目の層が {} バイトで、矩形 {}x{} の {expected} バイトと違う",
            self.returned + 1,
            self.layer.len(),
            rect.width,
            rect.height
        );
        self.returned += 1;
        each(rect, &self.layer);
    }

    /// 今のフレームが返す層の大きさへ受け皿を合わせ、復号器へ渡す
    fn hold_layer(&mut self) -> Result<(), Error> {
        let mut size = 0usize;
        check(unsafe { JxlDecoderImageOutBufferSize(self.raw.dec, &self.format, &mut size) })?;
        self.layer.resize(size, 0);
        check(unsafe {
            JxlDecoderSetImageOutBuffer(
                self.raw.dec,
                &self.format,
                self.layer.as_mut_ptr().cast::<c_void>(),
                size,
            )
        })
    }
}
