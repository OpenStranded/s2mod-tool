// openstranded-s2mod-tool — convert Stranded II mods to .s2mod format
// Copyright (C) 2025  openstranded-s2mod-tool contributors
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

//! Registry key derivation and registration for `.ron` files in the manifest.

use std::collections::HashMap;
use std::path::Path;

/// Derive the registry key from a .ron filename.
///
///   items_edible.ron   → "items"
///   combinations_basic.ron → "combinations"
///   game.ron           → "game"
///   vehicles_flying.ron → "vehicles"
///   strings.ron        → "strings"
pub fn registry_key_from_filename(stem: &str) -> String {
    // Use the first token before '_' as the key; if there's no '_', use the whole stem.
    stem.split('_').next().unwrap_or(stem).to_string()
}

/// Register a .ron file path in the registry under the key derived from its filename.
pub fn register_ron(ron_rel: &Path, registry: &mut HashMap<String, Vec<String>>) {
    let rel_str = ron_rel.to_string_lossy().to_string();
    let stem = ron_rel
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    let key = registry_key_from_filename(stem);
    registry.entry(key).or_default().push(rel_str);
}
