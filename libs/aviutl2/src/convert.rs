//! フレームデータのピクセルフォーマット変換

/// BGR24 (下から上・行4バイト境界) を RGB24 (上から下) へ変換する
///
/// `input` は `((width * 3 + 3) / 4) * 4` バイトストライドで `height` 行分あること。
pub fn bgr_bottomup_to_rgb(input: &[u8], width: usize, height: usize) -> Vec<u8> {
    let input_stride = (width * 3).next_multiple_of(4);
    let input = &input[..input_stride * height];

    let mut out = Vec::with_capacity(width * height * 3);

    #[cfg(target_arch = "x86_64")]
    if is_x86_feature_detected!("avx2") {
        unsafe { avx2::bgr_bottomup_to_rgb(input, width, height, &mut out) };
        return out;
    }

    scalar::bgr_bottomup_to_rgb(input, width, height, &mut out);
    out
}

/// PA64 (乗算済みアルファ 16bit/ch) を ストレートアルファ RGBA 8bit/ch へ変換する
///
/// `input` は `width * height * 4` 要素であること。
/// 乗算済みアルファの前提 (R,G,B ≤ A) を満たさない不正な入力に対する結果は未規定。
pub fn pa64_to_rgba8(input: &[u16], width: usize, height: usize) -> Vec<u8> {
    let input = &input[..width * height * 4];

    let mut out = Vec::with_capacity(width * height * 4);

    #[cfg(target_arch = "x86_64")]
    if is_x86_feature_detected!("avx2") {
        unsafe { avx2::pa64_to_rgba8(input, &mut out) };
        return out;
    }

    scalar::pa64_to_rgba8(input, &mut out);
    out
}

/// 1ピクセル分のPA64→RGBA8変換 (スカラ・SIMD両実装の端数処理が使用する参照実装)
#[inline]
fn unmultiply_pixel(r: u16, g: u16, b: u16, a: u16) -> [u8; 4] {
    let (r, g, b, a) = (r as u32, g as u32, b as u32, a as u32);
    if a < 128 {
        [0, 0, 0, 0]
    } else {
        [
            ((r * 255 + a / 2) / a) as u8,
            ((g * 255 + a / 2) / a) as u8,
            ((b * 255 + a / 2) / a) as u8,
            ((a + 128) / 257) as u8,
        ]
    }
}

mod scalar {
    pub fn bgr_bottomup_to_rgb(input: &[u8], width: usize, height: usize, out: &mut Vec<u8>) {
        let input_stride = (width * 3).next_multiple_of(4);

        // BMPは下から上に格納されているので反転してBGR→RGB変換
        for y in (0..height).rev() {
            let row_start = y * input_stride;
            let row = &input[row_start..row_start + width * 3];
            for bgr_pixel in row.chunks_exact(3) {
                out.extend_from_slice(&[bgr_pixel[2], bgr_pixel[1], bgr_pixel[0]]);
            }
        }
    }

    pub fn pa64_to_rgba8(input: &[u16], out: &mut Vec<u8>) {
        for chunk in input.chunks_exact(4) {
            out.extend_from_slice(&super::unmultiply_pixel(
                chunk[0], chunk[1], chunk[2], chunk[3],
            ));
        }
    }
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use std::arch::x86_64::*;

    /// BGR→RGB変換 (行反転付き)
    ///
    /// # Safety
    /// - AVX2が利用可能であること
    /// - `input.len() >= stride * height` (stride = `(width * 3).next_multiple_of(4)`)
    /// - `out.capacity() >= width * height * 3`
    #[target_feature(enable = "avx2")]
    pub unsafe fn bgr_bottomup_to_rgb(
        input: &[u8],
        width: usize,
        height: usize,
        out: &mut Vec<u8>,
    ) {
        let input_stride = (width * 3).next_multiple_of(4);
        let row_len = width * 3;
        debug_assert!(input.len() >= input_stride * height);
        debug_assert!(out.capacity() >= row_len * height);

        let src_base = input.as_ptr();
        let dst_base = out.as_mut_ptr();

        // 各128bitレーンで4ピクセル(12バイト)のB↔R入れ替え。上位4バイトは使わない
        #[rustfmt::skip]
        let shuf = _mm256_setr_epi8(
            2, 1, 0, 5, 4, 3, 8, 7, 6, 11, 10, 9, -1, -1, -1, -1,
            2, 1, 0, 5, 4, 3, 8, 7, 6, 11, 10, 9, -1, -1, -1, -1,
        );

        for y_out in 0..height {
            unsafe {
                let src = src_base.add((height - 1 - y_out) * input_stride);
                let dst = dst_base.add(y_out * row_len);

                let mut i = 0;
                // 24バイト(8ピクセル)ずつ処理: 12バイトずらした2つの128bitロードを合成し、
                // レーン毎にシャッフルして12バイトずらしで書き戻す。
                // 16バイトストアは4バイトはみ出すが、直後の反復か端数処理で上書きされる
                // (ループ条件 i + 28 <= row_len により行末でも領域内に収まる)。
                while i + 28 <= row_len {
                    let lo = _mm_loadu_si128(src.add(i) as *const __m128i);
                    let hi = _mm_loadu_si128(src.add(i + 12) as *const __m128i);
                    let v = _mm256_set_m128i(hi, lo);
                    let swapped = _mm256_shuffle_epi8(v, shuf);
                    _mm_storeu_si128(dst.add(i) as *mut __m128i, _mm256_castsi256_si128(swapped));
                    _mm_storeu_si128(
                        dst.add(i + 12) as *mut __m128i,
                        _mm256_extracti128_si256(swapped, 1),
                    );
                    i += 24;
                }
                // 行の端数はスカラで処理
                while i < row_len {
                    *dst.add(i) = *src.add(i + 2);
                    *dst.add(i + 1) = *src.add(i + 1);
                    *dst.add(i + 2) = *src.add(i);
                    i += 3;
                }
            }
        }

        unsafe { out.set_len(row_len * height) };
    }

    /// PA64→RGBA8変換
    ///
    /// 整数除算 `(c * 255 + a / 2) / a` をf32除算に置き換えている。
    /// 分子は最大 65535 * 255 + 32767 < 2^24 でf32で正確に表現でき、
    /// 正しく丸められた除算の誤差 (≤ 0.5ulp ≈ 7.6e-6) は商と整数境界の
    /// 最小距離 (≥ 1/65535 ≈ 1.5e-5) より小さいため、切り捨て結果は
    /// 有効な入力 (c ≤ a) に対してスカラ実装と厳密に一致する。
    ///
    /// # Safety
    /// - AVX2が利用可能であること
    /// - `input.len()` が4の倍数であること
    /// - `out.capacity() >= input.len()`
    #[target_feature(enable = "avx2")]
    pub unsafe fn pa64_to_rgba8(input: &[u16], out: &mut Vec<u8>) {
        let n_pixels = input.len() / 4;
        debug_assert!(out.capacity() >= n_pixels * 4);

        let src = input.as_ptr();
        let dst = out.as_mut_ptr();

        let mut px = 0;
        while px + 8 <= n_pixels {
            unsafe {
                // 8ピクセル = 32×u16 をロードし、u32×8 (2ピクセル) ×4ベクトルへ拡張
                let w0 = _mm256_loadu_si256(src.add(px * 4) as *const __m256i);
                let w1 = _mm256_loadu_si256(src.add(px * 4 + 16) as *const __m256i);

                let v0 = _mm256_cvtepu16_epi32(_mm256_castsi256_si128(w0));
                let v1 = _mm256_cvtepu16_epi32(_mm256_extracti128_si256(w0, 1));
                let v2 = _mm256_cvtepu16_epi32(_mm256_castsi256_si128(w1));
                let v3 = _mm256_cvtepu16_epi32(_mm256_extracti128_si256(w1, 1));

                let r0 = unmultiply2(v0);
                let r1 = unmultiply2(v1);
                let r2 = unmultiply2(v2);
                let r3 = unmultiply2(v3);

                // u32→u16→u8 と飽和パック。packはレーン毎に交互配置になるため
                // 最後にdword単位の並べ替えでピクセル順 (px0..px7) に戻す
                let p01 = _mm256_packus_epi32(r0, r1); // [px0 px2 | px1 px3] (u16)
                let p23 = _mm256_packus_epi32(r2, r3); // [px4 px6 | px5 px7] (u16)
                let packed = _mm256_packus_epi16(p01, p23); // [px0 px2 px4 px6 | px1 px3 px5 px7]
                let fixed =
                    _mm256_permutevar8x32_epi32(packed, _mm256_setr_epi32(0, 4, 1, 5, 2, 6, 3, 7));

                _mm256_storeu_si256(dst.add(px * 4) as *mut __m256i, fixed);
            }
            px += 8;
        }

        // 端数はスカラで処理
        while px < n_pixels {
            let rgba = unsafe {
                super::unmultiply_pixel(
                    *src.add(px * 4),
                    *src.add(px * 4 + 1),
                    *src.add(px * 4 + 2),
                    *src.add(px * 4 + 3),
                )
            };
            unsafe { std::ptr::copy_nonoverlapping(rgba.as_ptr(), dst.add(px * 4), 4) };
            px += 1;
        }

        unsafe { out.set_len(n_pixels * 4) };
    }

    /// 2ピクセル分 `[r g b a | r g b a]` (u32) の逆乗算
    #[inline]
    #[target_feature(enable = "avx2")]
    fn unmultiply2(v: __m256i) -> __m256i {
        let mul = _mm256_setr_ps(255.0, 255.0, 255.0, 1.0, 255.0, 255.0, 255.0, 1.0);
        let c128 = _mm256_set1_ps(128.0);
        let c257 = _mm256_set1_ps(257.0);
        let alpha_min = _mm256_set1_epi32(127);

        let a_i = _mm256_shuffle_epi32(v, 0b11_11_11_11); // [a a a a | a a a a]
        let a_half = _mm256_srli_epi32(a_i, 1); // floor(a / 2)

        let f = _mm256_cvtepi32_ps(v);
        let a_f = _mm256_cvtepi32_ps(a_i);

        // 分子: RGBレーン = c * 255 + floor(a / 2)、Aレーン = a + 128
        let addend = _mm256_blend_ps(_mm256_cvtepi32_ps(a_half), c128, 0b1000_1000);
        let num = _mm256_add_ps(_mm256_mul_ps(f, mul), addend);
        // 分母: RGBレーン = a、Aレーン = 257
        let den = _mm256_blend_ps(a_f, c257, 0b1000_1000);

        let q = _mm256_cvttps_epi32(_mm256_div_ps(num, den));

        // a < 128 のピクセルは全チャンネル0 (a=0の除算はここでマスクされる)
        let mask = _mm256_cmpgt_epi32(a_i, alpha_min);
        _mm256_and_si256(q, mask)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 決定的な疑似乱数 (xorshift64)
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    fn scalar_bgr(input: &[u8], width: usize, height: usize) -> Vec<u8> {
        let mut out = Vec::new();
        scalar::bgr_bottomup_to_rgb(input, width, height, &mut out);
        out
    }

    fn scalar_pa64(input: &[u16]) -> Vec<u8> {
        let mut out = Vec::new();
        scalar::pa64_to_rgba8(input, &mut out);
        out
    }

    #[test]
    fn bgr_matches_scalar() {
        let mut rng = Rng(1);
        // SIMDの本体パス・端数パス両方を通る幅を含める
        for &(width, height) in &[(0, 0), (1, 1), (3, 2), (8, 3), (9, 4), (10, 5), (33, 7)] {
            let stride = (width * 3usize).next_multiple_of(4);
            let input: Vec<u8> = (0..stride * height).map(|_| rng.next() as u8).collect();
            assert_eq!(
                bgr_bottomup_to_rgb(&input, width, height),
                scalar_bgr(&input, width, height),
                "width={width} height={height}"
            );
        }
    }

    #[test]
    fn pa64_matches_scalar_random() {
        let mut rng = Rng(2);
        // 端数 (17 % 8 != 0) を含むサイズで乗算済みアルファの有効な入力を検証
        for &(width, height) in &[(1, 1), (8, 1), (17, 3), (64, 2)] {
            let input: Vec<u16> = (0..width * height)
                .flat_map(|_| {
                    let a = rng.next() as u16;
                    // 乗算済みアルファの前提 (c <= a) を満たすよう [0, a] に収める
                    let mut ch = || ((rng.next() & 0xFFFF) * a as u64 / 65535) as u16;
                    [ch(), ch(), ch(), a]
                })
                .collect();
            assert_eq!(
                pa64_to_rgba8(&input, width, height),
                scalar_pa64(&input),
                "width={width} height={height}"
            );
        }
    }

    #[test]
    fn pa64_matches_scalar_edge_alphas() {
        // アルファの閾値・境界値と、チャンネルの最小/中間/最大を総当たり
        let alphas = [0u16, 1, 127, 128, 129, 255, 256, 257, 32767, 65534, 65535];
        let mut input = Vec::new();
        for &a in &alphas {
            for &c in &[0u16, a / 2, a] {
                input.extend_from_slice(&[c, a / 2, a, a]);
                input.extend_from_slice(&[a, c, 0, a]);
            }
        }
        // ピクセル数を8の倍数+端数にする
        input.extend_from_slice(&[1, 2, 3, 65535]);
        let n = input.len() / 4;
        assert_eq!(pa64_to_rgba8(&input, n, 1), scalar_pa64(&input));
    }

    #[test]
    fn pa64_exhaustive_max_alpha() {
        // a=65535 は誤差マージンが最小になるため、全チャンネル値を網羅検証
        let a = 65535u16;
        let input: Vec<u16> = (0..=65535u16).flat_map(|c| [c, 0, 0, a]).collect();
        assert_eq!(pa64_to_rgba8(&input, 65536, 1), scalar_pa64(&input));
    }
}
