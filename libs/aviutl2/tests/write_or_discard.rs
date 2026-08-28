//! `write_or_discard` が書きかけのファイルを残さないことを検証する

use aviutl2::write_or_discard;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

/// まだ存在しない一時ファイルの場所
fn temp_path() -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    std::env::temp_dir().join(format!(
        "write-or-discard-{}-{}.bin",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

/// 書き出しに成功したら、出力先のファイルはそのまま残る
#[test]
fn a_completed_write_keeps_its_file() {
    let path = temp_path();
    let result = write_or_discard(&path, |mut file| {
        file.write_all(b"done").map_err(|e| e.to_string())
    });

    assert_eq!(result, Ok(()));
    assert!(path.exists(), "{}", path.display());
    std::fs::remove_file(&path).unwrap();
}

/// 書き出しに失敗したら、そこまで書いたファイルは消える
#[test]
fn a_failed_write_leaves_no_file() {
    let path = temp_path();
    let result = write_or_discard(&path, |mut file| {
        file.write_all(b"partial").map_err(|e| e.to_string())?;
        Err("エンコード失敗".into())
    });

    assert_eq!(result, Err("エンコード失敗".into()));
    assert!(!path.exists(), "{}", path.display());
}

/// 消せなかったときは、ファイルが残ったことを元のエラーへ添える
///
/// 出力先をディレクトリへ入れ替えて、削除が必ず失敗する状況を作る。
#[test]
fn a_file_that_cannot_be_removed_is_reported_alongside_the_error() {
    let path = temp_path();
    let result = write_or_discard(&path, |file| {
        drop(file);
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        std::fs::create_dir(&path).map_err(|e| e.to_string())?;
        Err("エンコード失敗".into())
    });

    std::fs::remove_dir(&path).unwrap();

    let message = result.expect_err("ディレクトリは消せない");
    assert!(message.starts_with("エンコード失敗 ("), "{message}");
    assert!(
        message.contains("書きかけのファイルが残りました"),
        "{message}"
    );
}

/// 出力先を作れなかったときは `write` を呼ばずに返す
///
/// 出力先をディレクトリにして、作成が必ず失敗する状況を作る。
#[test]
fn a_path_that_cannot_be_created_never_reaches_the_write() {
    let path = temp_path();
    std::fs::create_dir(&path).unwrap();
    let mut called = false;

    let result = write_or_discard(&path, |_| {
        called = true;
        Ok(())
    });

    std::fs::remove_dir(&path).unwrap();

    let message = result.expect_err("ディレクトリは作成先にできない");
    assert!(message.starts_with("ファイル作成エラー: "), "{message}");
    assert!(!called);
}
