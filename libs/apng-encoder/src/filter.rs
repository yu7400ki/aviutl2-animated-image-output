//! 走査線のフィルタ (フィルタ方式0)

/// 予測値を引かない
const NONE: u8 = 0;
/// 左隣を予測値にする
const SUB: u8 = 1;
/// 真上を予測値にする
const UP: u8 = 2;
/// 左隣と真上の平均を予測値にする
const AVERAGE: u8 = 3;
/// Paeth予測値を使う
const PAETH: u8 = 4;

/// 画像全体のフィルタの決め方
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Strategy {
    /// 行ごとに5種すべてを適用し、フィルタ後のバイトを符号付きとみなした
    /// 絶対値の総和が最小のものを選ぶ
    Adaptive,
    /// 全行を予測値なし (フィルタ種別None) で通す
    Unfiltered,
}

/// 画像全体を行ごとにフィルタし、zlibへ渡すバイト列を `out` へ追記する
///
/// `data` は `stride` バイトの行が隙間なく並んでいること。`bpp` は3か4であること。
pub(crate) fn filter_image(
    data: &[u8],
    stride: usize,
    bpp: usize,
    strategy: Strategy,
    scratch: &mut Scratch,
    out: &mut Vec<u8>,
) {
    if data.is_empty() {
        return;
    }

    out.reserve(data.len() + data.len() / stride);
    match strategy {
        Strategy::Adaptive => {
            scratch.resize(stride);
            match bpp {
                3 => filter_image_bpp::<3>(data, stride, scratch, out),
                4 => filter_image_bpp::<4>(data, stride, scratch, out),
                other => panic!("1画素あたり3バイトか4バイトのみ扱える: {other}"),
            }
        }
        Strategy::Unfiltered => unfiltered_image(data, stride, out),
    }
}

/// 全行にフィルタ種別Noneを付け、行の内容をそのまま書き出す
fn unfiltered_image(data: &[u8], stride: usize, out: &mut Vec<u8>) {
    for row in data.chunks_exact(stride) {
        out.push(NONE);
        out.extend_from_slice(row);
    }
}

fn filter_image_bpp<const BPP: usize>(
    data: &[u8],
    stride: usize,
    scratch: &mut Scratch,
    out: &mut Vec<u8>,
) {
    #[cfg(target_arch = "x86_64")]
    if is_x86_feature_detected!("avx2") {
        filter_rows(data, stride, scratch, out, |cur, prev, choice| {
            // SAFETY: avx2の存在をこのクロージャを渡す前に確認している
            unsafe { avx2::select_row::<BPP>(cur, prev, choice) }
        });
        return;
    }

    filter_rows(data, stride, scratch, out, scalar::select_row::<BPP>);
}

/// `select_row` で行ごとにフィルタを選びながら、画像全体を書き出す
fn filter_rows(
    data: &[u8],
    stride: usize,
    scratch: &mut Scratch,
    out: &mut Vec<u8>,
    select_row: impl Fn(&[u8], &[u8], &mut RowChoice),
) {
    let Scratch { zero_row, choice } = scratch;

    for (y, row) in data.chunks_exact(stride).enumerate() {
        // 先頭行は上が存在しないため、予測値0として全0の行を上に置く
        let prev = if y == 0 {
            &zero_row[..]
        } else {
            &data[(y - 1) * stride..y * stride]
        };
        select_row(row, prev, choice);
        choice.emit(row, out);
    }
}

/// 行ごとの比較に使う作業領域
///
/// フレームをまたいで使い回す。
pub(crate) struct Scratch {
    /// 先頭行の上として使う全0の行
    zero_row: Vec<u8>,
    choice: RowChoice,
}

impl Scratch {
    pub(crate) fn new() -> Self {
        Scratch {
            zero_row: Vec::new(),
            choice: RowChoice::new(),
        }
    }

    /// `stride` バイトの行を扱えるようにする
    fn resize(&mut self, stride: usize) {
        // 上の行として読むだけなので、伸ばした部分は0のまま残る
        self.zero_row.resize(stride, 0);
        self.choice.best.resize(stride, 0);
        self.choice.candidate.resize(stride, 0);
    }
}

/// 1行ぶんの候補の比較と、勝っている候補の保持
///
/// [`Self::best`] は、その行で [`Self::offer`] が1度でも通ったときだけ内容が有効になる。
/// [`Self::begin`] が種別をNoneへ戻し、[`Self::emit`] がNoneのときは行そのものを出すため、
/// 前の行の内容が読まれることはない。
struct RowChoice {
    filter: u8,
    score: u64,
    /// 勝っている候補のフィルタ後バイト列
    best: Vec<u8>,
    /// これから比較する候補の書き込み先
    candidate: Vec<u8>,
}

impl RowChoice {
    fn new() -> Self {
        RowChoice {
            filter: NONE,
            score: 0,
            best: Vec::new(),
            candidate: Vec::new(),
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
    /// - AVX2が利用可能であること
    /// - `cur` と `prev` の長さが等しいこと
    /// - `choice` の候補バッファの長さが `cur` と等しいこと
    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn select_row<const BPP: usize>(
        cur: &[u8],
        prev: &[u8],
        choice: &mut RowChoice,
    ) {
        // SAFETY: 呼び出し元がavx2の存在を確認している
        unsafe {
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
    }

    /// [`scalar::abs_sum`] のAVX2版
    ///
    /// `_mm256_abs_epi8` は0x80を0x80のまま返すため、[`signed_abs`] と一致する。
    /// `_mm256_sad_epu8` は8バイトごとの総和を64bitレーンへ入れるので桁溢れしない。
    ///
    /// # Safety
    /// AVX2が利用可能であること。
    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn abs_sum(bytes: &[u8]) -> u64 {
        let zero = _mm256_setzero_si256();
        let mut acc = zero;

        let mut chunks = bytes.chunks_exact(LANES);
        for chunk in &mut chunks {
            // SAFETY: chunks_exactが返すのはちょうどLANESバイト
            let v = unsafe { _mm256_loadu_si256(chunk.as_ptr().cast()) };
            acc = _mm256_add_epi64(acc, _mm256_sad_epu8(_mm256_abs_epi8(v), zero));
        }

        // SAFETY: 呼び出し元がavx2の存在を確認している
        let sum = unsafe { horizontal_sum(acc) };
        sum + scalar::abs_sum(chunks.remainder())
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

        // SAFETY: 呼び出し元がavx2の存在を確認している
        sum += unsafe { horizontal_sum(acc) };
        sum + scalar::apply_range::<BPP, FILTER>(cur, prev, out, i..cur.len())
    }

    /// 16bitレーン16本ぶんのPaeth予測 (同点はa, b, cの順)
    ///
    /// `p = a + b - c` より `p - a = b - c`、`p - b = a - c`、`p - c = a + b - 2c`
    /// で、いずれも16bitに収まる。比較は仕様の `pa <= pb` と `pa <= pc`、`pb <= pc`
    /// の否定なので、同点では先の候補が残る。
    ///
    /// # Safety
    /// AVX2が利用可能であること。
    #[target_feature(enable = "avx2")]
    unsafe fn paeth_epi16(a: __m256i, b: __m256i, c: __m256i) -> __m256i {
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
    ///
    /// # Safety
    /// AVX2が利用可能であること。
    #[target_feature(enable = "avx2")]
    unsafe fn horizontal_sum(acc: __m256i) -> u64 {
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
    use crate::testing::noise;

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

    /// 画像の各行のフィルタ種別バイト
    fn filters_of(filtered: &[u8], stride: usize) -> Vec<u8> {
        filtered
            .chunks_exact(stride + 1)
            .map(|row| row[0])
            .collect()
    }

    fn filter_scalar(data: &[u8], stride: usize, bpp: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut scratch = Scratch::new();
        scratch.resize(stride);
        if !data.is_empty() {
            match bpp {
                3 => filter_rows(
                    data,
                    stride,
                    &mut scratch,
                    &mut out,
                    scalar::select_row::<3>,
                ),
                _ => filter_rows(
                    data,
                    stride,
                    &mut scratch,
                    &mut out,
                    scalar::select_row::<4>,
                ),
            }
        }
        out
    }

    #[cfg(target_arch = "x86_64")]
    fn filter_avx2(data: &[u8], stride: usize, bpp: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut scratch = Scratch::new();
        scratch.resize(stride);
        if !data.is_empty() {
            match bpp {
                3 => filter_rows(data, stride, &mut scratch, &mut out, |cur, prev, choice| {
                    // SAFETY: 呼び出し元がavx2の存在を確認している
                    unsafe { avx2::select_row::<3>(cur, prev, choice) }
                }),
                _ => filter_rows(data, stride, &mut scratch, &mut out, |cur, prev, choice| {
                    // SAFETY: 呼び出し元がavx2の存在を確認している
                    unsafe { avx2::select_row::<4>(cur, prev, choice) }
                }),
            }
        }
        out
    }

    /// 端数と境界を含む行長
    ///
    /// bpp 3・4それぞれのBPP - 1, BPP, BPP + 1 と、ベクタ本体の32バイト境界の前後、
    /// 絶対値の総和が16bitに収まらない512バイト超を含む。
    const ROW_LENGTHS: [usize; 20] = [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 31, 32, 33, 63, 64, 65, 95, 96, 97, 512, 513,
    ];

    /// フィルタの分岐を広く踏むバイト列
    fn patterns(len: usize, seed: u32) -> Vec<Vec<u8>> {
        vec![
            noise(len, seed),
            vec![0x00; len],
            vec![0x80; len],
            vec![0xFF; len],
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

    /// 行ごとに勝つ種別が入れ替わる画像を、両経路で逆適用して元に戻す
    ///
    /// 直前と同じ行 (Upが0点になる) の次に平坦な行 (Noneが勝つ) を置き、
    /// 行の比較が前の行の状態を引きずらないところまで踏む。
    #[test]
    fn images_with_varying_rows_are_reversible() {
        for bpp in [3, 4] {
            for width in [5usize, 64, 200] {
                let stride = width * bpp;
                let textured = noise(stride, width as u32);
                let gradient: Vec<u8> = (0..stride).map(|i| (i / 8) as u8).collect();
                let flat = vec![0u8; stride];
                let level = vec![0xC0u8; stride];

                let mut data = Vec::new();
                for row in [
                    &textured, &textured, &flat, &gradient, &flat, &level, &textured, &flat,
                ] {
                    data.extend_from_slice(row);
                }

                let filtered = filter_scalar(&data, stride, bpp);
                let filters = filters_of(&filtered, stride);

                assert!(filters.contains(&NONE), "bpp={bpp} w={width} {filters:?}");
                assert!(
                    filters.iter().any(|&f| f != NONE),
                    "bpp={bpp} w={width} {filters:?}"
                );
                assert_eq!(
                    unfilter(&filtered, stride, bpp),
                    data,
                    "bpp={bpp} w={width}"
                );

                #[cfg(target_arch = "x86_64")]
                if is_x86_feature_detected!("avx2") {
                    assert_eq!(
                        filter_avx2(&data, stride, bpp),
                        filtered,
                        "bpp={bpp} w={width}"
                    );
                }
            }
        }
    }
    /// 5種のフィルタは、どれを強制しても逆適用で元に戻る
    #[test]
    fn every_filter_is_reversible() {
        forced_filters_are_reversible::<3>();
        forced_filters_are_reversible::<4>();
    }

    fn forced_filters_are_reversible<const BPP: usize>() {
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

            assert_eq!(
                unfilter(&filtered, stride, BPP),
                data,
                "bpp={BPP} filter={filter}"
            );
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

    /// 予測値の距離が同点のときはa, b, cの順に選ぶ
    #[test]
    fn paeth_prefers_the_earlier_candidate_on_ties() {
        // a=6, b=12, c=10 では pa = 2, pb = 4, pc = 2 でaとcが同点
        assert_eq!(paeth_predictor(6, 12, 10), 6);
        // a=12, b=6, c=10 では pa = 4, pb = 2, pc = 2 でbとcが同点
        assert_eq!(paeth_predictor(12, 6, 10), 6);
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
        // 幅171(bpp3)・128(bpp4)の行は絶対値の総和が16bitに収まらない
        for (bpp, width) in [(3, 16), (3, 171), (4, 16), (4, 128)] {
            let stride = width * bpp;
            let data = vec![0x80u8; stride];

            let filtered = filter_scalar(&data, stride, bpp);

            assert_eq!(filters_of(&filtered, stride), [SUB], "bpp={bpp} w={width}");
        }
    }

    /// 一様な行はNoneが最小で、同点のSubより先に選ばれる
    #[test]
    fn a_uniform_row_selects_none() {
        let filtered = filter_scalar(&[0, 0, 0, 0, 0, 0], 6, 3);

        assert_eq!(filtered, [NONE, 0, 0, 0, 0, 0, 0]);
    }

    /// フィルタは行ごとに独立に選ばれ、前の行の選択を引きずらない
    ///
    /// 平坦な行はNone、勾配の行はSub、直前と同じ行はUpが最小になる。
    #[test]
    fn each_row_chooses_its_own_filter() {
        const BPP: usize = 3;
        const STRIDE: usize = 4 * BPP;
        let ramp = [10, 10, 10, 20, 20, 20, 30, 30, 30, 40, 40, 40];
        let data = [&[0u8; STRIDE][..], &ramp[..], &ramp[..]].concat();

        let mut filtered = Vec::new();
        filter_image(
            &data,
            STRIDE,
            BPP,
            Strategy::Adaptive,
            &mut Scratch::new(),
            &mut filtered,
        );

        assert_eq!(filters_of(&filtered, STRIDE), [NONE, SUB, UP]);
        assert_eq!(
            filters_of(&filter_scalar(&data, STRIDE, BPP), STRIDE),
            [NONE, SUB, UP]
        );
    }

    /// 1行が1ピクセルに満たない幅でも左隣を参照しない
    #[test]
    fn rows_shorter_than_one_pixel_are_passed_through() {
        let mut filtered = Vec::new();
        filter_image(
            &[1, 2, 3],
            3,
            4,
            Strategy::Adaptive,
            &mut Scratch::new(),
            &mut filtered,
        );

        assert_eq!(filtered, [NONE, 1, 2, 3]);
    }

    #[test]
    fn constant_rows_filter_to_zero() {
        let mut filtered = Vec::new();
        filter_image(
            &[9, 9, 9, 9, 9, 9],
            6,
            3,
            Strategy::Adaptive,
            &mut Scratch::new(),
            &mut filtered,
        );

        assert_eq!(filtered, [SUB, 9, 9, 9, 0, 0, 0]);
    }

    /// Unfilteredは、適応なら別の種別が選ばれる行にもNoneを付ける
    #[test]
    fn unfiltered_keeps_every_row_as_it_is() {
        const BPP: usize = 3;
        const STRIDE: usize = 4 * BPP;
        let ramp = [10, 10, 10, 20, 20, 20, 30, 30, 30, 40, 40, 40];
        let data = [&ramp[..], &ramp[..]].concat();

        let mut filtered = Vec::new();
        filter_image(
            &data,
            STRIDE,
            BPP,
            Strategy::Unfiltered,
            &mut Scratch::new(),
            &mut filtered,
        );

        assert_eq!(filters_of(&filtered, STRIDE), [NONE, NONE]);
        assert_eq!(
            filtered,
            [&[NONE][..], &ramp[..], &[NONE][..], &ramp[..]].concat()
        );
        assert_eq!(unfilter(&filtered, STRIDE, BPP), data);
    }

    /// Unfilteredはどの行長・画素サイズでも逆適用で元に戻る
    #[test]
    fn unfiltered_images_are_reversible() {
        for bpp in [3, 4] {
            for width in [1usize, 2, 7, 11, 32] {
                let (stride, height) = (width * bpp, 5);
                let data = noise(stride * height, width as u32);

                let mut filtered = Vec::new();
                filter_image(
                    &data,
                    stride,
                    bpp,
                    Strategy::Unfiltered,
                    &mut Scratch::new(),
                    &mut filtered,
                );

                assert_eq!(filtered.len(), (stride + 1) * height);
                assert_eq!(
                    unfilter(&filtered, stride, bpp),
                    data,
                    "bpp={bpp} w={width}"
                );
            }
        }
    }

    #[test]
    fn an_empty_unfiltered_image_produces_no_output() {
        let mut filtered = vec![0xAA];
        filter_image(
            &[],
            0,
            4,
            Strategy::Unfiltered,
            &mut Scratch::new(),
            &mut filtered,
        );

        assert_eq!(filtered, [0xAA]);
    }

    #[test]
    fn an_empty_image_produces_no_output() {
        let mut filtered = vec![0xAA];
        filter_image(
            &[],
            0,
            4,
            Strategy::Adaptive,
            &mut Scratch::new(),
            &mut filtered,
        );

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
                for height in [1, 2, 3, 4, 7] {
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
