//! Fonts remain an optional, licensed craft-fonts build input, never repository assets.
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo::rerun-if-env-changed=CRAFT_FONTS_DIR");
    let mut entries = String::from("const BUNDLED_FONTS: &[(&[u8], u32)] = &[\n");
    if let Some(root) = std::env::var_os("CRAFT_FONTS_DIR")
        && let Err(error) = fonts(Path::new(&root), &mut entries)
    {
        eprintln!("craft-fonts: {error}");
        std::process::exit(1);
    }
    entries.push_str("];\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("unicode_fonts.rs");
    if let Err(error) = std::fs::write(out, entries) {
        eprintln!("PDF font build input: {error}");
        std::process::exit(1);
    }
}

fn fonts(root: &Path, entries: &mut String) -> Result<(), String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let manifest = root.join("fonts/manifest.txt");
    println!("cargo::rerun-if-changed={}", manifest.display());
    let manifest = std::fs::read_to_string(manifest).map_err(|e| e.to_string())?;
    let mut kept = [false; 2];
    for line in manifest.lines().filter(|line| !line.trim_start().starts_with('#')) {
        let fields: Vec<_> = line.split('|').map(str::trim).collect();
        let [_, style, file, scripts, licence, licence_file, hash, _] = fields.as_slice() else { continue };
        let slot = if scripts.split(',').any(|s| s == "Hans") {
            0
        } else if scripts.split(',').any(|s| s == "Hant") {
            1
        } else {
            continue;
        };
        if kept[slot] || *style != "Regular" || *licence != "OFL-1.1" {
            continue;
        }
        let path = root.join(file).canonicalize().map_err(|e| e.to_string())?;
        let license = root.join(licence_file).canonicalize().map_err(|e| e.to_string())?;
        if !path.starts_with(&root) || !license.starts_with(&root) || !license.is_file() {
            return Err("font and licence must be inside craft-fonts".into());
        }
        println!("cargo::rerun-if-changed={}", path.display());
        println!("cargo::rerun-if-changed={}", license.display());
        if std::fs::metadata(&path).map_err(|e| e.to_string())?.len() > 64 * 1024 * 1024 {
            return Err(format!("font is too large: {file}"));
        }
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        let digest: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        if bytes.len() > 64 * 1024 * 1024 || digest != *hash {
            return Err(format!("font size/checksum mismatch: {file}"));
        }
        entries.push_str(&format!("(include_bytes!({:?}), 0),\n", path.to_string_lossy()));
        kept[slot] = true;
    }
    if !kept[0] {
        return Err("manifest has no OFL-licensed Regular Hans face".into());
    }
    Ok(())
}
