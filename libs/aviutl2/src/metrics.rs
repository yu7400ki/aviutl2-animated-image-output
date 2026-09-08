//! 出力プラグインのログに載せる簡単なメトリクス整形ヘルパー

/// バイト数を `"512B"` / `"1.5KB"` / `"2.3MB"` のように整形する (1024進数)
///
/// 1024バイト未満は整数バイト表記、それ以上はKB/MBを小数1桁で表示する。
/// 単位の選択は生バイト数のしきい値で決まるため、`1024*1024 - 1` は
/// 小数1桁に丸めると `"1024.0KB"` となる。
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;

    if bytes < KB {
        format!("{bytes}B")
    } else if bytes < MB {
        format!("{:.1}KB", bytes as f64 / KB as f64)
    } else {
        format!("{:.1}MB", bytes as f64 / MB as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_bytes() {
        assert_eq!(format_bytes(0), "0B");
    }

    #[test]
    fn just_under_kb() {
        assert_eq!(format_bytes(1023), "1023B");
    }

    #[test]
    fn exactly_one_kb() {
        assert_eq!(format_bytes(1024), "1.0KB");
    }

    #[test]
    fn just_under_mb_rounds_to_kb_display() {
        assert_eq!(format_bytes(1024 * 1024 - 1), "1024.0KB");
    }

    #[test]
    fn exactly_one_mb() {
        assert_eq!(format_bytes(1024 * 1024), "1.0MB");
    }

    #[test]
    fn multi_mb() {
        assert_eq!(format_bytes(3 * 1024 * 1024 + 512 * 1024), "3.5MB");
    }
}
