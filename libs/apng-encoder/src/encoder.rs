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
use anim_core::{ColorType, FrameDelay, Pacing, Rect, crop};
use std::io::Write;
use std::num::NonZeroUsize;
use std::ops::RangeInclusive;

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

/// 書き出しが持ち越す状態
struct Writing {
    /// 書き出しを待っているフレーム
    pending: Option<Pending>,
    /// blend_op=OVERの候補を立てるかどうかの間合い
    blend_pacing: Pacing,
}

impl Writing {
    /// まだ何も保留していない状態
    fn new() -> Self {
        Writing {
            pending: None,
            blend_pacing: Pacing::new(BLEND_LOSS_STREAK, BLEND_REST_FRAMES),
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
///
/// 落としたエンコーダはワーカーを畳んでから返る。
pub struct Encoder<W: Write> {
    /// チャンクを並べる書き出し先
    chunks: ChunkWriter<W>,
    /// キャンバスの大きさと入力フレームのバイト並び
    layout: Layout,
    /// 圧縮の投入口と、番号を指す結果の受け取り
    pipeline: Pipeline,
    /// 直前のフレームとキャンバスの追跡
    delta: Delta,
    /// 書き出しの状態
    writing: Writing,
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
    /// 圧縮を回すワーカーは1つ。
    ///
    /// # Errors
    /// 幅・高さ・フレーム数が0のとき、1フレームのバイト数が `usize` で表現できないとき、
    /// 圧縮レベルが範囲外のとき、または書き出しに失敗したとき。
    pub fn new(
        writer: W,
        width: u32,
        height: u32,
        num_frames: u32,
        config: Config,
    ) -> Result<Self, Error> {
        Self::with_workers(writer, width, height, num_frames, config, NonZeroUsize::MIN)
    }

    /// ワーカー数を指してエンコーダを作る
    ///
    /// 渡した数をそのまま起こす。ワーカーが1つなら群れを起こさず、投入した場で
    /// 圧縮する。決定も書き出しの順序もワーカー数に依らないので、出力はどの数でも
    /// 同じになる。
    ///
    /// # Errors
    /// [`Encoder::new`] と同じ。加えてスレッドを起こせないとき。
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
        let mut chunks = ChunkWriter::new(writer);
        Self::open(&mut chunks, &layout, num_frames, config)?;
        // 書き出し先がヘッダを受け取ってから起こす
        let pipeline = Pipeline::new(config.compression_level, workers)?;

        Ok(Encoder {
            chunks,
            layout,
            pipeline,
            delta: Delta::new(),
            writing: Writing::new(),
            num_frames,
            frames_accepted: 0,
            poisoned: false,
        })
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
    /// 投入されたフレームはその場では書き出さず、dispose_opが決まる次の呼び出し、
    /// または [`Encoder::finish`] で書き出す。
    ///
    /// # Errors
    /// `data` の長さが合わないとき、宣言したフレーム数を超えたとき、書き出しに
    /// 失敗したとき、または過去の失敗でエンコーダが使用不能なとき。
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

    /// 保留中のフレームを書き出し、投入されたフレームを保留にする
    fn accept(&mut self, data: &[u8], delay: FrameDelay) -> Result<(), Error> {
        let disposal = self.choose_dispose(data);
        let (dispose, rect) = (disposal.op, disposal.rect);
        let (blend, candidate) = self.choose_blend(data, disposal);
        let body = candidate.into_body();

        self.flush_pending(dispose)?;
        self.writing.pending = Some(Pending {
            rect,
            delay,
            blend,
            body,
        });
        self.delta.advance(data, dispose);
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
    /// 捨てないときの候補はワーカーへ回し、捨てるときの候補を駆動スレッドで圧縮する
    /// あいだに進む。
    fn choose_dispose(&mut self, data: &[u8]) -> Disposal {
        let frame = self.frames_accepted;
        let disposable = self.writing.pending.is_some();
        let kept = self.delta.kept_rect(&self.layout, data, frame);
        let restored = self
            .delta
            .restored_rect(&self.layout, data, kept, frame, disposable);

        let job = self.submit_rect(data, kept);
        let restored = restored.map(|rect| (rect, self.compress_rect(data, rect)));
        let kept_candidate = self.pipeline.take(job);

        let keep = |candidate| Disposal {
            op: DISPOSE_OP_NONE,
            rect: kept,
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

    /// 投入されたフレームをキャンバスへ重ねる方法を決める
    ///
    /// 矩形の中で変化した画素がすべて不透明なら、変化していない画素を完全な透明へ
    /// 潰した候補が立つ。blend_op=OVERはその画素でキャンバスを残すため、潰しても
    /// 元の値に戻る。`disposal` の候補と両方を圧縮して小さい方を採り、採った側を
    /// 戻り値へ残して、退けた側のバッファは配り直す先へ返す。同じ大きさならSOURCEを採る。
    ///
    /// 潰した画素を書けない出力では候補が立たない。先頭フレームはキャンバスがまだ空で、
    /// 重ねる先が無い。負けが続く間は [`Pacing`] が候補を立てるのを休ませる。
    fn choose_blend(&mut self, data: &[u8], disposal: Disposal) -> (u8, Candidate) {
        let Disposal {
            op: dispose,
            rect,
            candidate: source,
        } = disposal;

        // 潰した画素は完全な透明として書くため、アルファを持つ出力でしか置けない
        if !matches!(self.layout.input, ColorType::Rgba8) {
            return (BLEND_OP_SOURCE, source);
        }
        if self.frames_accepted == 0 || !self.writing.blend_pacing.should_try() {
            return (BLEND_OP_SOURCE, source);
        }

        // 保留中のフレームを捨てると、キャンバスはそれを描く直前の内容へ戻る
        let base = if dispose == DISPOSE_OP_NONE {
            &self.delta.previous
        } else {
            &self.delta.canvas
        };
        let stride = self.layout.stride;
        let mut over = self.pipeline.buffer();
        if !over::pack_over(base, data, stride, rect, &mut over) {
            self.pipeline.recycle(over);
            return (BLEND_OP_SOURCE, source);
        }

        let bpp = self.layout.bytes_per_pixel;
        let job = self.pipeline.submit(over, rect.width as usize * bpp, bpp);
        let over_candidate = self.pipeline.take(job);

        let taken = over_candidate.len() < source.len();
        self.writing.blend_pacing.record(taken);
        if taken {
            self.pipeline.recycle(source.into_body());
            (BLEND_OP_OVER, over_candidate)
        } else {
            self.pipeline.recycle(over_candidate.into_body());
            (BLEND_OP_SOURCE, source)
        }
    }

    /// 保留中のフレームを `dispose` で書き出す
    fn flush_pending(&mut self, dispose: u8) -> Result<(), Error> {
        let Some(pending) = self.writing.pending.take() else {
            return Ok(());
        };

        self.chunks.write_frame(
            pending.rect,
            pending.delay,
            dispose,
            pending.blend,
            &pending.body,
        )?;
        self.pipeline.recycle(pending.body);
        Ok(())
    }

    /// フレームから `rect` を切り出す
    fn crop_rect(&mut self, data: &[u8], rect: Rect) -> Vec<u8> {
        let bpp = self.layout.bytes_per_pixel;
        let mut region = self.pipeline.buffer();
        crop(data, rect, self.layout.stride, bpp, bpp, &mut region);
        region
    }

    /// フレームから `rect` を切り出し、圧縮を投入する
    fn submit_rect(&mut self, data: &[u8], rect: Rect) -> usize {
        let region = self.crop_rect(data, rect);
        let bpp = self.layout.bytes_per_pixel;
        self.pipeline.submit(region, rect.width as usize * bpp, bpp)
    }

    /// フレームから `rect` を切り出し、駆動スレッドで圧縮する
    fn compress_rect(&mut self, data: &[u8], rect: Rect) -> Candidate {
        let region = self.crop_rect(data, rect);
        let bpp = self.layout.bytes_per_pixel;
        let candidate = self
            .pipeline
            .compress(&region, rect.width as usize * bpp, bpp);
        self.pipeline.recycle(region);
        candidate
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk;
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

    /// フレームを受け付けたエンコーダが持つ間合い
    fn pacing_of<W: Write>(encoder: &Encoder<W>) -> &Pacing {
        &encoder.writing.blend_pacing
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
        assert_eq!(blend_pacing.resting(), BLEND_REST_FRAMES);

        encoder.add_frame(&speckled, delay).unwrap();
        let blend_pacing = pacing_of(&encoder);
        assert_eq!(blend_pacing.resting(), BLEND_REST_FRAMES - 1);
    }
}
