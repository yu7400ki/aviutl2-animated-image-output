//! `output2.h` — 出力プラグイン ヘッダーファイル for AviUtl ExEdit2 の忠実な翻訳

use crate::{DWORD, LPCWSTR};
use std::os::raw::{c_int, c_void};
use windows::Win32::Foundation::{HINSTANCE, HWND};

/// プロジェクトファイル構造体 (plugin2.hに定義されています)
#[repr(C)]
pub struct PROJECT_FILE {
    _private: [u8; 0],
}

/// 出力情報構造体
#[repr(C)]
pub struct OUTPUT_INFO {
    /// フラグ
    pub flag: c_int,
    /// 幅
    pub w: c_int,
    /// 高さ
    pub h: c_int,
    /// フレームレート
    pub rate: c_int,
    /// スケール
    pub scale: c_int,
    /// フレーム数
    pub n: c_int,
    /// 音声サンプリングレート
    pub audio_rate: c_int,
    /// 音声チャンネル数
    pub audio_ch: c_int,
    /// 音声サンプリング数
    pub audio_n: c_int,
    /// セーブファイル名へのポインタ
    pub savefile: LPCWSTR,

    /// DIB形式の画像データを取得します
    /// - frame: フレーム番号
    /// - format: 画像フォーマット
    ///   (0 = RGB24bit / 'PA64' = PA64 / 'HF64' = HF64 / 'YUY2' = YUY2 / 'YC48' = YC48)
    /// - 戻り値: データへのポインタ
    ///   (画像データポインタの内容は次に外部関数を使うかメインに処理を戻すまで有効)
    pub func_get_video: Option<unsafe extern "C" fn(frame: c_int, format: DWORD) -> *mut c_void>,

    /// PCM形式の音声データへのポインタを取得します
    /// - start: 開始サンプル番号
    /// - length: 読み込むサンプル数
    /// - readed: 読み込まれたサンプル数
    /// - format: 音声フォーマット (1 = PCM16bit / 3 = PCM(float)32bit)
    /// - 戻り値: データへのポインタ
    ///   (音声データポインタの内容は次に外部関数を使うかメインに処理を戻すまで有効)
    pub func_get_audio: Option<
        unsafe extern "C" fn(
            start: c_int,
            length: c_int,
            readed: *mut c_int,
            format: DWORD,
        ) -> *mut c_void,
    >,

    /// 中断するか調べます
    /// - 戻り値: trueなら中断
    pub func_is_abort: Option<unsafe extern "C" fn() -> bool>,

    /// 残り時間を表示させます
    /// - now: 処理しているフレーム番号
    /// - total: 処理する総フレーム数
    pub func_rest_time_disp: Option<unsafe extern "C" fn(now: c_int, total: c_int)>,

    /// データ取得のバッファ数(フレーム数)を設定します ※標準は4になります
    /// - video_size: 画像データのバッファ数
    /// - audio_size: 音声データのバッファ数
    pub func_set_buffer_size: Option<unsafe extern "C" fn(video_size: c_int, audio_size: c_int)>,
}

impl OUTPUT_INFO {
    /// フラグ定数: 画像データあり
    pub const FLAG_VIDEO: c_int = 1;
    /// フラグ定数: 音声データあり
    pub const FLAG_AUDIO: c_int = 2;
}

/// 出力プラグイン構造体
#[repr(C)]
pub struct OUTPUT_PLUGIN_TABLE {
    /// フラグ
    pub flag: c_int,
    /// プラグインの名前
    pub name: LPCWSTR,
    /// ファイルのフィルタ
    pub filefilter: LPCWSTR,
    /// プラグインの情報
    pub information: LPCWSTR,

    /// 出力時に呼ばれる関数へのポインタ
    /// - 戻り値: 成功時はtrueを返却
    pub func_output: Option<unsafe extern "C" fn(oip: *mut OUTPUT_INFO) -> bool>,

    /// 出力設定のダイアログを要求された時に呼ばれる関数へのポインタ (nullptrなら呼ばれません)
    /// - 戻り値: 成功時はtrueを返却
    pub func_config: Option<unsafe extern "C" fn(hwnd: HWND, dll_hinst: HINSTANCE) -> bool>,

    /// 出力設定のテキスト情報を取得する時に呼ばれる関数へのポインタ (nullptrなら呼ばれません)
    /// - 戻り値: 出力設定のテキスト情報へのポインタ (次に関数が呼ばれるまで内容を有効にしておく)
    pub func_get_config_text: Option<unsafe extern "C" fn() -> LPCWSTR>,

    /// プロジェクトファイル側から出力設定の読み込み要求時に呼ばれる関数へのポインタ
    /// (FLAG_PROJECT_CONFIGが有効の時のみ呼ばれます)
    /// - 戻り値: 成功時はtrueを返却
    pub func_load_project_config: Option<unsafe extern "C" fn(project: *mut PROJECT_FILE) -> bool>,

    /// プロジェクトファイル側への出力設定の書き込み要求時に呼ばれる関数へのポインタ
    /// (FLAG_PROJECT_CONFIGが有効の時のみ呼ばれます)
    /// - 戻り値: 成功時はtrueを返却
    pub func_save_project_config: Option<unsafe extern "C" fn(project: *mut PROJECT_FILE) -> bool>,
}

impl OUTPUT_PLUGIN_TABLE {
    /// フラグ定数: 画像をサポートする
    pub const FLAG_VIDEO: c_int = 1;
    /// フラグ定数: 音声をサポートする
    pub const FLAG_AUDIO: c_int = 2;
    /// フラグ定数: 静止画出力のみサポートする (OUTPUT_INFOが1フレーム出力になります)
    /// ※静止画出力では出力完了時の通知やサウンド再生をしません
    pub const FLAG_IMAGE: c_int = 4;
    /// フラグ定数: プロジェクトファイルの設定保持をサポートする
    /// ※プロジェクトファイル側に出力設定を保持する場合に指定します
    pub const FLAG_PROJECT_CONFIG: c_int = 8;
}

/// 画像フォーマット定数: RGB24bit
pub const BI_RGB: DWORD = 0;
/// 画像フォーマット定数: YUY2
pub const YUY2: DWORD = u32::from_le_bytes(*b"YUY2");
/// 画像フォーマット定数: PA64
/// DXGI_FORMAT_R16G16B16A16_UNORM(乗算済みα)
pub const PA64: DWORD = u32::from_le_bytes(*b"PA64");
/// 画像フォーマット定数: HF64
/// DXGI_FORMAT_R16G16B16A16_FLOAT(乗算済みα)
pub const HF64: DWORD = u32::from_le_bytes(*b"HF64");
/// 画像フォーマット定数: YC48
/// 互換対応のフォーマット
pub const YC48: DWORD = u32::from_le_bytes(*b"YC48");

/// 音声フォーマット定数: PCM 16bit
pub const WAVE_FORMAT_PCM: DWORD = 1;
/// 音声フォーマット定数: PCM (float) 32bit
pub const WAVE_FORMAT_IEEE_FLOAT: DWORD = 3;
