/// [`crate::OutputPlugin`] を実装した型をAviUtl ExEdit2の出力プラグインとして登録する
///
/// `DllMain` と `GetOutputPluginTable` のDLLエクスポートを生成する。
/// **1つのcdylibにつき1回だけ**呼び出すこと。
///
/// ```ignore
/// struct MyPlugin;
/// impl OutputPlugin for MyPlugin { ... }
/// register_output_plugin!(MyPlugin);
/// ```
///
/// SDKが定める任意のエクスポート (`RequiredVersion` / `InitializePlugin` /
/// `UninitializePlugin` 等) が必要な場合は、このマクロ呼び出しの隣に
/// `#[unsafe(no_mangle)]` 付きで手書きすればよい (マクロはそれらを生成しない)。
#[macro_export]
macro_rules! register_output_plugin {
    ($plugin:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "system" fn DllMain(
            _hinst: *mut $crate::__private::c_void,
            _reason: u32,
            _reserved: *mut $crate::__private::c_void,
        ) -> $crate::__private::BOOL {
            $crate::__private::TRUE
        }

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn GetOutputPluginTable() -> *mut $crate::sys::OUTPUT_PLUGIN_TABLE {
            static TABLE: $crate::__private::TableStorage = $crate::__private::TableStorage::new();
            TABLE.get_or_init::<$plugin>()
        }
    };
}
