//! 1フレームの符号化: 切り出し → `WebPEncode` → ペイロード抽出

use crate::Config;
use crate::error::{EncodingError, Error};
use crate::layout::Layout;
use crate::picture::Picture;
use anim_core::{Rect, crop};
use std::ffi::c_int;
use std::mem::MaybeUninit;
use std::ops::Range;
use webp_sys::{WebPConfig, WebPConfigInit, WebPValidateConfig};

/// RIFFのチャンクヘッダ (FourCCとサイズ) のバイト数
const CHUNK_HEADER: usize = 8;

/// ファイル先頭のRIFFヘッダ (FourCC、サイズ、`WEBP`) のバイト数
const FILE_HEADER: usize = 12;

/// 1フレームの符号化結果
///
/// 単葉の .webp と、その中でフレームを表すチャンクの位置を持つ。
pub(crate) struct EncodedFrame {
    bytes: Vec<u8>,
    alpha: Option<Range<usize>>,
    image: Range<usize>,
}

impl EncodedFrame {
    /// 単葉の .webp 全体
    pub(crate) fn still(&self) -> &[u8] {
        &self.bytes
    }

    /// αを別に持つ形式のときの `ALPH` チャンク
    ///
    /// 返すのはFourCCから詰めまでを含むチャンク全体。
    pub(crate) fn alpha(&self) -> Option<&[u8]> {
        self.alpha.clone().map(|range| &self.bytes[range])
    }

    /// `VP8 ` または `VP8L` チャンク
    ///
    /// 返すのはFourCCから詰めまでを含むチャンク全体。
    pub(crate) fn image(&self) -> &[u8] {
        &self.bytes[self.image.clone()]
    }
}

/// 設定を写した符号化器
pub(crate) struct Codec {
    config: WebPConfig,
    /// 矩形を切り出す先。フレームごとに使い回す
    buffer: Vec<u8>,
}

impl Codec {
    /// [`Config`] を `WebPConfig` へ写す
    ///
    /// # Errors
    /// 写した設定が値域に収まらないとき [`Error::Encode`]。
    pub(crate) fn new(config: &Config) -> Result<Self, Error> {
        let mut raw = MaybeUninit::<WebPConfig>::uninit();
        if unsafe { WebPConfigInit(raw.as_mut_ptr()) } == 0 {
            return Err(Error::Encode(EncodingError::InvalidConfiguration));
        }
        let mut raw = unsafe { raw.assume_init() };

        raw.lossless = c_int::from(config.lossless);
        raw.quality = config.quality;
        raw.method = c_int::from(config.method);
        raw.exact = c_int::from(config.lossless);

        if !config.quality.is_finite() || unsafe { WebPValidateConfig(&raw) } == 0 {
            return Err(Error::Encode(EncodingError::InvalidConfiguration));
        }

        Ok(Codec {
            config: raw,
            buffer: Vec::new(),
        })
    }

    /// `data` から `rect` を切り出して符号化する
    ///
    /// `data` は `layout` のとおりに並んでいること。
    ///
    /// # Errors
    /// 符号化に失敗したとき [`Error::Encode`]。結果のチャンク構成を読み取れない
    /// とき [`Error::MalformedOutput`]。
    pub(crate) fn encode(
        &mut self,
        data: &[u8],
        layout: &Layout,
        rect: Rect,
    ) -> Result<EncodedFrame, Error> {
        self.buffer.clear();
        crop(
            data,
            rect,
            layout.stride,
            layout.bytes_per_pixel,
            layout.bytes_per_pixel,
            &mut self.buffer,
        );

        let picture = Picture::import(&self.buffer, rect.width, rect.height, layout.color_type)?;
        let bytes = picture.encode(&self.config)?;
        let (alpha, image) = locate_payload(&bytes).ok_or(Error::MalformedOutput)?;

        Ok(EncodedFrame {
            bytes,
            alpha,
            image,
        })
    }
}

/// 単葉の .webp からフレームを表すチャンクの位置を取る
///
/// 返す位置はFourCCから始まり、奇数サイズのチャンクを詰める1バイトを含む。
/// 構成が読み取れなければ `None`。
fn locate_payload(bytes: &[u8]) -> Option<(Option<Range<usize>>, Range<usize>)> {
    let header = bytes.get(..FILE_HEADER)?;
    if &header[..4] != b"RIFF" || &header[8..] != b"WEBP" {
        return None;
    }
    let riff_size = u32::from_le_bytes(header[4..8].try_into().ok()?) as usize;
    if riff_size != bytes.len().checked_sub(CHUNK_HEADER)? {
        return None;
    }

    let mut alpha = None;
    let mut image = None;
    let mut cursor = FILE_HEADER;
    while cursor < bytes.len() {
        let fourcc: [u8; 4] = bytes.get(cursor..cursor + 4)?.try_into().ok()?;
        let size = u32::from_le_bytes(bytes.get(cursor + 4..cursor + 8)?.try_into().ok()?) as usize;
        let end = cursor
            .checked_add(CHUNK_HEADER)?
            .checked_add(size)?
            .checked_add(size & 1)?;
        if end > bytes.len() {
            return None;
        }

        match &fourcc {
            b"VP8X" => {}
            b"ALPH" if alpha.is_none() && image.is_none() => alpha = Some(cursor..end),
            b"VP8 " | b"VP8L" if image.is_none() => image = Some(cursor..end),
            _ => return None,
        }
        cursor = end;
    }

    Some((alpha, image?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::ColorType;

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

    /// FourCCとサイズを持つチャンク。奇数サイズは1バイトの0で詰める
    fn chunk(fourcc: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut bytes = fourcc.to_vec();
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        if payload.len() % 2 == 1 {
            bytes.push(0);
        }
        bytes
    }

    /// チャンクを並べた .webp
    fn file(chunks: &[&[u8]]) -> Vec<u8> {
        let body: Vec<u8> = chunks.concat();
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
        bytes.extend_from_slice(b"WEBP");
        bytes.extend_from_slice(&body);
        bytes
    }

    /// 位置を切り出す
    fn at(bytes: &[u8], range: Range<usize>) -> Vec<u8> {
        bytes[range].to_vec()
    }

    #[test]
    fn a_simple_lossless_file_yields_the_vp8l_chunk_alone() {
        let image = chunk(b"VP8L", &[0x2F, 0x00, 0x00, 0x00]);
        let bytes = file(&[&image]);

        let (alpha, found) = locate_payload(&bytes).unwrap();
        assert_eq!(alpha, None);
        assert_eq!(at(&bytes, found), image);
    }

    #[test]
    fn an_extended_file_yields_the_alpha_and_image_chunks_past_the_vp8x() {
        let alpha = chunk(b"ALPH", &[0x11, 0x22, 0x33, 0x44]);
        let image = chunk(b"VP8 ", &[0x55, 0x66, 0x77, 0x88]);
        let bytes = file(&[&chunk(b"VP8X", &[0; 10]), &alpha, &image]);

        let (found_alpha, found_image) = locate_payload(&bytes).unwrap();
        assert_eq!(at(&bytes, found_alpha.unwrap()), alpha);
        assert_eq!(at(&bytes, found_image), image);
    }

    /// 奇数サイズのチャンクを詰める1バイトを飛ばさないと、次のチャンクの
    /// FourCCを1バイトずれた位置から読むことになる
    #[test]
    fn an_odd_sized_alpha_chunk_is_followed_across_its_padding() {
        let alpha = chunk(b"ALPH", &[0x11, 0x22, 0x33]);
        let image = chunk(b"VP8 ", &[0x55, 0x66, 0x77, 0x88]);
        assert_eq!(alpha.len(), CHUNK_HEADER + 4);
        let bytes = file(&[&chunk(b"VP8X", &[0; 10]), &alpha, &image]);

        let (found_alpha, found_image) = locate_payload(&bytes).unwrap();
        assert_eq!(at(&bytes, found_alpha.unwrap()), alpha);
        assert_eq!(at(&bytes, found_image), image);
    }

    /// 末尾のチャンクの詰めもANMFが載せるバイト列に含まれる
    #[test]
    fn an_odd_sized_image_chunk_keeps_its_padding() {
        let image = chunk(b"VP8L", &[0x2F, 0x00, 0x00]);
        let bytes = file(&[&image]);

        let (_, found) = locate_payload(&bytes).unwrap();
        assert_eq!(found.len(), CHUNK_HEADER + 4);
        assert_eq!(at(&bytes, found), image);
        assert_eq!(bytes.last(), Some(&0));
    }

    #[test]
    fn a_file_without_an_image_chunk_is_refused() {
        let bytes = file(&[&chunk(b"VP8X", &[0; 10]), &chunk(b"ALPH", &[0x11])]);
        assert_eq!(locate_payload(&bytes), None);
    }

    #[test]
    fn a_truncated_file_is_refused() {
        let bytes = file(&[&chunk(b"VP8L", &[0x2F, 0x00, 0x00, 0x00])]);
        for len in 0..bytes.len() {
            assert_eq!(locate_payload(&bytes[..len]), None, "{len} バイト");
        }
        assert!(locate_payload(&bytes).is_some());
    }

    #[test]
    fn a_chunk_reaching_past_the_end_is_refused() {
        let mut bytes = file(&[&chunk(b"VP8L", &[0x2F, 0x00, 0x00, 0x00])]);
        let size = (bytes.len() as u32).to_le_bytes();
        bytes[FILE_HEADER + 4..FILE_HEADER + CHUNK_HEADER].copy_from_slice(&size);
        assert_eq!(locate_payload(&bytes), None);
    }

    #[test]
    fn a_file_that_is_not_riff_webp_is_refused() {
        let mut bytes = file(&[&chunk(b"VP8L", &[0x2F, 0x00, 0x00, 0x00])]);
        bytes[8..12].copy_from_slice(b"WEBQ");
        assert_eq!(locate_payload(&bytes), None);
    }

    /// 素材と設定から符号化した単葉
    ///
    /// `alpha` を載せるのはRGBAの素材だけで、RGBの素材は不透明になる。
    fn encode(color_type: ColorType, lossless: bool, alpha: u8) -> EncodedFrame {
        let (width, height) = (32, 24);
        let layout = Layout::new(width, height, color_type).unwrap();
        let mut data = noise(layout.frame_len, 0x5EED);
        if color_type == ColorType::Rgba8 {
            for pixel in data.chunks_exact_mut(4) {
                pixel[3] = alpha;
            }
        }

        let mut codec = Codec::new(&Config {
            color_type,
            lossless,
            quality: 75.0,
            method: 4,
            num_plays: 0,
        })
        .unwrap();
        codec.encode(&data, &layout, layout.whole()).unwrap()
    }

    /// チャンクが持つペイロードのバイト数
    fn payload_len(chunk: &[u8]) -> usize {
        u32::from_le_bytes(chunk[4..8].try_into().unwrap()) as usize
    }

    #[test]
    fn a_lossless_frame_comes_back_as_a_single_vp8l_chunk() {
        let frame = encode(ColorType::Rgba8, true, 0x80);
        assert_eq!(frame.alpha(), None);
        assert_eq!(&frame.image()[..4], b"VP8L");
        assert_eq!(&frame.still()[..4], b"RIFF");

        // 詰めを踏む素材であることが、この形の検出力を支える
        let payload = payload_len(frame.image());
        assert_eq!(payload % 2, 1);
        assert_eq!(frame.image().len(), CHUNK_HEADER + payload + 1);
    }

    #[test]
    fn a_lossy_frame_with_alpha_comes_back_as_an_alpha_and_a_vp8_chunk() {
        let layout = Layout::new(24, 18, ColorType::Rgba8).unwrap();
        let mut data = noise(layout.frame_len, 0x5EED);
        for (index, pixel) in data.chunks_exact_mut(4).enumerate() {
            if index % 5 == 0 {
                pixel.fill(0);
            }
        }

        let mut codec = Codec::new(&Config {
            color_type: ColorType::Rgba8,
            lossless: false,
            quality: 75.0,
            method: 4,
            num_plays: 0,
        })
        .unwrap();
        let frame = codec.encode(&data, &layout, layout.whole()).unwrap();

        let alpha = frame.alpha().unwrap();
        assert_eq!(&alpha[..4], b"ALPH");
        assert_eq!(&frame.image()[..4], b"VP8 ");

        // 詰めを踏む素材であることが、この形の検出力を支える
        let payload = payload_len(alpha);
        assert_eq!(payload % 2, 1);
        assert_eq!(alpha.len(), CHUNK_HEADER + payload + 1);
    }

    /// αを持たない素材はVP8XもALPHも伴わない。条件は入力の色種別ではなく
    /// 透過画素の有無で、不透明なRGBAも単独のVP8チャンクで来る
    #[test]
    fn an_opaque_lossy_frame_comes_back_as_a_single_vp8_chunk() {
        for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
            let frame = encode(color_type, false, 0xFF);
            assert_eq!(frame.alpha(), None, "{color_type:?}");
            assert_eq!(&frame.image()[..4], b"VP8 ", "{color_type:?}");
        }
    }

    #[test]
    fn only_a_lossless_config_asks_libwebp_to_keep_every_byte() {
        let config = Config {
            color_type: ColorType::Rgba8,
            lossless: true,
            quality: 60.0,
            method: 3,
            num_plays: 0,
        };

        let lossless = Codec::new(&config).unwrap().config;
        assert_eq!((lossless.lossless, lossless.exact), (1, 1));
        assert_eq!((lossless.quality, lossless.method), (60.0, 3));

        let lossy = Codec::new(&Config {
            lossless: false,
            ..config
        })
        .unwrap()
        .config;
        assert_eq!((lossy.lossless, lossy.exact), (0, 0));
        assert_eq!((lossy.quality, lossy.method), (60.0, 3));

        assert_eq!((lossless.thread_level, lossy.thread_level), (0, 0));
    }

    #[test]
    fn a_quality_or_method_outside_the_range_is_refused() {
        let base = Config {
            color_type: ColorType::Rgba8,
            lossless: false,
            quality: 75.0,
            method: 4,
            num_plays: 0,
        };
        for config in [
            Config {
                quality: 101.0,
                ..base
            },
            Config {
                quality: -1.0,
                ..base
            },
            Config {
                quality: f32::NAN,
                ..base
            },
            Config {
                quality: f32::INFINITY,
                ..base
            },
            Config { method: 7, ..base },
        ] {
            assert!(
                matches!(
                    Codec::new(&config),
                    Err(Error::Encode(EncodingError::InvalidConfiguration))
                ),
                "quality {} method {}",
                config.quality,
                config.method
            );
        }
    }

    /// 画素ごとに値の違うRGBAのキャンバス
    fn ramp_rgba(width: u32, height: u32) -> Vec<u8> {
        (0..height)
            .flat_map(|y| {
                (0..width).flat_map(move |x| [(x * 7) as u8, (y * 11) as u8, (x + y) as u8, 0xFF])
            })
            .collect()
    }

    /// `image-webp` でデコードした生RGBA
    fn decode(bytes: &[u8], width: u32, height: u32) -> Vec<u8> {
        let mut decoder = image_webp::WebPDecoder::new(std::io::Cursor::new(bytes))
            .expect("image-webp が読めない");
        assert_eq!(decoder.dimensions(), (width, height));
        let mut buffer = vec![0u8; decoder.output_buffer_size().expect("出力の大きさ")];
        decoder
            .read_image(&mut buffer)
            .expect("image-webp のデコード");
        if decoder.has_alpha() {
            buffer
        } else {
            buffer
                .chunks_exact(3)
                .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 0xFF])
                .collect()
        }
    }

    #[test]
    fn a_sub_rect_carries_the_pixels_inside_it() {
        let layout = Layout::new(32, 24, ColorType::Rgba8).unwrap();
        let data = ramp_rgba(layout.width, layout.height);
        let rect = Rect {
            x: 5,
            y: 6,
            width: 9,
            height: 7,
        };

        let mut codec = Codec::new(&Config {
            color_type: ColorType::Rgba8,
            lossless: true,
            quality: 75.0,
            method: 4,
            num_plays: 0,
        })
        .unwrap();
        let frame = codec.encode(&data, &layout, rect).unwrap();

        let mut expected = Vec::new();
        crop(
            &data,
            rect,
            layout.stride,
            layout.bytes_per_pixel,
            layout.bytes_per_pixel,
            &mut expected,
        );
        assert_eq!(
            decode(frame.still(), rect.width, rect.height),
            expected,
            "{rect:?}"
        );
    }
}
