use anyhow::Result;
use std::path::PathBuf;

pub fn storage_path() -> Result<PathBuf> {
    let home = dirs_next().ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;
    Ok(home.join(".wick").join("data"))
}

pub fn clear() -> Result<()> {
    let path = storage_path()?;
    if path.exists() {
        std::fs::remove_dir_all(&path)?;
    }
    std::fs::create_dir_all(&path)?;
    Ok(())
}

fn dirs_next() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
