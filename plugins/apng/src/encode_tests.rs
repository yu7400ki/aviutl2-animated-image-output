//! 偽ホストの素材を書き出し、読み直した`.apng`と突き合わせる

use super::*;
use aviutl2_host::{Host, Script, temp_path};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// 素材の寸法。軸の取り違えが値に出るよう幅と高さを違える
const WIDTH: u32 = 24;
const HEIGHT: u32 = 10;

/// 素材の枚数
const FRAMES: u32 = 6;

/// 素材の刻み。分子と分母の取り違えが値に出るよう互いに離す
const RATE: u32 = 30000;
const SCALE: u32 = 1001;

/// IHDRの寸法と、フレームごとのfcTLの表示時間 (分子, 分母)
fn decode(path: &Path) -> ((u32, u32), Vec<(u16, u16)>) {
    let mut reader = png::Decoder::new(BufReader::new(File::open(path).expect("書き出した先")))
        .read_info()
        .expect("PNGとして読める");
    let size = (reader.info().width, reader.info().height);
    let num_frames = reader
        .info()
        .animation_control()
        .expect("acTLがある")
        .num_frames;

    let mut buffer = vec![0u8; reader.output_buffer_size().expect("出力の大きさ")];
    let delays = (0..num_frames)
        .map(|_| {
            reader.next_frame(&mut buffer).expect("フレームが読める");
            let control = reader.info().frame_control().expect("fcTLがある");
            (control.delay_num, control.delay_den)
        })
        .collect();

    (size, delays)
}

/// 素材の寸法・表示時間・枚数がそのまま`.apng`になる
///
/// 1フレームはscale / rate秒なので、fcTLの分子がscale、分母がrateになる。
#[test]
fn an_encoded_animation_carries_the_size_and_the_timing_of_the_material() {
    let path = temp_path("png");
    let host = Host::open(
        Script::new()
            .size(WIDTH, HEIGHT)
            .frame_rate(RATE, SCALE)
            .frames(FRAMES)
            .savefile(&path),
    );

    host.encode::<ApngOutputPlugin>(&Config::default())
        .expect("書き出し");

    let (size, delays) = decode(&path);
    std::fs::remove_file(&path).expect("書き出した先の後始末");

    assert_eq!(size, (WIDTH, HEIGHT), "IHDRの寸法");
    assert_eq!(delays.len(), FRAMES as usize, "fcTLの数");
    assert_eq!(
        delays,
        vec![(SCALE as u16, RATE as u16); FRAMES as usize],
        "fcTLの表示時間"
    );
}

/// ホストが返さなかったフレームで書き出しを止め、その先を取りに行かない
#[test]
fn a_frame_the_host_refuses_stops_the_encoding() {
    const MISSING: i32 = 3;

    let path = temp_path("png");
    let host = Host::open(
        Script::new()
            .size(WIDTH, HEIGHT)
            .frames(FRAMES)
            .missing_at(MISSING)
            .savefile(&path),
    );

    let error = host
        .encode::<ApngOutputPlugin>(&Config::default())
        .expect_err("返らないフレームがある");

    assert_eq!(error, format!("フレーム取得エラー: フレーム {MISSING}"));
    assert!(!path.exists(), "{}", path.display());
    assert_eq!(
        host.get_video().len(),
        MISSING as usize + 1,
        "取りに行ったフレーム数"
    );
}
