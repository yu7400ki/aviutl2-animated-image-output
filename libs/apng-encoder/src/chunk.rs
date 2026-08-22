//! PNGチャンクの書き出し

use crate::error::Error;
use std::io::Write;

/// PNGシグネチャ
pub(crate) const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// チャンクのデータ長の上限
const MAX_LEN: usize = 0x7FFF_FFFF;

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
