//! 偽ホストの素材を書き出し、ffprobeで読み直した`.avif`と突き合わせる
//!
//! Rustの復号器が無いので外部の道具に頼る。ffprobeが見つからない環境では
//! それに依る主張を飛ばす。

use super::*;
use aviutl2_host::{Host, Script, temp_path};
use std::process::Command;

/// 素材の寸法。軸の取り違えが値に出るよう幅と高さを違える
const WIDTH: u32 = 24;
const HEIGHT: u32 = 16;

/// 素材の枚数
const FRAMES: u32 = 6;

/// 素材の刻み。分子と分母の取り違えが値に出るよう互いに離す
const RATE: u32 = 30000;
const SCALE: u32 = 1001;

/// ffprobeが並べた項目を1行ずつ返す。道具が無ければ `None`
fn probe(path: &Path, args: &[&str]) -> Option<Vec<String>> {
    let output = Command::new("ffprobe")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(args)
        .output();

    match output {
        Ok(output) => {
            assert!(
                output.status.success(),
                "ffprobe が失敗した: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            Some(
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .map(str::to_owned)
                    .collect(),
            )
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            assert!(
                std::env::var_os("CI").is_none(),
                "ffprobe が見つからない。読み直せなければ、書いた内容を確かめられない"
            );
            eprintln!("ffprobe が見つからないため、この主張を飛ばす");
            None
        }
        Err(e) => panic!("ffprobe の起動に失敗した: {e}"),
    }
}

/// 素材の寸法・表示時間・枚数がそのまま`.avif`になる
///
/// 1秒あたりの刻みをrateに据えるので、1フレームはscale刻みになる。
#[test]
fn an_encoded_animation_carries_the_size_and_the_timing_of_the_material() {
    let path = temp_path("avif");
    let host = Host::open(
        Script::new()
            .size(WIDTH, HEIGHT)
            .frame_rate(RATE, SCALE)
            .frames(FRAMES)
            .savefile(&path),
    );

    host.encode::<AvifOutputPlugin>(&Config::default())
        .expect("書き出し");

    let entries = probe(
        &path,
        &[
            // 透過無しの avif は 0 が静止画、1 が動画のトラック
            "-select_streams",
            "1",
            "-count_frames",
            "-show_entries",
            "stream=width,height,time_base,duration_ts,nb_read_frames",
            "-of",
            "default=noprint_wrappers=1",
        ],
    );
    std::fs::remove_file(&path).expect("書き出した先の後始末");

    let Some(entries) = entries else {
        return;
    };
    assert_eq!(
        entries,
        [
            format!("width={WIDTH}"),
            format!("height={HEIGHT}"),
            format!("time_base=1/{RATE}"),
            format!("duration_ts={}", SCALE * FRAMES),
            format!("nb_read_frames={FRAMES}"),
        ]
    );
}

/// ホストが返さなかったフレームで書き出しを止め、その先を取りに行かない
#[test]
fn a_frame_the_host_refuses_stops_the_encoding() {
    const MISSING: i32 = 3;

    let path = temp_path("avif");
    let host = Host::open(
        Script::new()
            .size(WIDTH, HEIGHT)
            .frames(FRAMES)
            .missing_at(MISSING)
            .savefile(&path),
    );

    let error = host
        .encode::<AvifOutputPlugin>(&Config::default())
        .expect_err("返らないフレームがある");

    assert_eq!(
        host.get_video().len(),
        MISSING as usize + 1,
        "取りに行ったフレーム数"
    );
    assert_eq!(error, format!("フレーム取得エラー: フレーム {MISSING}"));
    assert!(!path.exists(), "{}", path.display());
}
