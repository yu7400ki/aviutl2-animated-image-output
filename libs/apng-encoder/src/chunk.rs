//! PNGチャンクの書き出しと、APNGのフレームを並べる連番の管理

use crate::delay::FrameDelayExt;
use crate::error::Error;
use anim_core::{FrameDelay, Rect};
use std::io::Write;

/// PNGシグネチャ
pub(crate) const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// チャンクのデータ長の上限
const MAX_LEN: usize = 0x7FFF_FFFF;

/// 前のフレームを消さずに次のフレームを描画する
pub(crate) const DISPOSE_OP_NONE: u8 = 0;
/// フレームの領域を描画前の内容へ戻してから次のフレームを描画する
pub(crate) const DISPOSE_OP_PREVIOUS: u8 = 2;
/// フレームの内容で領域を置き換える
pub(crate) const BLEND_OP_SOURCE: u8 = 0;
/// フレームの内容をキャンバスへアルファ合成する
pub(crate) const BLEND_OP_OVER: u8 = 1;

/// チャンクを順に並べる書き出し先
///
/// fcTLとfdATが共有する連番と、書き出したフレーム数を保つ。
pub(crate) struct ChunkWriter<W: Write> {
    writer: W,
    /// fcTLとfdATで共有する連番
    sequence: u32,
    /// 実際に書き出したフレーム数
    emitted: u32,
}

impl<W: Write> ChunkWriter<W> {
    pub(crate) fn new(writer: W) -> Self {
        ChunkWriter {
            writer,
            sequence: 0,
            emitted: 0,
        }
    }

    /// PNGシグネチャを書き出す
    pub(crate) fn write_signature(&mut self) -> Result<(), Error> {
        self.writer.write_all(&SIGNATURE)?;
        Ok(())
    }

    /// 連番を持たないチャンクを1つ書き出す
    pub(crate) fn write(&mut self, chunk_type: [u8; 4], data: &[u8]) -> Result<(), Error> {
        write(&mut self.writer, chunk_type, data)
    }

    /// fcTLに続けて、圧縮した本体をIDATかfdATで書き出す
    pub(crate) fn write_frame(
        &mut self,
        rect: Rect,
        delay: FrameDelay,
        dispose: u8,
        blend: u8,
        body: &[u8],
    ) -> Result<(), Error> {
        self.write_fctl(rect, delay, dispose, blend)?;

        // 先頭フレームはIDATに入り、以降はfdATに入る
        if self.emitted == 0 {
            write(&mut self.writer, *b"IDAT", body)?;
        } else {
            write_parts(
                &mut self.writer,
                *b"fdAT",
                &[&self.sequence.to_be_bytes(), body],
            )?;
            self.sequence += 1;
        }

        self.emitted += 1;
        Ok(())
    }

    fn write_fctl(
        &mut self,
        rect: Rect,
        delay: FrameDelay,
        dispose: u8,
        blend: u8,
    ) -> Result<(), Error> {
        let (delay_num, delay_den) = delay.to_parts();

        let mut fctl = [0u8; 26];
        fctl[0..4].copy_from_slice(&self.sequence.to_be_bytes());
        fctl[4..8].copy_from_slice(&rect.width.to_be_bytes());
        fctl[8..12].copy_from_slice(&rect.height.to_be_bytes());
        fctl[12..16].copy_from_slice(&rect.x.to_be_bytes());
        fctl[16..20].copy_from_slice(&rect.y.to_be_bytes());
        fctl[20..22].copy_from_slice(&delay_num.to_be_bytes());
        fctl[22..24].copy_from_slice(&delay_den.to_be_bytes());
        fctl[24] = dispose;
        fctl[25] = blend;
        write(&mut self.writer, *b"fcTL", &fctl)?;

        self.sequence += 1;
        Ok(())
    }

    /// 書き出し先を返す
    pub(crate) fn into_inner(self) -> W {
        self.writer
    }
}

/// チャンクを1つ書き出す (長さ u32BE + 型 + データ + CRC32)
pub(crate) fn write<W: Write>(w: &mut W, chunk_type: [u8; 4], data: &[u8]) -> Result<(), Error> {
    write_parts(w, chunk_type, &[data])
}

/// データ部を複数の断片に分けてチャンクを1つ書き出す
///
/// CRCと長さは連結した全断片に対して計算する。
pub(crate) fn write_parts<W: Write>(
    w: &mut W,
    chunk_type: [u8; 4],
    parts: &[&[u8]],
) -> Result<(), Error> {
    let len: usize = parts.iter().map(|p| p.len()).sum();
    check_len(len)?;

    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&chunk_type);
    for part in parts {
        hasher.update(part);
    }

    w.write_all(&(len as u32).to_be_bytes())?;
    w.write_all(&chunk_type)?;
    for part in parts {
        w.write_all(part)?;
    }
    w.write_all(&hasher.finalize().to_be_bytes())?;

    Ok(())
}

/// データ長がチャンクに収まるか検証する
fn check_len(len: usize) -> Result<(), Error> {
    if len > MAX_LEN {
        return Err(Error::ChunkTooLarge { len });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iend_matches_known_vector() {
        let mut out = Vec::new();
        write(&mut out, *b"IEND", &[]).unwrap();
        assert_eq!(
            out,
            [
                0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82
            ]
        );
    }

    /// 1x1 RGBA8 のIHDR: 長さ13、CRC 0x1F15C489
    #[test]
    fn ihdr_matches_known_vector() {
        let data = [0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0];
        let mut out = Vec::new();
        write(&mut out, *b"IHDR", &data).unwrap();

        assert_eq!(&out[0..4], &[0x00, 0x00, 0x00, 0x0D]);
        assert_eq!(&out[4..8], b"IHDR");
        assert_eq!(&out[8..21], &data);
        assert_eq!(&out[21..25], &[0x1F, 0x15, 0xC4, 0x89]);
    }

    #[test]
    fn split_parts_produce_the_same_bytes_as_one_part() {
        let data = [1, 2, 3, 4, 5, 6, 7, 8];

        let mut whole = Vec::new();
        write(&mut whole, *b"fdAT", &data).unwrap();

        let mut split = Vec::new();
        write_parts(&mut split, *b"fdAT", &[&data[..4], &data[4..]]).unwrap();

        assert_eq!(whole, split);
    }

    /// 上限を超える長さはu32へ切り捨てず、エラーとして返す
    #[test]
    fn oversized_data_is_rejected() {
        assert!(check_len(MAX_LEN).is_ok());
        assert!(matches!(
            check_len(MAX_LEN + 1),
            Err(Error::ChunkTooLarge { len }) if len == MAX_LEN + 1
        ));
    }
}
