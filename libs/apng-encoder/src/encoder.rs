//! APNGのストリーミング書き出し

use crate::chunk::{
    BLEND_OP_OVER, BLEND_OP_SOURCE, ChunkWriter, DISPOSE_OP_NONE, DISPOSE_OP_PREVIOUS,
};
use crate::codec::{Candidate, Codec};
use crate::delta::Delta;
use crate::error::Error;
use crate::layout::{ColorType, Layout, Output};
use crate::over;
use crate::palette::{PLTE_PLACEHOLDER, Palette, TRNS_PLACEHOLDER};
use anim_core::{Colors, FrameDelay, Rect, crop};
use std::io::{Seek, Write};
use std::ops::RangeInclusive;

/// [`Config::compression_level`] に指定できる範囲
pub const COMPRESSION_LEVELS: RangeInclusive<u32> = 1..=9;

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
    /// 出力の色種別をパレット参照へ落とすか
    ///
    /// PNGの色種別はファイル全体で1つなので、先頭フレームの色数だけで決める。
    /// 先頭フレームの矩形はキャンバス全体なので、そこで色数がパレットに収まらなければ
    /// 全フレームの和集合も収まらない。収まらなければ入力の色種別のまま書き出す。
    ///
    /// 収まったらパレット参照で書き出しを始める。後のフレームで色数が溢れたときは
    /// [`Error::ColorLimitExceeded`] で失敗する。
    pub reduce_color: bool,
}

/// 既定は無限ループするRGB8で、圧縮レベルは6、色種別は落とさない
impl Default for Config {
    fn default() -> Self {
        Config {
            color_type: ColorType::Rgb8,
            compression_level: 6,
            num_plays: 0,
            reduce_color: false,
        }
    }
}

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

/// [`Config::reduce_color`] が出力の色種別に及ぼした結果
///
/// 先頭フレームの色数で決まり、それより後のフレームの内容では変わらない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorReduction {
    /// パレット参照へ落とした
    Palette {
        /// パレットに載せた色数
        colors: u16,
    },
    /// 先頭フレームの色数が多く、入力の色種別のままにした
    Kept,
}

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

/// 出力の画素表現と、パレット参照ならその添字の表
enum Encoding {
    /// 入力の色種別のまま書き出す
    Direct(Output),
    /// 見つけた順に振った添字で書き出す
    Indexed(Palette),
}

impl Encoding {
    /// 出力の画素表現
    fn output(&self) -> Output {
        match self {
            Encoding::Direct(output) => *output,
            Encoding::Indexed(_) => Output::Indexed8,
        }
    }

    /// blend_op=OVERの候補で、変化しなかった画素を潰す書き方
    ///
    /// 潰した画素はアルファを0で書き、キャンバスをそのまま残す。RGBA8の出力は
    /// 完全に透明な画素をいつでも書け、パレット参照はアルファが0の色に添字が
    /// 振られてから書ける。
    fn collapse(&self) -> Option<Collapse<'_>> {
        match self {
            Encoding::Direct(Output::Rgba8) => Some(Collapse::Transparent),
            Encoding::Direct(_) => None,
            Encoding::Indexed(palette) => Some(Collapse::Index(palette, palette.transparent()?)),
        }
    }
}

/// blend_op=OVERの候補で、変化しなかった画素へ書く値
enum Collapse<'a> {
    /// 完全に透明なRGBA8の画素
    Transparent,
    /// アルファが0の色を指す添字
    Index(&'a Palette, u8),
}

/// 書き出しが持ち越す状態
///
/// 先頭フレームで出力の画素表現が決まってから作られ、以降のフレームはこれに従う。
struct Writing {
    /// 出力の画素表現
    encoding: Encoding,
    /// 書き出しを待っているフレーム
    pending: Option<Pending>,
    /// blend_op=OVERの候補を立てるかどうかの間合い
    blend_pacing: BlendPacing,
}

impl Writing {
    /// 出力の画素表現を確定した直後の、まだ何も保留していない状態
    fn new(encoding: Encoding) -> Self {
        Writing {
            encoding,
            pending: None,
            blend_pacing: BlendPacing::new(),
        }
    }
}

/// APNGエンコーダ
///
/// [`Encoder::add_frame`] でフレームを1つずつ書き出し、[`Encoder::finish`] で終端する。
/// dispose_opは次のフレームの圧縮後サイズを見て決めるため、書き出しは1フレーム遅れる。
///
/// 直前のフレームとそれを描く前のキャンバスの2面を常に抱える
/// (1920x1080のRGBA8で約16.6MB)。フレームは投入された順にそのまま書き出す。
pub struct Encoder<W: Write + Seek> {
    /// チャンクを並べる書き出し先
    chunks: ChunkWriter<W>,
    /// キャンバスの大きさと入力フレームのバイト並び
    layout: Layout,
    /// 領域のフィルタと圧縮
    codec: Codec,
    /// 直前のフレームとキャンバスの追跡
    delta: Delta,
    /// 書き出しの状態。先頭フレームで出力の画素表現が決まるまでは `None`
    writing: Option<Writing>,
    /// 出力の色種別をパレット参照へ落とすよう求められたか
    reduce_color: bool,
    num_frames: u32,
    num_plays: u32,
    /// [`Self::add_frame`] が受け付けたフレーム数
    frames_accepted: u32,
    /// 書き出しに失敗し、チャンク列が中断しているか
    poisoned: bool,
}

impl<W: Write + Seek> Encoder<W> {
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

        let mut encoder = Encoder {
            chunks: ChunkWriter::new(writer),
            layout: Layout::new(width, height, config.color_type)?,
            codec: Codec::new(config.compression_level),
            delta: Delta::new(),
            writing: None,
            reduce_color: config.reduce_color,
            num_frames,
            num_plays: config.num_plays,
            frames_accepted: 0,
            poisoned: false,
        };

        if !config.reduce_color {
            let output = Output::from(config.color_type);
            let (_, mut parts) = encoder.split();
            parts.write_header(output)?;
            encoder.writing = Some(Writing::new(Encoding::Direct(output)));
        }
        Ok(encoder)
    }

    /// 出力の色種別を落とした結果
    ///
    /// [`Config::reduce_color`] が有効で、先頭フレームを投入した後に `Some` を返す。
    /// パレットに載る色数は最後のフレームまで伸びるため、全フレームを投入した後の値が
    /// 出力に載る色数になる。
    pub fn color_reduction(&self) -> Option<ColorReduction> {
        if !self.reduce_color {
            return None;
        }

        Some(match &self.writing.as_ref()?.encoding {
            Encoding::Direct(_) => ColorReduction::Kept,
            Encoding::Indexed(palette) => ColorReduction::Palette {
                colors: palette.colors(),
            },
        })
    }

    /// 書き出しの状態と、それに依らない部品に分けて借りる
    ///
    /// 状態を取り出したまま部品を触れるようにする。
    fn split(&mut self) -> (&mut Option<Writing>, Parts<'_, W>) {
        let Encoder {
            chunks,
            layout,
            codec,
            delta,
            writing,
            reduce_color: _,
            num_frames,
            num_plays,
            frames_accepted,
            poisoned: _,
        } = self;

        (
            writing,
            Parts {
                chunks,
                layout,
                codec,
                delta,
                num_frames: *num_frames,
                num_plays: *num_plays,
                frame: *frames_accepted,
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
    /// `data` の長さが合わないとき、宣言したフレーム数を超えたとき、パレットに載る
    /// 色数を超えたとき、書き出しに失敗したとき、または過去の失敗でエンコーダが
    /// 使用不能なとき。
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

    /// 投入されたフレームを書き出す
    ///
    /// 先頭フレームだけは、書き出す前に出力の画素表現を決めてヘッダを書く。
    fn accept(&mut self, data: &[u8], delay: FrameDelay) -> Result<(), Error> {
        let (slot, mut parts) = self.split();
        if slot.is_none() {
            *slot = Some(parts.open(data)?);
        }
        let writing = slot.as_mut().expect("先頭フレームで書き出しの状態が決まる");
        parts.write_frame(writing, data, delay)
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
        let (slot, mut parts) = self.split();
        if let Some(writing) = slot {
            parts.flush_pending(&mut writing.pending, DISPOSE_OP_NONE)?;
            if let Encoding::Indexed(palette) = &writing.encoding {
                parts.settle(palette)?;
            }
        }

        self.chunks.write(*b"IEND", &[])?;
        Ok(self.chunks.into_inner())
    }
}

/// [`Encoder`] から [`Writing`] 以外を借りたもの
///
/// 書き出しの状態は引数で受け取る。フレーム1つを処理する判断と書き出しを担う。
struct Parts<'a, W: Write + Seek> {
    chunks: &'a mut ChunkWriter<W>,
    layout: &'a Layout,
    codec: &'a mut Codec,
    delta: &'a mut Delta,
    num_frames: u32,
    num_plays: u32,
    /// 処理しているフレームの、投入された順の位置
    frame: u32,
}

impl<W: Write + Seek> Parts<'_, W> {
    /// シグネチャと、画素データより前に置くチャンクを書き出す
    fn write_header(&mut self, output: Output) -> Result<(), Error> {
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

        Ok(())
    }

    /// 先頭フレームから出力の画素表現を決め、ヘッダを書き出す
    ///
    /// 先頭フレームの矩形はキャンバス全体なので、ここで色数がパレットに収まらなければ
    /// 全フレームの和集合も収まらない。収まったらパレット参照で書き出し、PLTEとtRNSは
    /// 場所だけ確保して [`Encoder::finish`] で書き戻す。
    fn open(&mut self, data: &[u8]) -> Result<Writing, Error> {
        let mut colors = Colors::new();
        colors.observe(data, self.layout.bytes_per_pixel);
        if colors.exceeded() {
            let output = Output::from(self.layout.input);
            self.write_header(output)?;
            return Ok(Writing::new(Encoding::Direct(output)));
        }

        self.write_header(Output::Indexed8)?;
        let plte = self.chunks.position()?;
        self.chunks.write(*b"PLTE", &PLTE_PLACEHOLDER)?;
        // アルファを持たない入力にはアルファが現れないため、tRNS自体が要らない
        let trns = match self.layout.input {
            ColorType::Rgb8 => None,
            ColorType::Rgba8 => {
                let at = self.chunks.position()?;
                self.chunks.write(*b"tRNS", &TRNS_PLACEHOLDER)?;
                Some(at)
            }
        };

        Ok(Writing::new(Encoding::Indexed(Palette::new(
            colors, plte, trns,
        ))))
    }

    /// 場所を確保しておいた位置へPLTEとtRNSを書き戻す
    fn settle(&mut self, palette: &Palette) -> Result<(), Error> {
        self.chunks
            .rewrite(palette.plte_at(), *b"PLTE", &palette.plte())?;
        if let Some(at) = palette.trns_at() {
            self.chunks.rewrite(at, *b"tRNS", &palette.trns())?;
        }
        Ok(())
    }

    /// 保留中のフレームを書き出し、投入されたフレームを保留にする
    fn write_frame(
        &mut self,
        writing: &mut Writing,
        data: &[u8],
        delay: FrameDelay,
    ) -> Result<(), Error> {
        let disposal =
            self.choose_dispose(&mut writing.encoding, data, writing.pending.is_some())?;
        let (dispose, rect) = (disposal.op, disposal.rect);
        let (blend, candidate) =
            self.choose_blend(&writing.encoding, data, disposal, &mut writing.blend_pacing);
        let body = candidate.into_body(self.codec);

        self.flush_pending(&mut writing.pending, dispose)?;
        writing.pending = Some(Pending {
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
        encoding: &mut Encoding,
        data: &[u8],
        disposable: bool,
    ) -> Result<Disposal, Error> {
        let kept = self.delta.kept_rect(self.layout, data, self.frame);
        let restored = self
            .delta
            .restored_rect(self.layout, data, kept, self.frame, disposable);

        let kept_candidate = self.compress_rect(encoding, data, kept)?;
        let keep = |candidate| Disposal {
            op: DISPOSE_OP_NONE,
            rect: kept,
            candidate,
        };
        let Some(restored) = restored else {
            return Ok(keep(kept_candidate));
        };

        let restored_candidate = self.compress_rect(encoding, data, restored)?;

        if restored_candidate.len() < kept_candidate.len() {
            kept_candidate.discard(self.codec);
            Ok(Disposal {
                op: DISPOSE_OP_PREVIOUS,
                rect: restored,
                candidate: restored_candidate,
            })
        } else {
            restored_candidate.discard(self.codec);
            Ok(keep(kept_candidate))
        }
    }

    /// 投入されたフレームをキャンバスへ重ねる方法を決める
    ///
    /// 矩形の中で変化した画素がすべて不透明なら、変化していない画素を完全な透明へ
    /// 潰した候補が立つ。blend_op=OVERはその画素でキャンバスを残すため、潰しても
    /// 元の値に戻る。`disposal` の候補と両方を圧縮して小さい方を採り、採った側を
    /// 戻り値へ残して、退けた側のバッファはプールへ返す。同じ大きさならSOURCEを採る。
    ///
    /// 潰した画素を書けない出力では候補が立たない。先頭フレームはキャンバスがまだ空で、
    /// 重ねる先が無い。負けが続く間は [`BlendPacing`] が候補を立てるのを休ませる。
    fn choose_blend(
        &mut self,
        encoding: &Encoding,
        data: &[u8],
        disposal: Disposal,
        pacing: &mut BlendPacing,
    ) -> (u8, Candidate) {
        let Disposal {
            op: dispose,
            rect,
            candidate: source,
        } = disposal;

        let Some(collapse) = encoding.collapse() else {
            return (BLEND_OP_SOURCE, source);
        };
        if self.frame == 0 || !pacing.should_try() {
            return (BLEND_OP_SOURCE, source);
        }

        // 保留中のフレームを捨てると、キャンバスはそれを描く直前の内容へ戻る
        let base = if dispose == DISPOSE_OP_NONE {
            &self.delta.previous
        } else {
            &self.delta.canvas
        };
        let stride = self.layout.stride;
        let mut over = self.codec.take();
        let packed = match collapse {
            Collapse::Transparent => over::pack_over(base, data, stride, rect, &mut over),
            Collapse::Index(palette, transparent) => {
                over::pack_over_indexed(base, data, stride, rect, palette, transparent, &mut over)
            }
        };
        if !packed {
            self.codec.give(over);
            return (BLEND_OP_SOURCE, source);
        }

        let out_bpp = encoding.output().bytes_per_pixel();
        let over_candidate = self
            .codec
            .compress(&over, rect.width as usize * out_bpp, out_bpp);
        self.codec.give(over);

        let taken = over_candidate.len() < source.len();
        pacing.record(taken);
        if taken {
            source.discard(self.codec);
            (BLEND_OP_OVER, over_candidate)
        } else {
            over_candidate.discard(self.codec);
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

    /// フレームから `rect` を切り出し、出力の表現へ直してフィルタして圧縮する
    ///
    /// # Errors
    /// パレットに載る色数を超えたとき。
    fn compress_rect(
        &mut self,
        encoding: &mut Encoding,
        data: &[u8],
        rect: Rect,
    ) -> Result<Candidate, Error> {
        let stride = self.layout.stride;
        let bpp = self.layout.bytes_per_pixel;
        let row_len = rect.width as usize * bpp;
        let head = rect.y as usize * stride + rect.x as usize * bpp;

        match encoding {
            Encoding::Direct(_) => {
                if row_len == stride {
                    // 全幅の矩形は `data` 上で既に連続している
                    let len = row_len * rect.height as usize;
                    return Ok(self.codec.compress(&data[head..head + len], row_len, bpp));
                }

                let mut cropped = self.codec.take();
                crop(data, rect, stride, bpp, bpp, &mut cropped);
                let candidate = self.codec.compress(&cropped, row_len, bpp);
                self.codec.give(cropped);
                Ok(candidate)
            }
            Encoding::Indexed(palette) => {
                let mut indices = self.codec.take();
                indices.reserve(rect.width as usize * rect.height as usize);
                for y in 0..rect.height as usize {
                    let start = head + y * stride;
                    if !palette.append_indices(&data[start..start + row_len], bpp, &mut indices) {
                        self.codec.give(indices);
                        return Err(Error::ColorLimitExceeded { frame: self.frame });
                    }
                }
                let candidate = self.codec.compress(&indices, rect.width as usize, 1);
                self.codec.give(indices);
                Ok(candidate)
            }
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

    fn encode(input: &[Vec<u8>], config: Config) -> Vec<u8> {
        let mut encoder = Encoder::new(
            Cursor::new(Vec::new()),
            WIDTH,
            HEIGHT,
            input.len() as u32,
            config,
        )
        .unwrap();
        for frame in input {
            encoder
                .add_frame(frame, FrameDelay::new(1, 30).unwrap())
                .unwrap();
        }
        encoder.finish().unwrap().into_inner()
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

    /// 色種別をパレット参照へ落としても、プローブが戦略を決める
    #[test]
    fn reducing_the_color_type_still_settles_the_strategy() {
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

    /// RGBA8を入力する設定
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

        let mut encoder = Encoder::new(
            Cursor::new(Vec::new()),
            WIDTH,
            HEIGHT,
            input.len() as u32,
            rgba_config(),
        )
        .unwrap();
        let mut recorded = Vec::new();
        let mut totals = (0u64, 0u64);
        for frame in &input {
            encoder
                .add_frame(frame, FrameDelay::new(1, 30).unwrap())
                .unwrap();
            let probed = encoder.codec.probe_totals();
            recorded.push((
                (probed.0 - totals.0) as usize,
                (probed.1 - totals.1) as usize,
            ));
            totals = probed;
        }
        let bytes = encoder.finish().unwrap().into_inner();

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

    /// 連敗が閾値に届くまでは候補を立てるのをやめない
    ///
    /// 少ない負けで見切ると勝ち負けの揺れを拾い、まだ採られる素材でも候補が
    /// 立たなくなる。休みに入るのは閾値に届いたときだけで、1回の負けでは入らない。
    #[test]
    fn the_pacing_keeps_trying_below_the_streak() {
        let mut pacing = BlendPacing::new();
        pacing.record(false);
        assert_eq!(pacing.resting, 0, "1回の負けで休みに入っている");

        for loss in 2..BLEND_LOSS_STREAK {
            assert!(pacing.should_try(), "連敗 {loss} 回目");
            pacing.record(false);
            assert_eq!(pacing.resting, 0, "連敗 {loss} 回で休みに入っている");
        }
        assert!(pacing.should_try(), "閾値に届く前に休みに入っている");
    }

    /// フレームを受け付けたエンコーダが持つ間合い
    fn pacing_of<W: Write + Seek>(encoder: &Encoder<W>) -> &BlendPacing {
        let writing = encoder.writing.as_ref().expect("書き出しが始まっている");
        &writing.blend_pacing
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
        let blend_pacing = pacing_of(&encoder);
        assert_eq!(blend_pacing.resting, BLEND_REST_FRAMES);

        encoder.add_frame(&speckled, delay).unwrap();
        let blend_pacing = pacing_of(&encoder);
        assert_eq!(blend_pacing.resting, BLEND_REST_FRAMES - 1);
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
