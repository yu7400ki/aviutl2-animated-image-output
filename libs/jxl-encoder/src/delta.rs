//! 書き出しを待っているフレームと、比べる相手から決まる差分矩形

use crate::Config;
use crate::error::Error;
use crate::layers::Layers;
use crate::layout::Layout;
use crate::split::{THRESHOLD, cut, exact_profile};
use anim_core::{Rect, Refresh, Triggers, crop, dirty_rect, tolerance};

/// 差分が空のまま書き出すときの矩形
const UNCHANGED: Rect = Rect {
    x: 0,
    y: 0,
    width: 1,
    height: 1,
};

/// 表示フレームが合成後のキャンバスを置く参照スロット
///
/// 枠0は非0の表示時間では「参照されない」を意味し、枠3はlibjxlがpatchの参照フレームに
/// 使う。残る2枠を表示フレームごとに入れ替えると、直前のキャンバスと2つ前のキャンバスが
/// 同時に生きる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    First,
    Second,
}

impl Slot {
    /// もう一方の枠
    fn other(self) -> Self {
        match self {
            Slot::First => Slot::Second,
            Slot::Second => Slot::First,
        }
    }

    /// フレームヘッダが持つ枠の番号
    fn number(self) -> u32 {
        match self {
            Slot::First => 1,
            Slot::Second => 2,
        }
    }
}

/// 矩形の外に残るキャンバス
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Base {
    /// 直前のキャンバス
    Previous,
    /// 2つ前のキャンバス。矩形の外は2つ前の状態へ戻る
    TwoBack,
}

/// フレームが書き直すキャンバスの範囲
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Region {
    /// キャンバス全体
    Whole,
    /// `source` の枠に入っているキャンバスへ重ねる一部
    Part { source: u32, rect: Rect },
}

impl Region {
    /// 書き直す矩形
    pub(crate) fn rect(self, layout: &Layout) -> Rect {
        match self {
            Region::Whole => layout.canvas(),
            Region::Part { rect, .. } => rect,
        }
    }

    /// 書き直す画素のバイト数
    pub(crate) fn byte_len(self, layout: &Layout) -> usize {
        self.rect(layout).area() as usize * layout.bytes_per_pixel
    }
}

/// 1枚の表示フレームを書く形
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// キャンバス全体を1枚で書く
    Whole,
    /// 矩形1枚で書く
    One(Base, Rect),
    /// 矩形2枚で書く。先の1枚は表示時間0の副フレームになる
    Two(Base, Rect, Rect),
}

impl Shape {
    /// `base` のキャンバスとの差分 `rect` を書き直すフレームの書き方
    ///
    /// キャンバス全体を覆う矩形は1枚の全面フレームになる。それ以外は、書かずに
    /// 済む画素が固定費に見合うなら2枚へ割る。`profile` は `rect` の中で書き直す
    /// 画素の分布を引くもの。
    fn of(
        layout: &Layout,
        base: Base,
        rect: Rect,
        profile: impl FnOnce() -> anim_core::Profile,
    ) -> Self {
        if rect == layout.canvas() {
            Shape::Whole
        } else if let Some((head, tail)) = cut(rect, THRESHOLD, profile) {
            Shape::Two(base, head, tail)
        } else {
            Shape::One(base, rect)
        }
    }

    /// 矩形の外に残るキャンバス。全面フレームは何も残さない
    fn base(self) -> Option<Base> {
        match self {
            Shape::Whole => None,
            Shape::One(base, _) | Shape::Two(base, _, _) => Some(base),
        }
    }

    /// 書き直す矩形を書き出す順に返す
    fn rects(self, layout: &Layout) -> impl Iterator<Item = Rect> {
        let (head, tail) = match self {
            Shape::Whole => (None, layout.canvas()),
            Shape::One(_, rect) => (None, rect),
            Shape::Two(_, head, tail) => (Some(head), tail),
        };
        head.into_iter().chain(std::iter::once(tail))
    }
}

/// 書き出しを待っているフレーム
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pending {
    /// 書き直す形
    shape: Shape,
    /// 合成後のキャンバスを置く参照スロット
    slot: Slot,
    /// tick数の表示時間
    duration: u32,
}

impl Pending {
    /// 合成後のキャンバスを置く参照スロット
    ///
    /// 割った矩形は同じ枠へ重ねていくので、書き出す全ての枚数で同じ値になる。
    pub(crate) fn save(self) -> u32 {
        self.slot.number()
    }

    /// 書き出す順の、フレーム1枚が書き直す範囲と表示時間
    ///
    /// 最後の1枚が表示時間を持ち、手前の矩形は表示時間0で最後の1枚へ畳まれる。
    /// 手前の矩形を重ねた結果は [`Self::save`] の枠に入るので、続く1枚はそちらを
    /// 土台にする。
    pub(crate) fn frames(self) -> impl Iterator<Item = (Region, u32)> {
        let slot = self.slot;
        let part = |base: Base, rect: Rect| Region::Part {
            source: match base {
                Base::Previous => slot.other(),
                Base::TwoBack => slot,
            }
            .number(),
            rect,
        };
        let (leading, last) = match self.shape {
            Shape::Whole => (None, Region::Whole),
            Shape::One(base, rect) => (None, part(base, rect)),
            Shape::Two(base, head, tail) => (
                Some(part(base, head)),
                Region::Part {
                    source: slot.number(),
                    rect: tail,
                },
            ),
        };
        leading
            .map(|region| (region, 0))
            .into_iter()
            .chain(std::iter::once((last, self.duration)))
    }
}

/// `frame` の `rect` を覆う行
fn rows_of<'a>(frame: &'a [u8], rect: Rect, layout: &Layout) -> impl Iterator<Item = &'a [u8]> {
    let row_len = rect.width as usize * layout.bytes_per_pixel;
    let head = rect.y as usize * layout.stride + rect.x as usize * layout.bytes_per_pixel;
    let stride = layout.stride;
    (0..rect.height as usize).map(move |row| {
        let start = head + row * stride;
        &frame[start..start + row_len]
    })
}

/// 1行ずつ渡された画素を `canvas` の `rect` へ写す
fn paste<'a>(canvas: &mut [u8], rect: Rect, layout: &Layout, rows: impl Iterator<Item = &'a [u8]>) {
    let row_len = rect.width as usize * layout.bytes_per_pixel;
    let head = rect.y as usize * layout.stride + rect.x as usize * layout.bytes_per_pixel;
    for (row, pixels) in rows.enumerate() {
        let start = head + row * layout.stride;
        canvas[start..start + row_len].copy_from_slice(pixels);
    }
}

/// 書く面積の小さい土台
///
/// `kept` は直前のキャンバスとの差分の外接矩形、`restored` は2つ前のキャンバスとの
/// もの。面積が並んだときは直前のキャンバスを採る。
fn narrower(kept: Rect, restored: Option<Rect>) -> (Base, Rect) {
    match restored {
        Some(rect) if rect.area() < kept.area() => (Base::TwoBack, rect),
        _ => (Base::Previous, kept),
    }
}

/// 自分の出力を復号して組み上げた画面
struct Screen {
    /// 書き出しを待っているフレームまでを映した画面
    ///
    /// そのフレームの矩形には、まだ層が返っていないので投入された入力が乗る。
    shown: Vec<u8>,
    /// 層を当て終えた、1つ前の表示フレームの画面
    shown_back: Vec<u8>,
    refresh: Refresh,
    /// 画面が入力から離れてよい量
    tolerance: u8,
    layers: Layers,
}

/// 差分矩形を決めるとき比べる相手
enum Basis {
    /// 投入された入力どうしを厳密に比べる
    Inputs {
        /// 直前に投入されたフレームを描く直前のキャンバス
        canvas: Vec<u8>,
    },
    /// 自分の出力を復号した結果と比べる
    Screen(Box<Screen>),
}

impl Basis {
    /// 先頭フレームを全面で書いた後の状態へ進める
    fn start(&mut self, data: &[u8]) {
        match self {
            Basis::Inputs { .. } => {}
            Basis::Screen(screen) => {
                screen.refresh.commit_whole(data);
                screen.shown.clear();
                screen.shown.extend_from_slice(data);
            }
        }
    }

    /// 投入されたフレームを書き直す形を決め、書いた後の状態へ進める
    ///
    /// 書き直す画素が1つも無ければ `None`。
    fn commit(&mut self, layout: &Layout, previous: &[u8], data: &[u8]) -> Option<Shape> {
        match self {
            Basis::Inputs { canvas } => {
                let kept = dirty_rect(previous, data, layout.stride, layout.bytes_per_pixel)?;
                let restored = (!canvas.is_empty()).then(|| {
                    dirty_rect(canvas, data, layout.stride, layout.bytes_per_pixel)
                        .unwrap_or(UNCHANGED)
                });
                let (base, rect) = narrower(kept, restored);
                let against = match base {
                    Base::Previous => previous,
                    Base::TwoBack => canvas,
                };
                Some(Shape::of(layout, base, rect, || {
                    exact_profile(against, data, layout, rect)
                }))
            }
            Basis::Screen(screen) => {
                let kept = screen.triggers(data, &screen.shown);
                let bounds = kept.bounds()?;
                let restored = (!screen.shown_back.is_empty())
                    .then(|| screen.triggers(data, &screen.shown_back));
                let (base, rect) = narrower(
                    bounds,
                    restored
                        .as_ref()
                        .map(|map| map.bounds().unwrap_or(UNCHANGED)),
                );
                let map = match base {
                    Base::Previous => &kept,
                    Base::TwoBack => restored.as_ref().expect("2つ前の地図が無い"),
                };
                let shape = Shape::of(layout, base, rect, || map.profile(rect));
                screen.apply(layout, &shape, map, data);
                Some(shape)
            }
        }
    }

    /// 差分が空のまま `UNCHANGED` の1枚を書いた後の状態へ進める
    fn commit_unchanged(&mut self, layout: &Layout, previous: &[u8], data: &[u8]) -> Shape {
        match self {
            Basis::Inputs { .. } => Shape::of(layout, Base::Previous, UNCHANGED, || {
                exact_profile(previous, data, layout, UNCHANGED)
            }),
            Basis::Screen(screen) => {
                let map = screen.triggers(data, &screen.shown);
                let shape = Shape::of(layout, Base::Previous, UNCHANGED, || map.profile(UNCHANGED));
                screen.apply(layout, &shape, &map, data);
                shape
            }
        }
    }

    /// 比べる相手を1つ送り、書き出したフレームの層を受ける面を空ける
    fn rotate(&mut self, previous: &mut Vec<u8>) {
        match self {
            Basis::Inputs { canvas } => std::mem::swap(canvas, previous),
            Basis::Screen(screen) => std::mem::swap(&mut screen.shown, &mut screen.shown_back),
        }
    }

    /// 層を当て終えた面から、保留中のフレーム `shape` までの画面を組む
    ///
    /// `frame` は保留中のフレームの入力。矩形の中はまだ層が返っていないので、
    /// 入力をそのまま置く。
    fn settle(&mut self, layout: &Layout, shape: Shape, frame: &[u8]) {
        match self {
            Basis::Inputs { .. } => {}
            Basis::Screen(screen) => {
                if shape.base() != Some(Base::TwoBack) {
                    screen.shown.clear();
                    screen.shown.extend_from_slice(&screen.shown_back);
                }
                for rect in shape.rects(layout) {
                    paste(
                        &mut screen.shown,
                        rect,
                        layout,
                        rows_of(frame, rect, layout),
                    );
                }
            }
        }
    }

    /// 比べる相手を手放し、最後の層を受ける面だけを残す
    fn close(&mut self) {
        match self {
            Basis::Inputs { canvas } => *canvas = Vec::new(),
            Basis::Screen(screen) => {
                std::mem::swap(&mut screen.shown, &mut screen.shown_back);
                screen.shown = Vec::new();
            }
        }
    }
}

impl Screen {
    /// `against` の画面に対して書き直す画素の地図
    fn triggers(&self, data: &[u8], against: &[u8]) -> Triggers {
        self.refresh.triggers(data, against, self.tolerance)
    }

    /// `shape` の矩形を `data` で書いた後の状態へ進める
    ///
    /// 割った矩形は巻き込む画素が1枚ごとに違うので、1枚ずつ書く。
    fn apply(&mut self, layout: &Layout, shape: &Shape, map: &Triggers, data: &[u8]) {
        for rect in shape.rects(layout) {
            self.refresh.commit(map, rect, data);
        }
    }
}

/// 直前のフレームの追跡と、書き出しを待っているフレーム
///
/// [`Self::previous`] が埋まっているのは、書き出しを待っているフレームがある間だけ。
pub(crate) struct Delta {
    /// 直前に投入されたフレーム
    previous: Vec<u8>,
    pending: Option<Pending>,
    /// 取り出したフレームが書き直す画素
    staged: Vec<u8>,
    basis: Basis,
}

impl Delta {
    /// `config` が可逆なら入力どうしを、非可逆なら復号結果を比べる相手にする
    ///
    /// # Errors
    /// 復号器を組み立てられないとき [`Error::Decode`]。
    pub(crate) fn new(layout: &Layout, config: &Config) -> Result<Self, Error> {
        let basis = if config.is_lossless() {
            Basis::Inputs { canvas: Vec::new() }
        } else {
            Basis::Screen(Box::new(Screen {
                shown: Vec::new(),
                shown_back: Vec::new(),
                refresh: Refresh::new(layout.width, layout.height, layout.color_type.into()),
                tolerance: tolerance(config.quality),
                layers: Layers::new(layout.color_type, config.max_threads)?,
            }))
        };
        Ok(Delta {
            previous: Vec::new(),
            pending: None,
            staged: Vec::new(),
            basis,
        })
    }

    /// 投入されたフレームを保留し、押し出された保留中のフレームを返す
    ///
    /// 投入されたフレームに書き直す画素が無いときは、保留中のフレームの表示時間へ
    /// 畳んで `None` を返す。畳んだ表示時間が `u32` に収まらないときは、保留中の
    /// フレームを押し出して投入されたフレームを保留し直す。
    ///
    /// 返したフレームが書き直す画素は [`Self::pixels`] にある。書き出して層を
    /// [`Self::feed`] へ渡したら [`Self::settle`] を呼ぶ。
    pub(crate) fn advance(
        &mut self,
        layout: &Layout,
        data: &[u8],
        duration: u32,
    ) -> Option<Pending> {
        let Some(mut pending) = self.pending else {
            // 先頭フレームには土台にする直前のフレームが無い
            self.previous.clear();
            self.previous.extend_from_slice(data);
            self.basis.start(data);
            self.pending = Some(Pending {
                shape: Shape::Whole,
                slot: Slot::First,
                duration,
            });
            return None;
        };

        let shape = match self.commit(layout, data) {
            Some(shape) => shape,
            None => match pending.duration.checked_add(duration) {
                Some(total) => {
                    pending.duration = total;
                    self.pending = Some(pending);
                    return None;
                }
                None => self.commit_unchanged(layout, data),
            },
        };

        self.stage(layout, pending.shape);
        self.rotate();
        self.previous.clear();
        self.previous.extend_from_slice(data);
        self.pending = Some(Pending {
            shape,
            slot: pending.slot.other(),
            duration,
        });
        Some(pending)
    }

    /// 保留中のフレームを取り出し、比べる相手を手放す
    ///
    /// 返したフレームが書き直す画素は [`Self::pixels`] にある。
    pub(crate) fn take(&mut self, layout: &Layout) -> Option<Pending> {
        let pending = self.pending.take()?;
        self.stage(layout, pending.shape);
        self.basis.close();
        self.previous = Vec::new();
        Some(pending)
    }

    /// 書き出したフレームの層を当て終えた面から、次に比べる相手を組む
    pub(crate) fn settle(&mut self, layout: &Layout) {
        let Delta {
            previous,
            pending,
            basis,
            ..
        } = self;
        if let Some(pending) = pending {
            basis.settle(layout, pending.shape, previous);
        }
    }

    /// 取り出したフレームが書き直す画素
    ///
    /// [`Pending::frames`] が返す順に、矩形の画素が隙間なく並ぶ。
    pub(crate) fn pixels(&self) -> &[u8] {
        &self.staged
    }

    /// 書いた矩形を控える
    pub(crate) fn wrote(&mut self, rect: Rect) {
        if let Basis::Screen(screen) = &mut self.basis {
            screen.layers.wrote(rect);
        }
    }

    /// 書き出したバイト列を復号器へ継ぎ足し、揃った層を画面へ当てる
    ///
    /// # Errors
    /// libjxlの復号が失敗したとき [`Error::Decode`]。
    pub(crate) fn feed(&mut self, layout: &Layout, bytes: &[u8]) -> Result<(), Error> {
        let Basis::Screen(screen) = &mut self.basis else {
            return Ok(());
        };
        let Screen {
            layers, shown_back, ..
        } = &mut **screen;
        layers.feed(bytes, |rect, layer| {
            let row_len = rect.width as usize * layout.bytes_per_pixel;
            paste(shown_back, rect, layout, layer.chunks_exact(row_len));
        })
    }

    /// 層を取り出す復号器
    #[cfg(test)]
    pub(crate) fn decoder(&self) -> Option<&Layers> {
        match &self.basis {
            Basis::Inputs { .. } => None,
            Basis::Screen(screen) => Some(&screen.layers),
        }
    }

    /// 層を当て終えた、直近に書き出した表示フレームの画面
    #[cfg(test)]
    pub(crate) fn screen(&self) -> Option<&[u8]> {
        match &self.basis {
            Basis::Inputs { .. } => None,
            Basis::Screen(screen) => Some(&screen.shown_back),
        }
    }

    fn commit(&mut self, layout: &Layout, data: &[u8]) -> Option<Shape> {
        let Delta {
            previous, basis, ..
        } = self;
        basis.commit(layout, previous, data)
    }

    fn commit_unchanged(&mut self, layout: &Layout, data: &[u8]) -> Shape {
        let Delta {
            previous, basis, ..
        } = self;
        basis.commit_unchanged(layout, previous, data)
    }

    fn rotate(&mut self) {
        let Delta {
            previous, basis, ..
        } = self;
        basis.rotate(previous);
    }

    /// 直前のフレームのうち `shape` が書き直す画素を [`Self::staged`] へ移す
    fn stage(&mut self, layout: &Layout, shape: Shape) {
        self.staged.clear();
        for rect in shape.rects(layout) {
            crop(
                &self.previous,
                rect,
                layout.stride,
                layout.bytes_per_pixel,
                layout.bytes_per_pixel,
                &mut self.staged,
            );
        }
    }
}
