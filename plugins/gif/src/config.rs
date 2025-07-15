use ini::Ini;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use windows::Win32::Foundation::{HMODULE, MAX_PATH};
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GetModuleFileNameW, GetModuleHandleExW,
};
use windows::core::PCWSTR;

#[derive(Clone, Debug)]
pub struct KeyColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Default for KeyColor {
    fn default() -> Self {
        KeyColor { r: 0, g: 0, b: 255 } // デフォルト: 青色
    }
}

impl KeyColor {
    pub fn parse(color_str: &str) -> Result<Self, String> {
        let color_str = color_str.trim_start_matches('#');
        if color_str.len() != 6 {
            return Err("無効なカラーコードです。6桁の16進数で指定してください。".to_string());
        }
        let r = u8::from_str_radix(&color_str[0..2], 16)
            .map_err(|_| "無効なカラーコードです。".to_string())?;
        let g = u8::from_str_radix(&color_str[2..4], 16)
            .map_err(|_| "無効なカラーコードです。".to_string())?;
        let b = u8::from_str_radix(&color_str[4..6], 16)
            .map_err(|_| "無効なカラーコードです。".to_string())?;
        Ok(KeyColor { r, g, b })
    }

    pub fn to_array(&self) -> [u8; 3] {
        [self.r, self.g, self.b]
    }
}

impl ToString for KeyColor {
    fn to_string(&self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

#[derive(Copy, Clone, PartialEq)]
pub enum ColorFormat {
    Rgb24,  // RGB 24bit
    Rgba32, // RGBA 32bit
}

impl Default for ColorFormat {
    fn default() -> Self {
        ColorFormat::Rgb24
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

#[derive(Clone)]
pub struct Config {
    pub repeat: u16,
    pub color_format: ColorFormat,
    pub speed: i32,
    pub chroma_key_enabled: bool,
    pub chroma_key_color: KeyColor,
    pub chroma_key_hue_range: u16,
    pub chroma_key_saturation_range: u8,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            repeat: 0,
            color_format: ColorFormat::default(),
            speed: 10,
            chroma_key_enabled: false,
            chroma_key_color: KeyColor::default(),
            chroma_key_hue_range: 20,        // 20度
            chroma_key_saturation_range: 35, // 35%
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

    pub fn load() -> Self {
        let default = Self::default();

        let config_path = match Self::config_file_path() {
            Ok(path) => path,
            Err(_) => return default,
        };

        if !Path::new(&config_path).exists() {
            return default;
        }

        let ini = match Ini::load_from_file(&config_path) {
            Ok(ini) => ini,
            Err(_) => return default,
        };

        let section = ini.section(Some("Config"));

        let repeat = section
            .and_then(|s| s.get("repeat"))
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(default.repeat);

        let color_format = section
            .and_then(|s| s.get("color_format"))
            .and_then(|s| s.parse::<ColorFormat>().ok())
            .unwrap_or_default();

        let speed = section
            .and_then(|s| s.get("speed"))
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(default.speed);

        let chroma_key_enabled = section
            .and_then(|s| s.get("chroma_key_enabled"))
            .and_then(|s| s.parse::<bool>().ok())
            .unwrap_or(default.chroma_key_enabled);

        let chroma_key_color = section
            .and_then(|s| s.get("chroma_key_color"))
            .and_then(|s| KeyColor::parse(s).ok())
            .unwrap_or(default.chroma_key_color);

        let chroma_key_hue_range = section
            .and_then(|s| s.get("chroma_key_hue_range"))
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(default.chroma_key_hue_range);

        let chroma_key_saturation_range = section
            .and_then(|s| s.get("chroma_key_saturation_range"))
            .and_then(|s| s.parse::<u8>().ok())
            .unwrap_or(default.chroma_key_saturation_range);

        Self {
            repeat,
            color_format,
            speed,
            chroma_key_enabled,
            chroma_key_color,
            chroma_key_hue_range,
            chroma_key_saturation_range,
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let config_path = Self::config_file_path()?;
        let mut ini = Ini::new();

        ini.with_section(Some("Config"))
            .set("repeat", self.repeat.to_string())
            .set("color_format", self.color_format.to_index().to_string())
            .set("speed", self.speed.to_string())
            .set("chroma_key_enabled", self.chroma_key_enabled.to_string())
            .set("chroma_key_color", self.chroma_key_color.to_string())
            .set(
                "chroma_key_hue_range",
                self.chroma_key_hue_range.to_string(),
            )
            .set(
                "chroma_key_saturation_range",
                self.chroma_key_saturation_range.to_string(),
            );

        ini.write_to_file(&config_path).map_err(|e| e.to_string())
    }
}
