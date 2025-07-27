use ini::Ini;
use std::path::{Path, PathBuf};
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
    pub repeat: i32,
    pub color_format: ColorFormat,
    pub lossless: bool,
    pub quality: f32,
    pub method: u8,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            repeat: 0,
            color_format: ColorFormat::default(),
            lossless: false,
            quality: 75.0,
            method: 4,
        }
    }
}

impl Config {
    pub const fn default() -> Self {
        Self {
            repeat: 0,
            color_format: ColorFormat::Rgb24,
            lossless: false,
            quality: 75.0,
            method: 4,
        }
    }

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
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(default.repeat);

        let color_format = section
            .and_then(|s| s.get("color_format"))
            .and_then(|s| s.parse::<ColorFormat>().ok())
            .unwrap_or_default();

        let lossless = section
            .and_then(|s| s.get("lossless"))
            .and_then(|s| s.parse::<bool>().ok())
            .unwrap_or(default.lossless);

        let quality = section
            .and_then(|s| s.get("quality"))
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(default.quality)
            .clamp(0.0, 100.0);

        let method = section
            .and_then(|s| s.get("method"))
            .and_then(|s| s.parse::<u8>().ok())
            .unwrap_or(default.method)
            .clamp(0, 6);

        Self {
            repeat,
            color_format,
            lossless,
            quality,
            method,
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let config_path = Self::config_file_path()?;
        let mut ini = Ini::new();

        ini.with_section(Some("Config"))
            .set("repeat", self.repeat.to_string())
            .set("color_format", self.color_format.to_index().to_string())
            .set("lossless", self.lossless.to_string())
            .set("quality", self.quality.to_string())
            .set("method", self.method.to_string());

        ini.write_to_file(&config_path).map_err(|e| e.to_string())
    }
}
