//! 偽ホストの素材を書き出し、読み直した`.webp`と突き合わせる

use super::*;
use aviutl2_host::{Host, Script, temp_path};
use image_webp::WebPDecoder;
use std::io::BufReader;

/// 素材の寸法。軸の取り違えが値に出るよう幅と高さを違える
const WIDTH: u32 = 24;
const HEIGHT: u32 = 10;

/// 素材の枚数
const FRAMES: u32 = 6;

/// 素材の刻み。分子と分母の取り違えが値に出るよう互いに離す
const RATE: u32 = 30000;
const SCALE: u32 = 1001;

/// キャンバスの寸法と、フレームごとの表示時間 (ms)
fn decode(path: &Path) -> ((u32, u32), Vec<u32>) {
    let mut decoder = WebPDecoder::new(BufReader::new(File::open(path).expect("書き出した先")))
        .expect("WebPとして読める");
    let size = decoder.dimensions();

    let mut buffer = vec![0u8; decoder.output_buffer_size().expect("出力の大きさ")];
    let durations = (0..decoder.num_frames())
        .map(|_| decoder.read_frame(&mut buffer).expect("フレームが読める"))
        .collect();

    (size, durations)
}

/// 素材の寸法・表示時間・枚数がそのまま`.webp`になる
///
/// 1フレームはscale / rate秒で、ミリ秒へ累積で丸めると33と34を混ぜる。
#[test]
fn an_encoded_animation_carries_the_size_and_the_timing_of_the_material() {
    let path = temp_path("webp");
    let host = Host::open(
        Script::new()
            .size(WIDTH, HEIGHT)
            .frame_rate(RATE, SCALE)
            .frames(FRAMES)
            .savefile(&path),
    );

    host.encode::<WebpOutputPlugin>(&Config::default())
        .expect("書き出し");

    let (size, durations) = decode(&path);
    std::fs::remove_file(&path).expect("書き出した先の後始末");

    assert_eq!(size, (WIDTH, HEIGHT), "キャンバスの寸法");
    assert_eq!(durations.len(), FRAMES as usize, "フレーム数");
    assert_eq!(durations, [33, 34, 33, 33, 34, 33], "ANMFの表示時間");
}

/// ホストが返さなかったフレームで書き出しを止め、その先を取りに行かない
#[test]
fn a_frame_the_host_refuses_stops_the_encoding() {
    const MISSING: i32 = 3;

    let path = temp_path("webp");
    let host = Host::open(
        Script::new()
            .size(WIDTH, HEIGHT)
            .frames(FRAMES)
            .missing_at(MISSING)
            .savefile(&path),
    );

    let error = host
        .encode::<WebpOutputPlugin>(&Config::default())
        .expect_err("返らないフレームがある");

    assert_eq!(
        host.get_video().len(),
        MISSING as usize + 1,
        "取りに行ったフレーム数"
    );
    assert_eq!(error, format!("フレーム取得エラー: フレーム {MISSING}"));
    assert!(!path.exists(), "{}", path.display());
}
