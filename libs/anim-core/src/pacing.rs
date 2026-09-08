//! 採否が後から決まる候補を立てる間合い

/// 候補を立てるかどうかの間合い
///
/// 候補の採否は立ててから測るまで決まらず、採られなかった候補の測定はそのまま
/// 無駄になる。構築時に受け取った回数だけ続けて採られなかったら、同じく受け取った
/// フレーム数のあいだ候補を立てるのをやめ、休みが明けたらまた試す。一度でも
/// 採られれば連敗は解ける。
pub struct Pacing {
    /// 候補を立てるのをやめるまでの連敗数
    loss_streak: u32,
    /// 連敗した後、候補を立てないフレーム数
    rest_frames: u32,
    /// 採られないまま続いた回数
    losses: u32,
    /// 残りの休みフレーム数
    resting: u32,
}

impl Pacing {
    /// `loss_streak` 回続けて採られなかったら `rest_frames` フレーム休む間合いを作る
    pub fn new(loss_streak: u32, rest_frames: u32) -> Self {
        Pacing {
            loss_streak,
            rest_frames,
            losses: 0,
            resting: 0,
        }
    }

    /// 残りの休みフレーム数
    pub fn resting(&self) -> u32 {
        self.resting
    }

    /// 候補を立てるか。休んでいる間は1フレームぶん消費して偽を返す
    pub fn should_try(&mut self) -> bool {
        if self.resting == 0 {
            return true;
        }
        self.resting -= 1;
        false
    }

    /// 立てた候補が採られたかどうかを記録する
    pub fn record(&mut self, taken: bool) {
        if taken {
            self.losses = 0;
            return;
        }

        self.losses += 1;
        if self.losses == self.loss_streak {
            self.losses = 0;
            self.resting = self.rest_frames;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 連敗を見切る回数と休むフレーム数
    ///
    /// 2つを異なる値にして、受け取った順に置かれることまで測る。
    const LOSS_STREAK: u32 = 3;
    const REST_FRAMES: u32 = 5;

    fn pacing() -> Pacing {
        Pacing::new(LOSS_STREAK, REST_FRAMES)
    }

    /// 連敗が続くと候補を立てるのを休み、休みが明けたらまた試す
    #[test]
    fn the_pacing_rests_after_a_streak_of_losses() {
        let mut pacing = pacing();
        for _ in 0..LOSS_STREAK {
            assert!(pacing.should_try());
            pacing.record(false);
        }

        for frame in 0..REST_FRAMES {
            assert!(!pacing.should_try(), "休み {frame} フレーム目");
        }
        assert!(pacing.should_try());
    }

    /// 候補が採られると連敗は解ける
    #[test]
    fn a_taken_candidate_clears_the_losses() {
        let mut pacing = pacing();
        for _ in 0..LOSS_STREAK - 1 {
            pacing.record(false);
        }
        pacing.record(true);

        for _ in 0..LOSS_STREAK - 1 {
            assert!(pacing.should_try());
            pacing.record(false);
        }
        assert!(pacing.should_try());
    }
}
