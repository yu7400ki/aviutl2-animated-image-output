//! AviUtl ExEdit2 出力プラグインを安全に実装するための高レベルAPI
//!
//! 生FFIバインディングは [`sys`] (`aviutl2-sys` クレート) が提供する。
//! プラグインは [`OutputPlugin`] を実装し [`register_output_plugin!`] で登録する。

pub use aviutl2_sys as sys;
pub use ini;

pub mod config;
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
