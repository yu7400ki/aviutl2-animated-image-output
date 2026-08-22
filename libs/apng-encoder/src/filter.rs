//! 走査線のフィルタ (フィルタ方式0)

/// フィルタ種別バイト
const NONE: u8 = 0;
const SUB: u8 = 1;
const UP: u8 = 2;
const AVERAGE: u8 = 3;
const PAETH: u8 = 4;

/// 画像全体を行ごとにフィルタし、zlibへ渡すバイト列を `out` へ追記する
///
/// `data` は `stride` バイトの行が隙間なく並んでいること。`bpp` は3か4であること。
/// 行ごとに5種すべてを適用し、フィルタ後のバイトを符号付きとみなした絶対値の
/// 総和が最小のものを選ぶ。
pub(crate) fn filter_image(data: &[u8], stride: usize, bpp: usize, out: &mut Vec<u8>) {
    if data.is_empty() {
        return;
    }

    out.reserve(data.len() + data.len() / stride);
    match bpp {
        3 => filter_image_bpp::<3>(data, stride, out),
        4 => filter_image_bpp::<4>(data, stride, out),
        other => panic!("1画素あたり3バイトか4バイトのみ扱える: {other}"),
    }
}

fn filter_image_bpp<const BPP: usize>(data: &[u8], stride: usize, out: &mut Vec<u8>) {
    #[cfg(target_arch = "x86_64")]
    if is_x86_feature_detected!("avx2") {
        filter_rows(data, stride, out, |cur, prev, choice| {
            // SAFETY: avx2の存在をこのクロージャを渡す前に確認している
            unsafe { avx2::select_row::<BPP>(cur, prev, choice) }
        });
        return;
    }

    filter_rows(data, stride, out, scalar::select_row::<BPP>);
}

/// `select_row` で行ごとにフィルタを選びながら、画像全体を書き出す
fn filter_rows(
    data: &[u8],
    stride: usize,
    out: &mut Vec<u8>,
    select_row: impl Fn(&[u8], &[u8], &mut RowChoice),
) {
    // 先頭行は上が存在しないため、予測値0として全0の行を上に置く
    let zero_row = vec![0u8; stride];
    let mut choice = RowChoice::new(stride);

    for (y, row) in data.chunks_exact(stride).enumerate() {
        let prev = if y == 0 {
            &zero_row[..]
        } else {
            &data[(y - 1) * stride..y * stride]
        };
        select_row(row, prev, &mut choice);
        choice.emit(row, out);
    }
}

/// 1行ぶんの候補の比較と、勝っている候補の保持
struct RowChoice {
    filter: u8,
    score: u64,
    /// 勝っている候補のフィルタ後バイト列
    best: Vec<u8>,
    /// これから比較する候補の書き込み先
    candidate: Vec<u8>,
}

impl RowChoice {
    fn new(stride: usize) -> Self {
        RowChoice {
            filter: NONE,
            score: 0,
            best: vec![0; stride],
            candidate: vec![0; stride],
        }
    }

    /// Noneを暫定の勝者として、1行ぶんの比較を始める
    fn begin(&mut self, none_score: u64) {
        self.filter = NONE;
        self.score = none_score;
    }

    /// [`Self::candidate`] に書かれた候補を暫定の勝者と比べる
    ///
    /// 同点なら先に渡した候補が残るため、フィルタ番号の小さい順に渡すこと。
    fn offer(&mut self, filter: u8, score: u64) {
        if score < self.score {
            self.filter = filter;
            self.score = score;
            std::mem::swap(&mut self.best, &mut self.candidate);
        }
    }

    /// 勝った候補を、フィルタ種別バイトに続けて `out` へ追記する
    fn emit(&self, row: &[u8], out: &mut Vec<u8>) {
        out.push(self.filter);
        // Noneのフィルタ後バイト列は行そのもの
        if self.filter == NONE {
            out.extend_from_slice(row);
        } else {
            out.extend_from_slice(&self.best);
        }
    }
}

/// バイトを符号付きとみなした絶対値
///
/// 0x80は0x80のまま、それ以外は0..=127に収まる。
fn signed_abs(v: u8) -> u8 {
    v.min(v.wrapping_neg())
}

/// `a + b - c` に最も近い近傍を選ぶ (同点はa, b, cの順)
fn paeth_predictor(a: u8, b: u8, c: u8) -> u8 {
    let (a, b, c) = (a as i16, b as i16, c as i16);
    let p = a + b - c;
    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());

    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

mod scalar {
    use super::*;
    use std::ops::Range;

    /// 1行に5種のフィルタを適用し、絶対値の総和が最小のものを `choice` に残す
    pub(super) fn select_row<const BPP: usize>(cur: &[u8], prev: &[u8], choice: &mut RowChoice) {
        choice.begin(abs_sum(cur));

        let score = apply::<BPP, SUB>(cur, prev, &mut choice.candidate);
        choice.offer(SUB, score);
        let score = apply::<BPP, UP>(cur, prev, &mut choice.candidate);
        choice.offer(UP, score);
        let score = apply::<BPP, AVERAGE>(cur, prev, &mut choice.candidate);
        choice.offer(AVERAGE, score);
        let score = apply::<BPP, PAETH>(cur, prev, &mut choice.candidate);
        choice.offer(PAETH, score);
    }

    /// バイト列を符号付きとみなした絶対値の総和
    pub(super) fn abs_sum(bytes: &[u8]) -> u64 {
        bytes.iter().map(|&v| u64::from(signed_abs(v))).sum()
    }

    /// 1行に `FILTER` を適用して `out` へ書き、絶対値の総和を返す
    pub(super) fn apply<const BPP: usize, const FILTER: u8>(
        cur: &[u8],
        prev: &[u8],
        out: &mut [u8],
    ) -> u64 {
        apply_range::<BPP, FILTER>(cur, prev, out, 0..cur.len())
    }

    /// 行の一部に `FILTER` を適用して `out` の同じ位置へ書き、絶対値の総和を返す
    ///
    /// 添字は行頭からの絶対位置で、左と左上が範囲外になるのは行頭の `BPP` バイトだけ。
    pub(super) fn apply_range<const BPP: usize, const FILTER: u8>(
        cur: &[u8],
        prev: &[u8],
        out: &mut [u8],
        range: Range<usize>,
    ) -> u64 {
        debug_assert_eq!(cur.len(), prev.len());
        debug_assert_eq!(cur.len(), out.len());

        let mut sum = 0;
        for i in range {
            let a = if i >= BPP { cur[i - BPP] } else { 0 };
            let b = prev[i];
            let c = if i >= BPP { prev[i - BPP] } else { 0 };

            let predictor = match FILTER {
                NONE => 0,
                SUB => a,
                UP => b,
                AVERAGE => ((u16::from(a) + u16::from(b)) / 2) as u8,
                PAETH => paeth_predictor(a, b, c),
                other => unreachable!("フィルタ種別は0..=4のみ: {other}"),
            };

            let v = cur[i].wrapping_sub(predictor);
            out[i] = v;
            sum += u64::from(signed_abs(v));
        }
        sum
    }
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use super::*;
    use std::arch::x86_64::*;

    /// 一度に処理するバイト数
    const LANES: usize = 32;

    /// [`scalar::select_row`] のAVX2版
    ///
    /// # Safety
    /// AVX2が利用可能であること。
    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn select_row<const BPP: usize>(
        cur: &[u8],
        prev: &[u8],
        choice: &mut RowChoice,
    ) {
        choice.begin(abs_sum(cur));

        // SAFETY: 呼び出し元がavx2の存在を確認している
        unsafe {
            let score = apply::<BPP, SUB>(cur, prev, &mut choice.candidate);
            choice.offer(SUB, score);
            let score = apply::<BPP, UP>(cur, prev, &mut choice.candidate);
            choice.offer(UP, score);
            let score = apply::<BPP, AVERAGE>(cur, prev, &mut choice.candidate);
            choice.offer(AVERAGE, score);
            let score = apply::<BPP, PAETH>(cur, prev, &mut choice.candidate);
            choice.offer(PAETH, score);
        }
    }

    /// [`scalar::abs_sum`] のAVX2版
    ///
    /// `_mm256_abs_epi8` は0x80を0x80のまま返すため、[`signed_abs`] と一致する。
    /// `_mm256_sad_epu8` は8バイトごとの総和を64bitレーンへ入れるので桁溢れしない。
    #[target_feature(enable = "avx2")]
    pub(super) fn abs_sum(bytes: &[u8]) -> u64 {
        let zero = _mm256_setzero_si256();
        let mut acc = zero;

        let mut chunks = bytes.chunks_exact(LANES);
        for chunk in &mut chunks {
            // SAFETY: chunks_exactが返すのはちょうどLANESバイト
            let v = unsafe { _mm256_loadu_si256(chunk.as_ptr().cast()) };
            acc = _mm256_add_epi64(acc, _mm256_sad_epu8(_mm256_abs_epi8(v), zero));
        }

        horizontal_sum(acc) + scalar::abs_sum(chunks.remainder())
    }

    /// [`scalar::apply`] のAVX2版
    ///
    /// フィルタの適用と絶対値の総和を同じパスで行い、レジスタ上の結果から直接
    /// 総和を積む。行頭の `BPP` バイトと末尾の端数はスカラー実装に委ねる。
    ///
    /// # Safety
    /// AVX2が利用可能で、`cur`・`prev`・`out` の長さが等しいこと。
    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn apply<const BPP: usize, const FILTER: u8>(
        cur: &[u8],
        prev: &[u8],
        out: &mut [u8],
    ) -> u64 {
        debug_assert_eq!(cur.len(), prev.len());
        debug_assert_eq!(cur.len(), out.len());

        // 左と左上を参照するフィルタは、行頭のBPPバイトだけ予測値の作り方が変わる
        let head = if FILTER == UP { 0 } else { BPP.min(cur.len()) };
        let mut sum = scalar::apply_range::<BPP, FILTER>(cur, prev, out, 0..head);

        let zero = _mm256_setzero_si256();
        let ones = _mm256_set1_epi8(1);
        let mut acc = zero;

        let mut i = head;
        while i + LANES <= cur.len() {
            // SAFETY: i + LANES <= cur.len() で3つのスライスは同じ長さ。
            //         左と左上を読むフィルタでは head >= BPP なので i - BPP も行内。
            unsafe {
                let x = _mm256_loadu_si256(cur.as_ptr().add(i).cast());
                let predictor = match FILTER {
                    SUB => _mm256_loadu_si256(cur.as_ptr().add(i - BPP).cast()),
                    UP => _mm256_loadu_si256(prev.as_ptr().add(i).cast()),
                    AVERAGE => {
                        let a = _mm256_loadu_si256(cur.as_ptr().add(i - BPP).cast());
                        let b = _mm256_loadu_si256(prev.as_ptr().add(i).cast());
                        // avg_epu8は(a + b + 1) / 2 を返すため、a + b が奇数の
                        // レーンから1引いて floor((a + b) / 2) に直す
                        let odd = _mm256_and_si256(_mm256_xor_si256(a, b), ones);
                        _mm256_sub_epi8(_mm256_avg_epu8(a, b), odd)
                    }
                    PAETH => {
                        let a = _mm256_loadu_si256(cur.as_ptr().add(i - BPP).cast());
                        let b = _mm256_loadu_si256(prev.as_ptr().add(i).cast());
                        let c = _mm256_loadu_si256(prev.as_ptr().add(i - BPP).cast());
                        let lo = paeth_epi16(
                            _mm256_unpacklo_epi8(a, zero),
                            _mm256_unpacklo_epi8(b, zero),
                            _mm256_unpacklo_epi8(c, zero),
                        );
                        let hi = paeth_epi16(
                            _mm256_unpackhi_epi8(a, zero),
                            _mm256_unpackhi_epi8(b, zero),
                            _mm256_unpackhi_epi8(c, zero),
                        );
                        _mm256_packus_epi16(lo, hi)
                    }
                    other => unreachable!("予測値を持つフィルタ種別は1..=4のみ: {other}"),
                };

                let v = _mm256_sub_epi8(x, predictor);
                _mm256_storeu_si256(out.as_mut_ptr().add(i).cast(), v);
                acc = _mm256_add_epi64(acc, _mm256_sad_epu8(_mm256_abs_epi8(v), zero));
            }
            i += LANES;
        }

        sum += horizontal_sum(acc);
        sum + scalar::apply_range::<BPP, FILTER>(cur, prev, out, i..cur.len())
    }

    /// 16bitレーン16本ぶんのPaeth予測 (同点はa, b, cの順)
    ///
    /// `p = a + b - c` より `p - a = b - c`、`p - b = a - c`、`p - c = a + b - 2c`
    /// で、いずれも16bitに収まる。比較は仕様の `pa <= pb` と `pa <= pc`、`pb <= pc`
    /// の否定なので、同点では先の候補が残る。
    #[target_feature(enable = "avx2")]
    fn paeth_epi16(a: __m256i, b: __m256i, c: __m256i) -> __m256i {
        let pa = _mm256_abs_epi16(_mm256_sub_epi16(b, c));
        let pb = _mm256_abs_epi16(_mm256_sub_epi16(a, c));
        let pc = _mm256_abs_epi16(_mm256_sub_epi16(
            _mm256_add_epi16(a, b),
            _mm256_add_epi16(c, c),
        ));

        let not_a = _mm256_or_si256(_mm256_cmpgt_epi16(pa, pb), _mm256_cmpgt_epi16(pa, pc));
        let not_b = _mm256_cmpgt_epi16(pb, pc);
        _mm256_blendv_epi8(a, _mm256_blendv_epi8(b, c, not_b), not_a)
    }

    /// 64bitレーン4本の総和
    #[target_feature(enable = "avx2")]
    fn horizontal_sum(acc: __m256i) -> u64 {
        let lanes = _mm_add_epi64(
            _mm256_castsi256_si128(acc),
            _mm256_extracti128_si256(acc, 1),
        );
        let sum = _mm_add_epi64(lanes, _mm_unpackhi_epi64(lanes, lanes));
        _mm_cvtsi128_si64(sum) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// フィルタを逆適用して元の行を復元する
    ///
    /// [`paeth_predictor`] とは独立に予測値を組み立てる。
    fn unfilter_row(filter: u8, row: &mut [u8], prev: &[u8], bpp: usize) {
        for i in 0..row.len() {
            let a = if i >= bpp { i32::from(row[i - bpp]) } else { 0 };
            let b = i32::from(prev[i]);
            let c = if i >= bpp {
                i32::from(prev[i - bpp])
            } else {
                0
            };

            let predictor = match filter {
                NONE => 0,
                SUB => a,
                UP => b,
                AVERAGE => (a + b) / 2,
                PAETH => {
                    let mut best = a;
                    let mut distance = (b - c).abs();
                    if (a - c).abs() < distance {
                        best = b;
                        distance = (a - c).abs();
                    }
                    if (a + b - 2 * c).abs() < distance {
                        best = c;
                    }
                    best
                }
                other => panic!("フィルタ種別は0..=4のみ: {other}"),
            };

            row[i] = row[i].wrapping_add(predictor as u8);
        }
    }

    /// 行頭のフィルタ種別バイトを取り除きながら画像を復元する
    fn unfilter(filtered: &[u8], stride: usize, bpp: usize) -> Vec<u8> {
        let mut out: Vec<u8> = Vec::new();
        let mut prev = vec![0u8; stride];

        for chunk in filtered.chunks_exact(stride + 1) {
            let mut row = chunk[1..].to_vec();
            unfilter_row(chunk[0], &mut row, &prev, bpp);
            prev.copy_from_slice(&row);
            out.extend_from_slice(&row);
        }
        out
    }

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

    /// 画像の各行のフィルタ種別バイト
    fn filters_of(filtered: &[u8], stride: usize) -> Vec<u8> {
        filtered
            .chunks_exact(stride + 1)
            .map(|row| row[0])
            .collect()
    }

    fn filter_scalar(data: &[u8], stride: usize, bpp: usize) -> Vec<u8> {
        let mut out = Vec::new();
        if !data.is_empty() {
            match bpp {
                3 => filter_rows(data, stride, &mut out, scalar::select_row::<3>),
                _ => filter_rows(data, stride, &mut out, scalar::select_row::<4>),
            }
        }
        out
    }

    #[cfg(target_arch = "x86_64")]
    fn filter_avx2(data: &[u8], stride: usize, bpp: usize) -> Vec<u8> {
        let mut out = Vec::new();
        if !data.is_empty() {
            // SAFETY: 呼び出し元がavx2の存在を確認している
            match bpp {
                3 => filter_rows(data, stride, &mut out, |cur, prev, choice| unsafe {
                    avx2::select_row::<3>(cur, prev, choice)
                }),
                _ => filter_rows(data, stride, &mut out, |cur, prev, choice| unsafe {
                    avx2::select_row::<4>(cur, prev, choice)
                }),
            }
        }
        out
    }

    /// 端数と境界を含む行長 (bpp 3・4それぞれのBPP - 1, BPP, BPP + 1 を含む)
    const ROW_LENGTHS: [usize; 13] = [0, 1, 2, 3, 4, 5, 31, 32, 33, 63, 64, 65, 96];

    /// フィルタの分岐を広く踏むバイト列
    fn patterns(len: usize, seed: u32) -> Vec<Vec<u8>> {
        vec![
            noise(len, seed),
            vec![0x80; len],
            (0..len).map(|i| i as u8).collect(),
            (0..len).map(|i| (i / 16 * 16) as u8).collect(),
            noise(len, seed + 100).iter().map(|v| v & 3).collect(),
        ]
    }

    #[test]
    fn filtering_is_reversible() {
        for bpp in [3, 4] {
            for width in [1usize, 2, 7, 11, 32] {
                let (stride, height) = (width * bpp, 5);
                let data = noise(stride * height, width as u32);

                let filtered = filter_scalar(&data, stride, bpp);

                assert_eq!(filtered.len(), (stride + 1) * height);
                assert_eq!(
                    unfilter(&filtered, stride, bpp),
                    data,
                    "bpp={bpp} w={width}"
                );
            }
        }
    }

    /// 5種のフィルタは、どれを強制しても逆適用で元に戻る
    #[test]
    fn every_filter_is_reversible() {
        const BPP: usize = 4;
        let (stride, height) = (13 * BPP, 6);
        let data = noise(stride * height, 7);

        for filter in [NONE, SUB, UP, AVERAGE, PAETH] {
            let mut filtered = Vec::new();
            let zero_row = vec![0u8; stride];
            let mut row = vec![0u8; stride];

            for (y, cur) in data.chunks_exact(stride).enumerate() {
                let prev = if y == 0 {
                    &zero_row[..]
                } else {
                    &data[(y - 1) * stride..y * stride]
                };
                match filter {
                    NONE => scalar::apply::<BPP, NONE>(cur, prev, &mut row),
                    SUB => scalar::apply::<BPP, SUB>(cur, prev, &mut row),
                    UP => scalar::apply::<BPP, UP>(cur, prev, &mut row),
                    AVERAGE => scalar::apply::<BPP, AVERAGE>(cur, prev, &mut row),
                    _ => scalar::apply::<BPP, PAETH>(cur, prev, &mut row),
                };
                filtered.push(filter);
                filtered.extend_from_slice(&row);
            }

            assert_eq!(unfilter(&filtered, stride, BPP), data, "filter={filter}");
        }
    }

    /// 適用結果と返される絶対値の総和が一致する
    #[test]
    fn apply_returns_the_absolute_sum_of_its_output() {
        const BPP: usize = 3;
        let stride = 40 * BPP;
        let cur = noise(stride, 11);
        let prev = noise(stride, 12);
        let mut out = vec![0u8; stride];

        let sum = scalar::apply::<BPP, PAETH>(&cur, &prev, &mut out);

        assert_eq!(sum, scalar::abs_sum(&out));
    }

    /// 0x80は絶対値128として数える
    #[test]
    fn the_absolute_value_of_0x80_is_128() {
        assert_eq!(signed_abs(0x80), 128);
        assert_eq!(scalar::abs_sum(&[0x80, 0x80]), 256);
    }

    /// 直前の行と同じ内容の行はUpで全0になり、Upが選ばれる
    #[test]
    fn a_repeated_row_selects_up() {
        for bpp in [3, 4] {
            let stride = 16 * bpp;
            let row = noise(stride, 3);
            let data = [row.clone(), row].concat();

            let filtered = filter_scalar(&data, stride, bpp);

            assert_eq!(filters_of(&filtered, stride)[1], UP, "bpp={bpp}");
        }
    }

    /// 0x80が並ぶ行では、行頭のbppバイトだけが残るSubが最小になる
    #[test]
    fn a_row_of_0x80_selects_sub() {
        for bpp in [3, 4] {
            let stride = 16 * bpp;
            let data = vec![0x80u8; stride];

            let filtered = filter_scalar(&data, stride, bpp);

            assert_eq!(filters_of(&filtered, stride), [SUB], "bpp={bpp}");
        }
    }

    /// 一様な行はNoneが最小で、同点のSubより先に選ばれる
    #[test]
    fn a_uniform_row_selects_none() {
        let filtered = filter_scalar(&[0, 0, 0, 0, 0, 0], 6, 3);

        assert_eq!(filtered, [NONE, 0, 0, 0, 0, 0, 0]);
    }

    /// 1行が1ピクセルに満たない幅でも左隣を参照しない
    #[test]
    fn rows_shorter_than_one_pixel_are_passed_through() {
        let mut filtered = Vec::new();
        filter_image(&[1, 2, 3], 3, 4, &mut filtered);

        assert_eq!(filtered, [NONE, 1, 2, 3]);
    }

    #[test]
    fn constant_rows_filter_to_zero() {
        let mut filtered = Vec::new();
        filter_image(&[9, 9, 9, 9, 9, 9], 6, 3, &mut filtered);

        assert_eq!(filtered, [SUB, 9, 9, 9, 0, 0, 0]);
    }

    #[test]
    fn an_empty_image_produces_no_output() {
        let mut filtered = vec![0xAA];
        filter_image(&[], 0, 4, &mut filtered);

        assert_eq!(filtered, [0xAA]);
    }

    #[cfg(target_arch = "x86_64")]
    macro_rules! assert_apply_parity {
        ($bpp:literal, $filter:expr, $cur:expr, $prev:expr) => {{
            let mut expected = vec![0u8; $cur.len()];
            let mut actual = vec![0u8; $cur.len()];

            let expected_sum = scalar::apply::<$bpp, { $filter }>($cur, $prev, &mut expected);
            // SAFETY: 呼び出し元がavx2の存在を確認している
            let actual_sum = unsafe { avx2::apply::<$bpp, { $filter }>($cur, $prev, &mut actual) };

            let len = $cur.len();
            assert_eq!(
                actual, expected,
                "bpp={} filter={} len={len}",
                $bpp, $filter
            );
            assert_eq!(
                actual_sum, expected_sum,
                "bpp={} filter={} len={len}",
                $bpp, $filter
            );
        }};
    }

    /// AVX2実装は、フィルタごとの適用結果も絶対値の総和もスカラー実装と一致する
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn avx2_applies_each_filter_like_the_scalar_implementation() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }

        for len in ROW_LENGTHS {
            for cur in patterns(len, 1) {
                for prev in patterns(len, 2) {
                    // SAFETY: avx2の存在をこの関数の冒頭で確認している
                    assert_eq!(unsafe { avx2::abs_sum(&cur) }, scalar::abs_sum(&cur));

                    assert_apply_parity!(3, SUB, &cur, &prev);
                    assert_apply_parity!(3, UP, &cur, &prev);
                    assert_apply_parity!(3, AVERAGE, &cur, &prev);
                    assert_apply_parity!(3, PAETH, &cur, &prev);
                    assert_apply_parity!(4, SUB, &cur, &prev);
                    assert_apply_parity!(4, UP, &cur, &prev);
                    assert_apply_parity!(4, AVERAGE, &cur, &prev);
                    assert_apply_parity!(4, PAETH, &cur, &prev);
                }
            }
        }
    }

    /// AVX2実装は、選ばれるフィルタ種別も含めて画像全体がスカラー実装と一致する
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn avx2_filters_images_like_the_scalar_implementation() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }

        for bpp in [3, 4] {
            for stride in ROW_LENGTHS {
                for height in [1, 4] {
                    for data in patterns(stride * height, 3) {
                        assert_eq!(
                            filter_avx2(&data, stride, bpp),
                            filter_scalar(&data, stride, bpp),
                            "bpp={bpp} stride={stride} height={height}"
                        );
                    }
                }
            }
        }
    }
}
