//! 走査線のフィルタ (フィルタ方式0)

/// Subフィルタのフィルタ種別バイト
const SUB: u8 = 1;

/// 画像全体を行ごとにフィルタし、zlibへ渡すバイト列を `out` へ追記する
///
/// `data` は `stride` バイトの行が隙間なく並んでいること。
pub(crate) fn filter_image(data: &[u8], stride: usize, bpp: usize, out: &mut Vec<u8>) {
    out.reserve(data.len() + data.len() / stride);
    for row in data.chunks_exact(stride) {
        filter_row(row, bpp, out);
    }
}

/// 1行をフィルタし、フィルタ種別バイトに続けて `out` へ追記する
fn filter_row(row: &[u8], bpp: usize, out: &mut Vec<u8>) {
    out.push(SUB);

    // 行頭のbppバイトは左隣が存在しないため、予測値0としてそのまま出す
    let head = bpp.min(row.len());
    out.extend_from_slice(&row[..head]);
    for i in head..row.len() {
        out.push(row[i].wrapping_sub(row[i - bpp]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// フィルタを逆適用して元の画像を復元する
    fn unfilter(filtered: &[u8], stride: usize, bpp: usize) -> Vec<u8> {
        let mut out = Vec::new();
        for row in filtered.chunks_exact(stride + 1) {
            assert_eq!(row[0], SUB);
            let base = out.len();
            for (i, &x) in row[1..].iter().enumerate() {
                let left = if i < bpp { 0 } else { out[base + i - bpp] };
                out.push(x.wrapping_add(left));
            }
        }
        out
    }

    #[test]
    fn filtering_is_reversible() {
        let (width, height, bpp) = (7usize, 5usize, 4usize);
        let stride = width * bpp;
        let data: Vec<u8> = (0..stride * height).map(|i| (i * 37 % 251) as u8).collect();

        let mut filtered = Vec::new();
        filter_image(&data, stride, bpp, &mut filtered);

        assert_eq!(filtered.len(), (stride + 1) * height);
        assert_eq!(unfilter(&filtered, stride, bpp), data);
    }

    /// 1行が1ピクセルに満たない幅でも左隣を参照しない
    #[test]
    fn rows_shorter_than_one_pixel_are_passed_through() {
        let mut filtered = Vec::new();
        filter_image(&[1, 2, 3], 3, 4, &mut filtered);

        assert_eq!(filtered, [SUB, 1, 2, 3]);
    }

    #[test]
    fn constant_rows_filter_to_zero() {
        let mut filtered = Vec::new();
        filter_image(&[9, 9, 9, 9, 9, 9], 6, 3, &mut filtered);

        assert_eq!(filtered, [SUB, 9, 9, 9, 0, 0, 0]);
    }
}
