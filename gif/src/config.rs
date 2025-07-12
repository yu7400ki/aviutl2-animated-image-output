#[derive(Debug, Clone)]
pub struct Config {
    pub repeat: u16,
}

impl Config {
    pub const fn default() -> Self {
        Config { repeat: 0 }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::default()
    }
}
