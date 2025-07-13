/*
MIT License

Copyright (c) 2020 Jared Forth

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
 */
#![allow(dead_code)]

use libwebp_sys::*;
use std::ffi::CString;
use std::fs::File;
use std::io::{BufWriter, Write};

/// Pixel layout describing the order of color channels
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum PixelLayout {
    Rgb,
    Rgba,
}

impl PixelLayout {
    pub fn is_alpha(self) -> bool {
        self == PixelLayout::Rgba
    }

    pub fn bytes_per_pixel(self) -> usize {
        match self {
            PixelLayout::Rgb => 3,
            PixelLayout::Rgba => 4,
        }
    }
}

/// Animation frame data
pub struct AnimFrame {
    image: Vec<u8>,
    layout: PixelLayout,
    width: u32,
    height: u32,
    timestamp: i32,
}

impl AnimFrame {
    pub fn new(image: &[u8], layout: PixelLayout, width: u32, height: u32, timestamp: i32) -> Self {
        Self {
            image: image.to_vec(),
            layout,
            width,
            height,
            timestamp,
        }
    }

    /// Create a frame from RGB image data
    pub fn from_rgb(image: &[u8], width: u32, height: u32, timestamp: i32) -> Self {
        Self::new(image, PixelLayout::Rgb, width, height, timestamp)
    }

    /// Create a frame from RGBA image data
    pub fn from_rgba(image: &[u8], width: u32, height: u32, timestamp: i32) -> Self {
        Self::new(image, PixelLayout::Rgba, width, height, timestamp)
    }

    pub fn get_image(&self) -> &[u8] {
        &self.image
    }

    pub fn get_layout(&self) -> PixelLayout {
        self.layout
    }

    pub fn get_time_ms(&self) -> i32 {
        self.timestamp
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }
}

/// WebP animation encoder that accumulates frames and writes to file on finalization
pub struct AnimEncoder<'a> {
    encoder: *mut WebPAnimEncoder,
    config: &'a WebPConfig,
    width: u32,
    height: u32,
    writer: BufWriter<File>,
    muxparams: WebPMuxAnimParams,
    finalized: bool,
}

/// Errors that can occur during animation encoding
#[derive(Debug)]
pub enum StreamingAnimEncodeError {
    /// WebP encoding error
    WebPEncodingError(WebPEncodingError),
    /// WebP mux error
    WebPMuxError(WebPMuxError),
    /// WebP animation encoder error
    WebPAnimEncoderGetError(String),
    /// I/O error
    IoError(std::io::Error),
    /// Encoder not properly initialized
    EncoderNotInitialized,
    /// Encoder already finalized
    AlreadyFinalized,
}

impl std::fmt::Display for StreamingAnimEncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StreamingAnimEncodeError::WebPEncodingError(e) => {
                write!(f, "WebP encoding error: {:?}", e)
            }
            StreamingAnimEncodeError::WebPMuxError(e) => write!(f, "WebP mux error: {:?}", e),
            StreamingAnimEncodeError::WebPAnimEncoderGetError(s) => {
                write!(f, "WebP animation encoder error: {}", s)
            }
            StreamingAnimEncodeError::IoError(e) => write!(f, "IO error: {}", e),
            StreamingAnimEncodeError::EncoderNotInitialized => write!(f, "Encoder not initialized"),
            StreamingAnimEncodeError::AlreadyFinalized => write!(f, "Encoder already finalized"),
        }
    }
}

impl std::error::Error for StreamingAnimEncodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StreamingAnimEncodeError::IoError(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for StreamingAnimEncodeError {
    fn from(err: std::io::Error) -> Self {
        StreamingAnimEncodeError::IoError(err)
    }
}

impl<'a> AnimEncoder<'a> {
    /// Create a new streaming animation encoder
    pub fn new(
        width: u32,
        height: u32,
        config: &'a WebPConfig,
        file: File,
    ) -> Result<Self, StreamingAnimEncodeError> {
        let writer = BufWriter::new(file);

        unsafe {
            let mut uninit = std::mem::MaybeUninit::<WebPAnimEncoderOptions>::uninit();
            let mux_abi_version = WebPGetMuxABIVersion();
            WebPAnimEncoderOptionsInitInternal(uninit.as_mut_ptr(), mux_abi_version);

            let encoder = WebPAnimEncoderNewInternal(
                width as i32,
                height as i32,
                uninit.as_ptr(),
                mux_abi_version,
            );

            if encoder.is_null() {
                return Err(StreamingAnimEncodeError::EncoderNotInitialized);
            }

            Ok(Self {
                encoder,
                config,
                width,
                height,
                writer,
                muxparams: WebPMuxAnimParams {
                    bgcolor: 0,
                    loop_count: 0,
                },
                finalized: false,
            })
        }
    }

    /// Set background color (RGBA)
    pub fn set_bgcolor(&mut self, rgba: [u8; 4]) {
        let bgcolor = (u32::from(rgba[3]) << 24)
            + (u32::from(rgba[2]) << 16)
            + (u32::from(rgba[1]) << 8)
            + (u32::from(rgba[0]));
        self.muxparams.bgcolor = bgcolor;
    }

    /// Set loop count (0 = infinite loop)
    pub fn set_loop_count(&mut self, loop_count: i32) {
        self.muxparams.loop_count = loop_count;
    }

    /// Add a frame to the animation
    pub fn add_frame(&mut self, frame: AnimFrame) -> Result<(), StreamingAnimEncodeError> {
        if self.finalized {
            return Err(StreamingAnimEncodeError::AlreadyFinalized);
        }

        unsafe {
            let mut pic = new_picture(
                frame.get_image(),
                frame.get_layout(),
                self.width,
                self.height,
            )?;

            let ok = WebPAnimEncoderAdd(
                self.encoder,
                &mut pic as *mut _,
                frame.get_time_ms() as std::os::raw::c_int,
                self.config,
            );

            if ok == 0 {
                return Err(StreamingAnimEncodeError::WebPEncodingError(
                    WebPEncodingError::VP8_ENC_ERROR_OUT_OF_MEMORY,
                ));
            }
        }

        Ok(())
    }

    /// Finalize the animation and write to file
    pub fn finalize(mut self) -> Result<(), StreamingAnimEncodeError> {
        if self.finalized {
            return Err(StreamingAnimEncodeError::AlreadyFinalized);
        }

        unsafe {
            // Add the final null frame to signal end of animation
            WebPAnimEncoderAdd(self.encoder, std::ptr::null_mut(), 0, std::ptr::null());

            let mut webp_data = std::mem::MaybeUninit::<WebPData>::uninit();
            let ok = WebPAnimEncoderAssemble(self.encoder, webp_data.as_mut_ptr());

            if ok == 0 {
                let cstring = WebPAnimEncoderGetError(self.encoder);
                let cstring = CString::from_raw(cstring as *mut _);
                let string = cstring.to_string_lossy().to_string();
                return Err(StreamingAnimEncodeError::WebPAnimEncoderGetError(string));
            }

            let mux_abi_version = WebPGetMuxABIVersion();
            let mux = WebPMuxCreateInternal(webp_data.as_ptr(), 1, mux_abi_version);
            let mux_error = WebPMuxSetAnimationParams(mux, &self.muxparams);

            if mux_error != WebPMuxError::WEBP_MUX_OK {
                return Err(StreamingAnimEncodeError::WebPMuxError(mux_error));
            }

            let mut final_data = std::mem::MaybeUninit::<WebPData>::uninit();
            WebPMuxAssemble(mux, final_data.as_mut_ptr());
            WebPMuxDelete(mux);

            let final_raw_data: WebPData = final_data.assume_init();
            if final_raw_data.size > 0 {
                let data_slice =
                    std::slice::from_raw_parts(final_raw_data.bytes, final_raw_data.size);
                self.writer.write_all(data_slice)?;
            }

            self.writer.flush()?;
            self.finalized = true;
        }

        Ok(())
    }
}

impl<'a> Drop for AnimEncoder<'a> {
    fn drop(&mut self) {
        unsafe {
            if !self.encoder.is_null() {
                WebPAnimEncoderDelete(self.encoder);
            }
        }
    }
}

/// Helper function to create WebPPicture from raw image data
unsafe fn new_picture(
    image: &[u8],
    layout: PixelLayout,
    width: u32,
    height: u32,
) -> Result<WebPPicture, StreamingAnimEncodeError> {
    let mut pic = match WebPPicture::new() {
        Ok(pic) => pic,
        Err(_) => return Err(StreamingAnimEncodeError::EncoderNotInitialized),
    };

    pic.width = width as i32;
    pic.height = height as i32;

    let stride = width as i32 * layout.bytes_per_pixel() as i32;

    match layout {
        PixelLayout::Rgb => {
            unsafe { WebPPictureImportRGB(&mut pic, image.as_ptr(), stride) };
        }
        PixelLayout::Rgba => {
            unsafe { WebPPictureImportRGBA(&mut pic, image.as_ptr(), stride) };
        }
    }

    Ok(pic)
}

pub use libwebp_sys::WebPConfig;
