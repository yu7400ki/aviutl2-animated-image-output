//! RIFF/VP8X/ANIM/ANMFの書き出しと後埋め

use crate::error::Error;
use anim_core::Rect;
use std::io::{Seek, SeekFrom, Write};

/// RIFFのチャンクヘッダ (FourCCとサイズ) のバイト数
const CHUNK_HEADER: u64 = 8;

/// RIFFのサイズ欄の位置
const RIFF_SIZE_OFFSET: u64 = 4;

/// VP8Xのフラグの位置
const VP8X_FLAGS_OFFSET: u64 = 20;

/// VP8Xのペイロードのバイト数
const VP8X_PAYLOAD: u32 = 10;

/// ANIMのペイロードのバイト数
const ANIM_PAYLOAD: u32 = 6;

/// ANMFがフレームデータの前に置く矩形・表示時間・合成の指定のバイト数
const ANMF_HEADER: u64 = 16;

/// アニメーションであることを示すVP8Xのフラグ
const FLAG_ANIMATION: u8 = 0x02;

/// いずれかのフレームがαを持つことを示すVP8Xのフラグ
const FLAG_ALPHA: u8 = 0x10;

/// キャンバスの空きと廃棄の跡を埋める色 (BGRA)
const BACKGROUND_COLOR: [u8; 4] = [0, 0, 0, 0];

/// ANIMのループ数欄に収まる上限
const MAX_LOOP_COUNT: u32 = u16::MAX as u32;

/// ANMFへ載せる1フレーム
pub(crate) struct Frame<'a> {
    /// キャンバス上の矩形。`x` と `y` は偶数
    pub(crate) rect: Rect,
    /// 表示時間 (ms)
    pub(crate) duration: u32,
    /// 透過画素を下のキャンバスへ重ねるか
    pub(crate) blend: bool,
    /// 表示した後に矩形を背景色で抜くか
    pub(crate) dispose: bool,
    /// αを別に持つ形式のときの `ALPH` チャンク
    pub(crate) alpha: Option<&'a [u8]>,
    /// `VP8 ` または `VP8L` チャンク
    pub(crate) image: &'a [u8],
}

/// アニメーションのRIFFコンテナ
///
/// [`Riff::new`] がRIFFヘッダ・VP8X・ANIMを書き、[`Riff::write_frame`] が
/// ANMFを投入順に流し、[`Riff::finish`] がRIFFのサイズとALPHAフラグを
/// 書き戻す。サイズ欄は書き戻すまで0のまま残る。
pub(crate) struct Riff<W: Write + Seek> {
    writer: W,
    /// 書き出したバイト数
    written: u64,
}

impl<W: Write + Seek> Riff<W> {
    /// `width` x `height` のキャンバスを `num_plays` 回再生するコンテナを開く
    ///
    /// `writer` はストリームの先頭を指していること。後埋めは絶対位置へ seek する。
    ///
    /// # Errors
    /// 書き出しに失敗したとき [`Error::Io`]。
    pub(crate) fn new(
        mut writer: W,
        width: u32,
        height: u32,
        num_plays: u32,
    ) -> Result<Self, Error> {
        let loop_count = num_plays.min(MAX_LOOP_COUNT) as u16;

        let mut header = Vec::new();
        header.extend_from_slice(b"RIFF");
        header.extend_from_slice(&0u32.to_le_bytes());
        header.extend_from_slice(b"WEBP");
        header.extend_from_slice(b"VP8X");
        header.extend_from_slice(&VP8X_PAYLOAD.to_le_bytes());
        header.push(FLAG_ANIMATION);
        header.extend_from_slice(&[0; 3]);
        header.extend_from_slice(&u24(width - 1));
        header.extend_from_slice(&u24(height - 1));
        header.extend_from_slice(b"ANIM");
        header.extend_from_slice(&ANIM_PAYLOAD.to_le_bytes());
        header.extend_from_slice(&BACKGROUND_COLOR);
        header.extend_from_slice(&loop_count.to_le_bytes());

        writer.write_all(&header)?;
        Ok(Riff {
            writer,
            written: header.len() as u64,
        })
    }

    /// ANMFチャンクを1つ書く
    ///
    /// # Errors
    /// 累計がRIFFのサイズ欄に収まらなくなるとき [`Error::FileTooLarge`]。
    /// 書き出しに失敗したとき [`Error::Io`]。
    pub(crate) fn write_frame(&mut self, frame: &Frame<'_>) -> Result<(), Error> {
        debug_assert_eq!(frame.rect.x % 2, 0, "矩形のxは偶数のみ格納できる");
        debug_assert_eq!(frame.rect.y % 2, 0, "矩形のyは偶数のみ格納できる");

        let payload = frame.alpha.map_or(0, <[u8]>::len) + frame.image.len();
        debug_assert_eq!(payload % 2, 0, "ペイロードは詰めたチャンクを並べたもの");
        let size = ANMF_HEADER + payload as u64;
        self.reserve(CHUNK_HEADER + size)?;

        let mut header = Vec::new();
        header.extend_from_slice(b"ANMF");
        header.extend_from_slice(&(size as u32).to_le_bytes());
        header.extend_from_slice(&u24(frame.rect.x / 2));
        header.extend_from_slice(&u24(frame.rect.y / 2));
        header.extend_from_slice(&u24(frame.rect.width - 1));
        header.extend_from_slice(&u24(frame.rect.height - 1));
        header.extend_from_slice(&u24(frame.duration));
        header.push((u8::from(!frame.blend) << 1) | u8::from(frame.dispose));

        self.writer.write_all(&header)?;
        if let Some(alpha) = frame.alpha {
            self.writer.write_all(alpha)?;
        }
        self.writer.write_all(frame.image)?;
        Ok(())
    }

    /// RIFFのサイズとVP8XのALPHAフラグを書き戻してコンテナを閉じる
    ///
    /// `alpha` は書いたフレームのいずれかがαを持つかどうか。
    ///
    /// # Errors
    /// 書き出しに失敗したとき [`Error::Io`]。
    pub(crate) fn finish(mut self, alpha: bool) -> Result<W, Error> {
        let resume = self.writer.stream_position()?;
        let size = (self.written - CHUNK_HEADER) as u32;

        if alpha {
            self.writer.seek(SeekFrom::Start(VP8X_FLAGS_OFFSET))?;
            self.writer.write_all(&[FLAG_ANIMATION | FLAG_ALPHA])?;
        }
        self.writer.seek(SeekFrom::Start(RIFF_SIZE_OFFSET))?;
        self.writer.write_all(&size.to_le_bytes())?;

        self.writer.seek(SeekFrom::Start(resume))?;
        self.writer.flush()?;
        Ok(self.writer)
    }

    /// `len` バイトの書き出しを累計へ加える
    ///
    /// # Errors
    /// 累計がRIFFのサイズ欄に収まらなくなるとき [`Error::FileTooLarge`]。
    fn reserve(&mut self, len: u64) -> Result<(), Error> {
        let written = self.written + len;
        if written > u64::from(u32::MAX) {
            return Err(Error::FileTooLarge);
        }
        self.written = written;
        Ok(())
    }
}

/// 24bitのリトルエンディアン
fn u24(value: u32) -> [u8; 3] {
    debug_assert!(value <= 0x00FF_FFFF, "24bitに収まらない値: {value}");
    let bytes = value.to_le_bytes();
    [bytes[0], bytes[1], bytes[2]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// ファイル先頭のRIFFヘッダのバイト数
    const FILE_HEADER: usize = 12;

    /// ヘッダ (RIFF + VP8X + ANIM) のバイト数
    const HEADER: usize = 44;

    /// 全面を覆う矩形
    fn whole(width: u32, height: u32) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        }
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

    /// 位置 `offset` から始まる4バイトをリトルエンディアンで読む
    fn u32_at(bytes: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
    }

    /// 位置 `offset` から始まる3バイトをリトルエンディアンで読む
    fn u24_at(bytes: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], 0])
    }

    /// ヘッダを書いたコンテナ
    fn opened(width: u32, height: u32, num_plays: u32) -> Riff<Cursor<Vec<u8>>> {
        Riff::new(Cursor::new(Vec::new()), width, height, num_plays).unwrap()
    }

    #[test]
    fn the_header_declares_an_animated_canvas() {
        let riff = opened(300, 200, 7);
        let bytes = riff.writer.into_inner();

        assert_eq!(bytes.len(), HEADER);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WEBP");
        assert_eq!(&bytes[12..16], b"VP8X");
        assert_eq!(u32_at(&bytes, 16), VP8X_PAYLOAD);
        assert_eq!(bytes[VP8X_FLAGS_OFFSET as usize], FLAG_ANIMATION);
        assert_eq!(&bytes[21..24], [0, 0, 0]);
        assert_eq!(u24_at(&bytes, 24), 299);
        assert_eq!(u24_at(&bytes, 27), 199);
        assert_eq!(&bytes[30..34], b"ANIM");
        assert_eq!(u32_at(&bytes, 34), ANIM_PAYLOAD);
        assert_eq!(&bytes[38..42], BACKGROUND_COLOR);
        assert_eq!(u16::from_le_bytes([bytes[42], bytes[43]]), 7);
    }

    #[test]
    fn the_riff_size_stays_zero_until_the_container_is_closed() {
        let mut riff = opened(4, 4, 0);
        riff.write_frame(&Frame {
            rect: whole(4, 4),
            duration: 40,
            blend: false,
            dispose: false,
            alpha: None,
            image: &chunk(b"VP8L", &[0x2F; 6]),
        })
        .unwrap();

        let unfinished = riff.writer.get_ref().clone();
        assert!(unfinished.len() > FILE_HEADER);
        assert_eq!(u32_at(&unfinished, RIFF_SIZE_OFFSET as usize), 0);
    }

    #[test]
    fn closing_the_container_fills_in_the_riff_size() {
        let mut riff = opened(4, 4, 0);
        riff.write_frame(&Frame {
            rect: whole(4, 4),
            duration: 40,
            blend: false,
            dispose: false,
            alpha: None,
            image: &chunk(b"VP8L", &[0x2F; 6]),
        })
        .unwrap();

        let bytes = riff.finish(false).unwrap().into_inner();
        assert_eq!(
            u32_at(&bytes, RIFF_SIZE_OFFSET as usize) as usize,
            bytes.len() - CHUNK_HEADER as usize
        );
    }

    /// ALPHAフラグは後埋めでしか立たない
    #[test]
    fn the_alpha_flag_is_raised_only_when_a_frame_carries_alpha() {
        for alpha in [false, true] {
            let riff = opened(4, 4, 0);
            let bytes = riff.finish(alpha).unwrap().into_inner();
            let expected = if alpha {
                FLAG_ANIMATION | FLAG_ALPHA
            } else {
                FLAG_ANIMATION
            };
            assert_eq!(bytes[VP8X_FLAGS_OFFSET as usize], expected, "{alpha}");
        }
    }

    /// 後埋めの後もストリームの末尾に戻る
    #[test]
    fn closing_the_container_leaves_the_stream_at_its_end() {
        let riff = opened(4, 4, 0);
        let mut writer = riff.finish(true).unwrap();
        assert_eq!(writer.stream_position().unwrap(), HEADER as u64);
    }

    /// 書いた位置を順に控えるカーソル
    struct Trace {
        cursor: Cursor<Vec<u8>>,
        writes: Vec<u64>,
    }

    impl Write for Trace {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.writes.push(self.cursor.position());
            self.cursor.write(buf)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.cursor.flush()
        }
    }

    impl Seek for Trace {
        fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
            self.cursor.seek(pos)
        }
    }

    /// RIFFのサイズ欄を最後に書く
    ///
    /// 途中で落ちたファイルのサイズ欄は0のままになり、ALPHAフラグだけが
    /// 欠けた一見完成しているファイルは残らない。
    #[test]
    fn the_riff_size_is_the_last_thing_written() {
        let writer = Trace {
            cursor: Cursor::new(Vec::new()),
            writes: Vec::new(),
        };
        let mut riff = Riff::new(writer, 4, 4, 0).unwrap();
        riff.write_frame(&Frame {
            rect: whole(4, 4),
            duration: 40,
            blend: false,
            dispose: false,
            alpha: None,
            image: &chunk(b"VP8L", &[0x2F; 6]),
        })
        .unwrap();

        let writes = riff.finish(true).unwrap().writes;
        assert_eq!(writes.last(), Some(&RIFF_SIZE_OFFSET));
        assert!(writes.contains(&VP8X_FLAGS_OFFSET));
    }

    #[test]
    fn a_loop_count_beyond_the_field_width_saturates() {
        for (num_plays, expected) in [
            (0, 0),
            (1, 1),
            (65535, 65535),
            (65536, 65535),
            (u32::MAX, 65535),
        ] {
            let bytes = opened(4, 4, num_plays).writer.into_inner();
            assert_eq!(
                u16::from_le_bytes([bytes[42], bytes[43]]),
                expected,
                "{num_plays}"
            );
        }
    }

    #[test]
    fn a_frame_carries_its_rect_duration_and_flags() {
        let mut riff = opened(64, 48, 0);
        let image = chunk(b"VP8L", &[0x2F; 6]);
        riff.write_frame(&Frame {
            rect: Rect {
                x: 10,
                y: 6,
                width: 20,
                height: 12,
            },
            duration: 0x00AB_CDEF,
            blend: true,
            dispose: true,
            alpha: None,
            image: &image,
        })
        .unwrap();

        let bytes = riff.writer.into_inner();
        let anmf = &bytes[HEADER..];
        assert_eq!(&anmf[..4], b"ANMF");
        assert_eq!(u32_at(anmf, 4) as usize, ANMF_HEADER as usize + image.len());
        assert_eq!(u24_at(anmf, 8), 5);
        assert_eq!(u24_at(anmf, 11), 3);
        assert_eq!(u24_at(anmf, 14), 19);
        assert_eq!(u24_at(anmf, 17), 11);
        assert_eq!(u24_at(anmf, 20), 0x00AB_CDEF);
        assert_eq!(anmf[23], 0x01);
        assert_eq!(&anmf[24..], image);
    }

    /// ブレンドを切ったフレームはBのビットが立ち、廃棄しないフレームはDが倒れる
    #[test]
    fn the_blend_and_dispose_bits_follow_the_frame() {
        for (blend, dispose, expected) in [
            (true, true, 0x01),
            (true, false, 0x00),
            (false, true, 0x03),
            (false, false, 0x02),
        ] {
            let mut riff = opened(4, 4, 0);
            riff.write_frame(&Frame {
                rect: whole(4, 4),
                duration: 40,
                blend,
                dispose,
                alpha: None,
                image: &chunk(b"VP8L", &[0x2F; 6]),
            })
            .unwrap();

            let bytes = riff.writer.into_inner();
            assert_eq!(
                bytes[HEADER + 23],
                expected,
                "blend {blend} dispose {dispose}"
            );
        }
    }

    /// ALPHは画像チャンクの前に、詰めを含めてそのまま載る
    #[test]
    fn an_alpha_chunk_precedes_the_image_chunk_with_its_padding() {
        let alpha = chunk(b"ALPH", &[0x11, 0x22, 0x33]);
        let image = chunk(b"VP8 ", &[0x55; 8]);
        assert_eq!(alpha.len() % 2, 0);

        let mut riff = opened(4, 4, 0);
        riff.write_frame(&Frame {
            rect: whole(4, 4),
            duration: 40,
            blend: false,
            dispose: false,
            alpha: Some(&alpha),
            image: &image,
        })
        .unwrap();

        let bytes = riff.writer.into_inner();
        let anmf = &bytes[HEADER..];
        let size = u32_at(anmf, 4) as usize;
        assert_eq!(size, ANMF_HEADER as usize + alpha.len() + image.len());
        // 詰めたチャンクを並べたペイロードは偶数長で、ANMF自身は詰めを要さない
        assert_eq!(size % 2, 0);
        assert_eq!(anmf.len(), CHUNK_HEADER as usize + size);
        assert_eq!(&anmf[24..24 + alpha.len()], alpha);
        assert_eq!(&anmf[24 + alpha.len()..], image);
    }

    /// 累計がサイズ欄に収まらなくなるフレームは、書き出す前に弾く
    #[test]
    fn a_frame_that_would_overflow_the_riff_size_is_refused() {
        let image = chunk(b"VP8L", &[0x2F; 6]);
        let frame = Frame {
            rect: whole(4, 4),
            duration: 40,
            blend: false,
            dispose: false,
            alpha: None,
            image: &image,
        };
        let chunk_len = CHUNK_HEADER + ANMF_HEADER + image.len() as u64;

        let mut riff = opened(4, 4, 0);
        riff.written = u64::from(u32::MAX) - chunk_len;
        assert!(riff.write_frame(&frame).is_ok());
        assert_eq!(riff.written, u64::from(u32::MAX));

        let written = riff.written;
        let position = riff.writer.stream_position().unwrap();
        assert!(matches!(riff.write_frame(&frame), Err(Error::FileTooLarge)));
        assert_eq!(riff.written, written);
        assert_eq!(riff.writer.stream_position().unwrap(), position);
    }
}
