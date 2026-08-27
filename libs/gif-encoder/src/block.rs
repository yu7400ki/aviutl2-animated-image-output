//! ブロックの書き出し

use anim_core::MAX_COLORS;
use std::io::{self, Write};

/// データストリームの先頭に置く識別子とバージョン
///
/// グラフィック制御拡張とアプリケーション拡張が89aを要求する。
const HEADER: &[u8; 6] = b"GIF89a";
/// 拡張ブロックの開始を表す値
const EXTENSION_INTRODUCER: u8 = 0x21;
/// グラフィック制御拡張のラベル
const GRAPHIC_CONTROL_LABEL: u8 = 0xF9;
/// アプリケーション拡張のラベル
const APPLICATION_LABEL: u8 = 0xFF;
/// 画像記述子の開始を表す値
const IMAGE_SEPARATOR: u8 = 0x2C;
/// データストリームの終端を表す値
const TRAILER: u8 = 0x3B;
/// サブブロックの列を閉じるブロック終端
const BLOCK_TERMINATOR: u8 = 0x00;
/// 原色1つあたり8bitを表す色分解能
const COLOR_RESOLUTION: u8 = 7;
/// グローバルカラーテーブルのバイト数
const GLOBAL_TABLE_BYTES: usize = MAX_COLORS * 3;
/// [`MAX_COLORS`] エントリを表すカラーテーブルの大きさの欄
const GLOBAL_TABLE_SIZE: u8 = 7;
/// グローバルカラーテーブルが始まるデータストリーム上の位置
///
/// ヘッダと論理画面記述子の直後。[`global_color_table`] はここへ書き戻す。
pub(crate) const GLOBAL_TABLE_OFFSET: u64 = 13;
/// 書き戻す前のグローバルカラーテーブルを埋める色
///
/// 書き戻さないまま終わったファイルは、この色が並んだまま残る。
const PLACEHOLDER: [u8; 3] = [0xFF, 0x00, 0xFF];
/// ループ回数を持つアプリケーション拡張の識別子と認証コード
const NETSCAPE: &[u8; 11] = b"NETSCAPE2.0";
/// ループ回数を運ぶサブブロックの先頭に置く値
const NETSCAPE_LOOP: u8 = 1;
/// 廃棄方法「表示した画像をそのまま残す」
pub(crate) const DISPOSAL_DO_NOT_DISPOSE: u8 = 1;
/// 廃棄方法「矩形を背景で塗り直す」
pub(crate) const DISPOSAL_RESTORE_TO_BACKGROUND: u8 = 2;
/// 廃棄方法「描く直前のキャンバスへ戻す」
pub(crate) const DISPOSAL_RESTORE_TO_PREVIOUS: u8 = 3;

/// ヘッダを書く
pub(crate) fn header<W: Write>(writer: &mut W) -> io::Result<()> {
    writer.write_all(HEADER)
}

/// 論理画面記述子を書く
///
/// グローバルカラーテーブルを持つ宣言を立てる。その大きさの欄は
/// [`global_color_table`] が書く [`MAX_COLORS`] エントリを表す。背景色の添字と
/// アスペクト比、Sortフラグは0で、色分解能は8bitを表す7で固定する。
pub(crate) fn logical_screen_descriptor<W: Write>(
    writer: &mut W,
    width: u16,
    height: u16,
) -> io::Result<()> {
    let packed = 0x80 | COLOR_RESOLUTION << 4 | GLOBAL_TABLE_SIZE;
    writer.write_all(&width.to_le_bytes())?;
    writer.write_all(&height.to_le_bytes())?;
    writer.write_all(&[packed, 0, 0])
}

/// 色の決まっていないグローバルカラーテーブルを書く
///
/// 色が決まった時点で [`GLOBAL_TABLE_OFFSET`] へ戻り、[`global_color_table`] が
/// この上へ書き直す。
pub(crate) fn global_table_placeholder<W: Write>(writer: &mut W) -> io::Result<()> {
    let mut bytes = Vec::with_capacity(GLOBAL_TABLE_BYTES);
    for _ in 0..MAX_COLORS {
        bytes.extend_from_slice(&PLACEHOLDER);
    }
    writer.write_all(&bytes)
}

/// グローバルカラーテーブルを書く
///
/// 常に [`MAX_COLORS`] エントリぶんを書き、`bytes` に足りない分は黒で埋める。
pub(crate) fn global_color_table<W: Write>(writer: &mut W, bytes: &[u8]) -> io::Result<()> {
    const PADDING: [u8; GLOBAL_TABLE_BYTES] = [0; GLOBAL_TABLE_BYTES];
    writer.write_all(bytes)?;
    writer.write_all(&PADDING[bytes.len()..])
}

/// ローカルカラーテーブルを書く
pub(crate) fn color_table<W: Write>(writer: &mut W, bytes: &[u8]) -> io::Result<()> {
    writer.write_all(bytes)
}

/// ループ回数のアプリケーション拡張を書く
///
/// `num_plays` は再生回数で、0は無限を表す。1回だけ再生するときは拡張を書かない。
/// ループ数の欄はu16なので、`num_plays - 1` が超える場合は65535へ飽和させる。
pub(crate) fn netscape<W: Write>(writer: &mut W, num_plays: u32) -> io::Result<()> {
    if num_plays == 1 {
        return Ok(());
    }

    let loops = num_plays.saturating_sub(1).min(u32::from(u16::MAX)) as u16;
    writer.write_all(&[
        EXTENSION_INTRODUCER,
        APPLICATION_LABEL,
        NETSCAPE.len() as u8,
    ])?;
    writer.write_all(NETSCAPE)?;
    writer.write_all(&[3, NETSCAPE_LOOP])?;
    writer.write_all(&loops.to_le_bytes())?;
    writer.write_all(&[BLOCK_TERMINATOR])
}

/// グラフィック制御拡張を書く
///
/// `delay` は1/100秒。`transparent` を渡すとその添字が透過になる。
/// ユーザー入力の待機は行わない。
pub(crate) fn graphic_control<W: Write>(
    writer: &mut W,
    disposal: u8,
    delay: u16,
    transparent: Option<u8>,
) -> io::Result<()> {
    let packed = disposal << 2 | u8::from(transparent.is_some());
    writer.write_all(&[EXTENSION_INTRODUCER, GRAPHIC_CONTROL_LABEL, 4, packed])?;
    writer.write_all(&delay.to_le_bytes())?;
    writer.write_all(&[transparent.unwrap_or(0), BLOCK_TERMINATOR])
}

/// 画像記述子を書く
///
/// `table_size` を渡すとローカルカラーテーブルを持つ宣言になり、直後に
/// [`color_table`] でその中身を書く。インターレースもSortもしない。
pub(crate) fn image_descriptor<W: Write>(
    writer: &mut W,
    left: u16,
    top: u16,
    width: u16,
    height: u16,
    table_size: Option<u8>,
) -> io::Result<()> {
    let packed = table_size.map_or(0, |size| 0x80 | size);
    writer.write_all(&[IMAGE_SEPARATOR])?;
    writer.write_all(&left.to_le_bytes())?;
    writer.write_all(&top.to_le_bytes())?;
    writer.write_all(&width.to_le_bytes())?;
    writer.write_all(&height.to_le_bytes())?;
    writer.write_all(&[packed])
}

/// 圧縮済みの画像データを書く
///
/// `body` は [`crate::lzw::compress`] が書いたサブブロックの列とブロック終端。
pub(crate) fn image_body<W: Write>(
    writer: &mut W,
    min_code_size: u8,
    body: &[u8],
) -> io::Result<()> {
    writer.write_all(&[min_code_size])?;
    writer.write_all(body)
}

/// 終端を書く
pub(crate) fn trailer<W: Write>(writer: &mut W) -> io::Result<()> {
    writer.write_all(&[TRAILER])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lzw;

    fn written(write: impl FnOnce(&mut Vec<u8>) -> io::Result<()>) -> Vec<u8> {
        let mut bytes = Vec::new();
        write(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn the_header_declares_version_89a() {
        assert_eq!(written(header), b"GIF89a");
    }

    /// 論理画面記述子のカラーテーブルの大きさの欄は256エントリで固定
    #[test]
    fn the_logical_screen_descriptor_carries_the_fixed_fields() {
        let bytes = written(|out| logical_screen_descriptor(out, 0x0102, 0x0304));
        assert_eq!(bytes, [0x02, 0x01, 0x04, 0x03, 0b1111_0111, 0x00, 0x00]);
    }

    /// グローバルカラーテーブルは色数によらず768バイトで、余りは黒
    #[test]
    fn the_global_color_table_is_always_padded_to_256_entries() {
        for colors in [1usize, 2, 5, 128, 256] {
            let table: Vec<u8> = (0..colors * 3).map(|at| (at + 1) as u8).collect();
            let bytes = written(|out| global_color_table(out, &table));
            assert_eq!(bytes.len(), 768, "{colors}色");
            assert_eq!(bytes[..table.len()], table, "{colors}色");
            assert!(
                bytes[table.len()..].iter().all(|&byte| byte == 0),
                "{colors}色の埋め草が黒でない"
            );
        }
    }

    /// 色の決まっていないグローバルカラーテーブルは、確保した位置と大きさを持つ
    #[test]
    fn the_placeholder_fills_every_entry_with_magenta() {
        /// 書き戻す前のエントリの色
        const MAGENTA: [u8; 3] = [0xFF, 0x00, 0xFF];

        let mut bytes = Vec::new();
        header(&mut bytes).unwrap();
        logical_screen_descriptor(&mut bytes, 1, 1).unwrap();
        assert_eq!(bytes.len() as u64, GLOBAL_TABLE_OFFSET);

        global_table_placeholder(&mut bytes).unwrap();
        let table = &bytes[GLOBAL_TABLE_OFFSET as usize..];
        assert_eq!(table.len(), 768);
        assert!(
            table.chunks_exact(3).all(|entry| entry == MAGENTA),
            "確保しただけのエントリがマゼンタでない"
        );
    }

    /// 設定値の再生回数からループ数の欄が決まる
    #[test]
    fn the_loop_count_follows_the_number_of_plays() {
        assert!(
            written(|out| netscape(out, 1)).is_empty(),
            "1回再生で拡張が出た"
        );

        for (num_plays, loops) in [(0, 0u16), (2, 1), (3, 2), (65536, 65535), (u32::MAX, 65535)] {
            let bytes = written(|out| netscape(out, num_plays));
            assert_eq!(
                bytes,
                [
                    &[0x21, 0xFF, 0x0B][..],
                    b"NETSCAPE2.0",
                    &[0x03, 0x01],
                    &loops.to_le_bytes(),
                    &[0x00],
                ]
                .concat(),
                "再生回数 {num_plays}"
            );
        }
    }

    #[test]
    fn the_graphic_control_extension_carries_the_transparent_index() {
        let bytes = written(|out| graphic_control(out, DISPOSAL_DO_NOT_DISPOSE, 0x0203, Some(9)));
        assert_eq!(bytes, [0x21, 0xF9, 0x04, 0b0000_0101, 0x03, 0x02, 9, 0x00]);

        let bytes = written(|out| graphic_control(out, DISPOSAL_DO_NOT_DISPOSE, 2, None));
        assert_eq!(bytes, [0x21, 0xF9, 0x04, 0b0000_0100, 0x02, 0x00, 0, 0x00]);
    }

    #[test]
    fn the_image_descriptor_uses_neither_a_local_table_nor_interlace() {
        let bytes = written(|out| image_descriptor(out, 1, 2, 0x0304, 0x0506, None));
        assert_eq!(
            bytes,
            [0x2C, 0x01, 0x00, 0x02, 0x00, 0x04, 0x03, 0x06, 0x05, 0x00]
        );
    }

    /// ローカルカラーテーブルを持つフレームはその旗と大きさの欄を立てる
    #[test]
    fn a_local_table_sets_the_flag_and_the_size_field() {
        let bytes = written(|out| image_descriptor(out, 0, 0, 1, 1, Some(7)));
        assert_eq!(bytes[9], 0b1000_0111);

        let bytes = written(|out| image_descriptor(out, 0, 0, 1, 1, Some(0)));
        assert_eq!(bytes[9], 0b1000_0000);
    }

    #[test]
    fn the_image_data_opens_with_the_minimum_code_size() {
        let mut body = Vec::new();
        lzw::compress(&mut body, &[0], 2).unwrap();

        let bytes = written(|out| image_body(out, 2, &body));
        assert_eq!(bytes[0], 2);
        assert_eq!(&bytes[1..], [0x02, 0b0100_0100, 0b0000_0001, 0x00]);
    }

    #[test]
    fn the_trailer_closes_the_stream() {
        assert_eq!(written(trailer), [0x3B]);
    }
}
