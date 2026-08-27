//! GIFのストリーミング書き出し

use crate::block::{
    self, DISPOSAL_DO_NOT_DISPOSE, DISPOSAL_RESTORE_TO_BACKGROUND, DISPOSAL_RESTORE_TO_PREVIOUS,
    GLOBAL_TABLE_OFFSET,
};
use crate::delay::Hundredths;
use crate::error::Error;
use crate::frame::{Canvas, Screen};
use crate::layout::{ColorType, Layout};
use crate::lzw;
use crate::normalize::{self, Binarized, TRANSPARENT, pack};
use crate::quantize::{Histogram, Material, material};
use crate::ring::Ring;
use crate::table::{ColorTable, Palette, QUANTIZED_COLORS};
use anim_core::{Colors, FrameDelay, Rect};
use std::borrow::Cow;
use std::io::{Seek, SeekFrom, Write};

/// エンコード設定
#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// 入力フレームの色種別
    ///
    /// [`Encoder::add_frame`] に渡すバイト列の解釈を決める。
    pub color_type: ColorType,
    /// アニメーションの再生回数 (0で無限ループ)
    pub num_plays: u32,
}

/// 既定は無限ループするRGB8
impl Default for Config {
    fn default() -> Self {
        Config {
            color_type: ColorType::Rgb8,
            num_plays: 0,
        }
    }
}

/// グローバルカラーテーブルの据え方
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteKind {
    /// 全フレームの色がそのまま載った (可逆)
    Exact {
        /// 色の和集合の大きさ
        colors: u16,
    },
    /// 途中で色が載りきらなくなり、先読みの窓から量子化した
    Quantized {
        /// 載っている非透過色の数 (そこまでに見つけた色と量子化した色の合計)
        colors: u16,
    },
}

/// 色とタイミングの決定の結果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    /// グローバルカラーテーブルの据え方
    pub palette: PaletteKind,
    /// ローカルカラーテーブルを書いたフレーム数
    ///
    /// グローバルカラーテーブルで写しきれずに逃げたフレームは、自分の色表を
    /// 運ぶ。GIFには前のフレームの色表を参照する手段が無い。
    pub local_tables: u32,
    /// 完全一致が無く最近傍へ写した画素数
    ///
    /// 素材の色に近いエントリはあったが、そのものは無かった画素。可逆の経路では
    /// 常に0で、量子化した色へ写したときに増える。入力が変わった画素に加えて、
    /// 廃棄方法が抜いた矩形の中で書き直す持ち越し画素も数える。
    ///
    /// 0なら、全画素が据えたテーブルの色そのままで解決した。
    pub approximated_pixels: u64,
    /// 写す先が無く、埋め草の黒へ置いた画素数
    ///
    /// 据えたテーブルが非透過色を1つも持たないときに増える。近似と違って
    /// 素材の色は画面に残らない。
    pub substituted_pixels: u64,
    /// 据えたテーブルに非透過色が1つも無く、写す先として黒を足したか
    ///
    /// テーブルの形の記述で、その黒へ実際に置いた画素は
    /// [`Report::substituted_pixels`] が数える。
    pub black_fallback: bool,
    /// 見えていた画素を完全な透過へ潰した画素数
    ///
    /// アルファが閾値未満だった画素のうち、元から完全透過だったものは含まない。
    pub binarized_to_transparent: u64,
    /// 透けていた画素を不透明へ上げた画素数
    ///
    /// アルファが閾値以上だった画素のうち、元から完全不透明だったものは含まない。
    pub binarized_to_opaque: u64,
    /// 遅延を下限で切り上げたか
    pub delay_clamped: bool,
}

/// 書き出し位置から先を覗くフレーム数
///
/// 書き出しはこのフレーム数だけ遅れる。廃棄方法を決めるための1フレーム保留を
/// 含むので、リングに留まるのは `LOOKAHEAD - 1` フレーム。
const LOOKAHEAD: usize = 8;

/// グローバルカラーテーブルから逃げるかどうかを分ける誤差の床
///
/// 単位は二乗距離 (RGB各軸の差の二乗和) の平均。入力が変わった画素をグローバル
/// カラーテーブルへ写し、その二乗距離の平均がこれを超えたフレームは自分の色表を
/// 運ぶ。この値は `private/bench` の全素材で決める。
const BENCH_TUNED_ESCAPE_FLOOR: u64 = 256;

/// 溢れたときに量子化の材料へ積む画素
const QUANTIZE_MATERIAL: Material = Material::Changed;

/// 描く直前へ戻す候補を試すのをやめるまでの連敗数
const RESTORE_LOSS_STREAK: u32 = 6;

/// 連敗した後、描く直前へ戻す候補を立てないフレーム数
const RESTORE_REST_FRAMES: u32 = 8;

/// 描く直前へ戻す候補を立てるかどうかの間合い
///
/// [`RESTORE_LOSS_STREAK`] 回続けて負けたら [`RESTORE_REST_FRAMES`] フレーム
/// 立てるのを休む。
struct RestorePacing {
    /// 採られないまま続いた回数
    losses: u32,
    /// 残りの休みフレーム数
    resting: u32,
}

impl RestorePacing {
    fn new() -> Self {
        RestorePacing {
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
        if self.losses == RESTORE_LOSS_STREAK {
            self.losses = 0;
            self.resting = RESTORE_REST_FRAMES;
        }
    }
}

/// 書き出しを待っているフレーム
///
/// 廃棄方法は次のフレームを見るまで決まらず、グラフィック制御拡張は画像記述子の
/// 前に置く必要があるため、書けるようになるまで1つぶんを保持する。
///
/// 透過インデックス・最小符号長・ローカルカラーテーブルは、添字を作ったときに
/// 決めて一緒に運ぶ。添字はそのときのテーブルを引くものなので、書き出す時点の
/// テーブルから取り直すと組み合わせが崩れうる。
struct Pending {
    rect: Rect,
    /// 1/100秒へ丸めた遅延
    delay: u16,
    /// 添字を引いたテーブルの透過インデックス
    transparent: Option<u8>,
    /// 添字の並びが要するLZW最小符号長
    min_code_size: u8,
    /// このフレームに書くローカルカラーテーブル。グローバルのままなら `None`
    local: Option<ColorTable>,
    /// LZWで圧縮した画像データ
    body: Vec<u8>,
}

/// グローバルカラーテーブルと、そこから逃げたフレームの色表
///
/// 逃げた色表はそのフレーム1枚のもので、次のフレームは再びグローバルから
/// 判定する。保留中のフレームは自分が逃げた色表で符号化されているため、
/// 書き終えるまでその色表を持つ。
struct Palettes {
    /// グローバルカラーテーブル
    global: Palette,
    /// 書き出し位置のフレームが逃げた先。逃げていなければ `None`
    escaped: Option<Palette>,
    /// 保留中のフレームが逃げた先。逃げていなければ `None`
    held: Option<Palette>,
    /// 手放した色表が最近傍へ写した画素数の合計
    retired_approximated: u64,
    /// 手放した色表が埋め草へ置いた画素数の合計
    retired_substituted: u64,
    /// 手放した色表が写す先として黒を足したか
    retired_fallback: bool,
}

impl Palettes {
    fn new(global: Palette) -> Self {
        Palettes {
            global,
            escaped: None,
            held: None,
            retired_approximated: 0,
            retired_substituted: 0,
            retired_fallback: false,
        }
    }

    /// 書き出し位置のフレームを符号化するテーブル
    fn current(&mut self) -> &mut Palette {
        self.escaped.as_mut().unwrap_or(&mut self.global)
    }

    /// 保留中のフレームを符号化したテーブル
    fn earlier(&mut self) -> &mut Palette {
        self.held.as_mut().unwrap_or(&mut self.global)
    }

    /// 書き出し位置のフレームをこの色表へ逃がす
    fn escape(&mut self, palette: Palette) {
        debug_assert!(self.escaped.is_none(), "1フレームで2度逃げている");
        self.escaped = Some(palette);
    }

    /// 保留中のフレームを書き終えたので、その色表を手放す
    ///
    /// 書き出し位置のフレームが逃げた色表は、そのまま保留中のものへ移る。
    fn retire(&mut self) {
        if let Some(held) = self.held.take() {
            self.retired_approximated += held.approximated();
            self.retired_substituted += held.substituted();
            self.retired_fallback |= held.black_fallback();
        }
        self.held = self.escaped.take();
    }

    /// 逃げた色表の集計を畳む
    fn escaped_total(&self, of: impl Fn(&Palette) -> u64) -> u64 {
        self.escaped.as_ref().map_or(0, &of) + self.held.as_ref().map_or(0, &of)
    }

    /// 手放したものも含め、最近傍へ写した画素数
    fn approximated(&self) -> u64 {
        self.retired_approximated
            + self.global.approximated()
            + self.escaped_total(Palette::approximated)
    }

    /// 手放したものも含め、埋め草へ置いた画素数
    fn substituted(&self) -> u64 {
        self.retired_substituted
            + self.global.substituted()
            + self.escaped_total(Palette::substituted)
    }

    /// 手放したものも含め、どこかの色表が写す先として黒を足したか
    fn black_fallback(&self) -> bool {
        self.retired_fallback
            || self.global.black_fallback()
            || self.escaped.as_ref().is_some_and(Palette::black_fallback)
            || self.held.as_ref().is_some_and(Palette::black_fallback)
    }
}

/// 逃げたフレームが書きうる画素の色を、見つけた順に渡す
///
/// 入力が変わった画素は入力の色。`disposed` は廃棄方法が抜く矩形で、その中の
/// 持ち越し画素は画面に残っている色を書き直すことになる。透過標識は色を
/// 持たないため渡さない。`observe` が偽を返した時点で走査をやめる。
///
/// `rendered` は現在のテーブルで写した描画後の色。持ち越し画素の位置では
/// 画面の色がそのまま入っている。
fn escape_colors(
    layout: &Layout,
    previous: &[u8],
    pixels: &[u8],
    rendered: &[u8],
    disposed: Option<Rect>,
    mut observe: impl FnMut(u32) -> bool,
) {
    let bpp = layout.bytes_per_pixel;
    let carried =
        |at: usize| !previous.is_empty() && previous[at..at + bpp] == pixels[at..at + bpp];
    let mut take = |color: u32| color == TRANSPARENT || observe(color);

    for (index, pixel) in pixels.chunks_exact(bpp).enumerate() {
        let at = index * bpp;
        if carried(at) {
            continue;
        }
        if !take(pack(pixel, bpp)) {
            return;
        }
    }

    let Some(rect) = disposed else {
        return;
    };
    for y in 0..rect.height as usize {
        let row = (rect.y as usize + y) * layout.stride + rect.x as usize * bpp;
        for x in 0..rect.width as usize {
            let at = row + x * bpp;
            if !carried(at) {
                continue;
            }
            if !take(pack(&rendered[at..at + bpp], bpp)) {
                return;
            }
        }
    }
}

/// このフレームだけの色表を作る
///
/// 相異なる色が [`QUANTIZED_COLORS`] までならその色をそのまま載せ、超えるときは
/// 同じ色をヒストグラムへ積んで量子化する。
fn escape_table(
    layout: &Layout,
    previous: &[u8],
    pixels: &[u8],
    rendered: &[u8],
    disposed: Option<Rect>,
) -> Palette {
    let mut distinct = Colors::new();
    let mut overflowed = false;
    escape_colors(layout, previous, pixels, rendered, disposed, |color| {
        if !distinct.observe_color(color) || usize::from(distinct.count()) > QUANTIZED_COLORS {
            overflowed = true;
            return false;
        }
        true
    });

    if !overflowed {
        return Palette::from_colors(distinct.into_indexed(|_| ()).colors());
    }

    let mut histogram = Histogram::new();
    escape_colors(layout, previous, pixels, rendered, disposed, |color| {
        histogram.observe_color(color, 1);
        true
    });
    Palette::from_colors(&histogram.quantize(QUANTIZED_COLORS))
}

/// 書き出し位置のフレーム1つを処理する状態
///
/// 面が分かれる。[`Self::previous`] は「この画素は変わったか」を決め、
/// [`Self::canvas`] はデコーダが見ている色を持つ。差分矩形と透過ランは後者で
/// 求める。量子化を通すと別々の入力色が同じ色へ落ちることがあり、それは
/// 出力上は未変更だからで、比べる面を分けないとこの一致を見落とす。
struct Writing {
    /// 書き出しに使うカラーテーブル
    palettes: Palettes,
    /// 直前に書き出しへ渡されたフレームの正規化した入力
    previous: Vec<u8>,
    /// 描画後の色の面
    canvas: Canvas,
    /// 投入されたフレームを写した描画後の色
    rendered: Vec<u8>,
    /// 圧縮する添字を組み立てる作業領域
    indices: Vec<u8>,
    /// 書き出しを待っているフレーム
    pending: Option<Pending>,
    /// 描く直前へ戻す候補を立てる間合い
    pacing: RestorePacing,
}

impl Writing {
    fn new(layout: Layout) -> Self {
        Writing {
            palettes: Palettes::new(Palette::new()),
            previous: Vec::new(),
            canvas: Canvas::new(layout),
            rendered: Vec::new(),
            indices: Vec::new(),
            pending: None,
            pacing: RestorePacing::new(),
        }
    }
}

/// GIFのエンコーダ
///
/// [`Encoder::new`] で寸法とフレーム数を宣言し、[`Encoder::add_frame`] で
/// フレームを投入し、[`Encoder::finish`] で閉じる。
///
/// グローバルカラーテーブルは色が決まる前に位置だけ確保し、決まった時点で
/// そこへ書き戻す。投入されたフレームは先読みリングのぶんだけ遅れて書き出される。
pub struct Encoder<W: Write + Seek> {
    writer: W,
    layout: Layout,
    /// 書き出し位置から先を覗く窓
    ring: Ring,
    /// 書き出し位置のフレームを処理する状態
    writing: Writing,
    num_frames: u32,
    /// [`Self::add_frame`] が受け付けたフレーム数
    frames_accepted: u32,
    /// 書き出しに失敗し、ブロックの列が中断しているか
    poisoned: bool,
    /// グローバルカラーテーブルの据え方。書き戻すまでは `None`
    palette_kind: Option<PaletteKind>,
    /// ローカルカラーテーブルを書いたフレーム数
    local_tables: u32,
    /// 2値化で見た目が変わった画素数
    binarized: Binarized,
    /// 1/100秒への累積の丸め
    hundredths: Hundredths,
    /// 遅延を下限で切り上げたか
    delay_clamped: bool,
}

impl<W: Write + Seek> Encoder<W> {
    /// `width` x `height` の `num_frames` フレームを `writer` へ書き出す
    ///
    /// ヘッダ・論理画面記述子・グローバルカラーテーブルの場所・ループ回数を
    /// ここで書く。
    ///
    /// # Errors
    /// 寸法が0か65535を超えるとき [`Error::InvalidDimensions`]。フレーム数が0の
    /// とき [`Error::InvalidFrameCount`]。1フレームのバイト数が `usize` で
    /// 表現できないとき [`Error::ImageTooLarge`]。書き出しに失敗したとき
    /// [`Error::Io`]。
    pub fn new(
        mut writer: W,
        width: u32,
        height: u32,
        num_frames: u32,
        config: Config,
    ) -> Result<Self, Error> {
        if num_frames == 0 {
            return Err(Error::InvalidFrameCount);
        }
        let layout = Layout::new(width, height, config.color_type)?;

        block::header(&mut writer)?;
        block::logical_screen_descriptor(&mut writer, layout.width, layout.height)?;
        block::global_table_placeholder(&mut writer)?;
        block::netscape(&mut writer, config.num_plays)?;

        Ok(Encoder {
            writer,
            layout,
            ring: Ring::new(LOOKAHEAD),
            writing: Writing::new(layout),
            num_frames,
            frames_accepted: 0,
            poisoned: false,
            palette_kind: None,
            local_tables: 0,
            binarized: Binarized::default(),
            hundredths: Hundredths::new(),
            delay_clamped: false,
        })
    }

    /// フレームを1つ投入する
    ///
    /// `data` は [`Config::color_type`] の画素が左上から右下へ隙間なく
    /// 並んでいること。
    ///
    /// # Errors
    /// バイト数が寸法と色種別から決まる長さと違うとき
    /// [`Error::FrameSizeMismatch`]。宣言したフレーム数を超えたとき
    /// [`Error::FrameCountMismatch`]。
    pub fn add_frame(&mut self, data: &[u8], delay: FrameDelay) -> Result<(), Error> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        if data.len() != self.layout.frame_len {
            return Err(Error::FrameSizeMismatch {
                expected: self.layout.frame_len,
                actual: data.len(),
            });
        }
        if self.frames_accepted == self.num_frames {
            return Err(Error::FrameCountMismatch {
                expected: self.num_frames,
                actual: self.frames_accepted + 1,
            });
        }

        let pixels = match self.layout.color_type {
            ColorType::Rgb8 => Cow::Borrowed(data),
            ColorType::Rgba8 => {
                let mut pixels = data.to_vec();
                self.binarized += normalize::binarize(&mut pixels);
                Cow::Owned(pixels)
            }
        };

        // 途中で失敗するとブロックの列が中断した状態で残るため、以降の投入を拒否する
        self.accept(&pixels, delay)
            .inspect_err(|_| self.poisoned = true)?;

        self.frames_accepted += 1;
        Ok(())
    }

    /// 書き出しを終え、終端を書いて `writer` と結果を返す
    ///
    /// 一度も溢れずに来たなら、ここで色が決まってグローバルカラーテーブルへ
    /// 書き戻す。
    ///
    /// # Errors
    /// 投入されたフレーム数が宣言したフレーム数に満たないとき
    /// [`Error::FrameCountMismatch`]。
    pub fn finish(mut self) -> Result<(W, Report), Error> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        if self.frames_accepted != self.num_frames {
            return Err(Error::FrameCountMismatch {
                expected: self.num_frames,
                actual: self.frames_accepted,
            });
        }

        let (ring, writing, mut parts) = self.split();
        parts.drain(ring, writing)?;
        if let Some(pending) = writing.pending.take() {
            // 次のフレームが無く、廃棄方法が変えられるキャンバスの続きも無い
            parts.write_pending(pending, DISPOSAL_DO_NOT_DISPOSE)?;
        }
        parts.settle_exact(&writing.palettes.global)?;

        let approximated_pixels = writing.palettes.approximated();
        let substituted_pixels = writing.palettes.substituted();
        let black_fallback = writing.palettes.black_fallback();

        block::trailer(&mut self.writer)?;
        self.writer.flush()?;

        let report = Report {
            palette: self
                .palette_kind
                .expect("全フレームを書き終えた時点で色は決まっている"),
            local_tables: self.local_tables,
            approximated_pixels,
            substituted_pixels,
            black_fallback,
            binarized_to_transparent: self.binarized.to_transparent,
            binarized_to_opaque: self.binarized.to_opaque,
            delay_clamped: self.delay_clamped,
        };
        Ok((self.writer, report))
    }

    /// 先読みリング・書き出し位置の状態・それ以外の部品に分けて借りる
    fn split(&mut self) -> (&mut Ring, &mut Writing, Parts<'_, W>) {
        let Encoder {
            writer,
            layout,
            ring,
            writing,
            num_frames: _,
            frames_accepted: _,
            poisoned: _,
            palette_kind,
            local_tables,
            binarized: _,
            hundredths,
            delay_clamped,
        } = self;

        (
            ring,
            writing,
            Parts {
                writer,
                layout,
                palette_kind,
                local_tables,
                hundredths,
                delay_clamped,
            },
        )
    }

    /// 正規化したフレームを先読みリングへ入れ、溢れたぶんを書き出しへ渡す
    fn accept(&mut self, pixels: &[u8], delay: FrameDelay) -> Result<(), Error> {
        let (ring, writing, mut parts) = self.split();
        let Some(due) = ring.push(pixels.to_vec(), delay) else {
            return Ok(());
        };
        parts.write_frame(writing, ring, &due.pixels, due.delay)
    }
}

/// [`Encoder`] からフレームの窓と書き出し位置の状態以外を借りたもの
///
/// フレーム1つを処理する判断と書き出しを担う。
struct Parts<'a, W: Write + Seek> {
    writer: &'a mut W,
    layout: &'a Layout,
    palette_kind: &'a mut Option<PaletteKind>,
    local_tables: &'a mut u32,
    hundredths: &'a mut Hundredths,
    delay_clamped: &'a mut bool,
}

impl<W: Write + Seek> Parts<'_, W> {
    /// 先読みリングに残ったフレームをすべて書き出しへ渡す
    fn drain(&mut self, ring: &mut Ring, writing: &mut Writing) -> Result<(), Error> {
        while let Some(due) = ring.take() {
            self.write_frame(writing, ring, &due.pixels, due.delay)?;
        }
        Ok(())
    }

    /// 保留中のフレームを書き出し、渡されたフレームを保留にする
    ///
    /// 矩形も透過ランも、入力ではなく写した後の色の面で求める。廃棄方法は
    /// 保留中のフレームのもので、渡されたフレームが載る画面を決めるため、
    /// 先に決めてからその画面で矩形と添字を求める。
    fn write_frame(
        &mut self,
        writing: &mut Writing,
        ring: &Ring,
        pixels: &[u8],
        delay: FrameDelay,
    ) -> Result<(), Error> {
        let Writing {
            palettes,
            previous,
            canvas,
            rendered,
            indices,
            pending,
            pacing,
        } = writing;

        // 開いたテーブルはこのフレームの色をまとめて受け入れる。収まらなければ
        // そこで閉じ、以降の色は最近傍へ写る
        if palettes.global.is_open()
            && !palettes
                .global
                .admit(self.layout.bytes_per_pixel, previous, pixels)
        {
            self.settle(&mut palettes.global, ring, previous, pixels)?;
        }

        let mut mapped = canvas.render(previous, pixels, &mut palettes.global, rendered);
        // 色で埋まったテーブルは透過添字を持たない。標識を書く先が無いフレームは
        // 表現できないため、閉じて透過添字を取り直す
        if palettes.global.is_open() && self.lacks_transparent(&palettes.global, rendered, ring) {
            self.settle(&mut palettes.global, ring, previous, pixels)?;
            mapped = canvas.render(previous, pixels, &mut palettes.global, rendered);
        }

        // 変わった画素の誤差が床を超えたフレームと、透過添字の要るフレームは、
        // このフレームだけの色表へ逃げる
        if mapped.mean_error_exceeds(BENCH_TUNED_ESCAPE_FLOOR)
            || self.lacks_transparent(&palettes.global, rendered, ring)
        {
            debug_assert!(
                !palettes.global.is_open(),
                "書き戻していないテーブルから逃げている"
            );
            let disposed = pending.as_ref().and_then(|waiting| {
                (!canvas.kept().expressible(rendered)).then(|| canvas.widen(waiting.rect, rendered))
            });
            palettes.escape(escape_table(
                self.layout,
                previous,
                pixels,
                rendered,
                disposed,
            ));
            mapped = canvas.render(previous, pixels, palettes.current(), rendered);
        }
        palettes.current().note_approximated(mapped.approximated);
        palettes.current().note_substituted(mapped.substituted);

        let (delay, clamped) = self.hundredths.next(delay);
        *self.delay_clamped |= clamped;
        match pending.take() {
            None => {
                let laid = lay_out(canvas.kept(), rendered, palettes.current(), indices, delay);
                canvas.start(rendered);
                *pending = Some(laid);
            }
            Some(mut waiting) => {
                let (disposal, laid) = choose_disposal(
                    canvas,
                    palettes,
                    indices,
                    &mut waiting,
                    rendered,
                    delay,
                    pacing,
                );
                let disposed = waiting.rect;
                self.write_pending(waiting, disposal)?;
                canvas.advance(disposal, disposed, rendered, laid.rect);
                *pending = Some(laid);
            }
        }
        palettes.retire();

        previous.clear();
        previous.extend_from_slice(pixels);
        Ok(())
    }

    /// 開いたテーブルを閉じ、グローバルカラーテーブルを書き戻す
    ///
    /// 量子化の材料は書き出し位置のフレームと先読みの窓で、そこで入力が変わった
    /// 画素の色を積む。目標色数は透過添字の余地を残した空きぶん。
    fn settle(
        &mut self,
        palette: &mut Palette,
        ring: &Ring,
        previous: &[u8],
        pixels: &[u8],
    ) -> Result<(), Error> {
        let free = QUANTIZED_COLORS.saturating_sub(usize::from(palette.colors()));
        let mut quantized = Vec::new();
        if free > 0 {
            let histogram = material(
                self.layout,
                QUANTIZE_MATERIAL,
                previous,
                std::iter::once(pixels).chain(ring.window()),
            );
            if histogram.distinct() > 0 {
                quantized = histogram.quantize(free);
            }
        }

        palette.settle(&quantized);
        *self.palette_kind = Some(PaletteKind::Quantized {
            colors: palette.colors(),
        });
        self.write_global_table(palette)
    }

    /// 一度も溢れずに来たテーブルを、そのままグローバルカラーテーブルへ書き戻す
    fn settle_exact(&mut self, palette: &Palette) -> Result<(), Error> {
        if !palette.is_open() {
            return Ok(());
        }

        *self.palette_kind = Some(PaletteKind::Exact {
            colors: palette.colors(),
        });
        self.write_global_table(palette)
    }

    /// 確保しておいた位置へグローバルカラーテーブルを書き、元の位置へ戻る
    fn write_global_table(&mut self, palette: &Palette) -> Result<(), Error> {
        let resume = self.writer.stream_position()?;
        self.writer.seek(SeekFrom::Start(GLOBAL_TABLE_OFFSET))?;
        block::global_color_table(self.writer, &palette.global_bytes())?;
        self.writer.seek(SeekFrom::Start(resume))?;
        Ok(())
    }

    /// 透過標識があるのに、それを書く添字が無いか
    ///
    /// 見るのは写した面と、窓の先頭の1枚。廃棄方法はこのフレームを次のフレームと
    /// 突き合わせて決まり、矩形を抜いた先を透過として読ませるには、抜くフレーム
    /// 自身が透過添字を宣言している必要がある。
    fn lacks_transparent(&self, palette: &Palette, rendered: &[u8], ring: &Ring) -> bool {
        if self.layout.color_type != ColorType::Rgba8 || palette.transparent().is_some() {
            return false;
        }

        let bpp = self.layout.bytes_per_pixel;
        let marked = |plane: &[u8]| {
            plane
                .chunks_exact(bpp)
                .any(|pixel| pack(pixel, bpp) == TRANSPARENT)
        };
        marked(rendered) || ring.window().next().is_some_and(marked)
    }

    /// 保留していたフレームを `disposal` で書き出す
    fn write_pending(&mut self, pending: Pending, disposal: u8) -> Result<(), Error> {
        block::graphic_control(self.writer, disposal, pending.delay, pending.transparent)?;
        block::image_descriptor(
            self.writer,
            pending.rect.x as u16,
            pending.rect.y as u16,
            pending.rect.width as u16,
            pending.rect.height as u16,
            pending.local.as_ref().map(ColorTable::size_field),
        )?;
        if let Some(table) = &pending.local {
            block::color_table(self.writer, table.bytes())?;
            *self.local_tables += 1;
        }
        block::image_body(self.writer, pending.min_code_size, &pending.body)?;
        Ok(())
    }
}

/// 保留中のフレームの廃棄方法と、投入されたフレームの符号化を決める
///
/// 廃棄方法は保留中のフレーム自身のバイト列を変えず、投入されたフレームが載る
/// 画面だけを変える。「不透明 → 透過」の遷移を含まないフレームはキャンバスを
/// そのまま残し、含むフレームだけがキャンバスから画素を抜く候補を立てる。
///
/// 抜く候補が2つ立ったときは、両方を符号化して圧縮後の大きさで選ぶ。抜きたい
/// 画素が保留中のフレームの矩形の外にあるときは、その矩形を広げて符号化し直す。
/// 符号化し直すのは保留中のフレームなので、そのフレームを符号化したテーブルを
/// 引く。
fn choose_disposal(
    canvas: &mut Canvas,
    palettes: &mut Palettes,
    indices: &mut Vec<u8>,
    pending: &mut Pending,
    rendered: &mut [u8],
    delay: u16,
    pacing: &mut RestorePacing,
) -> (u8, Pending) {
    if canvas.kept().expressible(rendered) {
        let laid = lay_out(canvas.kept(), rendered, palettes.current(), indices, delay);
        return (DISPOSAL_DO_NOT_DISPOSE, laid);
    }

    // ここへ来るのは投入されたフレームに透過画素があるときだけで、そのとき
    // テーブルは必ず透過インデックスを持つ。抜いた画素を書かずに済ませる添字が
    // 無ければ、透過の位置そのものを表現できない
    debug_assert!(
        palettes.current().transparent().is_some(),
        "透過インデックスの無いテーブルに透過画素が現れた"
    );

    // 広げた分は描く直前の画面と一致する画素なので透過ランに潰れ、
    // 描いた後の画面は変わらない
    let widened = canvas.widen(pending.rect, rendered);
    if widened != pending.rect {
        let delay = pending.delay;
        let (restored, composite) = canvas.pending_frame();
        *pending = encode_on(
            restored,
            composite,
            widened,
            palettes.earlier(),
            indices,
            delay,
        );
    }

    let disposed = canvas.dispose(pending.rect);
    let background = disposed.background();
    debug_assert!(
        background.expressible(rendered),
        "広げた矩形を抜いても遷移が残っている"
    );

    let previous = disposed.previous();
    let cleared = lay_out(background, rendered, palettes.current(), indices, delay);
    if !previous.expressible(rendered) || !pacing.should_try() {
        return (DISPOSAL_RESTORE_TO_BACKGROUND, cleared);
    }

    let restored = lay_out(previous, rendered, palettes.current(), indices, delay);
    let taken = restored.body.len() < cleared.body.len();
    pacing.record(taken);
    if taken {
        (DISPOSAL_RESTORE_TO_PREVIOUS, restored)
    } else {
        (DISPOSAL_RESTORE_TO_BACKGROUND, cleared)
    }
}

/// `screen` の上で `frame` を符号化し、書き出しを待つフレームにする
///
/// 圧縮まで済ませる。廃棄方法の候補は圧縮後の大きさで比べるため、採った候補の
/// 圧縮結果をそのまま書き出しへ回す。
fn lay_out(
    screen: Screen<'_>,
    frame: &mut [u8],
    palette: &mut Palette,
    indices: &mut Vec<u8>,
    delay: u16,
) -> Pending {
    let rect = screen.rect_of(frame);
    encode_on(screen, frame, rect, palette, indices, delay)
}

/// `screen` の上で `frame` の `rect` を符号化し、書き出しを待つフレームにする
fn encode_on(
    screen: Screen<'_>,
    frame: &mut [u8],
    rect: Rect,
    palette: &mut Palette,
    indices: &mut Vec<u8>,
    delay: u16,
) -> Pending {
    indices.clear();
    screen.append_indices(frame, rect, palette, indices);

    let min_code_size = lzw::min_code_size(indices);
    let mut body = Vec::new();
    lzw::compress(&mut body, indices, min_code_size).expect("Vecへの書き出しは失敗しない");

    Pending {
        rect,
        delay,
        transparent: palette.transparent(),
        min_code_size,
        local: palette.local_table(),
        body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::Fit;
    use std::io::Cursor;

    /// 1色だけの閉じたテーブル
    fn table_of(pixel: &[u8; 3]) -> Palette {
        let mut palette = Palette::new();
        palette.admit(3, &[], pixel);
        palette.settle(&[]);
        palette
    }

    /// 手放した色表が最近傍へ写した画素も、報告に残る
    ///
    /// 逃げたフレームは自分の色表で符号化され、その色表は書き終えた時点で
    /// 手放される。数え落とすと、劣化しているのに
    /// [`Report::approximated_pixels`] が0を報せうる。
    #[test]
    fn a_retired_table_keeps_the_pixels_it_approximated() {
        let mut palettes = Palettes::new(table_of(&[1, 2, 3]));
        palettes.global.note_approximated(3);

        palettes.escape(table_of(&[4, 5, 6]));
        palettes.current().note_approximated(5);
        assert_eq!(palettes.approximated(), 8);

        // 逃げた色表は保留中のフレームのものになり、その次で手放される
        palettes.retire();
        palettes.retire();
        assert_eq!(
            palettes.approximated(),
            8,
            "手放した色表が写した画素が消えている"
        );
    }

    /// 逃げた色表は1フレームで手放され、次のフレームはグローバルへ戻る
    #[test]
    fn an_escaped_table_lasts_a_single_frame() {
        let mut palettes = Palettes::new(table_of(&[1, 2, 3]));
        palettes.escape(table_of(&[4, 5, 6]));
        assert_eq!(palettes.current().color_at(0), 0xFF06_0504);

        palettes.retire();
        assert_eq!(
            palettes.current().color_at(0),
            0xFF03_0201,
            "次のフレームが逃げた色表を引きずっている"
        );
        assert_eq!(
            palettes.earlier().color_at(0),
            0xFF06_0504,
            "保留中のフレームの色表が失われている"
        );
    }

    /// 逃げた色表は変わった画素の色をそのまま載せる
    #[test]
    fn an_escape_table_takes_the_colors_of_the_changed_pixels() {
        let layout = Layout::new(2, 1, ColorType::Rgb8).unwrap();
        let previous = [1u8, 2, 3, 9, 9, 9];
        let pixels = [1u8, 2, 3, 4, 5, 6];

        let mut table = escape_table(&layout, &previous, &pixels, &pixels, None);
        assert_eq!(table.colors(), 1, "変わっていない画素まで載せている");
        assert!(matches!(table.map(&[4, 5, 6], 3).fit, Fit::Exact));
        assert!(table.transparent().is_some(), "透過スロットが無い");
    }

    /// 変わった画素の色が上限を超えるフレームは、そのフレームで量子化する
    #[test]
    fn an_escape_table_quantizes_a_frame_with_too_many_colors() {
        const COLORS: u32 = 400;
        let layout = Layout::new(COLORS, 1, ColorType::Rgb8).unwrap();
        let pixels: Vec<u8> = (0..COLORS)
            .flat_map(|i| [(i % 20) as u8 * 12, (i / 20) as u8 * 12, 0x40])
            .collect();

        let table = escape_table(&layout, &[], &pixels, &pixels, None);
        assert_eq!(table.colors() as usize, QUANTIZED_COLORS);
        assert_eq!(table.transparent(), Some(QUANTIZED_COLORS as u8));
    }

    /// 連敗が続くと候補を立てるのを休み、休みが明けたらまた試す
    #[test]
    fn the_pacing_rests_after_a_streak_of_losses() {
        let mut pacing = RestorePacing::new();
        for _ in 0..RESTORE_LOSS_STREAK {
            assert!(pacing.should_try());
            pacing.record(false);
        }

        for frame in 0..RESTORE_REST_FRAMES {
            assert!(!pacing.should_try(), "休み {frame} フレーム目");
        }
        assert!(pacing.should_try());
    }

    /// 連敗が閾値に届くまでは候補を立てるのをやめない
    ///
    /// 少ない負けで見切ると勝ち負けの揺れを拾い、まだ採られる素材でも候補が
    /// 立たなくなる。休みに入るのは閾値に届いたときだけ。
    #[test]
    fn the_pacing_keeps_trying_below_the_streak() {
        let mut pacing = RestorePacing::new();
        for loss in 1..RESTORE_LOSS_STREAK {
            assert!(pacing.should_try(), "連敗 {loss} 回目");
            pacing.record(false);
            assert_eq!(pacing.resting, 0, "連敗 {loss} 回で休みに入っている");
        }
        assert!(pacing.should_try(), "閾値に届く前に休みに入っている");
    }

    /// 候補が採られると連敗は解ける
    #[test]
    fn a_taken_candidate_clears_the_losses() {
        let mut pacing = RestorePacing::new();
        for _ in 0..RESTORE_LOSS_STREAK - 1 {
            pacing.record(false);
        }
        pacing.record(true);

        for _ in 0..RESTORE_LOSS_STREAK - 1 {
            assert!(pacing.should_try());
            pacing.record(false);
        }
        assert!(pacing.should_try());
    }

    /// 透過背景を1画素ずつ動く不透明なスプライト
    ///
    /// 毎フレーム「不透明 → 透過」を作るので、抜く候補が毎フレーム立つ。抜いた
    /// 画面と戻した画面はどちらもスプライトの無い背景になり、候補は必ず引き分ける。
    fn moving_sprite(count: u32) -> Vec<Vec<u8>> {
        const WIDTH: u32 = 32;
        const HEIGHT: u32 = 4;

        (0..count)
            .map(|index| {
                let mut frame = vec![0u8; (WIDTH * HEIGHT) as usize * 4];
                for y in 0..2 {
                    let at = (y * WIDTH + index) as usize * 4;
                    frame[at..at + 8]
                        .copy_from_slice(&[0x80, 0x20, 0x40, 0xFF, 0x80, 0x20, 0x40, 0xFF]);
                }
                frame
            })
            .collect()
    }

    /// 廃棄方法を `decisions` 回決めた時点の間合い
    ///
    /// 廃棄方法が決まるのは書き出しへ渡されたフレームの1つ前なので、書き出しへ
    /// 渡すのは1つ多い。書き出しは先読みリングのぶん遅れる。
    fn pacing_after(decisions: u32) -> RestorePacing {
        let count = decisions + 1 + (LOOKAHEAD as u32 - 1);
        let config = Config {
            color_type: ColorType::Rgba8,
            ..Config::default()
        };
        let mut encoder = Encoder::new(Cursor::new(Vec::new()), 32, 4, count, config).unwrap();
        let delay = FrameDelay::new(1, 30).unwrap();
        for frame in moving_sprite(count) {
            encoder.add_frame(&frame, delay).unwrap();
        }

        encoder.writing.pacing
    }

    /// 書き出しの経路は、候補を立てる前に間合いを見る
    #[test]
    fn the_write_path_consults_the_pacing() {
        let pacing = pacing_after(RESTORE_LOSS_STREAK);
        assert_eq!(pacing.resting, RESTORE_REST_FRAMES, "連敗で休みに入らない");

        let pacing = pacing_after(RESTORE_LOSS_STREAK + 1);
        assert_eq!(
            pacing.resting,
            RESTORE_REST_FRAMES - 1,
            "休みがフレームごとに減らない"
        );
    }
}
