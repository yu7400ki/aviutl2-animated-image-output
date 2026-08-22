//! AviUtl ExEdit2 出力プラグインを安全に実装するための高レベルAPI
//!
//! 生FFIバインディングは [`sys`] (`aviutl2-sys` クレート) が提供する。
//! プラグインは [`OutputPlugin`] を実装し [`register_output_plugin!`] で登録する。
//!
//! ログ出力を使う場合は [`logger`] モジュールの関数と [`register_logger!`] を
//! 併用する ([`register_output_plugin!`] とは別呼び出しなので呼び忘れに注意)。

pub use aviutl2_sys as sys;
pub use ini;

pub mod config;
pub mod convert;
pub mod logger;
mod metrics;
pub mod module;
pub mod output;
pub mod pixel;

#[doc(hidden)]
#[path = "private.rs"]
pub mod __private;
mod macros;

pub use config::IniConfig;
pub use output::{FileFilter, OutputInfo, OutputPlugin, PluginFlags, PluginInfo};
pub use pixel::ColorFormat;
