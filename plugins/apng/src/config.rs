use ini::Ini;
use std::path::PathBuf;
use std::str::FromStr;
use windows::Win32::Foundation::{HMODULE, MAX_PATH};
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GetModuleFileNameW, GetModuleHandleExW,
};
use windows::core::PCWSTR;

#[derive(Copy, Clone, PartialEq)]
pub enum ColorFormat {
    Rgb24,
    Rgba32,
}

impl Default for ColorFormat {
    fn default() -> Self {
        ColorFormat::Rgb24
    }
}

impl Into<png::ColorType> for ColorFormat {
    fn into(self) -> png::ColorType {
        match self {
            ColorFormat::Rgb24 => png::ColorType::Rgb,
            ColorFormat::Rgba32 => png::ColorType::Rgba,
        }
    }
}

impl Into<&'static str> for ColorFormat {
    fn into(self) -> &'static str {
        match self {
            ColorFormat::Rgb24 => "RGB 24bit",
            ColorFormat::Rgba32 => "RGBA 32bit",
        }
    }
}

impl FromStr for ColorFormat {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.parse::<u32>() {
            Ok(0) => Ok(ColorFormat::Rgb24),
            Ok(1) => Ok(ColorFormat::Rgba32),
            _ => Err(()),
        }
    }
}

impl ColorFormat {
    fn to_index(&self) -> u32 {
        match self {
            ColorFormat::Rgb24 => 0,
            ColorFormat::Rgba32 => 1,
        }
    }
}

#[derive(Copy, Clone, PartialEq)]
pub enum CompressionType {
    Default,
    Fast,
    Best,
}

impl Default for CompressionType {
    fn default() -> Self {
        CompressionType::Default
    }
}

impl Into<png::Compression> for CompressionType {
    fn into(self) -> png::Compression {
        match self {
            CompressionType::Default => png::Compression::Default,
            CompressionType::Fast => png::Compression::Fast,
            CompressionType::Best => png::Compression::Best,
        }
    }
}

impl Into<&'static str> for CompressionType {
    fn into(self) -> &'static str {
        match self {
            CompressionType::Default => "標準",
            CompressionType::Fast => "高速",
            CompressionType::Best => "最高",
        }
    }
}

impl FromStr for CompressionType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.parse::<u32>() {
            Ok(0) => Ok(CompressionType::Default),
            Ok(1) => Ok(CompressionType::Fast),
            Ok(2) => Ok(CompressionType::Best),
            _ => Err(()),
        }
    }
}

impl CompressionType {
    fn to_index(&self) -> u32 {
        match self {
            CompressionType::Default => 0,
            CompressionType::Fast => 1,
            CompressionType::Best => 2,
        }
    }
}

#[derive(Copy, Clone, PartialEq)]
pub enum FilterType {
    None,
    Sub,
    Up,
    Average,
    Paeth,
}

impl Default for FilterType {
    fn default() -> Self {
        FilterType::Sub
    }
}

impl Into<png::FilterType> for FilterType {
    fn into(self) -> png::FilterType {
        match self {
            FilterType::None => png::FilterType::NoFilter,
            FilterType::Sub => png::FilterType::Sub,
            FilterType::Up => png::FilterType::Up,
            FilterType::Average => png::FilterType::Avg,
            FilterType::Paeth => png::FilterType::Paeth,
        }
    }
}

impl Into<&'static str> for FilterType {
    fn into(self) -> &'static str {
        match self {
            FilterType::None => "なし",
            FilterType::Sub => "Sub",
            FilterType::Up => "Up",
            FilterType::Average => "Average",
            FilterType::Paeth => "Paeth",
        }
    }
}

impl FromStr for FilterType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.parse::<u32>() {
            Ok(0) => Ok(FilterType::None),
            Ok(1) => Ok(FilterType::Sub),
            Ok(2) => Ok(FilterType::Up),
            Ok(3) => Ok(FilterType::Average),
            Ok(4) => Ok(FilterType::Paeth),
            _ => Err(()),
        }
    }
}

impl FilterType {
    fn to_index(&self) -> u32 {
        match self {
            FilterType::None => 0,
            FilterType::Sub => 1,
            FilterType::Up => 2,
            FilterType::Average => 3,
            FilterType::Paeth => 4,
        }
    }
}

#[derive(Clone)]
pub struct Config {
    pub repeat: u32,
    pub color_format: ColorFormat,
    pub compression_type: CompressionType,
    pub filter_type: FilterType,
    pub adaptive_filter: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            repeat: 0,
            color_format: ColorFormat::default(),
            compression_type: CompressionType::default(),
            filter_type: FilterType::default(),
            adaptive_filter: true,
        }
    }
}

impl Config {
    fn config_file_path() -> Result<PathBuf, String> {
        let (buffer, len) = unsafe {
            let mut hmodule: HMODULE = HMODULE::default();
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
                PCWSTR(Self::config_file_path as *const () as *const u16),
                &mut hmodule as *mut HMODULE,
            )
            .map_err(|e| format!("GetModuleHandleExW failed: {}", e))?;

            let mut buffer = [0u16; MAX_PATH as usize];
            let len = GetModuleFileNameW(Some(hmodule), &mut buffer);

            (buffer, len)
        };

        if len > 0 {
            let dll_path = String::from_utf16_lossy(&buffer[..len as usize]);
            let dll_path = PathBuf::from(&dll_path);
            let dll_dir = dll_path
                .parent()
                .ok_or("プラグインのディレクトリが取得できません")?;
            Ok(dll_dir.join(concat!(env!("CARGO_PKG_NAME"), ".ini")))
        } else {
            Err("GetModuleFileNameW failed".to_string())
        }
    }

    pub const fn default() -> Self {
        Config {
            repeat: 0,
            color_format: ColorFormat::Rgb24,
            compression_type: CompressionType::Default,
            filter_type: FilterType::Sub,
            adaptive_filter: false,
        }
    }

    pub fn load() -> Self {
        let default = Self::default();

        let config_path = match Self::config_file_path() {
            Ok(path) => path,
            Err(_) => return default,
        };

        if let Ok(ini) = Ini::load_from_file(&config_path) {
            if let Some(section) = ini.section(Some("Config")) {
                let repeat = section
                    .get("repeat")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(default.repeat);

                let color_format = section
                    .get("color_format")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(default.color_format);

                let compression_type = section
                    .get("compression_type")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(default.compression_type);

                let filter_type = section
                    .get("filter_type")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(default.filter_type);

                let adaptive_filter = section
                    .get("adaptive_filter")
                    .and_then(|s| s.parse::<u32>().ok())
                    .map(|v| v != 0)
                    .unwrap_or(default.adaptive_filter);

                Config {
                    repeat,
                    color_format,
                    compression_type,
                    filter_type,
                    adaptive_filter,
                }
            } else {
                default
            }
        } else {
            default
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let mut ini = Ini::new();

        ini.with_section(Some("Config"))
            .set("repeat", self.repeat.to_string())
            .set("color_format", self.color_format.to_index().to_string())
            .set(
                "compression_type",
                self.compression_type.to_index().to_string(),
            )
            .set("filter_type", self.filter_type.to_index().to_string())
            .set("adaptive_filter", (self.adaptive_filter as u32).to_string());

        let config_path = Self::config_file_path()?;
        ini.write_to_file(&config_path).map_err(|e| e.to_string())
    }
}
