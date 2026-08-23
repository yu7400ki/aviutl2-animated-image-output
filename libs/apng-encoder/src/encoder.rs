//! APNGのストリーミング書き出し

use crate::chunk;
use crate::delay::FrameDelay;
use crate::diff::{self, Rect};
use crate::error::Error;
use crate::filter;
use crate::palette::Palette;
use crate::region;
use crate::spool::{Spool, Spooled};
use crate::zlib::Compressor;
use std::io::Write;
use std::ops::RangeInclusive;

/// 前のフレームを消さずに次のフレームを描画する
const DISPOSE_OP_NONE: u8 = 0;
/// フレームの領域を描画前の内容へ戻してから次のフレームを描画する
const DISPOSE_OP_PREVIOUS: u8 = 2;
/// フレームの内容で領域を置き換える
const BLEND_OP_SOURCE: u8 = 0;

/// 画素の色種別 (ビット深度8固定)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorType {
    /// 8bit/chのRGB
    Rgb8,
    /// 8bit/chのRGBA
    Rgba8,
}

impl ColorType {
    /// 1画素あたりのバイト数
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            ColorType::Rgb8 => 3,
            ColorType::Rgba8 => 4,
        }
    }
}

/// 出力の画素表現 (ビット深度8固定)
///
/// 入力に取れる色種別のほか、パレットを引く添字を持つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Output {
    /// 8bit/chのRGB
    Rgb8,
    /// 8bit/chのRGBA
    Rgba8,
    /// PLTEを引く1バイトの添字
    Indexed8,
}

impl Output {
    /// 1画素あたりのバイト数
    fn bytes_per_pixel(self) -> usize {
        match self {
            Output::Rgb8 => 3,
            Output::Rgba8 => 4,
            Output::Indexed8 => 1,
        }
    }

    /// IHDRのcolour type
    fn code(self) -> u8 {
        match self {
            Output::Rgb8 => 2,
            Output::Rgba8 => 6,
            Output::Indexed8 => 3,
        }
    }
}

impl From<ColorType> for Output {
    fn from(color_type: ColorType) -> Self {
        match color_type {
            ColorType::Rgb8 => Output::Rgb8,
            ColorType::Rgba8 => Output::Rgba8,
        }
    }
}

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

/// フィルタ戦略を決めるまでに両方の戦略で圧縮するフレーム数
///
/// 先頭フレームはキャンバス全体を書くため、差分矩形を書く以降のフレームとは
/// 中身の性質が違う。差分矩形のフレームも何枚か見てから決めるだけの回数を取る。
const PROBE_FRAMES: u32 = 4;

/// 出力の色種別を決めるまでに両方の表現で圧縮するフレーム数
///
/// 先頭フレームはキャンバス全体を書くため、差分矩形を書く以降のフレームとは
/// 中身の性質が違う。アルファを落とせるかどうかの傾きはフレームごとの振れが
/// 大きく、行ごとのフィルタを選ぶときより多くの差分矩形を見ないと定まらない。
/// 比べるための圧縮は書き出しに使い回せないため、増やした分だけ丸ごと余分になる。
const COLOR_PROBE_FRAMES: u32 = 8;

/// フィルタ戦略の決定
///
/// 先頭の [`PROBE_FRAMES`] フレームは両方の戦略で圧縮して小さい方を採り、
/// 圧縮後のバイト数を戦略ごとに積む。プローブを終えた時点で合計の小さい戦略へ
/// 固定し、以降のフレームはその戦略だけを実行する。
struct FilterChoice {
    /// 残りのプローブ回数
    remaining: u32,
    /// プローブで [`filter::Strategy::Adaptive`] が出した圧縮後バイト数の合計
    adaptive_bytes: u64,
    /// プローブで [`filter::Strategy::Unfiltered`] が出した圧縮後バイト数の合計
    unfiltered_bytes: u64,
    /// 固定した戦略。プローブが残っていれば `None`
    fixed: Option<filter::Strategy>,
}

/// プローブ1回ぶんの、戦略ごとの圧縮後バイト数
#[derive(Debug, Clone, Copy)]
struct Probe {
    adaptive: usize,
    unfiltered: usize,
}

impl FilterChoice {
    fn new() -> Self {
        FilterChoice {
            remaining: PROBE_FRAMES,
            adaptive_bytes: 0,
            unfiltered_bytes: 0,
            fixed: None,
        }
    }

    /// プローブ1回ぶんの圧縮後バイト数を記録し、残りが尽きたら戦略を固定する
    fn record(&mut self, probe: Probe) {
        debug_assert!(self.fixed.is_none());

        self.adaptive_bytes += probe.adaptive as u64;
        self.unfiltered_bytes += probe.unfiltered as u64;
        self.remaining -= 1;

        if self.remaining == 0 {
            self.fixed = Some(if self.adaptive_bytes < self.unfiltered_bytes {
                filter::Strategy::Adaptive
            } else {
                filter::Strategy::Unfiltered
            });
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
    /// フィルタして圧縮した本体
    body: Vec<u8>,
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
    writer: W,
    width: u32,
    height: u32,
    num_frames: u32,
    num_plays: u32,
    /// [`Self::add_frame`] が受け付けたフレーム数
    frames_accepted: u32,
    /// 実際に書き出したフレーム数
    frames_emitted: u32,
    /// fcTLとfdATで共有する連番
    sequence: u32,
    /// 書き出しに失敗し、チャンク列が中断しているか
    poisoned: bool,
    /// 入力の1画素あたりのバイト数
    bytes_per_pixel: usize,
    /// 入力の1行のバイト数
    stride: usize,
    /// 入力の1フレームのバイト数
    frame_len: usize,
    /// 入力の色種別
    input: ColorType,
    /// 出力の画素表現
    ///
    /// [`Self::spool`] が `Some` の間は暫定で入力の色種別が入り、
    /// [`Self::commit`] で確定してヘッダに載る。
    output: Output,
    /// 出力がパレット参照のときの、添字と色の対応
    palette: Option<Palette>,
    /// 出力の色種別が決まるまでフレームを溜める領域
    spool: Option<Spool>,
    /// アルファを落とすかどうかを圧縮して比べた結果。比べる前は `None`
    alpha_choice: Option<Output>,
    /// 出力の色種別を落とした結果。決まるまでは `None`
    reduction: Option<ColorReduction>,
    /// 溜めたバイト数の最大値
    peak_spool_bytes: usize,
    compressor: Compressor,
    /// 書き出しを待っているフレーム
    pending: Option<Pending>,
    /// 直前に投入されたフレーム
    ///
    /// blend_op=SOURCE のもとでは、それを描いた後のキャンバスと一致する。
    previous: Vec<u8>,
    /// [`Self::previous`] を描く直前のキャンバス
    ///
    /// 保留中のフレームをdispose_op=PREVIOUSで捨てると、この内容が復元される。
    /// 復元先は常に過去のいずれかのフレームそのものなので、1面あれば足りる。
    canvas: Vec<u8>,
    /// 差分矩形を切り出した連続バッファ
    region: Vec<u8>,
    /// 行ごとのフィルタ選択に使う作業領域
    scratch: filter::Scratch,
    filtered: Vec<u8>,
    compressed: Vec<u8>,
    /// フィルタ戦略の決定
    filter_choice: FilterChoice,
    /// プローブでもう一方の候補を圧縮しておく領域
    probed: Vec<u8>,
    /// dispose_opの候補を比べるために、もう一方の候補を圧縮しておく領域
    dispose_probed: Vec<u8>,
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

        let bytes_per_pixel = config.color_type.bytes_per_pixel();
        let stride = (width as usize)
            .checked_mul(bytes_per_pixel)
            .ok_or(Error::ImageTooLarge { width, height })?;
        let frame_len = stride
            .checked_mul(height as usize)
            .ok_or(Error::ImageTooLarge { width, height })?;

        let mut encoder = Encoder {
            writer,
            width,
            height,
            num_frames,
            num_plays: config.num_plays,
            frames_accepted: 0,
            frames_emitted: 0,
            sequence: 0,
            poisoned: false,
            bytes_per_pixel,
            stride,
            frame_len,
            input: config.color_type,
            output: Output::from(config.color_type),
            palette: None,
            spool: config
                .reduce_color
                .then(|| Spool::new(config.max_spool_bytes)),
            alpha_choice: None,
            reduction: None,
            peak_spool_bytes: 0,
            compressor: Compressor::new(config.compression_level),
            pending: None,
            previous: Vec::new(),
            canvas: Vec::new(),
            region: Vec::new(),
            scratch: filter::Scratch::new(),
            filtered: Vec::new(),
            compressed: Vec::new(),
            filter_choice: FilterChoice::new(),
            probed: Vec::new(),
            dispose_probed: Vec::new(),
        };
        if encoder.spool.is_none() {
            encoder.write_header()?;
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

    fn write_header(&mut self) -> Result<(), Error> {
        self.writer.write_all(&chunk::SIGNATURE)?;

        let mut ihdr = [0u8; 13];
        ihdr[0..4].copy_from_slice(&self.width.to_be_bytes());
        ihdr[4..8].copy_from_slice(&self.height.to_be_bytes());
        ihdr[8] = 8;
        ihdr[9] = self.output.code();
        chunk::write(&mut self.writer, *b"IHDR", &ihdr)?;

        let mut actl = [0u8; 8];
        actl[0..4].copy_from_slice(&self.num_frames.to_be_bytes());
        actl[4..8].copy_from_slice(&self.num_plays.to_be_bytes());
        chunk::write(&mut self.writer, *b"acTL", &actl)?;

        // PLTEとtRNSは画素データより前に置く
        if let Some(palette) = &self.palette {
            chunk::write(&mut self.writer, *b"PLTE", &palette.plte())?;
            let trns = palette.trns();
            if !trns.is_empty() {
                chunk::write(&mut self.writer, *b"tRNS", &trns)?;
            }
        }

        Ok(())
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

        if data.len() != self.frame_len {
            return Err(Error::FrameSizeMismatch {
                expected: self.frame_len,
                actual: data.len(),
            });
        }

        // 途中で失敗するとfcTLだけが書かれた状態で残るため、以降の書き出しを拒否する
        self.accept(data, delay)
            .inspect_err(|_| self.poisoned = true)?;

        self.frames_accepted += 1;
        Ok(())
    }

    /// 保留中のフレームを書き出し、投入されたフレームを保留にする
    fn accept(&mut self, data: &[u8], delay: FrameDelay) -> Result<(), Error> {
        // 出力の色種別が決まるまでは、候補を実際に書き出す色種別で圧縮できず
        // 大きさを比べられない。溜めている間はdispose_opをNONEに固定し、
        // 保留を挟まずに溜める側へ渡す。
        if let Some(spool) = self.spool.take() {
            let rect = self.kept_rect(data);
            self.spool_frame(spool, data, rect, delay)?;
            self.advance(data, DISPOSE_OP_NONE);
            return Ok(());
        }

        let (dispose, rect) = self.choose_dispose(data);
        let mut body = self.flush_pending(dispose)?;
        std::mem::swap(&mut self.compressed, &mut body);
        self.pending = Some(Pending { rect, delay, body });
        self.advance(data, dispose);
        Ok(())
    }

    /// 投入されたフレームを直前のフレームとして覚え、キャンバスを進める
    fn advance(&mut self, data: &[u8], dispose: u8) {
        // 捨てない場合だけ、直前のフレームがそのままキャンバスとして残る
        if dispose == DISPOSE_OP_NONE {
            std::mem::swap(&mut self.canvas, &mut self.previous);
        }
        self.previous.clear();
        self.previous.extend_from_slice(data);
    }

    /// 保留中のフレームを捨てないときの、投入されたフレームの矩形
    ///
    /// 先頭フレームはIDATに入るためキャンバス全体とする。以降は保留中のフレームとの
    /// 差分の外接矩形を使う。
    fn kept_rect(&self, data: &[u8]) -> Rect {
        if self.frames_accepted == 0 {
            return Rect {
                x: 0,
                y: 0,
                width: self.width,
                height: self.height,
            };
        }

        self.bounding_rect(&self.previous, data)
    }

    /// 保留中のフレームをdispose_op=PREVIOUSで捨てるときの、投入されたフレームの矩形
    ///
    /// 次の場合は捨てても割に合わないため、候補にせず `None` を返す。
    /// - 書き出しを待っているフレームが無いとき。捨てる先が無く、
    ///   [`Self::canvas`] もまだ埋まっていない
    /// - 保留中のフレームが先頭フレームのとき。先頭のfcTLの
    ///   dispose_op=PREVIOUSはBACKGROUNDとして扱われてキャンバスが復元されず、
    ///   [`Self::canvas`] もまだ埋まっていない
    /// - 矩形が捨てない場合より小さくならないとき。圧縮すれば小さくなることは
    ///   あるが、それを測る圧縮の方が高くつく
    fn restored_rect(&self, data: &[u8], kept: Rect) -> Option<Rect> {
        if self.pending.is_none() || self.frames_accepted < 2 {
            return None;
        }

        let rect = self.bounding_rect(&self.canvas, data);
        (rect.area() < kept.area()).then_some(rect)
    }

    /// `base` と `data` の差分の外接矩形
    ///
    /// 差分が無い場合はfcTLの個数を保つために1画素だけ書き直す。
    fn bounding_rect(&self, base: &[u8], data: &[u8]) -> Rect {
        const UNCHANGED: Rect = Rect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        };

        diff::dirty_rect(base, data, self.stride, self.bytes_per_pixel).unwrap_or(UNCHANGED)
    }

    /// 保留中のフレームのdispose_opと、投入されたフレームの矩形を決める
    ///
    /// 保留中のフレームをdispose_op=PREVIOUSで捨てると、投入されたフレームは
    /// それを描く直前のキャンバスとの差分になる。両方の候補を圧縮して小さい方を採り、
    /// 採った側の本体を [`Self::compressed`] へ残す。同じ大きさなら捨てない。
    fn choose_dispose(&mut self, data: &[u8]) -> (u8, Rect) {
        let kept = self.kept_rect(data);
        let restored = self.restored_rect(data, kept);

        let kept_probe = self.compress_rect(data, kept);
        let Some(restored) = restored else {
            self.record(kept_probe);
            return (DISPOSE_OP_NONE, kept);
        };

        let kept_len = self.compressed.len();
        std::mem::swap(&mut self.compressed, &mut self.dispose_probed);
        let restored_probe = self.compress_rect(data, restored);

        if self.compressed.len() < kept_len {
            self.record(restored_probe);
            (DISPOSE_OP_PREVIOUS, restored)
        } else {
            std::mem::swap(&mut self.compressed, &mut self.dispose_probed);
            self.record(kept_probe);
            (DISPOSE_OP_NONE, kept)
        }
    }

    /// 保留中のフレームを `dispose` で書き出し、本体に使っていた領域を返す
    fn flush_pending(&mut self, dispose: u8) -> Result<Vec<u8>, Error> {
        let Some(pending) = self.pending.take() else {
            return Ok(Vec::new());
        };

        let mut body = pending.body;
        std::mem::swap(&mut self.compressed, &mut body);
        self.write_frame(pending.rect, pending.delay, dispose)?;
        std::mem::swap(&mut self.compressed, &mut body);
        Ok(body)
    }

    /// 溜めているフレームへ1つ加え、決めたとおりに処理する
    fn spool_frame(
        &mut self,
        mut spool: Spool,
        data: &[u8],
        rect: Rect,
        delay: FrameDelay,
    ) -> Result<(), Error> {
        let region_len = rect.width as usize * rect.height as usize * self.bytes_per_pixel;

        // 抱えきれない大きさが来たら、入力の色種別で確定して溜めたぶんを流す
        if !spool.can_hold(region_len) {
            self.commit(spool, Decision::Abandoned)?;
            return self.emit_frame(data, rect, delay);
        }

        spool.push(data, rect, delay, self.stride, self.bytes_per_pixel);
        self.peak_spool_bytes = self.peak_spool_bytes.max(spool.len());

        let is_last = self.frames_accepted + 1 == self.num_frames;
        match self.decide_output(&spool, is_last) {
            Some(decision) => self.commit(spool, decision),
            None => {
                self.spool = Some(spool);
                Ok(())
            }
        }
    }

    /// 溜めたフレームから出力の画素表現を決める
    ///
    /// 色の和集合がパレットに収まっている間は候補が残るため、最後のフレームを見るまで
    /// 決まらない。収まらないと分かった後は、アルファを落とせるかどうかだけが残る。
    /// まだ決まらないときは `None` を返す。
    fn decide_output(&mut self, spool: &Spool, is_last: bool) -> Option<Decision> {
        if !spool.colors_exceeded() {
            return is_last.then_some(Decision::Fixed(Output::Indexed8));
        }

        match self.input {
            ColorType::Rgb8 => Some(Decision::Fixed(Output::Rgb8)),
            ColorType::Rgba8 if spool.transparent() => Some(Decision::Fixed(Output::Rgba8)),
            ColorType::Rgba8 => self.decide_alpha(spool, is_last),
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
    fn decide_alpha(&mut self, spool: &Spool, is_last: bool) -> Option<Decision> {
        let frames = spool.frames();
        if !is_last && frames.len() < COLOR_PROBE_FRAMES as usize {
            return None;
        }

        let output = match self.alpha_choice {
            Some(output) => output,
            None => {
                let output = self.choose_output(frames);
                self.alpha_choice = Some(output);
                output
            }
        };

        (is_last || output == Output::Rgba8).then_some(Decision::Compared(output))
    }

    /// 出力の画素表現を確定し、ヘッダに続けて溜めたフレームを書き出す
    fn commit(&mut self, spool: Spool, decision: Decision) -> Result<(), Error> {
        let (frames, colors) = spool.into_parts();
        let output = match decision {
            Decision::Fixed(output) | Decision::Compared(output) => output,
            Decision::Abandoned => Output::from(self.input),
        };

        let color_count = colors.len();
        self.palette = (output == Output::Indexed8).then(|| colors.into_palette());
        self.reduction = Some(match decision {
            Decision::Abandoned => ColorReduction::Abandoned,
            // 圧縮して比べた結果なので、残った理由は落とすと大きくなること
            Decision::Compared(_) => match output {
                Output::Rgb8 => ColorReduction::AlphaDropped,
                _ => ColorReduction::AlphaKept,
            },
            // 溜めた内容だけで定まる先は、パレットか透過を含むRGBAか入力そのもの
            Decision::Fixed(_) => match output {
                Output::Indexed8 => ColorReduction::Palette {
                    colors: color_count,
                },
                Output::Rgba8 => ColorReduction::AlphaRequired,
                Output::Rgb8 => ColorReduction::Kept,
            },
        });
        self.output = output;
        self.write_header()?;

        for frame in &frames {
            let probe = self.compress_spooled(frame, output);
            self.record(probe);
            // 溜めている間はdispose_opを決められないため、捨てずに残す
            self.write_frame(frame.rect, frame.delay, DISPOSE_OP_NONE)?;
        }

        Ok(())
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
        debug_assert_eq!(self.input, ColorType::Rgba8);
        debug_assert!(self.palette.is_none());
        debug_assert!(self.filter_choice.fixed.is_none());

        let (mut dropped, mut kept) = (0u64, 0u64);
        for frame in frames.iter().take(COLOR_PROBE_FRAMES as usize) {
            self.compress_spooled(frame, Output::Rgb8);
            dropped += self.compressed.len() as u64;
            self.compress_spooled(frame, Output::Rgba8);
            kept += self.compressed.len() as u64;
        }

        smaller_output(dropped, kept)
    }

    /// 溜めたフレームを `output` の表現へ直してフィルタして圧縮し、[`Self::compressed`] へ格納する
    fn compress_spooled(&mut self, frame: &Spooled, output: Output) -> Option<Probe> {
        let out_bpp = output.bytes_per_pixel();
        let region_stride = frame.rect.width as usize * out_bpp;

        if self.bytes_per_pixel == out_bpp {
            return self.compress(&frame.data, region_stride, out_bpp);
        }

        let mut converted = std::mem::take(&mut self.region);
        converted.clear();
        self.append_output(&frame.data, output, &mut converted);
        let probe = self.compress(&converted, region_stride, out_bpp);
        self.region = converted;
        probe
    }

    /// 溜めるのをやめたフレームを1つ書き出す
    ///
    /// 溜めている間はdispose_opを決められないため、捨てずに残す。
    fn emit_frame(&mut self, data: &[u8], rect: Rect, delay: FrameDelay) -> Result<(), Error> {
        let probe = self.compress_rect(data, rect);
        self.record(probe);
        self.write_frame(rect, delay, DISPOSE_OP_NONE)
    }

    /// 入力の画素列を `output` の表現へ直しながら `out` へ追記する
    fn append_output(&self, pixels: &[u8], output: Output, out: &mut Vec<u8>) {
        match &self.palette {
            Some(palette) => palette.append_indices(pixels, self.bytes_per_pixel, out),
            None => {
                region::append_pixels(pixels, self.bytes_per_pixel, output.bytes_per_pixel(), out)
            }
        }
    }

    /// フレームから `rect` を切り出してフィルタして圧縮し、[`Self::compressed`] へ格納する
    ///
    /// この経路を通るのは出力が決まった後のフレームだけで、その表現は入力と同じか
    /// アルファを落としたものになる。
    fn compress_rect(&mut self, data: &[u8], rect: Rect) -> Option<Probe> {
        debug_assert!(self.palette.is_none());

        let in_bpp = self.bytes_per_pixel;
        let out_bpp = self.output.bytes_per_pixel();
        let region_stride = rect.width as usize * out_bpp;

        if in_bpp == out_bpp && rect.width as usize * in_bpp == self.stride {
            // 変換の要らない全幅の矩形は `data` 上で既に連続している
            let head = rect.y as usize * self.stride;
            let len = region_stride * rect.height as usize;
            self.compress(&data[head..head + len], region_stride, out_bpp)
        } else {
            let mut cropped = std::mem::take(&mut self.region);
            cropped.clear();
            region::crop(data, rect, self.stride, in_bpp, out_bpp, &mut cropped);
            let probe = self.compress(&cropped, region_stride, out_bpp);
            self.region = cropped;
            probe
        }
    }

    /// 連続した領域をフィルタして圧縮し、[`Self::compressed`] へ格納する
    ///
    /// フィルタ戦略が固まるまでは両方を試し、それ以降は固めた戦略だけを使う。
    /// 両方を試した場合は、そのバイト数を戦略ごとに返す。
    fn compress(&mut self, region: &[u8], region_stride: usize, bpp: usize) -> Option<Probe> {
        match self.filter_choice.fixed {
            Some(strategy) => {
                self.compress_with(region, region_stride, bpp, strategy);
                None
            }
            None => Some(self.probe(region, region_stride, bpp)),
        }
    }

    /// 書き出すフレーム1つぶんのプローブを記録する
    ///
    /// dispose_opの候補を選ぶための圧縮は、採らなかった側を二重に数えないよう
    /// 記録しない。
    fn record(&mut self, probe: Option<Probe>) {
        if let Some(probe) = probe {
            self.filter_choice.record(probe);
        }
    }

    /// `strategy` でフィルタして圧縮し、[`Self::compressed`] へ格納する
    fn compress_with(
        &mut self,
        region: &[u8],
        region_stride: usize,
        bpp: usize,
        strategy: filter::Strategy,
    ) {
        self.filtered.clear();
        filter::filter_image(
            region,
            region_stride,
            bpp,
            strategy,
            &mut self.scratch,
            &mut self.filtered,
        );

        self.compressed.clear();
        self.compressor
            .compress_into(&self.filtered, &mut self.compressed);
    }

    /// 両方の戦略で圧縮し、小さい方を [`Self::compressed`] に残して結果を返す
    ///
    /// 同じ大きさなら [`filter::Strategy::Unfiltered`] を残す。
    fn probe(&mut self, region: &[u8], region_stride: usize, bpp: usize) -> Probe {
        self.compress_with(region, region_stride, bpp, filter::Strategy::Adaptive);
        std::mem::swap(&mut self.compressed, &mut self.probed);
        self.compress_with(region, region_stride, bpp, filter::Strategy::Unfiltered);

        let (adaptive, unfiltered) = (self.probed.len(), self.compressed.len());
        if adaptive < unfiltered {
            std::mem::swap(&mut self.compressed, &mut self.probed);
        }
        Probe {
            adaptive,
            unfiltered,
        }
    }

    fn write_frame(&mut self, rect: Rect, delay: FrameDelay, dispose: u8) -> Result<(), Error> {
        self.write_fctl(rect, delay, dispose)?;

        // 先頭フレームはIDATに入り、以降はfdATに入る
        if self.frames_emitted == 0 {
            chunk::write(&mut self.writer, *b"IDAT", &self.compressed)?;
        } else {
            chunk::write_parts(
                &mut self.writer,
                *b"fdAT",
                &[&self.sequence.to_be_bytes(), &self.compressed],
            )?;
            self.sequence += 1;
        }

        self.frames_emitted += 1;
        Ok(())
    }

    fn write_fctl(&mut self, rect: Rect, delay: FrameDelay, dispose: u8) -> Result<(), Error> {
        let (delay_num, delay_den) = delay.to_parts();

        let mut fctl = [0u8; 26];
        fctl[0..4].copy_from_slice(&self.sequence.to_be_bytes());
        fctl[4..8].copy_from_slice(&rect.width.to_be_bytes());
        fctl[8..12].copy_from_slice(&rect.height.to_be_bytes());
        fctl[12..16].copy_from_slice(&rect.x.to_be_bytes());
        fctl[16..20].copy_from_slice(&rect.y.to_be_bytes());
        fctl[20..22].copy_from_slice(&delay_num.to_be_bytes());
        fctl[22..24].copy_from_slice(&delay_den.to_be_bytes());
        fctl[24] = dispose;
        fctl[25] = BLEND_OP_SOURCE;
        chunk::write(&mut self.writer, *b"fcTL", &fctl)?;

        self.sequence += 1;
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
        self.flush_pending(DISPOSE_OP_NONE)?;

        chunk::write(&mut self.writer, *b"IEND", &[])?;
        Ok(self.writer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn probe(adaptive: usize, unfiltered: usize) -> Probe {
        Probe {
            adaptive,
            unfiltered,
        }
    }

    /// プローブは候補ごとの圧縮後バイト数を積み、合計の小さい方へ固める
    #[test]
    fn the_probe_fixes_the_candidate_with_the_smaller_total() {
        let mut choice = FilterChoice::new();
        for _ in 0..PROBE_FRAMES - 1 {
            choice.record(probe(100, 120));
            assert_eq!(choice.fixed, None);
        }
        choice.record(probe(100, 1));
        assert_eq!(choice.fixed, Some(filter::Strategy::Unfiltered));

        let mut choice = FilterChoice::new();
        for _ in 0..PROBE_FRAMES {
            choice.record(probe(100, 120));
        }
        assert_eq!(choice.fixed, Some(filter::Strategy::Adaptive));
    }

    /// 合計が同じならフィルタを掛けない方へ固める
    #[test]
    fn a_tie_settles_on_the_unfiltered_strategy() {
        let mut choice = FilterChoice::new();
        for _ in 0..PROBE_FRAMES {
            choice.record(probe(64, 64));
        }
        assert_eq!(choice.fixed, Some(filter::Strategy::Unfiltered));
    }
}
