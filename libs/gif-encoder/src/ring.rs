//! 書き出し位置から先を覗く先読みリング

use anim_core::FrameDelay;
use std::collections::VecDeque;

/// 先読みリングが抱えているフレーム1つ
pub(crate) struct Held {
    /// 正規化した入力そのまま
    pub(crate) pixels: Vec<u8>,
    pub(crate) delay: FrameDelay,
}

/// 書き出し位置から先のフレームを覗くための窓
///
/// 投入されたフレームをそのまま入れ、窓から溢れたぶんを書き出しへ渡す。
/// 窓が `lookahead` フレームなら、書き出し位置のフレーム1つと、リングに残る
/// `lookahead - 1` フレームが色を決める材料になる。
pub(crate) struct Ring {
    frames: VecDeque<Held>,
    /// リングに留めておくフレーム数
    capacity: usize,
}

impl Ring {
    /// `lookahead` フレームの窓を持つリング
    pub(crate) fn new(lookahead: usize) -> Self {
        Ring {
            frames: VecDeque::new(),
            capacity: lookahead.saturating_sub(1),
        }
    }

    /// フレームを1つ入れ、窓から溢れたぶんを返す
    pub(crate) fn push(&mut self, pixels: Vec<u8>, delay: FrameDelay) -> Option<Held> {
        self.frames.push_back(Held { pixels, delay });
        if self.frames.len() > self.capacity {
            self.frames.pop_front()
        } else {
            None
        }
    }

    /// 残っているフレームのうち最も古いものを取り出す
    pub(crate) fn take(&mut self) -> Option<Held> {
        self.frames.pop_front()
    }

    /// 書き出し位置より先のフレームを、投入された順に見る
    pub(crate) fn window(&self) -> impl Iterator<Item = &[u8]> {
        self.frames.iter().map(|held| held.pixels.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delay() -> FrameDelay {
        FrameDelay::new(1, 30).unwrap()
    }

    /// リングは窓のぶんだけ留め、溢れたフレームを投入された順に返す
    #[test]
    fn the_ring_holds_back_the_window_and_releases_the_rest() {
        const LOOKAHEAD: usize = 4;
        let mut ring = Ring::new(LOOKAHEAD);

        for value in 0..LOOKAHEAD as u8 - 1 {
            assert!(
                ring.push(vec![value], delay()).is_none(),
                "窓が埋まる前に溢れた"
            );
        }
        for value in LOOKAHEAD as u8 - 1..LOOKAHEAD as u8 + 3 {
            let due = ring.push(vec![value], delay()).expect("溢れていない");
            assert_eq!(due.pixels, [value - (LOOKAHEAD as u8 - 1)]);
        }

        let rest: Vec<u8> = std::iter::from_fn(|| ring.take())
            .map(|held| held.pixels[0])
            .collect();
        assert_eq!(rest, [4, 5, 6]);
    }

    /// 窓が1フレームなら留めずにそのまま流す
    #[test]
    fn a_window_of_one_frame_holds_nothing_back() {
        let mut ring = Ring::new(1);
        let due = ring.push(vec![7], delay()).expect("留めている");
        assert_eq!(due.pixels, [7]);
        assert!(ring.take().is_none());
    }
}
