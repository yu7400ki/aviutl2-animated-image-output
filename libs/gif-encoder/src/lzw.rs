//! 可変長符号LZWと255バイトのサブブロック分割

use std::io::{self, Write};

/// 符号長の上限
const MAX_CODE_SIZE: u8 = 12;
/// 符号の総数
const MAX_CODES: usize = 1 << MAX_CODE_SIZE;

/// 辞書の表の添字に使うビット数
const TABLE_BITS: u32 = 13;
/// 開放アドレス法の表の大きさ
///
/// [`MAX_CODES`] より大きく取る。表に必ず空きが残ることが、鍵を探す走査が
/// 一周して戻ってこないことの根拠になる。
const TABLE_LEN: usize = 1 << TABLE_BITS;
const _: () = assert!(TABLE_LEN > MAX_CODES);
/// 表の添字を取り出すマスク
const TABLE_MASK: usize = TABLE_LEN - 1;
/// 値を表全体へ散らす乗数 (2^32を黄金比で割った奇数)
const HASH_MULTIPLIER: u32 = 0x9E37_79B1;

/// 1つのサブブロックが持てるバイト数
const MAX_SUB_BLOCK: usize = 255;
/// サブブロックの列を閉じるブロック終端
const BLOCK_TERMINATOR: u8 = 0x00;

/// 最小符号長の範囲
const MIN_CODE_SIZES: std::ops::RangeInclusive<u8> = 2..=8;

/// `indices` をLZWで圧縮し、サブブロックへ分けて書き出す
///
/// 先頭にClear、末尾にEOIを出し、ブロック終端で閉じる。`min_code_size` は
/// 2..=8 で、`indices` の値はすべて `2^min_code_size` 未満であること。
pub(crate) fn compress<W: Write>(
    writer: &mut W,
    indices: &[u8],
    min_code_size: u8,
) -> io::Result<()> {
    assert!(
        MIN_CODE_SIZES.contains(&min_code_size),
        "最小符号長は 2..=8: {min_code_size}"
    );
    let clear = 1u16 << min_code_size;
    debug_assert!(
        indices.iter().all(|&index| u16::from(index) < clear),
        "カラーテーブルの外を指す添字"
    );

    let mut compressor = Compressor::new(writer, min_code_size);
    compressor.emit(clear)?;

    let mut bytes = indices.iter();
    if let Some(&first) = bytes.next() {
        let mut prefix = u16::from(first);
        for &byte in bytes {
            let key = u32::from(prefix) << 8 | u32::from(byte);
            match compressor.dictionary.probe(key) {
                Ok(code) => prefix = code,
                Err(slot) => {
                    compressor.emit(prefix)?;
                    if usize::from(compressor.next_free) < MAX_CODES {
                        compressor
                            .dictionary
                            .insert(slot, key, compressor.next_free);
                        compressor.next_free += 1;
                    } else {
                        compressor.emit(clear)?;
                        compressor.reset();
                    }
                    prefix = u16::from(byte);
                }
            }
        }
        compressor.emit(prefix)?;
    }

    compressor.emit(clear + 1)?;
    compressor.finish()
}

/// 符号をビット列へ詰め、サブブロックへ流す
struct Compressor<'w, W: Write> {
    blocks: SubBlocks<'w, W>,
    /// LSB先頭で詰めた、まだバイトにならないビット
    bits: u32,
    /// [`Self::bits`] が持つビット数
    bit_count: u32,
    /// 1符号あたりのビット数
    code_size: u8,
    min_code_size: u8,
    /// 次に辞書へ入れる符号
    next_free: u16,
    dictionary: Dictionary,
}

impl<'w, W: Write> Compressor<'w, W> {
    fn new(writer: &'w mut W, min_code_size: u8) -> Self {
        Compressor {
            blocks: SubBlocks::new(writer),
            bits: 0,
            bit_count: 0,
            code_size: min_code_size + 1,
            min_code_size,
            next_free: (1 << min_code_size) + 2,
            dictionary: Dictionary::new(),
        }
    }

    /// 符号を1つ出す
    ///
    /// 出した後、次に入る符号が現在の符号長で表せなくなっていれば符号長を1増やす。
    /// デコーダは符号を1つ読むごとに辞書を1つ伸ばすため、増やす位置がここから
    /// ずれると符号長が食い違う。
    fn emit(&mut self, code: u16) -> io::Result<()> {
        self.bits |= u32::from(code) << self.bit_count;
        self.bit_count += u32::from(self.code_size);
        while self.bit_count >= 8 {
            self.blocks.push(self.bits as u8)?;
            self.bits >>= 8;
            self.bit_count -= 8;
        }

        if usize::from(self.next_free) >= 1 << self.code_size && self.code_size < MAX_CODE_SIZE {
            self.code_size += 1;
        }
        Ok(())
    }

    /// 辞書と符号長を張り直す
    fn reset(&mut self) {
        self.dictionary.clear();
        self.code_size = self.min_code_size + 1;
        self.next_free = (1 << self.min_code_size) + 2;
    }

    /// 端数のビットをバイトへ詰め、サブブロックを閉じる
    fn finish(mut self) -> io::Result<()> {
        if self.bit_count > 0 {
            self.blocks.push(self.bits as u8)?;
        }
        self.blocks.finish()
    }
}

/// 接頭符号と次のバイトの組から符号を引く表
///
/// [`Self::values`] が0の位置は空で、それ以外は符号に1を足した値が入る。
/// 同じ位置の [`Self::keys`] にその組が入る。
struct Dictionary {
    keys: Box<[u32]>,
    values: Box<[u16]>,
}

/// 組が最初に占める表の位置
fn slot_of(key: u32) -> usize {
    (key.wrapping_mul(HASH_MULTIPLIER) >> (u32::BITS - TABLE_BITS)) as usize
}

impl Dictionary {
    fn new() -> Self {
        Dictionary {
            keys: vec![0; TABLE_LEN].into_boxed_slice(),
            values: vec![0; TABLE_LEN].into_boxed_slice(),
        }
    }

    /// 組に対応する符号。表に無ければ、その組を入れる空きの位置
    fn probe(&self, key: u32) -> Result<u16, usize> {
        let mut slot = slot_of(key);
        loop {
            let value = self.values[slot];
            // 表に必ず残る空きに当たれば、その組はどこにも入っていない
            if value == 0 {
                return Err(slot);
            }
            if self.keys[slot] == key {
                return Ok(value - 1);
            }
            slot = (slot + 1) & TABLE_MASK;
        }
    }

    /// [`Self::probe`] が返した空きへ組と符号を入れる
    fn insert(&mut self, slot: usize, key: u32, code: u16) {
        self.keys[slot] = key;
        self.values[slot] = code + 1;
    }

    fn clear(&mut self) {
        self.values.fill(0);
    }
}

/// バイト列を255バイトのサブブロックへ分けて書き出す
struct SubBlocks<'w, W: Write> {
    writer: &'w mut W,
    buffer: [u8; MAX_SUB_BLOCK],
    len: usize,
}

impl<'w, W: Write> SubBlocks<'w, W> {
    fn new(writer: &'w mut W) -> Self {
        SubBlocks {
            writer,
            buffer: [0; MAX_SUB_BLOCK],
            len: 0,
        }
    }

    fn push(&mut self, byte: u8) -> io::Result<()> {
        self.buffer[self.len] = byte;
        self.len += 1;
        if self.len == MAX_SUB_BLOCK {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.write_all(&[self.len as u8])?;
        self.writer.write_all(&self.buffer[..self.len])?;
        self.len = 0;
        Ok(())
    }

    /// 残りを書き、ブロック終端で閉じる
    fn finish(mut self) -> io::Result<()> {
        if self.len > 0 {
            self.flush()?;
        }
        self.writer.write_all(&[BLOCK_TERMINATOR])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::noise;

    /// サブブロックの列を平らなバイト列へ戻す
    ///
    /// 末尾はブロック終端で閉じられていること。
    fn unpack(stream: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut at = 0;
        loop {
            let len = stream[at] as usize;
            assert!(
                at + 1 + len <= stream.len(),
                "サブブロックが途中で切れている"
            );
            if len == 0 {
                assert_eq!(at + 1, stream.len(), "ブロック終端の後にバイトが残っている");
                return out;
            }
            out.extend_from_slice(&stream[at + 1..at + 1 + len]);
            at += 1 + len;
        }
    }

    /// 各サブブロックの長さ
    fn block_lengths(stream: &[u8]) -> Vec<usize> {
        let mut lengths = Vec::new();
        let mut at = 0;
        loop {
            let len = stream[at] as usize;
            if len == 0 {
                return lengths;
            }
            lengths.push(len);
            at += 1 + len;
        }
    }

    /// 展開の結果と、読み取ったClearの回数
    struct Decoded {
        data: Vec<u8>,
        clears: u32,
    }

    /// 仕様の規定どおりに符号を読み、辞書を組み立て直す
    ///
    /// 符号を1つ読むごとに辞書を1つ伸ばし、辞書が現在の符号長を超えたら
    /// 符号長を1増やす。
    fn decompress(stream: &[u8], min_code_size: u8) -> Decoded {
        let bytes = unpack(stream);
        let clear = 1u16 << min_code_size;
        let end = clear + 1;

        let mut bits: u32 = 0;
        let mut bit_count: u32 = 0;
        let mut at = 0;
        let mut code_size = min_code_size + 1;
        let mut table: Vec<Vec<u8>> = Vec::new();
        let mut previous: Option<u16> = None;
        let mut data = Vec::new();
        let mut clears = 0;

        fn reset(table: &mut Vec<Vec<u8>>, end: u16) {
            table.clear();
            table.extend((0..=end).map(|code| vec![code as u8]));
        }
        reset(&mut table, end);

        loop {
            while bit_count < u32::from(code_size) {
                assert!(at < bytes.len(), "EOIより前にデータが尽きた");
                bits |= u32::from(bytes[at]) << bit_count;
                bit_count += 8;
                at += 1;
            }
            let code = (bits & ((1 << code_size) - 1)) as u16;
            bits >>= code_size;
            bit_count -= u32::from(code_size);

            if code == clear {
                clears += 1;
                reset(&mut table, end);
                code_size = min_code_size + 1;
                previous = None;
                continue;
            }
            if code == end {
                assert!(bit_count < 8, "EOIの後に端数を超えるビットが残っている");
                assert_eq!(at, bytes.len(), "EOIの後にバイトが残っている");
                return Decoded { data, clears };
            }

            let entry = match table.get(usize::from(code)) {
                Some(entry) => entry.clone(),
                None => {
                    let at = usize::from(previous.expect("Clearの直後に未知の符号"));
                    let mut entry = table[at].clone();
                    entry.push(table[at][0]);
                    entry
                }
            };
            data.extend_from_slice(&entry);

            if let Some(previous) = previous
                && table.len() < MAX_CODES
            {
                let mut grown = table[usize::from(previous)].clone();
                grown.push(entry[0]);
                table.push(grown);
            }
            previous = Some(code);

            if table.len() >= 1 << code_size && code_size < MAX_CODE_SIZE {
                code_size += 1;
            }
        }
    }

    fn round_trip(indices: &[u8], min_code_size: u8) -> Decoded {
        let mut stream = Vec::new();
        compress(&mut stream, indices, min_code_size).unwrap();
        let decoded = decompress(&stream, min_code_size);
        assert_eq!(decoded.data, indices);
        decoded
    }

    /// 5bitの符号は仕様の並びでバイトへ詰まる
    ///
    /// 仕様 Appendix F の 3 が載せる `bbbaaaaa` / `dcccccbb` / `eeeedddd` /
    /// `ggfffffe` / `hhhhhggg` の並びをそのまま踏む。
    #[test]
    fn five_bit_codes_are_packed_least_significant_bit_first() {
        const CODES: [u16; 8] = [0x15, 0x0A, 0x1F, 0x00, 0x1B, 0x06, 0x11, 0x1E];

        let mut stream = Vec::new();
        let mut compressor = Compressor::new(&mut stream, 4);
        assert_eq!(compressor.code_size, 5, "符号長が5bitでない");
        for code in CODES {
            compressor.emit(code).unwrap();
        }
        compressor.finish().unwrap();

        assert_eq!(unpack(&stream), [0x55, 0x7D, 0xB0, 0x4D, 0xF4]);
    }

    /// 画像データはClearで始まりEOIで終わる
    #[test]
    fn the_stream_opens_with_clear_and_closes_with_end_of_information() {
        let mut stream = Vec::new();
        compress(&mut stream, &[0], 2).unwrap();

        // 3bitの符号でClear(4)、添字0、EOI(5) の順に9bit
        assert_eq!(unpack(&stream), [0b0100_0100, 0b0000_0001]);
        assert_eq!(round_trip(&[0], 2).clears, 1);
    }

    /// 空の添字の並びでもClearとEOIだけは出る
    #[test]
    fn an_empty_index_run_still_carries_clear_and_end_of_information() {
        let decoded = round_trip(&[], 2);
        assert!(decoded.data.is_empty());
        assert_eq!(decoded.clears, 1);
    }

    /// 既知の添字の並びは元へ戻る
    #[test]
    fn known_index_runs_survive_the_round_trip() {
        round_trip(&[1, 1, 1, 1, 1, 1, 1, 1], 2);
        round_trip(&[0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3], 2);
        round_trip(&[7; 4096], 3);
        round_trip(&noise(65536, 1), 8);
        round_trip(&(0..=255).collect::<Vec<u8>>(), 8);

        let ramp: Vec<u8> = (0..40000).map(|i| (i % 251) as u8).collect();
        round_trip(&ramp, 8);
    }

    /// 最小符号長が変わっても元へ戻る
    #[test]
    fn every_minimum_code_size_survives_the_round_trip() {
        for min_code_size in MIN_CODE_SIZES {
            let limit = 1u32 << min_code_size;
            let indices: Vec<u8> = noise(20000, u32::from(min_code_size))
                .iter()
                .map(|&byte| (u32::from(byte) % limit) as u8)
                .collect();
            round_trip(&indices, min_code_size);
        }
    }

    /// 辞書が4096で埋まるとClearを出して張り直す
    #[test]
    fn a_full_dictionary_is_rebuilt_after_a_clear() {
        let decoded = round_trip(&noise(1 << 18, 7), 8);
        assert!(
            decoded.clears > 1,
            "辞書の張り直しを踏んでいない: Clear {} 回",
            decoded.clears
        );
    }

    /// 出力は255バイトのサブブロックへ分かれる
    #[test]
    fn the_output_is_split_into_sub_blocks_of_at_most_255_bytes() {
        let mut stream = Vec::new();
        compress(&mut stream, &noise(1 << 16, 3), 8).unwrap();

        let lengths = block_lengths(&stream);
        assert!(lengths.len() > 1, "サブブロックが1つしかない");
        let (last, full) = lengths.split_last().unwrap();
        assert!(full.iter().all(|&len| len == MAX_SUB_BLOCK), "{lengths:?}");
        assert!(*last <= MAX_SUB_BLOCK, "{lengths:?}");
    }

    /// ちょうど255バイトで終わる出力も、余分な空のサブブロックを作らない
    #[test]
    fn an_output_ending_on_a_block_boundary_is_closed_without_an_empty_block() {
        let mut stream = Vec::new();
        {
            let mut compressor = Compressor::new(&mut stream, 7);
            for _ in 0..MAX_SUB_BLOCK {
                compressor.emit(0xFF).unwrap();
            }
            assert_eq!(compressor.bit_count, 0, "端数のビットが残っている");
            compressor.finish().unwrap();
        }

        assert_eq!(block_lengths(&stream), [MAX_SUB_BLOCK]);
        assert_eq!(stream.len(), 1 + MAX_SUB_BLOCK + 1);
    }

    /// 辞書は空きが残る大きさで、鍵の走査が一周しない
    #[test]
    fn a_missing_key_is_reported_as_an_empty_slot() {
        let mut dictionary = Dictionary::new();
        for code in 0..MAX_CODES as u16 {
            let key = u32::from(code);
            let slot = dictionary.probe(key).unwrap_err();
            dictionary.insert(slot, key, code);
        }
        for key in MAX_CODES as u32..MAX_CODES as u32 + 8192 {
            assert!(dictionary.probe(key).is_err(), "{key}");
        }
    }
}
