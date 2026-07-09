// openstranded-convert — convert Stranded II mods to .s2mod format
// Copyright (C) 2025  openstranded-convert contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! General utilities for file I/O, path handling, and data serialisation.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

/// Compute the relative path of a file within the input tree.
pub fn relative_path(file: &Path, input_root: &Path) -> PathBuf {
    file.strip_prefix(input_root).unwrap_or(file).to_path_buf()
}

/// Read a file to String, trying UTF-8 first, falling back to
/// Latin-1 (ISO-8859-1) which maps bytes 128-255 directly to
/// Unicode codepoints U+0080–U+00FF.
pub fn read_file_lossy(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("reading {:?}", path))?;
    if let Ok(s) = String::from_utf8(bytes.clone()) {
        return Ok(s);
    }
    Ok(bytes.iter().map(|&b| b as char).collect())
}

/// Parse an .inf file, handling non-UTF-8 encodings gracefully.
pub fn parse_inf_file(path: &Path) -> Result<Vec<inf2ron::InfEntry>> {
    let content = read_file_lossy(path)?;
    inf2ron::InfParser::parse_str(&content).map_err(|e| anyhow::anyhow!("{}", e))
}

/// Serialize data to RON and write to path.
pub fn write_ron<T: Serialize + ?Sized>(path: &Path, data: &T) -> Result<()> {
    let ron_str = ron::to_string(data)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, &ron_str)?;
    Ok(())
}

/// Copy a file from the input tree into the staging directory,
/// preserving its relative path.
pub fn copy_to_staging(file: &Path, input_root: &Path, stage: &Path) -> Result<()> {
    let relative = relative_path(file, input_root);
    let dest = stage.join(relative);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(file, &dest)?;
    Ok(())
}

/// Convert a directory/file name into a safe alphanumeric ID for use as a pack identifier.
/// E.g. "Stranded II" → "stranded_ii",  "MyMod-v1!" → "mymod_v1".
pub fn normalize_id(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_was_space = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_was_space = false;
        } else if ch == '_' || ch == '-' {
            out.push(ch);
            prev_was_space = false;
        } else if ch.is_ascii_whitespace() {
            if !prev_was_space {
                out.push('_');
                prev_was_space = true;
            }
        } else {
            if !prev_was_space && !out.is_empty() {
                out.push('_');
                prev_was_space = true;
            }
        }
    }
    let trimmed = out.trim_end_matches('_').to_string();
    if trimmed.is_empty() { "content".to_string() } else { trimmed }
}
