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
    pub quality: u8,
    pub speed: u8,
    pub color_format: ColorFormat,
    pub threads: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            quality: 75,
            speed: 10,
            color_format: ColorFormat::Rgb24,
            threads: std::thread::available_parallelism().map_or(1, |p| p.get()),
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

        let quality = section
            .and_then(|s| s.get("quality"))
            .and_then(|s| s.parse::<u8>().ok())
            .unwrap_or(default.quality)
            .clamp(0, 100);

        let speed = section
            .and_then(|s| s.get("speed"))
            .and_then(|s| s.parse::<u8>().ok())
            .unwrap_or(default.speed)
            .clamp(0, 10);

        let color_format = section
            .and_then(|s| s.get("color_format"))
            .and_then(|s| s.parse::<ColorFormat>().ok())
            .unwrap_or_default();

        let threads = section
            .and_then(|s| s.get("threads"))
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(default.threads);

        Self {
            quality,
            speed,
            color_format,
            threads,
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let config_path = Self::config_file_path()?;
        let mut ini = Ini::new();

        ini.with_section(Some("Config"))
            .set("quality", self.quality.to_string())
            .set("speed", self.speed.to_string())
            .set("color_format", self.color_format.to_index().to_string())
            .set("threads", self.threads.to_string());

        ini.write_to_file(&config_path).map_err(|e| e.to_string())
    }
}
