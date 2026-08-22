//! zlibストリームの生成

use flate2::Compression;
use flate2::write::ZlibEncoder;
use std::io::Write;

/// deflate圧縮器
pub(crate) struct Compressor {
    level: Compression,
}

impl Compressor {
    /// 圧縮レベル `level` (1..=9) の圧縮器を作る
    pub(crate) fn new(level: u32) -> Self {
        debug_assert!((1..=9).contains(&level));
        Compressor {
            level: Compression::new(level),
        }
    }

    /// `data` を完結したzlibストリームへ圧縮し `out` へ追記する
    ///
    /// 呼び出しごとに圧縮器を作り直すため、ストリームは互いに独立する。
    pub(crate) fn compress_into(&self, data: &[u8], out: &mut Vec<u8>) {
        let mut encoder = ZlibEncoder::new(std::mem::take(out), self.level);
        encoder
            .write_all(data)
            .expect("Vecへの書き込みは失敗しない");
        *out = encoder.finish().expect("Vecへの書き込みは失敗しない");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::ZlibDecoder;
    use std::io::Read;

    fn decompress(stream: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        ZlibDecoder::new(stream).read_to_end(&mut out).unwrap();
        out
    }

    #[test]
    fn roundtrips_through_a_zlib_stream() {
        let data: Vec<u8> = (0..4096u32).map(|i| (i / 16) as u8).collect();
        let mut out = Vec::new();
        Compressor::new(6).compress_into(&data, &mut out);

        assert_eq!(decompress(&out), data);
        assert!(out.len() < data.len());
    }

    #[test]
    fn streams_are_independent_of_each_other() {
        let compressor = Compressor::new(6);
        let first = vec![0xABu8; 1024];
        let second: Vec<u8> = (0..1024u32).map(|i| i as u8).collect();

        let mut out = Vec::new();
        compressor.compress_into(&first, &mut out);
        let boundary = out.len();
        compressor.compress_into(&second, &mut out);

        assert_eq!(decompress(&out[..boundary]), first);
        assert_eq!(decompress(&out[boundary..]), second);
    }

    #[test]
    fn output_is_appended_to_the_buffer() {
        let mut out = vec![0xAA, 0xBB];
        Compressor::new(1).compress_into(&[0u8; 16], &mut out);

        assert_eq!(&out[..2], &[0xAA, 0xBB]);
        assert_eq!(decompress(&out[2..]), [0u8; 16]);
    }
}
