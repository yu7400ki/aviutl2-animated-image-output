//! APNGのストリーミング書き出し

use crate::chunk::{
    BLEND_OP_OVER, BLEND_OP_SOURCE, ChunkWriter, DISPOSE_OP_NONE, DISPOSE_OP_PREVIOUS,
};
use crate::codec::{Candidate, Codec};
use crate::delay::FrameDelay;
use crate::delta::Delta;
use crate::diff::{self, Rect};
use crate::error::Error;
use crate::layout::{ColorType, Layout, Output};
use crate::palette::Palette;
use crate::region;
use crate::spool::{Spool, Spooled};
use std::io::Write;
use std::ops::RangeInclusive;

/// [`Config::compression_level`] に指定できる範囲
pub const COMPRESSION_LEVELS: RangeInclusive<u32> = 1..=9;

/// [`Config::max_spool_bytes`] の目安となる値
///
/// PNGの色種別はファイル全体で1つなので、出力の色種別は最後のフレームまで
/// 決まらないことがある。フレームを1回しか取得しない前提では、決まるまでの
/// フレームをエンコーダが抱えることになり、その量は素材の大きさとフレーム数に
/// 比例する。
///
/// この512MiBは、1920x1080のRGBA8 (1フレーム約8.29MB) が全画面差分で続く場合の
/// 64フレーム、30fpsで約2.1秒に相当する。これを超える長さのHD素材では
/// [`Config::reduce_color`] は色種別を落とさず、入力の色種別のまま書き出す。
pub const DEFAULT_MAX_SPOOL_BYTES: usize = 512 << 20;

/// エンコード設定
#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// 入力フレームの色種別
    ///
    /// [`Encoder::add_frame`] に渡すバイト列の解釈を決める。出力の色種別は
    /// [`Config::reduce_color`] によってこれより小さくなることがある。
    pub color_type: ColorType,
    /// deflateの圧縮レベル ([`COMPRESSION_LEVELS`] の範囲)
    pub compression_level: u32,
    /// アニメーションの再生回数 (0で無限ループ)
    pub num_plays: u32,
    /// 出力の色種別を入力より小さいものへ落とすか
    ///
    /// 全フレームの色の和集合がパレットに収まるならパレット参照へ落とす。収まらず
    /// 全画素が不透明なら、アルファを落とした表現と落とさない表現を先頭の何フレームか
    /// 圧縮して比べ、小さい方を採る。PNGの色種別はファイル全体で1つなので、
    /// 落とせるかは最後のフレームまで決まらないことがある。有効にすると、
    /// 決まるまでのフレームをエンコーダ内部に溜める。
    ///
    /// 出力が小さくなるとは限らない。アルファを落とした先には重ねる先が無く、
    /// 変化していない画素を潰す blend_op=OVER の候補が立たなくなる。溜めている間の
    /// フレームも書き出す表現が決まらないため候補を立てられない。どちらもRGBA8の
    /// まま書いた方が小さくなる素材があり、その場合は有効にすると大きくなる。
    pub reduce_color: bool,
    /// 溜めたフレームが抱えるメモリの上限バイト数 ([`DEFAULT_MAX_SPOOL_BYTES`] が目安)
    ///
    /// クロップ済みの画素データに、フレームごとの管理領域を加えた概算で数える。
    /// 超える場合は色種別を落とすのをやめ、入力の色種別のまま書き出す。
    ///
    /// 溜めている間はdispose_opを決められないため、そこまでのフレームは捨てる
    /// 判断を経ずに書き出される。上限に達して落とすのをやめた場合、溜めた区間の
    /// ぶんだけ書き出しが大きくなることがある。
    pub max_spool_bytes: usize,
}

/// 既定は無限ループするRGB8で、圧縮レベルは6、色種別は落とさない
impl Default for Config {
    fn default() -> Self {
        Config {
            color_type: ColorType::Rgb8,
            compression_level: 6,
            num_plays: 0,
            reduce_color: false,
            max_spool_bytes: DEFAULT_MAX_SPOOL_BYTES,
        }
    }
}

/// 出力の色種別を決めるまでに両方の表現で圧縮するフレーム数
///
/// 先頭フレームはキャンバス全体を書くため、差分矩形を書く以降のフレームとは
/// 中身の性質が違う。アルファを落とせるかどうかの傾きはフレームごとの振れが
/// 大きく、行ごとのフィルタを選ぶときより多くの差分矩形を見ないと定まらない。
/// 比べるための圧縮は書き出しに使い回せないため、増やした分だけ丸ごと余分になる。
const COLOR_PROBE_FRAMES: u32 = 8;

/// blend_op=OVERの候補を試すのをやめるまでの連敗数
///
/// 候補が立つかどうかは矩形の中身で決まるため、素材によっては何十フレームも
/// 立ち続けて負け続ける。数フレームで見切ると勝ち負けの揺れを拾ってしまうので、
/// 傾きがはっきりするまでの回数を取る。
const BLEND_LOSS_STREAK: u32 = 6;

/// 連敗した後、blend_op=OVERの候補を立てないフレーム数
///
/// 素材の性質は途中で変わるため、休みを置いてまた試す。長く休むほど圧縮の回数は
/// 減るが、変わり目を見つけるのが遅れる。
const BLEND_REST_FRAMES: u32 = 8;

/// blend_op=OVERの候補を立てるかどうかの間合い
///
/// 候補は圧縮するまで採否が決まらず、負けた側の圧縮はそのまま無駄になる。
/// [`BLEND_LOSS_STREAK`] 回続けて負けたら [`BLEND_REST_FRAMES`] フレーム
/// 立てるのをやめ、休みが明けたらまた試す。一度でも採れば連敗は解ける。
struct BlendPacing {
    /// 採られないまま続いた回数
    losses: u32,
    /// 残りの休みフレーム数
    resting: u32,
}

impl BlendPacing {
    fn new() -> Self {
        BlendPacing {
            losses: 0,
            resting: 0,
        }
    }

    /// 候補を立てるか。休んでいる間は1フレームぶん消費して偽を返す
    fn should_try(&mut self) -> bool {
        if self.resting == 0 {
            return true;
        }
        self.resting -= 1;
        false
    }

    /// 立てた候補が採られたかどうかを記録する
    fn record(&mut self, taken: bool) {
        if taken {
            self.losses = 0;
            return;
        }

        self.losses += 1;
        if self.losses == BLEND_LOSS_STREAK {
            self.losses = 0;
            self.resting = BLEND_REST_FRAMES;
        }
    }
}

/// 溜めたフレームから決まる出力の画素表現
#[derive(Clone, Copy)]
enum Decision {
    /// 溜めたフレームの内容だけで1つに定まった
    Fixed(Output),
    /// アルファを落とす表現と落とさない表現を圧縮して比べ、小さい方に定まった
    Compared(Output),
    /// 溜めきれなくなったため、入力の色種別のままにする
    Abandoned,
}

/// [`Config::reduce_color`] が出力の色種別に及ぼした結果
///
/// 出力の色種別を決めた時点の判断で、それより後のフレームの内容では変わらない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorReduction {
    /// パレット参照へ落とした
    Palette {
        /// パレットに載せた色数
        colors: u16,
    },
    /// アルファを落とした
    AlphaDropped,
    /// 決めた時点で透過する画素が見つかり、アルファを落とせなかった
    AlphaRequired,
    /// 決めた時点で、アルファを落とすと大きくなるため落とさなかった
    AlphaKept,
    /// 落とせる要素が無く、入力の色種別のままにした
    Kept,
    /// 溜めたフレームが上限に達し、解析を打ち切って入力の色種別のままにした
    Abandoned,
}

/// 圧縮後の合計から、アルファを落とすかどうかを決める
///
/// 同じ大きさなら1画素のバイト数が小さいアルファを落とした方を採る。
fn smaller_output(dropped: u64, kept: u64) -> Output {
    if dropped <= kept {
        Output::Rgb8
    } else {
        Output::Rgba8
    }
}

/// 書き出しを待っているフレーム
///
/// フレームのdispose_opは次のフレームの圧縮後サイズを見るまで決まらないため、
/// fcTLを書けるようになるまで1つぶんを保持する。
struct Pending {
    rect: Rect,
    delay: FrameDelay,
    /// キャンバスへ重ねる方法
    blend: u8,
    /// フィルタして圧縮した本体
    body: Vec<u8>,
}

/// エンコーダが進む段階
///
/// PNGの色種別はファイル全体で1つなので、出力の画素表現が決まるまではヘッダを
/// 書けず、候補を書き出す表現で圧縮して比べることもできない。決まった時点で
/// [`Stage::Deciding`] は [`Stage::Streaming`] へ移り、後戻りしない。
enum Stage {
    /// 出力の画素表現が決まるまでフレームを溜めている
    Deciding {
        /// 決まるまでのフレームを溜める領域
        spool: Spool,
        /// アルファを落とすかどうかを圧縮して比べた結果。比べる前は `None`
        alpha_choice: Option<Output>,
    },
    /// ヘッダを書き終え、1フレーム遅れで書き出している
    Streaming {
        /// 出力の画素表現
        output: Output,
        /// 書き出しを待っているフレーム
        pending: Option<Pending>,
    },
}

impl Stage {
    /// 出力の画素表現を確定した直後の、まだ何も保留していない段階
    fn streaming(output: Output) -> Self {
        Stage::Streaming {
            output,
            pending: None,
        }
    }
}

/// APNGエンコーダ
///
/// [`Encoder::add_frame`] でフレームを1つずつ書き出し、[`Encoder::finish`] で終端する。
/// dispose_opは次のフレームの圧縮後サイズを見て決めるため、書き出しは1フレーム遅れる。
/// [`Config::reduce_color`] が有効なときだけ、出力の色種別が決まるまでのフレームを
/// さらに内部へ溜める。
///
/// 溜めるかどうかに関わらず、直前のフレームとそれを描く前のキャンバスの2面を常に抱える
/// (1920x1080のRGBA8で約16.6MB)。
pub struct Encoder<W: Write> {
    /// チャンクを並べる書き出し先
    chunks: ChunkWriter<W>,
    /// キャンバスの大きさと入力フレームのバイト並び
    layout: Layout,
    /// 領域のフィルタと圧縮
    codec: Codec,
    /// 直前のフレームとキャンバスの追跡
    delta: Delta,
    /// 進んでいる段階
    stage: Stage,
    /// blend_op=OVERの候補を立てるかどうかの間合い
    blend_pacing: BlendPacing,
    num_frames: u32,
    num_plays: u32,
    /// [`Self::add_frame`] が受け付けたフレーム数
    frames_accepted: u32,
    /// 書き出しに失敗し、チャンク列が中断しているか
    poisoned: bool,
    /// 出力の色種別を落とした結果。決まるまでは `None`
    reduction: Option<ColorReduction>,
    /// 溜めたバイト数の最大値
    peak_spool_bytes: usize,
}

impl<W: Write> Encoder<W> {
    /// `num_frames` フレームを受け付ける状態にする
    ///
    /// 出力の色種別が最初から決まっていれば、この時点でシグネチャとヘッダを書き出す。
    ///
    /// # Errors
    /// 幅・高さ・フレーム数が0のとき、1フレームのバイト数が `usize` で表現できないとき、
    /// または圧縮レベルが範囲外のとき。
    pub fn new(
        writer: W,
        width: u32,
        height: u32,
        num_frames: u32,
        config: Config,
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

        let stage = if config.reduce_color {
            Stage::Deciding {
                spool: Spool::new(config.max_spool_bytes),
                alpha_choice: None,
            }
        } else {
            Stage::streaming(Output::from(config.color_type))
        };
        let mut encoder = Encoder {
            chunks: ChunkWriter::new(writer),
            layout: Layout::new(width, height, config.color_type)?,
            codec: Codec::new(config.compression_level),
            delta: Delta::new(),
            stage,
            blend_pacing: BlendPacing::new(),
            num_frames,
            num_plays: config.num_plays,
            frames_accepted: 0,
            poisoned: false,
            reduction: None,
            peak_spool_bytes: 0,
        };

        if let Stage::Streaming { output, .. } = &encoder.stage {
            let output = *output;
            let (_, mut parts) = encoder.split();
            parts.write_header(output, None)?;
        }
        Ok(encoder)
    }

    /// 溜めたフレームのバイト数の最大値
    pub fn peak_spool_bytes(&self) -> usize {
        self.peak_spool_bytes
    }

    /// 出力の色種別を落とした結果
    ///
    /// [`Config::reduce_color`] が有効で、出力の色種別が決まった後に `Some` を返す。
    /// 色種別は遅くとも最後のフレームで決まるため、全フレームを投入した後は必ず
    /// `Some` になる。
    pub fn color_reduction(&self) -> Option<ColorReduction> {
        self.reduction
    }

    /// 段階と、段階に依らない部品に分けて借りる
    ///
    /// 段階ごとの値を取り出したまま部品を触れるようにする。
    fn split(&mut self) -> (&mut Stage, Parts<'_, W>) {
        let Encoder {
            chunks,
            layout,
            codec,
            delta,
            stage,
            blend_pacing,
            num_frames,
            num_plays,
            frames_accepted,
            poisoned: _,
            reduction,
            peak_spool_bytes,
        } = self;

        (
            stage,
            Parts {
                chunks,
                layout,
                codec,
                delta,
                blend_pacing,
                reduction,
                peak_spool_bytes,
                num_frames: *num_frames,
                num_plays: *num_plays,
                frames_accepted: *frames_accepted,
            },
        )
    }

    /// フレームを1つ投入する
    ///
    /// `data` は上から下・左から右の順に並んだ `幅 * 高さ * 1画素のバイト数` バイトであること。
    ///
    /// 投入されたフレームはその場では書き出さず、dispose_opが決まる次の呼び出し、
    /// または [`Encoder::finish`] で書き出す。
    ///
    /// # Errors
    /// `data` の長さが合わないとき、宣言したフレーム数を超えたとき、書き出しに失敗したとき、
    /// または過去の書き出し失敗でエンコーダが使用不能なとき。
    ///
    /// 書き出しの失敗は1つ前に投入されたフレームのものになる。最後に投入したフレームの
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

    /// 投入されたフレームを段階に応じて処理し、段階が移ったらそれを覚える
    fn accept(&mut self, data: &[u8], delay: FrameDelay) -> Result<(), Error> {
        let (stage, mut parts) = self.split();

        let next = match stage {
            Stage::Deciding {
                spool,
                alpha_choice,
            } => parts.spool_frame(spool, alpha_choice, data, delay)?,
            Stage::Streaming { output, pending } => {
                parts.stream_frame(*output, pending, data, delay)?;
                None
            }
        };

        if let Some(next) = next {
            *stage = next;
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

        // 次のフレームが無いため、最後のフレームは捨てても復元される先が無い
        let (stage, mut parts) = self.split();
        if let Stage::Streaming { pending, .. } = stage {
            parts.flush_pending(pending, DISPOSE_OP_NONE)?;
        }

        self.chunks.write(*b"IEND", &[])?;
        Ok(self.chunks.into_inner())
    }
}

/// [`Encoder`] から [`Stage`] 以外を借りたもの
///
/// 段階ごとの値は引数で受け取る。フレーム1つを処理する判断と書き出しを担う。
struct Parts<'a, W: Write> {
    chunks: &'a mut ChunkWriter<W>,
    layout: &'a Layout,
    codec: &'a mut Codec,
    delta: &'a mut Delta,
    blend_pacing: &'a mut BlendPacing,
    reduction: &'a mut Option<ColorReduction>,
    peak_spool_bytes: &'a mut usize,
    num_frames: u32,
    num_plays: u32,
    frames_accepted: u32,
}

impl<W: Write> Parts<'_, W> {
    fn write_header(&mut self, output: Output, palette: Option<&Palette>) -> Result<(), Error> {
        self.chunks.write_signature()?;

        let mut ihdr = [0u8; 13];
        ihdr[0..4].copy_from_slice(&self.layout.width.to_be_bytes());
        ihdr[4..8].copy_from_slice(&self.layout.height.to_be_bytes());
        ihdr[8] = 8;
        ihdr[9] = output.code();
        self.chunks.write(*b"IHDR", &ihdr)?;

        let mut actl = [0u8; 8];
        actl[0..4].copy_from_slice(&self.num_frames.to_be_bytes());
        actl[4..8].copy_from_slice(&self.num_plays.to_be_bytes());
        self.chunks.write(*b"acTL", &actl)?;

        // PLTEとtRNSは画素データより前に置く
        if let Some(palette) = palette {
            self.chunks.write(*b"PLTE", &palette.plte())?;
            let trns = palette.trns();
            if !trns.is_empty() {
                self.chunks.write(*b"tRNS", &trns)?;
            }
        }

        Ok(())
    }

    /// 溜めているフレームへ1つ加え、決めたとおりに処理する
    ///
    /// 出力の色種別が決まるまでは、候補を実際に書き出す色種別で圧縮できず大きさを
    /// 比べられない。溜めている間はdispose_opをNONEに固定し、保留を挟まずに溜める。
    /// 決まったら溜めたぶんを流し、書き出しの段階を返す。
    fn spool_frame(
        &mut self,
        spool: &mut Spool,
        alpha_choice: &mut Option<Output>,
        data: &[u8],
        delay: FrameDelay,
    ) -> Result<Option<Stage>, Error> {
        let rect = self
            .delta
            .kept_rect(self.layout, data, self.frames_accepted);
        let region_len = rect.width as usize * rect.height as usize * self.layout.bytes_per_pixel;

        // 抱えきれない大きさが来たら、入力の色種別で確定して溜めたぶんを流す
        if !spool.can_hold(region_len) {
            let output = self.commit(spool, Decision::Abandoned)?;
            self.emit_frame(data, rect, delay, output)?;
            self.delta.advance(data, DISPOSE_OP_NONE);
            return Ok(Some(Stage::streaming(output)));
        }

        spool.push(
            data,
            rect,
            delay,
            self.layout.stride,
            self.layout.bytes_per_pixel,
        );
        *self.peak_spool_bytes = (*self.peak_spool_bytes).max(spool.len());

        let is_last = self.frames_accepted + 1 == self.num_frames;
        let next = match self.decide_output(spool, alpha_choice, is_last) {
            Some(decision) => Some(Stage::streaming(self.commit(spool, decision)?)),
            None => None,
        };
        self.delta.advance(data, DISPOSE_OP_NONE);
        Ok(next)
    }

    /// 溜めたフレームから出力の画素表現を決める
    ///
    /// 色の和集合がパレットに収まっている間は候補が残るため、最後のフレームを見るまで
    /// 決まらない。収まらないと分かった後は、アルファを落とせるかどうかだけが残る。
    /// まだ決まらないときは `None` を返す。
    fn decide_output(
        &mut self,
        spool: &Spool,
        alpha_choice: &mut Option<Output>,
        is_last: bool,
    ) -> Option<Decision> {
        if !spool.colors_exceeded() {
            return is_last.then_some(Decision::Fixed(Output::Indexed8));
        }

        match self.layout.input {
            ColorType::Rgb8 => Some(Decision::Fixed(Output::Rgb8)),
            ColorType::Rgba8 if spool.transparent() => Some(Decision::Fixed(Output::Rgba8)),
            ColorType::Rgba8 => self.decide_alpha(spool, alpha_choice, is_last),
        }
    }

    /// 全画素が不透明なときに、アルファを落とすかどうかを決める
    ///
    /// アルファが定数の列は圧縮がよく効くうえ、落とすと1画素のバイト数が変わって
    /// フィルタの当たり方も変わるため、落とすのが得かどうかは素材によって割れる。
    /// 比べるのは先頭の [`COLOR_PROBE_FRAMES`] フレームまでなので、それだけ溜まれば
    /// 結果は後のフレームで動かない。一度比べた結果を覚えて使い回す。
    ///
    /// アルファを残す側に決まれば、後のフレームに透過が現れてもその判断は覆らないため、
    /// そこで確定して溜めるのをやめられる。落とす側は残りのフレームも不透明である
    /// ことを要するため、最後のフレームまで溜め続ける。
    fn decide_alpha(
        &mut self,
        spool: &Spool,
        alpha_choice: &mut Option<Output>,
        is_last: bool,
    ) -> Option<Decision> {
        let frames = spool.frames();
        if !is_last && frames.len() < COLOR_PROBE_FRAMES as usize {
            return None;
        }

        let output = match *alpha_choice {
            Some(output) => output,
            None => {
                let output = self.choose_output(frames);
                *alpha_choice = Some(output);
                output
            }
        };

        (is_last || output == Output::Rgba8).then_some(Decision::Compared(output))
    }

    /// アルファを落とした表現と落とさない表現を圧縮して比べ、小さい方を採る
    ///
    /// 見るのは先頭の [`COLOR_PROBE_FRAMES`] フレームまで。
    ///
    /// ここでの圧縮はどちらの表現を採るかを決めるためのもので、フィルタ戦略の
    /// プローブには数えない。数えないままなので戦略はまだ固まっておらず、どちらの
    /// 表現も両方の戦略を試した小さい方で比べられる。採る方の表現での圧縮は、
    /// 書き出しのときに改めて行う。
    fn choose_output(&mut self, frames: &[Spooled]) -> Output {
        debug_assert_eq!(self.layout.input, ColorType::Rgba8);
        debug_assert!(self.codec.choice.fixed.is_none());

        let (mut dropped, mut kept) = (0u64, 0u64);
        for frame in frames.iter().take(COLOR_PROBE_FRAMES as usize) {
            let (body, _) = self.compress_spooled(frame, Output::Rgb8, None);
            dropped += body.len() as u64;
            self.codec.give(body);
            let (body, _) = self.compress_spooled(frame, Output::Rgba8, None);
            kept += body.len() as u64;
            self.codec.give(body);
        }

        smaller_output(dropped, kept)
    }

    /// 出力の画素表現を確定し、ヘッダに続けて溜めたフレームを書き出す
    ///
    /// パレット参照へ落とした場合の添字と色の対応は、ヘッダと溜めたフレームを
    /// 書き終えるまでしか要らない。出力がパレット参照に決まるのは最後のフレームで、
    /// 以降のフレームは来ないため、書き出しへ移った後に引くことがない。
    fn commit(&mut self, spool: &mut Spool, decision: Decision) -> Result<Output, Error> {
        let (frames, colors) = spool.drain();
        let output = match decision {
            Decision::Fixed(output) | Decision::Compared(output) => output,
            Decision::Abandoned => Output::from(self.layout.input),
        };

        let color_count = colors.len();
        let palette = (output == Output::Indexed8).then(|| colors.into_palette());
        *self.reduction = Some(match decision {
            Decision::Abandoned => ColorReduction::Abandoned,
            Decision::Compared(Output::Rgb8) => ColorReduction::AlphaDropped,
            // 圧縮して比べた結果なので、残った理由は落とすと大きくなること
            Decision::Compared(_) => ColorReduction::AlphaKept,
            // 溜めた内容だけで定まる先は、パレットか透過を含むRGBAか入力そのもの
            Decision::Fixed(_) => match output {
                Output::Indexed8 => ColorReduction::Palette {
                    colors: color_count,
                },
                Output::Rgba8 => ColorReduction::AlphaRequired,
                Output::Rgb8 => ColorReduction::Kept,
            },
        });
        self.write_header(output, palette.as_ref())?;

        for frame in &frames {
            let (body, probe) = self.compress_spooled(frame, output, palette.as_ref());
            self.codec.record(probe);
            // 溜めている間は出力の色種別が決まらず、blend_opの候補も圧縮できない
            self.chunks.write_frame(
                frame.rect,
                frame.delay,
                DISPOSE_OP_NONE,
                BLEND_OP_SOURCE,
                &body,
            )?;
            self.codec.give(body);
        }

        Ok(output)
    }

    /// 溜めるのをやめたフレームを1つ書き出す
    ///
    /// 溜めている間はdispose_opを決められず、blend_opの候補も圧縮できない。
    fn emit_frame(
        &mut self,
        data: &[u8],
        rect: Rect,
        delay: FrameDelay,
        output: Output,
    ) -> Result<(), Error> {
        let (body, probe) = self.compress_rect(data, rect, output);
        self.codec.record(probe);
        self.chunks
            .write_frame(rect, delay, DISPOSE_OP_NONE, BLEND_OP_SOURCE, &body)?;
        self.codec.give(body);
        Ok(())
    }

    /// 保留中のフレームを書き出し、投入されたフレームを保留にする
    fn stream_frame(
        &mut self,
        output: Output,
        pending: &mut Option<Pending>,
        data: &[u8],
        delay: FrameDelay,
    ) -> Result<(), Error> {
        let (dispose, rect, candidate) = self.choose_dispose(data, output, pending.is_some());
        let (blend, (body, probe)) = self.choose_blend(data, dispose, rect, candidate, output);
        self.codec.record(probe);

        self.flush_pending(pending, dispose)?;
        *pending = Some(Pending {
            rect,
            delay,
            blend,
            body,
        });
        self.delta.advance(data, dispose);
        Ok(())
    }

    /// 保留中のフレームのdispose_opと、投入されたフレームの矩形を決める
    ///
    /// 保留中のフレームをdispose_op=PREVIOUSで捨てると、投入されたフレームは
    /// それを描く直前のキャンバスとの差分になる。両方の候補を圧縮して小さい方を採り、
    /// 採った側を戻り値へ残して、退けた側のバッファはプールへ返す。同じ大きさなら捨てない。
    fn choose_dispose(
        &mut self,
        data: &[u8],
        output: Output,
        disposable: bool,
    ) -> (u8, Rect, Candidate) {
        let kept = self
            .delta
            .kept_rect(self.layout, data, self.frames_accepted);
        let restored =
            self.delta
                .restored_rect(self.layout, data, kept, self.frames_accepted, disposable);

        let kept_candidate = self.compress_rect(data, kept, output);
        let Some(restored) = restored else {
            return (DISPOSE_OP_NONE, kept, kept_candidate);
        };

        let restored_candidate = self.compress_rect(data, restored, output);

        if restored_candidate.0.len() < kept_candidate.0.len() {
            self.codec.give(kept_candidate.0);
            (DISPOSE_OP_PREVIOUS, restored, restored_candidate)
        } else {
            self.codec.give(restored_candidate.0);
            (DISPOSE_OP_NONE, kept, kept_candidate)
        }
    }

    /// 投入されたフレームをキャンバスへ重ねる方法を決める
    ///
    /// 矩形の中で変化した画素がすべて不透明なら、変化していない画素を完全な透明へ
    /// 潰した候補が立つ。blend_op=OVERはその画素でキャンバスを残すため、潰しても
    /// 元の値に戻る。`source` の候補と両方を圧縮して小さい方を採り、採った側を戻り値へ
    /// 残して、退けた側のバッファはプールへ返す。同じ大きさならSOURCEを採る。
    ///
    /// アルファを持たない出力には重ねる先が無いため、候補が立つのは出力がRGBA8のとき
    /// だけになる。先頭フレームはキャンバスがまだ空で、重ねる先が無い。負けが続く間は
    /// [`BlendPacing`] が候補を立てるのを休ませる。
    fn choose_blend(
        &mut self,
        data: &[u8],
        dispose: u8,
        rect: Rect,
        source: Candidate,
        output: Output,
    ) -> (u8, Candidate) {
        if output != Output::Rgba8 || self.frames_accepted == 0 {
            return (BLEND_OP_SOURCE, source);
        }
        if !self.blend_pacing.should_try() {
            return (BLEND_OP_SOURCE, source);
        }

        // 保留中のフレームを捨てると、キャンバスはそれを描く直前の内容へ戻る
        let base = if dispose == DISPOSE_OP_NONE {
            &self.delta.previous
        } else {
            &self.delta.canvas
        };
        let mut over = self.codec.take();
        let packed = diff::pack_over(base, data, self.layout.stride, rect, &mut over);
        if !packed {
            self.codec.give(over);
            return (BLEND_OP_SOURCE, source);
        }

        let out_bpp = output.bytes_per_pixel();
        let over_candidate = self
            .codec
            .compress(&over, rect.width as usize * out_bpp, out_bpp);
        self.codec.give(over);

        let taken = over_candidate.0.len() < source.0.len();
        self.blend_pacing.record(taken);
        if taken {
            self.codec.give(source.0);
            (BLEND_OP_OVER, over_candidate)
        } else {
            self.codec.give(over_candidate.0);
            (BLEND_OP_SOURCE, source)
        }
    }

    /// 保留中のフレームを `dispose` で書き出す
    fn flush_pending(&mut self, pending: &mut Option<Pending>, dispose: u8) -> Result<(), Error> {
        let Some(pending) = pending.take() else {
            return Ok(());
        };

        self.chunks.write_frame(
            pending.rect,
            pending.delay,
            dispose,
            pending.blend,
            &pending.body,
        )?;
        self.codec.give(pending.body);
        Ok(())
    }

    /// フレームから `rect` を切り出してフィルタして圧縮する
    ///
    /// この経路を通るのは出力が決まった後のフレームだけで、その表現は入力と同じか
    /// アルファを落としたものになる。
    fn compress_rect(&mut self, data: &[u8], rect: Rect, output: Output) -> Candidate {
        debug_assert_ne!(output, Output::Indexed8);

        let stride = self.layout.stride;
        let in_bpp = self.layout.bytes_per_pixel;
        let out_bpp = output.bytes_per_pixel();
        let region_stride = rect.width as usize * out_bpp;

        if in_bpp == out_bpp && rect.width as usize * in_bpp == stride {
            // 変換の要らない全幅の矩形は `data` 上で既に連続している
            let head = rect.y as usize * stride;
            let len = region_stride * rect.height as usize;
            self.codec
                .compress(&data[head..head + len], region_stride, out_bpp)
        } else {
            let mut cropped = self.codec.take();
            region::crop(data, rect, stride, in_bpp, out_bpp, &mut cropped);
            let candidate = self.codec.compress(&cropped, region_stride, out_bpp);
            self.codec.give(cropped);
            candidate
        }
    }

    /// 溜めたフレームを `output` の表現へ直してフィルタして圧縮する
    fn compress_spooled(
        &mut self,
        frame: &Spooled,
        output: Output,
        palette: Option<&Palette>,
    ) -> Candidate {
        let out_bpp = output.bytes_per_pixel();
        let region_stride = frame.rect.width as usize * out_bpp;

        if self.layout.bytes_per_pixel == out_bpp {
            return self.codec.compress(&frame.data, region_stride, out_bpp);
        }

        let mut converted = self.codec.take();
        self.append_output(&frame.data, output, palette, &mut converted);
        let candidate = self.codec.compress(&converted, region_stride, out_bpp);
        self.codec.give(converted);
        candidate
    }

    /// 入力の画素列を `output` の表現へ直しながら `out` へ追記する
    fn append_output(
        &self,
        pixels: &[u8],
        output: Output,
        palette: Option<&Palette>,
        out: &mut Vec<u8>,
    ) {
        let in_bpp = self.layout.bytes_per_pixel;
        match palette {
            Some(palette) => palette.append_indices(pixels, in_bpp, out),
            None => region::append_pixels(pixels, in_bpp, output.bytes_per_pixel(), out),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk;
    use crate::codec::PROBE_FRAMES;
    use crate::testing::noise;
    use flate2::read::ZlibDecoder;
    use std::io::Read;

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

    /// 少数の色を秩序ディザで敷き、微小なノイズを載せたフレーム
    ///
    /// 同じ色が短い周期で並び直すため、アルファを落として1画素のバイト数を
    /// 変えるとかえって大きくなる。
    fn dithered_frame(seed: u32) -> Vec<u8> {
        /// 4x4の閾値行列
        const BAYER: [[usize; 4]; 4] =
            [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
        /// ディザで敷き分ける色数
        const COLORS: usize = 24;

        let grain = noise((WIDTH * HEIGHT) as usize, seed);
        let mut frame = Vec::new();
        for y in 0..HEIGHT as usize {
            for x in 0..WIDTH as usize {
                let shade = (x + seed as usize) * 255 / WIDTH as usize + y * 97 / HEIGHT as usize;
                let threshold = BAYER[y % 4][(x + seed as usize) % 4] * 4;
                let index = (shade + threshold) / (256 / COLORS) % COLORS;
                let base = (index * 251 + seed as usize * 37) as u8;
                let grit = grain[y * WIDTH as usize + x] & 15;
                frame.extend_from_slice(&[
                    base.wrapping_add(grit),
                    base.wrapping_mul(3),
                    base.wrapping_add(88).wrapping_add(grit),
                ]);
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

    fn encode(input: &[Vec<u8>], config: Config) -> Vec<u8> {
        let mut encoder =
            Encoder::new(Vec::new(), WIDTH, HEIGHT, input.len() as u32, config).unwrap();
        for frame in input {
            encoder
                .add_frame(frame, FrameDelay::new(1, 30).unwrap())
                .unwrap();
        }
        encoder.finish().unwrap()
    }

    fn rgb_config() -> Config {
        Config {
            color_type: ColorType::Rgb8,
            ..Config::default()
        }
    }

    /// IHDRが示す出力の1画素あたりのバイト数
    fn output_bytes_per_pixel(bytes: &[u8]) -> usize {
        // シグネチャ・長さ・型に続くIHDRの9バイト目がcolour type
        let code = bytes[chunk::SIGNATURE.len() + 8 + 9];
        match code {
            2 => 3,
            3 => 1,
            6 => 4,
            other => panic!("扱わないcolour type: {other}"),
        }
    }

    /// フレームごとの、圧縮した本体のバイト数
    fn body_lengths(bytes: &[u8]) -> Vec<usize> {
        let mut lengths = Vec::new();
        let mut offset = chunk::SIGNATURE.len();

        while offset + 12 <= bytes.len() {
            let len = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
            match &bytes[offset + 4..offset + 8] {
                b"IDAT" => lengths.push(len),
                // fdATは先頭4バイトが連番
                b"fdAT" => lengths.push(len - 4),
                _ => {}
            }
            offset += 12 + len;
        }

        lengths
    }

    /// フレームごとのフィルタ種別バイト
    fn filter_types(bytes: &[u8], bpp: usize) -> Vec<Vec<u8>> {
        written_frames(bytes)
            .iter()
            .map(|frame| frame.filter_types(bpp))
            .collect()
    }

    /// フィルタを掛けない方が小さい素材は、プローブ中のフレームも含めてNoneだけになる
    #[test]
    fn a_flat_source_settles_on_the_unfiltered_strategy() {
        let input: Vec<Vec<u8>> = (0..PROBE_FRAMES + 4).map(flat_frame).collect();
        let bytes = encode(&input, rgb_config());

        let types = filter_types(&bytes, 3);
        assert_eq!(types.len(), input.len());
        for (index, frame) in types.iter().enumerate() {
            assert!(frame.iter().all(|&f| f == 0), "フレーム {index}: {frame:?}");
        }
    }

    /// 適応フィルタが効く素材は、プローブ中のフレームからNone以外を選ぶ
    #[test]
    fn a_detailed_source_settles_on_the_adaptive_strategy() {
        let input: Vec<Vec<u8>> = (0..PROBE_FRAMES + 4).map(detailed_frame).collect();
        let bytes = encode(&input, rgb_config());

        let types = filter_types(&bytes, 3);
        assert_eq!(types.len(), input.len());
        for (index, frame) in types.iter().enumerate() {
            assert!(frame.iter().any(|&f| f != 0), "フレーム {index}: {frame:?}");
        }
    }

    /// 固めた戦略は、プローブ後に素材が変わっても変わらない
    #[test]
    fn the_strategy_stays_fixed_after_the_probe() {
        let mut input: Vec<Vec<u8>> = (0..PROBE_FRAMES).map(detailed_frame).collect();
        input.extend((0..4).map(flat_frame));
        let bytes = encode(&input, rgb_config());

        for (index, frame) in filter_types(&bytes, 3)
            .iter()
            .enumerate()
            .skip(PROBE_FRAMES as usize)
        {
            assert!(frame.iter().any(|&f| f != 0), "フレーム {index}: {frame:?}");
        }

        let mut input: Vec<Vec<u8>> = (0..PROBE_FRAMES).map(flat_frame).collect();
        input.extend((0..4).map(detailed_frame));
        let bytes = encode(&input, rgb_config());

        for (index, frame) in filter_types(&bytes, 3)
            .iter()
            .enumerate()
            .skip(PROBE_FRAMES as usize)
        {
            assert!(frame.iter().all(|&f| f == 0), "フレーム {index}: {frame:?}");
        }
    }

    /// dispose_opの候補を2つ圧縮しても、プローブは1フレームにつき1回しか進まない
    ///
    /// 3フレーム目は先頭フレームと同じ内容なので捨てる候補が立ち、両方が圧縮される。
    /// 二重に数えるとプローブが1フレーム早く尽き、4フレーム目が固めた戦略で書かれる。
    #[test]
    fn dispose_candidates_do_not_consume_extra_probes() {
        let input = vec![
            detailed_frame(0),
            detailed_frame(1),
            detailed_frame(0),
            flat_frame(0),
            flat_frame(1),
        ];
        let bytes = encode(&input, rgb_config());

        let types = filter_types(&bytes, 3);
        assert_eq!(types.len(), input.len());
        // プローブの最後の1回に入るため、フィルタを掛けない方が小さいこのフレームはNoneだけになる
        assert!(types[3].iter().all(|&f| f == 0), "{:?}", types[3]);
        // 固めた戦略は適応フィルタなので、同じ素材でもNone以外を選ぶ
        assert!(types[4].iter().any(|&f| f != 0), "{:?}", types[4]);
    }

    /// プローブが終わらないまま入力が尽きても、フレームはすべて書き出される
    #[test]
    fn an_input_shorter_than_the_probe_is_written_in_full() {
        let input: Vec<Vec<u8>> = (0..PROBE_FRAMES - 1).map(flat_frame).collect();
        let bytes = encode(&input, rgb_config());

        let types = filter_types(&bytes, 3);
        assert_eq!(types.len(), input.len());
        for (index, frame) in types.iter().enumerate() {
            assert!(frame.iter().all(|&f| f == 0), "フレーム {index}: {frame:?}");
        }
    }

    /// 色種別を落とす経路でも、溜めたフレームがプローブを通って戦略が決まる
    #[test]
    fn spooled_frames_go_through_the_probe() {
        let config = Config {
            color_type: ColorType::Rgba8,
            reduce_color: true,
            ..Config::default()
        };

        let input: Vec<Vec<u8>> = (0..PROBE_FRAMES + 4)
            .map(|seed| with_alpha(&flat_frame(seed)))
            .collect();
        let bytes = encode(&input, config);
        let bpp = output_bytes_per_pixel(&bytes);
        for (index, frame) in filter_types(&bytes, bpp).iter().enumerate() {
            assert!(frame.iter().all(|&f| f == 0), "フレーム {index}: {frame:?}");
        }

        let input: Vec<Vec<u8>> = (0..PROBE_FRAMES + 4)
            .map(|seed| with_alpha(&detailed_frame(seed)))
            .collect();
        let bytes = encode(&input, config);
        let bpp = output_bytes_per_pixel(&bytes);
        for (index, frame) in filter_types(&bytes, bpp).iter().enumerate() {
            assert!(frame.iter().any(|&f| f != 0), "フレーム {index}: {frame:?}");
        }
    }

    /// 溜めたフレームで固めた戦略は、書き出しへ移った後も変わらない
    ///
    /// 透過が見つかった時点で溜めたぶんがまとめて圧縮されるため、そこでプローブが
    /// 尽きる。以降のフレームは書き出し経路を通る。
    #[test]
    fn the_strategy_fixed_by_spooled_frames_stays_fixed() {
        let config = Config {
            color_type: ColorType::Rgba8,
            reduce_color: true,
            ..Config::default()
        };

        let mut input: Vec<Vec<u8>> = (0..PROBE_FRAMES)
            .map(|seed| with_alpha(&flat_frame(seed)))
            .collect();
        input.last_mut().expect("フレームがある")[3] = 0x80;
        input.extend((0..4).map(|seed| with_alpha(&detailed_frame(seed))));

        let bytes = encode(&input, config);
        let types = filter_types(&bytes, 4);
        assert_eq!(types.len(), input.len());
        for (index, frame) in types.iter().enumerate().skip(PROBE_FRAMES as usize) {
            assert!(frame.iter().all(|&f| f == 0), "フレーム {index}: {frame:?}");
        }
    }

    /// アルファを保つ設定
    fn rgba_config() -> Config {
        Config {
            color_type: ColorType::Rgba8,
            ..Config::default()
        }
    }

    /// 1行おきに `other` の内容へ差し替えたフレーム
    ///
    /// 矩形はキャンバス全体に広がり、その中の半分の画素が変化しない。
    fn interleaved(base: &[u8], other: &[u8], bpp: usize) -> Vec<u8> {
        let stride = WIDTH as usize * bpp;
        let mut frame = base.to_vec();
        for y in (1..HEIGHT as usize).step_by(2) {
            let row = y * stride..(y + 1) * stride;
            frame[row.clone()].copy_from_slice(&other[row]);
        }
        frame
    }

    /// blend_opの候補を2つ圧縮しても、プローブは1フレームにつき1回しか進まない
    ///
    /// 2フレーム目は矩形の半分が変化しないため、潰した候補が立って両方が圧縮される。
    /// 二重に数えるとプローブが1フレーム早く尽き、4フレーム目が固めた戦略で書かれる。
    #[test]
    fn blend_candidates_do_not_consume_extra_probes() {
        let base = with_alpha(&detailed_frame(0));
        let input = vec![
            interleaved(&base, &with_alpha(&detailed_frame(1)), 4),
            base,
            with_alpha(&detailed_frame(2)),
            with_alpha(&flat_frame(0)),
            with_alpha(&flat_frame(1)),
        ];
        let bytes = encode(&input, rgba_config());

        let types = filter_types(&bytes, 4);
        assert_eq!(types.len(), input.len());
        // プローブの最後の1回に入るため、フィルタを掛けない方が小さいこのフレームはNoneだけになる
        assert!(types[3].iter().all(|&f| f == 0), "{:?}", types[3]);
        // 固めた戦略は適応フィルタなので、同じ素材でもNone以外を選ぶ
        assert!(types[4].iter().any(|&f| f != 0), "{:?}", types[4]);
    }

    /// 色種別を落とす設定
    fn reduce_rgba_config() -> Config {
        Config {
            color_type: ColorType::Rgba8,
            reduce_color: true,
            ..Config::default()
        }
    }

    /// 全画素が不透明でも、アルファを落として小さくなる素材だけが落とされる
    #[test]
    fn the_smaller_of_the_two_opaque_representations_is_written() {
        let input: Vec<Vec<u8>> = (0..COLOR_PROBE_FRAMES + 2)
            .map(|seed| with_alpha(&detailed_frame(seed)))
            .collect();
        let bytes = encode(&input, reduce_rgba_config());
        assert_eq!(output_bytes_per_pixel(&bytes), 3);

        let input: Vec<Vec<u8>> = (0..COLOR_PROBE_FRAMES + 2)
            .map(|seed| with_alpha(&dithered_frame(seed)))
            .collect();
        let bytes = encode(&input, reduce_rgba_config());
        assert_eq!(output_bytes_per_pixel(&bytes), 4);
    }

    /// 比べるのは先頭のフレームで、後ろのフレームは色種別を動かさない
    ///
    /// 見る位置がずれると、比べたかったフレームの性質が結果に出なくなる。
    #[test]
    fn the_color_types_are_compared_over_the_leading_frames() {
        let span = COLOR_PROBE_FRAMES;

        let mut frames: Vec<Vec<u8>> = (0..span).map(dithered_frame).collect();
        frames.extend((0..span).map(detailed_frame));
        let input: Vec<Vec<u8>> = frames.iter().map(|frame| with_alpha(frame)).collect();
        let bytes = encode(&input, reduce_rgba_config());
        assert_eq!(output_bytes_per_pixel(&bytes), 4);

        let mut frames: Vec<Vec<u8>> = (0..span).map(detailed_frame).collect();
        frames.extend((0..span).map(dithered_frame));
        let input: Vec<Vec<u8>> = frames.iter().map(|frame| with_alpha(frame)).collect();
        let bytes = encode(&input, reduce_rgba_config());
        assert_eq!(output_bytes_per_pixel(&bytes), 3);
    }

    /// 色種別の候補を2つ圧縮しても、プローブは1フレームにつき1回しか進まない
    ///
    /// 二重に数えるとプローブが尽きるのが早まり、書き出しの先頭から固めた戦略になる。
    #[test]
    fn color_candidates_do_not_consume_extra_probes() {
        let mut frames: Vec<Vec<u8>> = (0..PROBE_FRAMES - 1).map(detailed_frame).collect();
        frames.extend((0..2).map(flat_frame));
        let input: Vec<Vec<u8>> = frames.iter().map(|frame| with_alpha(frame)).collect();

        let bytes = encode(&input, reduce_rgba_config());
        let types = filter_types(&bytes, output_bytes_per_pixel(&bytes));
        assert_eq!(types.len(), input.len());
        // プローブの最後の1回に入るため、フィルタを掛けない方が小さいこのフレームはNoneだけになる
        assert!(types[3].iter().all(|&f| f == 0), "{:?}", types[3]);
        // 固めた戦略は適応フィルタなので、同じ素材でもNone以外を選ぶ
        assert!(types[4].iter().any(|&f| f != 0), "{:?}", types[4]);
    }

    /// 圧縮後の合計が同じならアルファを落とす
    #[test]
    fn a_tie_drops_the_alpha() {
        assert_eq!(smaller_output(64, 64), Output::Rgb8);
        assert_eq!(smaller_output(63, 64), Output::Rgb8);
        assert_eq!(smaller_output(65, 64), Output::Rgba8);
    }

    /// プローブに記録するのは、書き出す候補を圧縮したときのバイト数
    ///
    /// 2フレーム目は矩形の中身が一様になり、潰した候補は周期的な穴が空くぶん大きい。
    /// 採らなかった候補を記録すると、以降の戦略が書き出していない大きさで決まる。
    #[test]
    fn the_probe_records_the_candidate_that_is_written() {
        const PIXELS: usize = (WIDTH * HEIGHT) as usize;
        /// まだらに置き換える画素の間隔
        const STEP: usize = 7;

        let uniform = |value: u8| with_alpha(&vec![value; PIXELS * 3]);
        let mut speckled = uniform(0x30);
        for pixel in (0..PIXELS).step_by(STEP) {
            speckled[pixel * 4..pixel * 4 + 4].copy_from_slice(&[0xC0, 0xB0, 0xA0, 0xFF]);
        }
        let input = vec![speckled, uniform(0x30), uniform(0x50), uniform(0x70)];
        assert_eq!(input.len(), PROBE_FRAMES as usize);

        let mut encoder =
            Encoder::new(Vec::new(), WIDTH, HEIGHT, input.len() as u32, rgba_config()).unwrap();
        let mut recorded = Vec::new();
        let mut totals = (0u64, 0u64);
        for frame in &input {
            encoder
                .add_frame(frame, FrameDelay::new(1, 30).unwrap())
                .unwrap();
            let choice = &encoder.codec.choice;
            recorded.push((
                (choice.adaptive_bytes - totals.0) as usize,
                (choice.unfiltered_bytes - totals.1) as usize,
            ));
            totals = (choice.adaptive_bytes, choice.unfiltered_bytes);
        }
        let bytes = encoder.finish().unwrap();

        let bodies = body_lengths(&bytes);
        assert_eq!(bodies.len(), input.len());
        for (index, ((adaptive, unfiltered), body)) in recorded.iter().zip(&bodies).enumerate() {
            assert_eq!(adaptive.min(unfiltered), body, "フレーム {index}");
        }
    }

    /// 連敗が続くと候補を立てるのを休み、休みが明けたらまた試す
    #[test]
    fn the_pacing_rests_after_a_streak_of_losses() {
        let mut pacing = BlendPacing::new();
        for _ in 0..BLEND_LOSS_STREAK {
            assert!(pacing.should_try());
            pacing.record(false);
        }

        for frame in 0..BLEND_REST_FRAMES {
            assert!(!pacing.should_try(), "休み {frame} フレーム目");
        }
        assert!(pacing.should_try());
    }

    /// 書き出しの経路は、候補を立てる前に間合いを見る
    ///
    /// まだらな半透明の画素は不透明でないため候補が立たず、それを塗り潰すフレームだけが
    /// 一様な矩形の候補を立てて必ず負ける。連敗が尽きた後は、フレームごとに休みが減る。
    #[test]
    fn the_write_path_consults_the_pacing() {
        const PIXELS: usize = (WIDTH * HEIGHT) as usize;
        /// まだらに置き換える画素の間隔
        const STEP: usize = 7;

        let uniform = with_alpha(&vec![0x30u8; PIXELS * 3]);
        let mut speckled = uniform.clone();
        for pixel in (0..PIXELS).step_by(STEP) {
            speckled[pixel * 4..pixel * 4 + 4].copy_from_slice(&[0xC0, 0xB0, 0xA0, 0x80]);
        }

        let mut input = vec![uniform.clone()];
        for _ in 0..BLEND_LOSS_STREAK {
            input.push(speckled.clone());
            input.push(uniform.clone());
        }
        let delay = FrameDelay::new(1, 30).unwrap();

        let mut encoder = Encoder::new(
            Vec::new(),
            WIDTH,
            HEIGHT,
            input.len() as u32 + 1,
            rgba_config(),
        )
        .unwrap();
        for frame in &input {
            encoder.add_frame(frame, delay).unwrap();
        }
        assert_eq!(encoder.blend_pacing.resting, BLEND_REST_FRAMES);

        encoder.add_frame(&speckled, delay).unwrap();
        assert_eq!(encoder.blend_pacing.resting, BLEND_REST_FRAMES - 1);
    }

    /// 候補が採られると連敗は解ける
    #[test]
    fn a_taken_candidate_clears_the_losses() {
        let mut pacing = BlendPacing::new();
        for _ in 0..BLEND_LOSS_STREAK - 1 {
            pacing.record(false);
        }
        pacing.record(true);

        for _ in 0..BLEND_LOSS_STREAK - 1 {
            assert!(pacing.should_try());
            pacing.record(false);
        }
        assert!(pacing.should_try());
    }
}
