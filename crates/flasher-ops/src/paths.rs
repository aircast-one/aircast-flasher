use std::path::PathBuf;

pub const IDENTIFIER: &str = "one.aircast.flasher";

pub fn cache_dir() -> Result<PathBuf, String> {
    dirs::cache_dir()
        .map(|dir| dir.join(IDENTIFIER))
        .ok_or_else(|| "This system has no cache directory".to_string())
}

pub fn image_cache_dir() -> Result<PathBuf, String> {
    cache_dir().map(|dir| dir.join("images"))
}

pub fn config_dir() -> Result<PathBuf, String> {
    dirs::config_dir()
        .map(|dir| dir.join(IDENTIFIER))
        .ok_or_else(|| "This system has no config directory".to_string())
}
