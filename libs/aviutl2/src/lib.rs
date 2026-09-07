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
pub mod dialog;
pub mod logger;
mod metrics;
pub mod module;
pub mod output;
pub mod pipeline;
pub mod pixel;

#[doc(hidden)]
#[path = "private.rs"]
pub mod __private;
mod macros;

pub use anim_core::FrameDelay;
pub use config::{
    IniConfig, MAX_REPEAT, default_threads, max_threads, read, read_clamped, read_flag,
    read_threads,
};
pub use output::{
    Audio, ConfigDialog, FileFilter, OutputInfo, OutputPlugin, PluginFlags, PluginInfo, Video,
    write_or_discard,
};
pub use pipeline::PipelineError;
pub use pixel::ColorFormat;
