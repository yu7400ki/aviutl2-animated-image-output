/*
---------------------------------
AviUtl ExEdit2 Plugin SDK License
---------------------------------

The MIT License

Copyright (c) 2025 Kenkun

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
 */

//! AviUtl ExEdit2 Plugin SDK の生FFIバインディング
//!
//! `vendor/aviutl2_sdk/include/aviutl2_sdk/output2.h` に忠実な `#[repr(C)]` 定義のみを提供する。
//! 安全なラッパーは `aviutl2` クレートを使用すること。
//!
//! # 出力プラグインの外部公開関数 (output2.h より)
//!
//! 出力プラグインは下記の関数を外部公開すると呼び出される:
//!
//! - `OUTPUT_PLUGIN_TABLE* GetOutputPluginTable(void)` — 出力プラグイン構造体のポインタを渡す関数 (必須)
//! - `DWORD RequiredVersion()` — 必要とする本体バージョン番号取得関数 (任意)
//! - `bool InitializePlugin(DWORD version)` — プラグインDLL初期化関数 (任意)
//! - `void UninitializePlugin()` — プラグインDLL終了関数 (任意)
//! - `void InitializeLogger(LOG_HANDLE* logger)` — ログ出力機能初期化関数 (任意) ※logger2.h
//! - `void InitializeConfig(CONFIG_HANDLE* config)` — 設定関連機能初期化関数 (任意) ※config2.h
//! - `void InitializeCache(CACHE_HANDLE* cache)` — キャッシュ関連機能初期化関数 ※cache2.h

#![allow(non_camel_case_types)]

pub type DWORD = u32;
pub type LPCWSTR = *const u16;

pub mod logger2;
pub mod output2;
pub use logger2::*;
pub use output2::*;

#[cfg(test)]
mod layout {
    use super::*;
    use std::mem::{offset_of, size_of};
    use std::os::raw::c_int;

    unsafe extern "C" {
        fn aviutl2_sys_sizeof_output_info() -> usize;
        fn aviutl2_sys_sizeof_output_plugin_table() -> usize;
        fn aviutl2_sys_sizeof_log_handle() -> usize;
        fn aviutl2_sys_output_info_offsets(out: *mut usize);
        fn aviutl2_sys_output_plugin_table_offsets(out: *mut usize);
        fn aviutl2_sys_log_handle_offsets(out: *mut usize);
        fn aviutl2_sys_output_info_flags(out: *mut c_int);
        fn aviutl2_sys_output_plugin_table_flags(out: *mut c_int);
    }

    /// 並べた順に項目の位置を取る
    macro_rules! offsets {
        ($ty:ty, $($field:ident),+ $(,)?) => {
            [$(offset_of!($ty, $field)),+]
        };
    }

    /// C 側の出口が同じ順で並べた値
    fn from_header<T: Copy + Default, const N: usize>(
        fill: unsafe extern "C" fn(*mut T),
    ) -> [T; N] {
        let mut values = [T::default(); N];
        unsafe { fill(values.as_mut_ptr()) };
        values
    }

    /// 写した構造体は同梱ヘッダと同じ大きさになる
    ///
    /// 位置の突き合わせは末尾の詰め物を見ないため、大きさを別に検める。
    #[test]
    fn struct_sizes_match_the_vendored_header() {
        assert_eq!(size_of::<OUTPUT_INFO>(), unsafe {
            aviutl2_sys_sizeof_output_info()
        });
        assert_eq!(size_of::<OUTPUT_PLUGIN_TABLE>(), unsafe {
            aviutl2_sys_sizeof_output_plugin_table()
        });
        assert_eq!(size_of::<LOG_HANDLE>(), unsafe {
            aviutl2_sys_sizeof_log_handle()
        });
    }

    #[test]
    fn output_info_field_offsets_match_the_vendored_header() {
        assert_eq!(
            from_header(aviutl2_sys_output_info_offsets),
            offsets!(
                OUTPUT_INFO,
                flag,
                w,
                h,
                rate,
                scale,
                n,
                audio_rate,
                audio_ch,
                audio_n,
                savefile,
                func_get_video,
                func_get_audio,
                func_is_abort,
                func_rest_time_disp,
                func_set_buffer_size,
            )
        );
    }

    #[test]
    fn output_plugin_table_field_offsets_match_the_vendored_header() {
        assert_eq!(
            from_header(aviutl2_sys_output_plugin_table_offsets),
            offsets!(
                OUTPUT_PLUGIN_TABLE,
                flag,
                name,
                filefilter,
                information,
                func_output,
                func_config,
                func_get_config_text,
                func_load_project_config,
                func_save_project_config,
            )
        );
    }

    #[test]
    fn log_handle_field_offsets_match_the_vendored_header() {
        assert_eq!(
            from_header(aviutl2_sys_log_handle_offsets),
            offsets!(LOG_HANDLE, log, info, warn, error, verbose)
        );
    }

    #[test]
    fn flag_constants_match_the_vendored_header() {
        assert_eq!(
            from_header(aviutl2_sys_output_info_flags),
            [OUTPUT_INFO::FLAG_VIDEO, OUTPUT_INFO::FLAG_AUDIO]
        );
        assert_eq!(
            from_header(aviutl2_sys_output_plugin_table_flags),
            [
                OUTPUT_PLUGIN_TABLE::FLAG_VIDEO,
                OUTPUT_PLUGIN_TABLE::FLAG_AUDIO,
                OUTPUT_PLUGIN_TABLE::FLAG_IMAGE,
                OUTPUT_PLUGIN_TABLE::FLAG_PROJECT_CONFIG,
            ]
        );
    }
}
