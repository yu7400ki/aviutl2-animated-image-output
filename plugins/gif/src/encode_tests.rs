//! 偽ホストの素材を書き出し、読み直した`.gif`と突き合わせる

use super::*;
use aviutl2_host::{Host, Script, temp_path};
use std::path::Path;

/// 素材の寸法。軸の取り違えが値に出るよう幅と高さを違える
const WIDTH: u32 = 24;
const HEIGHT: u32 = 10;

/// 素材の枚数
const FRAMES: u32 = 6;

/// 素材の刻み。分子と分母の取り違えが値に出るよう互いに離す
const RATE: u32 = 30000;
const SCALE: u32 = 1001;

/// 論理画面の寸法と、フレームごとの表示時間 (1/100秒)
fn decode(path: &Path) -> ((u16, u16), Vec<u16>) {
    let bytes = std::fs::read(path).expect("書き出した先");
    let mut decoder = gif::DecodeOptions::new()
        .read_info(bytes.as_slice())
        .expect("GIFとして読める");
    let size = (decoder.width(), decoder.height());

    let mut delays = Vec::new();
    while let Some(frame) = decoder.read_next_frame().expect("フレームが読める") {
        delays.push(frame.delay);
    }

    (size, delays)
}

/// 素材の寸法・表示時間・枚数がそのまま`.gif`になる
///
/// 1フレームはscale / rate秒で、1/100秒へ累積で丸めると3と4を繰り返す。
#[test]
fn an_encoded_animation_carries_the_size_and_the_timing_of_the_material() {
    let path = temp_path("gif");
    let host = Host::open(
        Script::new()
            .size(WIDTH, HEIGHT)
            .frame_rate(RATE, SCALE)
            .frames(FRAMES)
            .savefile(&path),
    );

    host.encode::<GifOutputPlugin>(&Config::default())
        .expect("書き出し");

    let (size, delays) = decode(&path);
    std::fs::remove_file(&path).expect("書き出した先の後始末");

    assert_eq!(size, (WIDTH as u16, HEIGHT as u16), "論理画面の寸法");
    assert_eq!(delays.len(), FRAMES as usize, "フレーム数");
    assert_eq!(delays, [3, 4, 3, 3, 4, 3], "グラフィック制御の表示時間");
}

/// ホストが返さなかったフレームで書き出しを止め、その先を取りに行かない
#[test]
fn a_frame_the_host_refuses_stops_the_encoding() {
    const MISSING: i32 = 3;

    let path = temp_path("gif");
    let host = Host::open(
        Script::new()
            .size(WIDTH, HEIGHT)
            .frames(FRAMES)
            .missing_at(MISSING)
            .savefile(&path),
    );

    let error = host
        .encode::<GifOutputPlugin>(&Config::default())
        .expect_err("返らないフレームがある");

    assert_eq!(error, format!("フレーム取得エラー: フレーム {MISSING}"));
    assert!(!path.exists(), "{}", path.display());
    assert_eq!(
        host.get_video().len(),
        MISSING as usize + 1,
        "取りに行ったフレーム数"
    );
}
