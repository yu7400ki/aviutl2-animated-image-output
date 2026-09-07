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
