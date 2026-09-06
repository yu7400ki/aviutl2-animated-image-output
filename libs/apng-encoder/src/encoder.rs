//! APNGのストリーミング書き出し

use crate::chunk::{
    BLEND_OP_OVER, BLEND_OP_SOURCE, ChunkWriter, DISPOSE_OP_NONE, DISPOSE_OP_PREVIOUS,
};
use crate::codec::Candidate;
use crate::delta::Delta;
use crate::error::Error;
use crate::layout::Layout;
use crate::over;
use crate::pipeline::Pipeline;
use anim_core::{ColorType, FrameDelay, Pacing, Rect};
use std::collections::VecDeque;
use std::io::Write;
use std::num::NonZeroUsize;
use std::ops::RangeInclusive;
use std::sync::Arc;
use std::thread::available_parallelism;

/// [`Config::compression_level`] に指定できる範囲
pub const COMPRESSION_LEVELS: RangeInclusive<u32> = 1..=9;

/// エンコード設定
#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// 入力フレームの色種別
    ///
    /// [`Encoder::add_frame`] に渡すバイト列の解釈を決める。出力の色種別も同じになる。
    pub color_type: ColorType,
    /// deflateの圧縮レベル ([`COMPRESSION_LEVELS`] の範囲)
    pub compression_level: u32,
    /// アニメーションの再生回数 (0で無限ループ)
    pub num_plays: u32,
}

/// 既定は無限ループするRGB8で、圧縮レベルは6
impl Default for Config {
    fn default() -> Self {
        Config {
            color_type: ColorType::Rgb8,
            compression_level: 6,
            num_plays: 0,
        }
    }
}

/// blend_op=OVERの候補を採るのをやめるまでの連敗数
///
/// 候補が立つかどうかは矩形の中身で決まるため、素材によっては何十フレームも
/// 立ち続けて負け続ける。数フレームで見切ると勝ち負けの揺れを拾ってしまうので、
/// 傾きがはっきりするまでの回数を取る。
const BLEND_LOSS_STREAK: u32 = 6;

/// 連敗した後、blend_op=OVERの候補を採らないフレーム数
///
/// 素材の性質は途中で変わるため、休みを置いてまた試す。長く休むほど変わり目を
/// 見つけるのが遅れる。
const BLEND_REST_FRAMES: u32 = 8;

/// dispose_opを決めた結果
///
/// 捨てるかどうかで投入されたフレームの矩形が変わり、圧縮した候補もそれに従う。
struct Disposal {
    /// 保留中のフレームに与えるdispose_op
    op: u8,
    /// 投入されたフレームを書き出す矩形
    rect: Rect,
    /// `rect` をblend_op=SOURCEで圧縮した候補
    candidate: Candidate,
}

/// 保留中のフレームを捨てないときの候補
///
/// 矩形も候補も投入した先で決まるため、受け取るまで矩形は分からない。
enum Kept {
    /// 圧縮を投入した番号
    Submitted(usize),
    /// 受け取った矩形と、それをblend_op=SOURCEで圧縮した候補
    Taken {
        rect: Rect,
        candidate: Candidate,
        /// 保留中のフレームを捨てるときの候補を走査する投入の番号
        restored: usize,
    },
}

/// 投入済み・未決定のフレーム
///
/// 決定点はここから、保留中のフレームのdispose_opを決め、キャンバスへ重ねる候補を
/// 投入する。
struct Staged {
    /// 投入された順の位置
    index: u32,
    /// 投入されたフレームの画素
    data: Arc<Vec<u8>>,
    delay: FrameDelay,
    /// 保留中のフレームを捨てないときの候補
    kept: Kept,
}

/// 決定点が組み立てた、blend_op=OVERの候補
///
/// 書き出し点はこの3つから、間合いを進めるかどうかと、比べる相手があるかどうかを読む。
enum Over {
    /// アルファを持たない出力と先頭フレーム。そのままblend_op=SOURCEで書く
    Skipped,
    /// 詰め直せなかったフレーム。間合いを1つ進めてblend_op=SOURCEで書く
    Unpacked,
    /// 詰め直して投入したフレーム。間合いを1つ進め、休みが明けていれば比べる
    Packed(usize),
}

/// 決定を終えたフレーム
///
/// dispose_opは次のフレームの決定で、blend_opは書き出し点で確定する。
struct Decided {
    rect: Rect,
    delay: FrameDelay,
    /// `rect` をblend_op=SOURCEで圧縮した候補
    source: Candidate,
    /// blend_op=OVERの候補
    over: Over,
}

/// 書き出しを待っているフレーム
///
/// dispose_opが確定したフレームだけがこの形を取る。
struct Pending {
    frame: Decided,
    /// 確定したdispose_op
    dispose: u8,
}

/// 書き出しが持ち越す状態
struct Writing {
    /// 書き出しを待っているフレーム
    pending: VecDeque<Pending>,
    /// dispose_opがまだ決まっていない、直前に決定したフレーム
    open: Option<Decided>,
    /// blend_op=OVERの候補を採るかどうかの間合い
    blend_pacing: Pacing,
}

impl Writing {
    /// まだ何も保留していない状態
    fn new() -> Self {
        Writing {
            pending: VecDeque::new(),
            open: None,
            blend_pacing: Pacing::new(BLEND_LOSS_STREAK, BLEND_REST_FRAMES),
        }
    }

    /// 決定済み・未書き出しのフレーム数
    fn depth(&self) -> usize {
        self.pending.len() + usize::from(self.open.is_some())
    }

    /// 直前に決定したフレームのdispose_opを `dispose` に確定させる
    fn settle(&mut self, dispose: u8) {
        if let Some(frame) = self.open.take() {
            self.pending.push_back(Pending { frame, dispose });
        }
    }

    /// 直前に決定したフレームを `dispose` で確定させ、`decided` を保留にする
    fn advance(&mut self, dispose: u8, decided: Decided) {
        self.settle(dispose);
        self.open = Some(decided);
    }

    /// 深さが `depth` を超えていれば、書き出せる先頭のフレームを取り出す
    fn overflowing(&mut self, depth: usize) -> Option<Pending> {
        if self.depth() <= depth {
            return None;
        }

        Some(
            self.pending
                .pop_front()
                .expect("あふれたぶんは確定している"),
        )
    }
}

/// APNGエンコーダ
///
/// [`Encoder::add_frame`] でフレームを1つずつ書き出し、[`Encoder::finish`] で終端する。
/// 投入されたフレームは決定と書き出しの列を通ってから出るため、書き出しは投入から
/// 遅れる。
///
/// 生のフレームを `ワーカー数 × 2 + 3` 面抱える。[`Encoder::new`] はワーカー数を
/// 機械の並列度に合わせるため、抱える面数もそれに比例する
/// ([`Encoder::with_workers`] で指せば固定できる)。加えて、書き出しを待つ
/// フレームごとに、圧縮した本体と、キャンバスへ重ねる候補の詰め直した領域および
/// 圧縮した本体を抱える。フレームは投入された順にそのまま書き出す。
///
/// 落としたエンコーダはワーカーを畳んでから返る。
pub struct Encoder<W: Write> {
    /// チャンクを並べる書き出し先
    chunks: ChunkWriter<W>,
    /// キャンバスの大きさと入力フレームのバイト並び
    layout: Layout,
    /// 圧縮の投入口と、番号を指す結果の受け取り
    pipeline: Pipeline,
    /// 投入と決定それぞれが見るフレームの追跡
    delta: Delta,
    /// 投入済み・未決定のフレーム
    staged: VecDeque<Staged>,
    /// [`Self::staged`] に置いたまま決定を待たせるフレーム数
    staged_depth: usize,
    /// 書き出しの状態
    writing: Writing,
    /// 決定済みのまま書き出しを待たせるフレーム数
    pending_depth: NonZeroUsize,
    num_frames: u32,
    /// [`Self::add_frame`] が受け付けたフレーム数
    frames_accepted: u32,
    /// 書き出しに失敗し、チャンク列が中断しているか
    poisoned: bool,
}

impl<W: Write> Encoder<W> {
    /// `num_frames` フレームを受け付ける状態にする
    ///
    /// この時点でシグネチャと、画素データより前に置くチャンクを書き出す。
    ///
    /// 圧縮を回すワーカー数は機械の並列度になる。実際に起こした数は
    /// [`Encoder::workers`] が返す。
    ///
    /// # Errors
    /// 幅・高さ・フレーム数が0のとき、1フレームのバイト数が `usize` で表現できないとき、
    /// 圧縮レベルが範囲外のとき、スレッドを起こせないとき、または書き出しに失敗したとき。
    pub fn new(
        writer: W,
        width: u32,
        height: u32,
        num_frames: u32,
        config: Config,
    ) -> Result<Self, Error> {
        let workers = available_parallelism().unwrap_or(NonZeroUsize::MIN);
        Self::with_workers(writer, width, height, num_frames, config, workers)
    }

    /// ワーカー数を指してエンコーダを作る
    ///
    /// 渡した数をそのまま起こす。ワーカーが1つなら群れを起こさず、投入した場で
    /// 圧縮する。決定も書き出しの順序もワーカー数に依らないので、出力はどの数でも
    /// 同じになる。
    ///
    /// # Errors
    /// [`Encoder::new`] と同じ。
    pub fn with_workers(
        writer: W,
        width: u32,
        height: u32,
        num_frames: u32,
        config: Config,
        workers: NonZeroUsize,
    ) -> Result<Self, Error> {
        if width == 0 || height == 0 {
            return Err(Error::InvalidDimensions { width, height });
        }
        if num_frames == 0 {
            return Err(Error::InvalidFrameCount);
        }
        if !COMPRESSION_LEVELS.contains(&config.compression_level) {
            return Err(Error::InvalidCompressionLevel(config.compression_level));
        }

        let layout = Layout::new(width, height, config.color_type)?;
        let pipeline = Pipeline::new(config.compression_level, workers, layout)?;
        let mut chunks = ChunkWriter::new(writer);
        Self::open(&mut chunks, &layout, num_frames, config)?;

        Ok(Encoder {
            chunks,
            layout,
            pipeline,
            delta: Delta::new(),
            staged: VecDeque::new(),
            staged_depth: workers.get().saturating_mul(2),
            writing: Writing::new(),
            pending_depth: workers.saturating_add(1),
            num_frames,
            frames_accepted: 0,
            poisoned: false,
        })
    }

    /// 圧縮を回すワーカー数
    pub fn workers(&self) -> NonZeroUsize {
        self.pipeline.workers()
    }

    /// 画素データより前に置くチャンクを書き出す
    fn open(
        chunks: &mut ChunkWriter<W>,
        layout: &Layout,
        num_frames: u32,
        config: Config,
    ) -> Result<(), Error> {
        chunks.write_signature()?;

        let mut ihdr = [0u8; 13];
        ihdr[0..4].copy_from_slice(&layout.width.to_be_bytes());
        ihdr[4..8].copy_from_slice(&layout.height.to_be_bytes());
        ihdr[8] = 8;
        ihdr[9] = layout.color_type_code();
        chunks.write(*b"IHDR", &ihdr)?;

        let mut actl = [0u8; 8];
        actl[0..4].copy_from_slice(&num_frames.to_be_bytes());
        actl[4..8].copy_from_slice(&config.num_plays.to_be_bytes());
        chunks.write(*b"acTL", &actl)
    }

    /// フレームを1つ投入する
    ///
    /// `data` は上から下・左から右の順に並んだ `幅 * 高さ * 1画素のバイト数` バイトであること。
    ///
    /// 投入されたフレームはその場では書き出さず、以降の呼び出し、または
    /// [`Encoder::finish`] で書き出す。
    ///
    /// # Errors
    /// `data` の長さが合わないとき、宣言したフレーム数を超えたとき、書き出しに
    /// 失敗したとき、または過去の失敗でエンコーダが使用不能なとき。
    ///
    /// 書き出しの失敗は先に投入されたフレームのものになる。列に残ったフレームの
    /// 書き出しは [`Encoder::finish`] で報告される。
    pub fn add_frame(&mut self, data: &[u8], delay: FrameDelay) -> Result<(), Error> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        if self.frames_accepted == self.num_frames {
            return Err(Error::FrameCountMismatch {
                expected: self.num_frames,
                actual: self.frames_accepted + 1,
            });
        }

        if data.len() != self.layout.frame_len {
            return Err(Error::FrameSizeMismatch {
                expected: self.layout.frame_len,
                actual: data.len(),
            });
        }

        // 途中で失敗するとfcTLだけが書かれた状態で残るため、以降の書き出しを拒否する
        self.accept(data, delay)
            .inspect_err(|_| self.poisoned = true)?;

        self.frames_accepted += 1;
        Ok(())
    }

    /// フレームを投入し、列からあふれたぶんを決定へ回す
    fn accept(&mut self, data: &[u8], delay: FrameDelay) -> Result<(), Error> {
        self.stage(data, delay);
        while self.staged.len() > self.staged_depth {
            self.decide()?;
        }
        Ok(())
    }

    /// フレームを写し取り、捨てないときの候補の圧縮を投入する
    ///
    /// 先頭フレームはIDATに入るためキャンバス全体を切り出し、以降は直前に投入された
    /// フレームとの差分を採る。走査も切り出しも圧縮を回す側で行う。
    fn stage(&mut self, data: &[u8], delay: FrameDelay) {
        let index = self.frames_accepted;
        let (previous, data) = self.delta.stage(data);

        let job = if index == 0 {
            self.pipeline
                .submit_crop(Arc::clone(&data), self.layout.whole())
        } else {
            self.pipeline.submit_diff(previous, Arc::clone(&data))
        };

        self.staged.push_back(Staged {
            index,
            data,
            delay,
            kept: Kept::Submitted(job),
        });
    }

    /// 投入された最も古いフレームを決定し、列からあふれたぶんを書き出す
    fn decide(&mut self) -> Result<(), Error> {
        let Staged {
            index,
            data,
            delay,
            kept,
        } = self.staged.pop_front().expect("決めるフレームがある");

        let Disposal {
            op: dispose,
            rect,
            candidate: source,
        } = self.choose_dispose(kept);
        let over = self.submit_over(&data, index, dispose, rect);

        self.writing.advance(
            dispose,
            Decided {
                rect,
                delay,
                source,
                over,
            },
        );
        self.delta.advance(data, dispose);
        self.prepare_next();

        while let Some(pending) = self.writing.overflowing(self.pending_depth.get()) {
            self.write(pending)?;
        }
        Ok(())
    }

    /// 終端して書き出し先を返す
    ///
    /// # Errors
    /// 投入されたフレーム数が宣言したフレーム数に満たないとき、書き出しに失敗したとき、
    /// または過去の書き出し失敗でエンコーダが使用不能なとき。
    pub fn finish(mut self) -> Result<W, Error> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        if self.frames_accepted != self.num_frames {
            return Err(Error::FrameCountMismatch {
                expected: self.num_frames,
                actual: self.frames_accepted,
            });
        }

        while !self.staged.is_empty() {
            self.decide()?;
        }
        // 次のフレームが無いため、最後のフレームは捨てても復元される先が無い
        self.writing.settle(DISPOSE_OP_NONE);
        while let Some(pending) = self.writing.overflowing(0) {
            self.write(pending)?;
        }

        self.chunks.write(*b"IEND", &[])?;
        Ok(self.chunks.into_inner())
    }

    /// 保留中のフレームのdispose_opと、投入されたフレームの矩形を決める
    ///
    /// 保留中のフレームをdispose_op=PREVIOUSで捨てると、投入されたフレームは
    /// それを描く直前のキャンバスとの差分になる。両方の候補を圧縮して小さい方を採り、
    /// 採った側を戻り値へ残して、退けた側のバッファは配り直す先へ返す。同じ大きさなら
    /// 捨てない。
    ///
    /// 捨てるときの候補は矩形が狭いときだけ立ち、その判定は投入した先で済んでいる。
    fn choose_dispose(&mut self, kept: Kept) -> Disposal {
        let (kept_rect, kept_candidate, restored) = match kept {
            Kept::Submitted(job) => {
                let (rect, candidate) = self.pipeline.take_cut(job);
                (rect, candidate, None)
            }
            Kept::Taken {
                rect,
                candidate,
                restored,
            } => (rect, candidate, self.pipeline.take_restored(restored)),
        };

        let keep = |candidate| Disposal {
            op: DISPOSE_OP_NONE,
            rect: kept_rect,
            candidate,
        };
        let Some((rect, restored_candidate)) = restored else {
            return keep(kept_candidate);
        };

        if restored_candidate.len() < kept_candidate.len() {
            self.pipeline.recycle(kept_candidate.into_body());
            Disposal {
                op: DISPOSE_OP_PREVIOUS,
                rect,
                candidate: restored_candidate,
            }
        } else {
            self.pipeline.recycle(restored_candidate.into_body());
            keep(kept_candidate)
        }
    }

    /// 次に決定するフレームの、捨てないときの候補を受け取り、捨てるときの候補を投入する
    ///
    /// 捨てたときに復元されるキャンバスは直前の決定で確定するため、決定の1手前に
    /// あたるこの時点で投入できる。投入には比べる相手の面積が要るので、捨てないときの
    /// 候補もここで受け取る。列の深さは2以上なので、決定の後も列には次のフレームが
    /// 残る。空なのは終端で流し切る最後の決定だけで、そのときは投入する相手がない。
    ///
    /// 投入された順の位置が2に満たないフレームは、捨てるときの候補が立たない。
    /// キャンバスがまだ埋まっておらず、先頭のfcTLのdispose_op=PREVIOUSも
    /// BACKGROUNDとして扱われてキャンバスを復元しないため。比べる相手の要らない
    /// そのフレームは、捨てないときの候補を決定の場で受け取る。
    fn prepare_next(&mut self) {
        let canvas = self.delta.canvas();
        let Some(next) = self.staged.front() else {
            return;
        };
        if next.index < 2 {
            return;
        }
        let Kept::Submitted(job) = next.kept else {
            panic!("捨てないときの候補を2度受け取っている")
        };
        let data = Arc::clone(&next.data);

        let (rect, candidate) = self.pipeline.take_cut(job);
        let restored = self.pipeline.submit_restored(canvas, data, rect.area());
        self.staged
            .front_mut()
            .expect("受け取ったフレームが残る")
            .kept = Kept::Taken {
            rect,
            candidate,
            restored,
        };
    }

    /// 投入されたフレームをキャンバスへ重ねる候補を詰め直し、圧縮を投入する
    ///
    /// 矩形の中で変化した画素がすべて不透明なら、変化していない画素を完全な透明へ
    /// 潰した候補が立つ。blend_op=OVERはその画素でキャンバスを残すため、潰しても
    /// 元の値に戻る。重ねる先は確定した `dispose` から決まる。
    ///
    /// 潰した画素は完全な透明として書くため、アルファを持つ出力でだけ候補が立つ。
    /// 重ねる先を持つのは、キャンバスの埋まった2フレーム目以降になる。
    ///
    /// 条件を満たすフレームは、間合いに依らず投入する。
    fn submit_over(&mut self, data: &[u8], index: u32, dispose: u8, rect: Rect) -> Over {
        if !matches!(self.layout.input, ColorType::Rgba8) || index == 0 {
            return Over::Skipped;
        }

        let base = self.delta.base(dispose);
        let stride = self.layout.stride;
        let mut region = self.pipeline.buffer();
        if !over::pack_over(base, data, stride, rect, &mut region) {
            self.pipeline.recycle(region);
            return Over::Unpacked;
        }

        let bpp = self.layout.bytes_per_pixel;
        Over::Packed(
            self.pipeline
                .submit_region(region, rect.width as usize * bpp),
        )
    }

    /// 書き出すフレームをキャンバスへ重ねる方法を決める
    ///
    /// 候補が立ったフレームは、詰め直せたかどうかに依らず間合いを1つ進める。休みが
    /// 明けていれば圧縮した候補と `source` を比べて小さい方を採り、その結果を間合いへ
    /// 記録して、退けた側のバッファを配り直す先へ返す。同じ大きさならSOURCEを採る。
    ///
    /// 休みの最中も投入した候補を受け取り、そのバッファを配り直す先へ返す。
    fn resolve_blend(&mut self, over: Over, source: Candidate) -> (u8, Candidate) {
        if matches!(over, Over::Skipped) {
            return (BLEND_OP_SOURCE, source);
        }

        let trying = self.writing.blend_pacing.should_try();
        let Over::Packed(job) = over else {
            return (BLEND_OP_SOURCE, source);
        };
        let candidate = self.pipeline.take(job);
        if !trying {
            self.pipeline.recycle(candidate.into_body());
            return (BLEND_OP_SOURCE, source);
        }

        let taken = candidate.len() < source.len();
        self.writing.blend_pacing.record(taken);
        if taken {
            self.pipeline.recycle(source.into_body());
            (BLEND_OP_OVER, candidate)
        } else {
            self.pipeline.recycle(candidate.into_body());
            (BLEND_OP_SOURCE, source)
        }
    }

    /// 確定したフレームを書き出す
    fn write(&mut self, pending: Pending) -> Result<(), Error> {
        let Pending { frame, dispose } = pending;
        let Decided {
            rect,
            delay,
            source,
            over,
        } = frame;

        let (blend, candidate) = self.resolve_blend(over, source);
        let body = candidate.into_body();
        self.chunks
            .write_frame(rect, delay, dispose, blend, &body)?;
        self.pipeline.recycle(body);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk;
    use crate::testing::noise;
    use anim_core::dirty_rect;
    use flate2::read::ZlibDecoder;
    use std::io::{Cursor, Read};

    const WIDTH: u32 = 64;
    const HEIGHT: u32 = 48;

    /// 少数の色のブロックが並ぶフレーム
    ///
    /// 行の中で同じバイト列が繰り返すため、フィルタを掛けない方が小さくなる。
    fn flat_frame(seed: u32) -> Vec<u8> {
        const PALETTE: [[u8; 3]; 4] = [
            [0x1E, 0x1E, 0x28],
            [0xD0, 0xD0, 0xC8],
            [0x40, 0x80, 0xC0],
            [0xC0, 0x40, 0x60],
        ];

        let blocks = noise((WIDTH * HEIGHT) as usize, seed);
        let mut frame = Vec::new();
        for y in 0..HEIGHT as usize {
            for x in 0..WIDTH as usize {
                let block = x / 7 + y / 5 * 9;
                let index = (blocks[block % blocks.len()] as usize + seed as usize) % PALETTE.len();
                frame.extend_from_slice(&PALETTE[index]);
            }
        }
        frame
    }

    /// なだらかな階調に微小なノイズを載せたフレーム
    ///
    /// 隣接画素の差が小さいため、行ごとの適応フィルタが効く。
    fn detailed_frame(seed: u32) -> Vec<u8> {
        let grain = noise((WIDTH * HEIGHT) as usize * 3, seed);
        let mut frame = Vec::new();
        for y in 0..HEIGHT as usize {
            for x in 0..WIDTH as usize {
                for channel in 0..3 {
                    let base = (x * 3 + y * 5 + channel * 17 + seed as usize * 2) as u8;
                    frame
                        .push(base.wrapping_add(grain[(y * WIDTH as usize + x) * 3 + channel] & 7));
                }
            }
        }
        frame
    }

    /// RGB8のフレームに不透明なアルファを足す
    fn with_alpha(frame: &[u8]) -> Vec<u8> {
        frame
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 0xFF])
            .collect()
    }

    /// 書き出された1フレーム
    struct Written {
        /// fcTLが示す矩形の幅
        width: u32,
        /// zlibを解いたフィルタ後のバイト列
        filtered: Vec<u8>,
    }

    impl Written {
        /// 各行の先頭にあるフィルタ種別バイト
        fn filter_types(&self, bpp: usize) -> Vec<u8> {
            self.filtered
                .chunks_exact(self.width as usize * bpp + 1)
                .map(|row| row[0])
                .collect()
        }
    }

    /// チャンクを順に辿り、フレームごとの矩形の幅とフィルタ後のバイト列を取り出す
    fn written_frames(bytes: &[u8]) -> Vec<Written> {
        let mut frames = Vec::new();
        let mut width = 0;
        let mut offset = chunk::SIGNATURE.len();

        while offset + 12 <= bytes.len() {
            let len = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
            let kind = &bytes[offset + 4..offset + 8];
            let data = &bytes[offset + 8..offset + 8 + len];

            match kind {
                b"fcTL" => width = u32::from_be_bytes(data[4..8].try_into().unwrap()),
                b"IDAT" | b"fdAT" => {
                    let stream = if kind == b"fdAT" { &data[4..] } else { data };
                    let mut filtered = Vec::new();
                    ZlibDecoder::new(stream).read_to_end(&mut filtered).unwrap();
                    frames.push(Written { width, filtered });
                }
                _ => {}
            }
            offset += 12 + len;
        }

        frames
    }

    /// `workers` 個のワーカーでフレーム列を書き出す
    fn encode_with_workers(input: &[Vec<u8>], config: Config, workers: usize) -> Vec<u8> {
        let mut encoder = Encoder::with_workers(
            Cursor::new(Vec::new()),
            WIDTH,
            HEIGHT,
            input.len() as u32,
            config,
            NonZeroUsize::new(workers).unwrap(),
        )
        .unwrap();
        for frame in input {
            encoder
                .add_frame(frame, FrameDelay::new(1, 30).unwrap())
                .unwrap();
        }
        encoder.finish().unwrap().into_inner()
    }

    fn encode(input: &[Vec<u8>], config: Config) -> Vec<u8> {
        encode_with_workers(input, config, 1)
    }

    fn rgb_config() -> Config {
        Config {
            color_type: ColorType::Rgb8,
            ..Config::default()
        }
    }

    /// フレームごとのフィルタ種別バイト
    fn filter_types(bytes: &[u8], bpp: usize) -> Vec<Vec<u8>> {
        written_frames(bytes)
            .iter()
            .map(|frame| frame.filter_types(bpp))
            .collect()
    }

    /// 戦略の確認に使うフレーム数
    const FRAMES: u32 = 8;

    /// フィルタを掛けない方が小さい素材は、どのフレームもNoneだけになる
    #[test]
    fn a_flat_source_takes_the_unfiltered_strategy_on_every_frame() {
        let input: Vec<Vec<u8>> = (0..FRAMES).map(flat_frame).collect();
        let bytes = encode(&input, rgb_config());

        let types = filter_types(&bytes, 3);
        assert_eq!(types.len(), input.len());
        for (index, frame) in types.iter().enumerate() {
            assert!(frame.iter().all(|&f| f == 0), "フレーム {index}: {frame:?}");
        }
    }

    /// 適応フィルタが効く素材は、どのフレームもNone以外を選ぶ
    #[test]
    fn a_detailed_source_takes_the_adaptive_strategy_on_every_frame() {
        let input: Vec<Vec<u8>> = (0..FRAMES).map(detailed_frame).collect();
        let bytes = encode(&input, rgb_config());

        let types = filter_types(&bytes, 3);
        assert_eq!(types.len(), input.len());
        for (index, frame) in types.iter().enumerate() {
            assert!(frame.iter().any(|&f| f != 0), "フレーム {index}: {frame:?}");
        }
    }

    /// 素材の途中で有利な戦略が入れ替わると、選ばれる戦略もそこで入れ替わる
    #[test]
    fn the_strategy_follows_the_source_frame_by_frame() {
        const HALF: u32 = FRAMES / 2;

        let mut input: Vec<Vec<u8>> = (0..HALF).map(flat_frame).collect();
        input.extend((0..HALF).map(detailed_frame));
        let bytes = encode(&input, rgb_config());

        let types = filter_types(&bytes, 3);
        assert_eq!(types.len(), input.len());
        for (index, frame) in types.iter().take(HALF as usize).enumerate() {
            assert!(frame.iter().all(|&f| f == 0), "フレーム {index}: {frame:?}");
        }
        for (index, frame) in types.iter().enumerate().skip(HALF as usize) {
            assert!(frame.iter().any(|&f| f != 0), "フレーム {index}: {frame:?}");
        }
    }

    /// RGBA8を入力する設定
    fn rgba_config() -> Config {
        Config {
            color_type: ColorType::Rgba8,
            ..Config::default()
        }
    }

    /// 背景を四角が飛び回り、ときどき全面が閃くRGB8の列
    ///
    /// 四角は離れた4点を順に移動するため、差分の外接矩形は広く取りながら中身の
    /// ほとんどが変化しない。閃光は背景ごと塗り替え、その次のフレームで直前の
    /// 内容へ戻すので、閃光を捨てたキャンバスとの差分が1画素になり、捨てる候補が
    /// 勝つ。
    fn jumping_frames() -> Vec<Vec<u8>> {
        /// 四角を置く位置
        const SPOTS: [(u32, u32); 4] = [(2, 2), (50, 36), (48, 4), (4, 34)];
        /// 四角の一辺
        const SIDE: u32 = 6;
        /// 四角を動かす回数
        const STEPS: usize = 18;
        /// 閃光を挟む間隔
        const FLASH_EVERY: usize = 5;

        let background = detailed_frame(0);
        let flash = detailed_frame(9);
        let square = |(left, top): (u32, u32)| {
            let mut frame = background.clone();
            for y in top..top + SIDE {
                for x in left..left + SIDE {
                    let at = ((y * WIDTH + x) * 3) as usize;
                    frame[at..at + 3].copy_from_slice(&[0xF0, 0x20, 0x40]);
                }
            }
            frame
        };

        let mut frames: Vec<Vec<u8>> = Vec::new();
        for step in 0..STEPS {
            frames.push(square(SPOTS[step % SPOTS.len()]));
            if step % FLASH_EVERY == FLASH_EVERY - 1 {
                let restored = frames.last().expect("四角を置いたフレームがある").clone();
                frames.push(flash.clone());
                frames.push(restored);
            }
        }
        frames
    }

    /// [`jumping_frames`] を色種別ごとに揃えた素材
    ///
    /// αを足した列では、四角の動きが残す変化しない画素を潰した候補が立つ。
    /// αの無い列で立つのは捨てる候補だけで、幅はすべてそちらから出る。
    fn jumping_material() -> [(Config, Vec<Vec<u8>>); 2] {
        let rgb = jumping_frames();
        let rgba = rgb.iter().map(|frame| with_alpha(frame)).collect();
        [(rgb_config(), rgb), (rgba_config(), rgba)]
    }

    /// 潰した候補が負け続ける前半のフレーム数
    ///
    /// 一様な面から始め、まだらな半透明の面と一様な面を [`BLEND_LOSS_STREAK`] 回
    /// 繰り返す。最後のフレームで連敗が閾値に届く。
    const LOSING_FRAMES: usize = 1 + 2 * BLEND_LOSS_STREAK as usize;

    /// 休みの最中に潰した候補が負け続けるフレーム数
    ///
    /// [`BLEND_LOSS_STREAK`] のぶんだけ並べ、残りの休みを勝つフレームへ譲る。負けを
    /// 休みの最中にも数えると、この列の終わりで連敗が閾値に届いて休みが張り直される。
    const RESTING_LOSSES: usize = BLEND_LOSS_STREAK as usize;

    /// [`resting_frames`] が画素ごとに違う面へ切り替わるフレームの位置
    const DENSE_STARTS_AT: usize = LOSING_FRAMES + RESTING_LOSSES;

    /// [`resting_frames`] で潰した候補が初めて採られるフレームの位置
    const REST_ENDS_AT: usize = LOSING_FRAMES + BLEND_REST_FRAMES as usize;

    /// 休みが明けてから潰した候補が採られるフレーム数
    const WINNING_FRAMES: usize = 5;

    /// RGBA8の1画素を、色を反転した不透明な値へ書き換える
    fn invert_rgba(frame: &mut [u8], x: usize, y: usize) {
        let at = (y * WIDTH as usize + x) * 4;
        for channel in &mut frame[at..at + 3] {
            *channel ^= 0xFF;
        }
        frame[at + 3] = 0xFF;
    }

    /// 潰した候補が負け続けてから勝ちに変わるRGBA8の列
    ///
    /// 前半は一様な面とまだらな半透明の面を交互に置く。まだらへ変わるフレームは
    /// 変化した画素が不透明でないため詰め直せず、一様へ戻るフレームは1画素の矩形を
    /// 詰め直して必ず負ける。連敗が閾値に届いた後は同じ面を並べ、休みが明ける手前まで
    /// 1画素の矩形を詰め直しては負け続ける。
    ///
    /// 後半は画素ごとに違う不透明な色を敷き、そこへ離れた2画素ずつ印を書き足す。
    /// 矩形は2つの印を囲んで広がり、その中のほとんどが変化しないため、潰した候補が
    /// 必ず勝つ。印は消さずに足すので、捨てた場合の矩形は捨てない場合より広くなる。
    fn resting_frames() -> Vec<Vec<u8>> {
        const PIXELS: usize = (WIDTH * HEIGHT) as usize;
        /// まだらに置き換える画素の間隔
        const STEP: usize = 7;

        let uniform = with_alpha(&vec![0x30u8; PIXELS * 3]);
        let mut speckled = uniform.clone();
        for pixel in (0..PIXELS).step_by(STEP) {
            speckled[pixel * 4..pixel * 4 + 4].copy_from_slice(&[0xC0, 0xB0, 0xA0, 0x80]);
        }

        let mut frames = vec![uniform.clone()];
        for _ in 0..BLEND_LOSS_STREAK {
            frames.push(speckled.clone());
            frames.push(uniform.clone());
        }
        for _ in 0..RESTING_LOSSES {
            frames.push(uniform.clone());
        }

        let mut dense = with_alpha(&noise(PIXELS * 3, 5));
        frames.push(dense.clone());
        while frames.len() < REST_ENDS_AT + WINNING_FRAMES {
            let step = frames.len() - DENSE_STARTS_AT - 1;
            invert_rgba(&mut dense, 1 + step, 1);
            invert_rgba(&mut dense, WIDTH as usize - 2 - step, HEIGHT as usize - 2);
            frames.push(dense.clone());
        }
        frames
    }

    /// [`resting_frames`] を色種別と揃えた素材
    fn resting_material() -> (Config, Vec<Vec<u8>>) {
        (rgba_config(), resting_frames())
    }

    /// 捨てる候補が圧縮した上で退けられるRGBA8の列
    ///
    /// 最後のフレームは狭い領域だけが擬似乱数で、残りは一様。1つ前はそこに広い帯を
    /// 重ね、2つ前は狭い領域を持たない。捨てた場合の矩形は狭い領域に縮んで候補に立つが、
    /// 一様な広い帯より大きく圧縮されるため退けられる。
    fn rejected_restore_material() -> (Config, Vec<Vec<u8>>) {
        /// 擬似乱数で埋める領域 (左, 上, 幅, 高さ)
        const BLOCK: (usize, usize, usize, usize) = (20, 20, 8, 4);
        /// 一様に塗り替える帯の行数
        const BAND: usize = 8;

        let base = with_alpha(&vec![0x30u8; (WIDTH * HEIGHT) as usize * 3]);
        let grain = noise(BLOCK.2 * BLOCK.3 * 3, 7);
        let mut last = base.clone();
        for y in 0..BLOCK.3 {
            for x in 0..BLOCK.2 {
                let at = ((BLOCK.1 + y) * WIDTH as usize + BLOCK.0 + x) * 4;
                let from = (y * BLOCK.2 + x) * 3;
                last[at..at + 3].copy_from_slice(&grain[from..from + 3]);
            }
        }

        let mut middle = last.clone();
        for pixel in middle[..BAND * WIDTH as usize * 4].chunks_exact_mut(4) {
            pixel[..3].copy_from_slice(&[0x80, 0x80, 0x80]);
        }

        (rgba_config(), vec![base, middle, last])
    }

    /// fcTLの並びから、フレームごとの値を1つ取り出す
    fn frame_control<T>(bytes: &[u8], pick: impl Fn(&[u8]) -> T) -> Vec<T> {
        let mut values = Vec::new();
        let mut offset = chunk::SIGNATURE.len();

        while offset + 12 <= bytes.len() {
            let len = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
            if &bytes[offset + 4..offset + 8] == b"fcTL" {
                values.push(pick(&bytes[offset + 8..offset + 8 + len]));
            }
            offset += 12 + len;
        }

        values
    }

    /// fcTLが並べるdispose_opとblend_op
    fn frame_ops(bytes: &[u8]) -> Vec<(u8, u8)> {
        /// fcTLの中でdispose_opが始まる位置
        const DISPOSE_OP: usize = 24;

        frame_control(bytes, |data| (data[DISPOSE_OP], data[DISPOSE_OP + 1]))
    }

    /// blend_op=OVERで書かれたフレームの位置
    fn over_frames(bytes: &[u8]) -> Vec<usize> {
        frame_ops(bytes)
            .iter()
            .enumerate()
            .filter(|&(_, &(_, blend))| blend == BLEND_OP_OVER)
            .map(|(index, _)| index)
            .collect()
    }

    /// fcTLが並べる遅延の分子と分母
    fn frame_delays(bytes: &[u8]) -> Vec<(u16, u16)> {
        /// fcTLの中でdelay_numが始まる位置
        const DELAY_NUM: usize = 20;

        frame_control(bytes, |data| {
            let part = |at: usize| u16::from_be_bytes(data[at..at + 2].try_into().unwrap());
            (part(DELAY_NUM), part(DELAY_NUM + 2))
        })
    }

    /// 並列に圧縮しても、逐次に圧縮した出力とバイト一致する
    ///
    /// 決定も書き出しもフレーム順に進み、待つのは投入済みの番号だけなので、
    /// ワーカー数は出力に現れない。フレーム数を超えるワーカー数も回す。
    #[test]
    fn the_output_does_not_depend_on_the_number_of_workers() {
        let materials = jumping_material().into_iter().chain([resting_material()]);
        for (config, input) in materials {
            let color = config.color_type;
            let expected = encode_with_workers(&input, config, 1);

            for workers in [2, 3, 4, 8, 12, 32] {
                let bytes = encode_with_workers(&input, config, workers);
                assert_eq!(bytes, expected, "{color:?} ワーカー{workers}個の出力");
            }
        }
    }

    /// 素材が決定の経路を踏んでいることを、逐次の出力で確かめる
    ///
    /// 捨てる候補も潰した候補も現れない素材では、ワーカー数の比較が薄いところしか
    /// 通らない。潰した候補が立つのはαを持つ列だけで、そこはαの無い列との違いに
    /// なる。
    #[test]
    fn the_compared_material_exercises_the_decisions() {
        for (config, input) in jumping_material() {
            let color = config.color_type;
            let ops = frame_ops(&encode_with_workers(&input, config, 1));

            assert_eq!(ops.len(), input.len());
            assert!(
                ops.iter()
                    .any(|&(dispose, _)| dispose == DISPOSE_OP_PREVIOUS),
                "{color:?}: 捨てる候補が一度も勝っていない: {ops:?}"
            );
            assert_eq!(
                ops.iter().any(|&(_, blend)| blend == BLEND_OP_OVER),
                matches!(color, ColorType::Rgba8),
                "{color:?}: 潰した候補の立ち方が色種別と合わない: {ops:?}"
            );
        }
    }

    /// 潰した候補は、休みが明けたフレームから採られ始める
    ///
    /// 連敗を積む前半を外した後半だけの列では、先頭を除くすべてのフレームで潰した
    /// 候補が採られる。前半を戻すと同じフレームがblend_op=SOURCEで書かれるので、
    /// 休みの最中に投入した候補を捨てていることと、休みが明ける位置の両方が出る。
    #[test]
    fn the_over_candidate_is_taken_once_the_rest_ends() {
        const {
            assert!(
                DENSE_STARTS_AT + 1 < REST_ENDS_AT,
                "休みの最中に潰した候補が勝つフレームがある"
            );
        }

        let (config, input) = resting_material();
        let winning = input[DENSE_STARTS_AT..].to_vec();
        assert_eq!(
            over_frames(&encode_with_workers(&winning, config, 1)),
            (1..winning.len()).collect::<Vec<_>>(),
            "後半だけの列で潰した候補が採られるフレーム"
        );
        assert_eq!(
            over_frames(&encode_with_workers(&input, config, 1)),
            (REST_ENDS_AT..input.len()).collect::<Vec<_>>(),
            "素材全体で潰した候補が採られるフレーム"
        );
    }

    /// 捨てる候補は、矩形が狭くても圧縮して比べた上で退けられる
    ///
    /// 面積で先に落ちていないことは素材の2つの矩形の広さが示し、退けられたことは
    /// dispose_opの並びが示す。この2つが揃うときだけ、退けた候補の本体が戻る経路を通る。
    #[test]
    fn the_rejected_restore_material_compresses_both_candidates() {
        let (config, input) = rejected_restore_material();
        let layout = Layout::new(WIDTH, HEIGHT, config.color_type).unwrap();
        let rect = |base: &[u8], data: &[u8]| {
            dirty_rect(base, data, layout.stride, layout.bytes_per_pixel).expect("差分がある")
        };

        assert!(
            rect(&input[0], &input[2]).area() < rect(&input[1], &input[2]).area(),
            "捨てた場合の矩形が面積で落ちている"
        );
        let ops = frame_ops(&encode_with_workers(&input, config, 1));
        assert!(
            ops.iter().all(|&(dispose, _)| dispose == DISPOSE_OP_NONE),
            "捨てる候補が採られている: {ops:?}"
        );
    }

    /// 圧縮に配ったバッファは、退けた候補も捨てた候補もパイプラインへ戻る
    ///
    /// 決定は詰め直した候補と、次に決めるフレームの捨てる候補のぶんだけバッファを
    /// 持ち出し、受け取った捨てる候補のぶん — 退けた本体か、候補にしなかった本体 — を
    /// 返す。書き出しは持ち出したぶんと、書き出した本体を返す。休みの最中の候補も
    /// 受け取ってから返すので、出入りの数はどのフレームでも候補の有無だけで決まり、
    /// 書き出し切ると配った数がそのまま戻る。
    ///
    /// 仕掛かりの上限を超えるバッファを先に満たしておく。確保が入れば最後の数がそのぶん
    /// 増える。
    ///
    /// ワーカーは1つに固定し、投入した場で圧縮が済んで領域がその場で戻る経路で数える。
    #[test]
    fn every_compression_buffer_comes_back() {
        /// 先に満たしておくバッファの数
        const PREFILLED: usize = 64;

        let materials = jumping_material()
            .into_iter()
            .chain([resting_material(), rejected_restore_material()]);
        for (config, input) in materials {
            let color = config.color_type;
            let delay = FrameDelay::new(1, 30).unwrap();
            let mut encoder = Encoder::with_workers(
                Cursor::new(Vec::new()),
                WIDTH,
                HEIGHT,
                input.len() as u32,
                config,
                NonZeroUsize::MIN,
            )
            .unwrap();
            for _ in 0..PREFILLED {
                encoder.pipeline.recycle(Vec::new());
            }
            // 決定と書き出しを別々に数えるため、どちらも列へ溜めさせる
            encoder.staged_depth = input.len();
            encoder.pending_depth = NonZeroUsize::new(input.len() + 1).unwrap();
            for frame in &input {
                encoder.add_frame(frame, delay).unwrap();
            }

            // 捨てる候補を投入したフレームだけが、捨てないときの候補を受け取っている
            let prepared = |staged: &VecDeque<Staged>| {
                usize::from(
                    staged
                        .front()
                        .is_some_and(|staged| matches!(staged.kept, Kept::Taken { .. })),
                )
            };
            for index in 0..input.len() {
                let taken = prepared(&encoder.staged);
                let before = encoder.pipeline.pooled();
                encoder.decide().unwrap();
                let submitted = prepared(&encoder.staged);
                let decided = encoder.writing.open.as_ref().expect("決めたフレームが残る");
                let packed = usize::from(matches!(decided.over, Over::Packed(_)));
                assert_eq!(
                    encoder.pipeline.pooled() + packed + submitted,
                    before + taken,
                    "{color:?} フレーム {index}: 決定が持ち出したバッファ"
                );
            }

            encoder.writing.settle(DISPOSE_OP_NONE);
            for index in 0..input.len() {
                let pending = encoder
                    .writing
                    .overflowing(0)
                    .expect("書き出すフレームが残る");
                let packed = usize::from(matches!(pending.frame.over, Over::Packed(_)));
                let before = encoder.pipeline.pooled();
                encoder.write(pending).unwrap();
                assert_eq!(
                    encoder.pipeline.pooled(),
                    before + 1 + packed,
                    "{color:?} フレーム {index}: 書き出しが返したバッファ"
                );
            }
            assert_eq!(
                encoder.pipeline.pooled(),
                PREFILLED,
                "{color:?}: 書き出し切った後に配り直せるバッファ"
            );
        }
    }

    /// 生のフレームの写し先は使い回され、暖機を過ぎると面を確保しない
    ///
    /// 決定を終えて指す先を失った面が1つずつ戻るので、戻り始めた後は配り直しを待つ
    /// 面がフレームごとに1つで、その容量は次の1面を写すのに足りている。戻り始める
    /// までに確保する面の数は、決定を待たせるフレーム数から決まる。
    #[test]
    fn the_frame_buffer_is_handed_back_for_the_next_copy() {
        for (config, input) in jumping_material() {
            let color = config.color_type;
            let delay = FrameDelay::new(1, 30).unwrap();

            for workers in [1, 3] {
                let mut encoder = Encoder::with_workers(
                    Cursor::new(Vec::new()),
                    WIDTH,
                    HEIGHT,
                    input.len() as u32,
                    config,
                    NonZeroUsize::new(workers).unwrap(),
                )
                .unwrap();
                assert_eq!(encoder.workers().get(), workers, "起こしたワーカー数");

                let warm_up = encoder.staged_depth + 2;
                let mut allocated = 0;
                for (index, frame) in input.iter().enumerate() {
                    let spare = encoder.delta.spare();
                    if spare
                        .first()
                        .is_none_or(|face| face.capacity() < frame.len())
                    {
                        allocated += 1;
                    }

                    encoder.add_frame(frame, delay).unwrap();
                    if index < warm_up {
                        continue;
                    }

                    let spare = encoder.delta.spare();
                    assert_eq!(
                        spare.len(),
                        1,
                        "{color:?} ワーカー{workers}個 フレーム {index}: 配り直しを待つ面"
                    );
                    assert!(
                        spare[0].capacity() >= frame.len(),
                        "{color:?} ワーカー{workers}個 フレーム {index}: 写し先の容量が1面に足りない"
                    );
                }
                assert_eq!(
                    allocated,
                    encoder.staged_depth + 3,
                    "{color:?} ワーカー{workers}個: 写し先に確保した面"
                );
            }
        }
    }

    /// フレームは投入された順に書き出される
    ///
    /// 決定も書き出しも列の先頭から取るため、列に何フレーム溜まっていても並びは
    /// 投入の順のまま出る。フレームごとに違う遅延を渡し、fcTLの並びで確かめる。
    #[test]
    fn the_frames_are_written_in_the_order_they_were_added() {
        for (config, input) in jumping_material() {
            let color = config.color_type;
            let delays: Vec<FrameDelay> = (0..input.len())
                .map(|index| FrameDelay::new(index as u32 + 1, 1).unwrap())
                .collect();
            let expected: Vec<(u16, u16)> = (0..input.len())
                .map(|index| (index as u16 + 1, 1))
                .collect();

            for workers in [1, 3] {
                let mut encoder = Encoder::with_workers(
                    Cursor::new(Vec::new()),
                    WIDTH,
                    HEIGHT,
                    input.len() as u32,
                    config,
                    NonZeroUsize::new(workers).unwrap(),
                )
                .unwrap();
                for (frame, delay) in input.iter().zip(&delays) {
                    encoder.add_frame(frame, *delay).unwrap();
                }
                let bytes = encoder.finish().unwrap().into_inner();

                assert_eq!(
                    frame_delays(&bytes),
                    expected,
                    "{color:?} ワーカー{workers}個の書き出し順"
                );
            }
        }
    }

    /// 連敗が閾値に届くまでは候補を立てるのをやめない
    ///
    /// 少ない負けで見切ると勝ち負けの揺れを拾い、まだ採られる素材でも候補が
    /// 立たなくなる。休みに入るのは閾値に届いたときだけで、1回の負けでは入らない。
    #[test]
    fn the_pacing_keeps_trying_below_the_streak() {
        let mut pacing = Pacing::new(BLEND_LOSS_STREAK, BLEND_REST_FRAMES);
        pacing.record(false);
        assert_eq!(pacing.resting(), 0, "1回の負けで休みに入っている");

        for loss in 2..BLEND_LOSS_STREAK {
            assert!(pacing.should_try(), "連敗 {loss} 回目");
            pacing.record(false);
            assert_eq!(pacing.resting(), 0, "連敗 {loss} 回で休みに入っている");
        }
        assert!(pacing.should_try(), "閾値に届く前に休みに入っている");
    }

    /// 投入済みのフレームを決定して書き出し、そのときの間合いを返す
    ///
    /// 決定も書き出しも投入の順に進むため、列に溜めたまま進めた場合と並びは変わらない。
    /// 最後に決定したフレームはdispose_opが次の決定で確定するため、書き出しは
    /// その1つ手前まで届く。
    fn pacing_after_writing<W: Write>(encoder: &mut Encoder<W>) -> &Pacing {
        while !encoder.staged.is_empty() {
            encoder.decide().unwrap();
        }
        while let Some(pending) = encoder.writing.overflowing(1) {
            encoder.write(pending).unwrap();
        }
        &encoder.writing.blend_pacing
    }

    /// 書き出しの経路は、詰め直せなかったフレームでも間合いを1つ進める
    ///
    /// まだらな半透明の画素は不透明でないため詰め直せず、それを塗り潰すフレームだけが
    /// 一様な矩形を詰め直して必ず負ける。連敗が尽きた後は、詰め直せないフレームでも
    /// 休みが1つ減る。
    #[test]
    fn the_write_path_consults_the_pacing() {
        // 交互の列は一様な面から始まり、まだらな半透明の面がその次に来る
        let frames = resting_frames();
        let (uniform, speckled) = (frames[0].clone(), frames[1].clone());

        let mut input = frames[..LOSING_FRAMES].to_vec();
        // 連敗が閾値に届くフレームを書き出しへ届かせる
        input.push(speckled);
        let delay = FrameDelay::new(1, 30).unwrap();

        let mut encoder = Encoder::new(
            Cursor::new(Vec::new()),
            WIDTH,
            HEIGHT,
            input.len() as u32 + 1,
            rgba_config(),
        )
        .unwrap();
        for frame in &input {
            encoder.add_frame(frame, delay).unwrap();
        }
        let blend_pacing = pacing_after_writing(&mut encoder);
        assert_eq!(blend_pacing.resting(), BLEND_REST_FRAMES);

        // 新しく書き出しへ届くのは、詰め直せないまだらなフレーム
        encoder.add_frame(&uniform, delay).unwrap();
        let blend_pacing = pacing_after_writing(&mut encoder);
        assert_eq!(blend_pacing.resting(), BLEND_REST_FRAMES - 1);
    }
}
