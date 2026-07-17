//! `register_output_plugin!` が生成するプラグインテーブルの内容を検証する

use aviutl2::__private::TableStorage;
use aviutl2::{FileFilter, OutputInfo, OutputPlugin, PluginFlags, PluginInfo};
use widestring::U16CStr;

struct TestPlugin;

impl OutputPlugin for TestPlugin {
    type Error = String;

    const HAS_CONFIG_DIALOG: bool = true;

    fn info() -> PluginInfo {
        PluginInfo {
            flags: PluginFlags::VIDEO,
            name: "テスト出力プラグイン".into(),
            file_filter: FileFilter::new()
                .add("GIF Files (*.gif)", "*.gif")
                .add("All Files (*)", "*"),
            information: "テスト出力プラグイン v0.1.0".into(),
        }
    }

    fn output(_info: &OutputInfo) -> Result<(), String> {
        Ok(())
    }
}

/// filefilterのポインタから二重NUL終端までを読み出す
unsafe fn read_double_nul(mut ptr: *const u16) -> Vec<u16> {
    let mut buffer = Vec::new();
    unsafe {
        loop {
            let c = *ptr;
            if c == 0 && *ptr.add(1) == 0 {
                buffer.push(0);
                buffer.push(0);
                break;
            }
            buffer.push(c);
            ptr = ptr.add(1);
        }
    }
    buffer
}

#[test]
fn table_is_built_correctly() {
    static TABLE: TableStorage = TableStorage::new();

    let ptr = TABLE.get_or_init::<TestPlugin>();
    assert!(!ptr.is_null());

    // 2回目の呼び出しでも同じポインタを返す
    assert_eq!(ptr, TABLE.get_or_init::<TestPlugin>());

    let table = unsafe { &*ptr };

    assert_eq!(table.flag, aviutl2::sys::OUTPUT_PLUGIN_TABLE::FLAG_VIDEO);

    let name = unsafe { U16CStr::from_ptr_str(table.name) };
    assert_eq!(name.to_string().unwrap(), "テスト出力プラグイン");

    let information = unsafe { U16CStr::from_ptr_str(table.information) };
    assert_eq!(
        information.to_string().unwrap(),
        "テスト出力プラグイン v0.1.0"
    );

    let filefilter = unsafe { read_double_nul(table.filefilter) };
    let expected: Vec<u16> = "GIF Files (*.gif)\0*.gif\0All Files (*)\0*\0\0"
        .encode_utf16()
        .collect();
    assert_eq!(filefilter, expected);

    assert!(table.func_output.is_some());
    assert!(table.func_config.is_some());
    assert!(table.func_get_config_text.is_none());
    assert!(table.func_load_project_config.is_none());
    assert!(table.func_save_project_config.is_none());

    // nullのOUTPUT_INFOにはfalseを返す (クラッシュしない)
    let result = unsafe { table.func_output.unwrap()(std::ptr::null_mut()) };
    assert!(!result);
}
