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

//! S2S reference scanning and classification.
//!
//! Scans inline scripts and standalone `.s2s` files for commands that reference
//! other files (`dialogue`, `msgbox`, `button`, `addscript`, `extendscript`,
//! `def_extend`, `def_override`), then classifies each referenced file for
//! downstream conversion.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::util::read_file_lossy;

/// How another file references/loads a given .s2s file.
#[derive(Debug, Clone, PartialEq)]
pub enum S2sRefType {
    /// `dialogue "startpage", "path"` — dialog page data
    Dialogue { start_page: String },
    /// `msgbox "Title", "path"` — plain text displayed in a message box
    Msgbox { title: String },
    /// `button id, "text", font, "path"` — script executed on button click
    Button,
    /// `addscript class, id, "path"` — event handler added to an entity
    AddScript,
    /// `extendscript class, id, "path"` — global event handler extension
    ExtendScript,
    /// `def_extend class, type_id, "path", "section"` — type extension
    DefExtend,
    /// `def_override class, type_id, "path", "section"` — type override
    DefOverride,
}

/// A single reference from one file to another.
#[derive(Debug, Clone)]
pub struct S2sReference {
    pub target: PathBuf,
    pub ref_type: S2sRefType,
    pub _section: Option<String>,
}

/// Map from .s2s file path → how it's referenced (file-level).
pub type S2sRefMap = HashMap<PathBuf, Vec<S2sRefType>>;

/// Section-level reference map: file path → section name → ref types for that section.
/// Only populated for files whose references include a section name.
pub type S2sSectionMap = HashMap<PathBuf, HashMap<String, Vec<S2sRefType>>>;

/// Classify a .s2s file based on how it's referenced.
///
/// Returns what kind of data the file contains and how it should be converted.
#[derive(Debug, Clone, PartialEq)]
pub enum S2sClass {
    /// Standard event-handler script (transpile s2s→lua)
    Script,
    /// Dialog page data (parse as dialogue, output Lua table)
    Dialogue { start_page: String },
    /// Plain text displayed via msgbox (copy as .txt)
    Msgbox { title: String },
    /// Unknown — try s2s2lua, fall back to copy-as-is
    Unknown,
}

/// Scan script content for commands that load other .s2s files,
/// and record the references found.
///
/// This scans BOTH inline scripts from .inf entries AND standalone
/// .s2s file content, using lightweight text matching (no full parse).
pub fn scan_references(
    content: &str,
    _source_path: &Path,
    input_root: &Path,
    refs: &mut Vec<S2sReference>,
) {
    // Normalise line endings and strip comments for cleaner matching
    let text = content.replace("\r\n", "\n");

    // Helper: extract a quoted string starting at position `start` after an opening `"`.
    // Returns (content, end_pos) where end_pos is the position after the closing `"`.
    let extract_quoted = |text: &str, start: usize| -> Option<(String, usize)> {
        let bytes = text.as_bytes();
        if start >= bytes.len() || bytes[start] != b'"' {
            return None;
        }
        let mut i = start + 1;
        while i < bytes.len() {
            if bytes[i] == b'\\' {
                i += 2; // skip escaped char
                continue;
            }
            if bytes[i] == b'"' {
                let s = text[start + 1..i].to_string();
                return Some((s, i + 1));
            }
            i += 1;
        }
        None
    };

    // Helper: extract a non-string token (number, variable, or bare identifier)
    // starting at position `start`. Returns (content, end_pos).
    let extract_token = |text: &str, start: usize| -> Option<(String, usize)> {
        let bytes = text.as_bytes();
        if start >= bytes.len() {
            return None;
        }
        // Skip whitespace
        let mut s = start;
        while s < bytes.len() && (bytes[s] == b' ' || bytes[s] == b'\t') {
            s += 1;
        }
        if s >= bytes.len() {
            return None;
        }
        if bytes[s] == b'$' || bytes[s].is_ascii_digit() || bytes[s] == b'-' {
            // Variable, number, or negative number
            let mut e = s + 1;
            while e < bytes.len()
                && (bytes[e].is_ascii_alphanumeric() || bytes[e] == b'_' || bytes[e] == b'.')
            {
                e += 1;
            }
            let tok = text[s..e].to_string();
            Some((tok, e))
        } else if bytes[s].is_ascii_alphabetic() {
            let mut e = s + 1;
            while e < bytes.len()
                && (bytes[e].is_ascii_alphanumeric() || bytes[e] == b'_')
            {
                e += 1;
            }
            let tok = text[s..e].to_string();
            Some((tok, e))
        } else {
            // Single character token (e.g. `0` for class 0)
            Some((text[s..s + 1].to_string(), s + 1))
        }
    };

    // Helper: skip comma and optional whitespace
    let skip_comma = |text: &str, pos: usize| -> Option<usize> {
        let bytes = text.as_bytes();
        let mut i = pos;
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
            i += 1;
        }
        if i < bytes.len() && bytes[i] == b',' {
            i += 1;
            while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
                i += 1;
            }
            Some(i)
        } else {
            None
        }
    };

    // Resolve a relative path referenced in a script to an absolute path.
    //
    // In Stranded II, all script paths are relative to the mod root (input_root),
    // not to the source file's directory. This applies to both standalone .s2s
    // files and embedded scripts inside .inf entries.
    let resolve_path = |rel: &str| -> Option<PathBuf> {
        let p = Path::new(rel);
        if p.is_absolute() {
            return Some(p.to_path_buf());
        }
        // Strip leading separator or "./" prefix
        let cleaned = rel.trim_start_matches(['/', '\\', '.']);
        let abs = input_root.join(cleaned);
        // Normalise path separators
        let normalised: PathBuf = abs.components().collect();
        Some(normalised)
    };

    // ── Scan line by line for known commands ──
    // We do a simple case-insensitive search for command names
    let lower = text.to_lowercase();

    // Pattern 1: dialogue "startpage", "path" [, "section"]
    let mut search_pos = 0;
    while let Some(pos) = lower[search_pos..].find("dialogue ") {
        let abs_pos = search_pos + pos;
        let after_cmd = abs_pos + "dialogue ".len();

        // After "dialogue ", should be a quoted string (the start page)
        if let Some((start_page, after_page)) = extract_quoted(&text, after_cmd)
            && let Some(after_comma1) = skip_comma(&text, after_page)
                && let Some((path, after_path)) = extract_quoted(&text, after_comma1) {
                    // Optional section
                    let section = skip_comma(&text, after_path)
                        .and_then(|p| extract_quoted(&text, p))
                        .map(|(s, _)| s);

                    if let Some(target) = resolve_path(&path) {
                        refs.push(S2sReference {
                            target,
                            ref_type: S2sRefType::Dialogue { start_page },
                            _section: section,
                        });
                    }
                }

        search_pos = abs_pos + 1;
    }

    // Pattern 2: msgbox "Title", "path"
    let mut search_pos = 0;
    while let Some(pos) = lower[search_pos..].find("msgbox ") {
        let abs_pos = search_pos + pos;
        let after_cmd = abs_pos + "msgbox ".len();

        if let Some((_title, after_title)) = extract_quoted(&text, after_cmd)
            && let Some(after_comma) = skip_comma(&text, after_title)
                && let Some((path, _)) = extract_quoted(&text, after_comma)
                    && let Some(target) = resolve_path(&path) {
                        refs.push(S2sReference {
                            target,
                            ref_type: S2sRefType::Msgbox { title: _title },
                            _section: None,
                        });
                    }

        search_pos = abs_pos + 1;
    }

    // Pattern 3: button id, "text", font, "path"
    let mut search_pos = 0;
    while let Some(pos) = lower[search_pos..].find("button ") {
        let abs_pos = search_pos + pos;
        let after_cmd = abs_pos + "button ".len();

        // Skip the first token (button ID)
        if let Some((_, after_id)) = extract_token(&text, after_cmd)
            && let Some(after_comma1) = skip_comma(&text, after_id) {
                // Skip the button text string
                if let Some((_, after_text)) = extract_quoted(&text, after_comma1)
                    && let Some(after_comma2) = skip_comma(&text, after_text) {
                        // Skip the font number
                        if let Some((_, after_font)) = extract_token(&text, after_comma2)
                            && let Some(after_comma3) = skip_comma(&text, after_font)
                                && let Some((path, _)) = extract_quoted(&text, after_comma3)
                                    && let Some(target) = resolve_path(&path) {
                                        refs.push(S2sReference {
                                            target,
                                            ref_type: S2sRefType::Button,
                                            _section: None,
                                        });
                                    }
                        }
                }

        search_pos = abs_pos + 1;
    }

    // Pattern 4: addscript "class", id, "path" [, "section"]
    let mut search_pos = 0;
    while let Some(pos) = lower[search_pos..].find("addscript ") {
        let abs_pos = search_pos + pos;
        let after_cmd = abs_pos + "addscript ".len();

        if let Some((_class, after_class)) = extract_quoted(&text, after_cmd)
            && let Some(after_comma1) = skip_comma(&text, after_class)
                && let Some((_, after_id)) = extract_token(&text, after_comma1)
                    && let Some(after_comma2) = skip_comma(&text, after_id)
                        && let Some((path, after_path)) = extract_quoted(&text, after_comma2) {
                            let section = skip_comma(&text, after_path)
                                .and_then(|p| extract_quoted(&text, p))
                                .map(|(s, _)| s);

                            if let Some(target) = resolve_path(&path) {
                                refs.push(S2sReference {
                                    target,
                                    ref_type: S2sRefType::AddScript,
                                    _section: section,
                                });
                            }
                        }

        search_pos = abs_pos + 1;
    }

    // Pattern 5: extendscript class, id, "path"
    let mut search_pos = 0;
    while let Some(pos) = lower[search_pos..].find("extendscript ") {
        let abs_pos = search_pos + pos;
        let after_cmd = abs_pos + "extendscript ".len();

        if let Some((_, after_class)) = extract_token(&text, after_cmd)
            && let Some(after_comma1) = skip_comma(&text, after_class)
                && let Some((_, after_id)) = extract_token(&text, after_comma1)
                    && let Some(after_comma2) = skip_comma(&text, after_id)
                        && let Some((path, _)) = extract_quoted(&text, after_comma2)
                            && let Some(target) = resolve_path(&path) {
                                refs.push(S2sReference {
                                    target,
                                    ref_type: S2sRefType::ExtendScript,
                                    _section: None,
                                });
                            }

        search_pos = abs_pos + 1;
    }

    // Pattern 6: def_extend "class", type_id, "path", "section"
    // Pattern 7: def_override "class", type_id, "path", "section"
    for keyword in &["def_extend ", "def_override "] {
        let mut search_pos = 0;
        while let Some(pos) = lower[search_pos..].find(keyword) {
            let abs_pos = search_pos + pos;
            let after_cmd = abs_pos + keyword.len();
            let ref_type = if *keyword == "def_extend " {
                S2sRefType::DefExtend
            } else {
                S2sRefType::DefOverride
            };

            if let Some((_class, after_class)) = extract_quoted(&text, after_cmd)
                && let Some(after_comma1) = skip_comma(&text, after_class)
                    && let Some((_, after_id)) = extract_token(&text, after_comma1)
                        && let Some(after_comma2) = skip_comma(&text, after_id)
                            && let Some((path, after_path)) =
                                extract_quoted(&text, after_comma2)
                            {
                                let section = skip_comma(&text, after_path)
                                    .and_then(|p| extract_quoted(&text, p))
                                    .map(|(s, _)| s);

                                if let Some(target) = resolve_path(&path) {
                                    refs.push(S2sReference {
                                        target,
                                        ref_type: ref_type.clone(),
                                    _section: section,
                                    });
                                }
                            }

            search_pos = abs_pos + 1;
        }
    }
}

/// Build a reference map: for each .s2s file, what commands reference it and how.
///
/// Scans both embedded scripts in .inf entries and standalone .s2s files.
/// Also builds a section-level map for files referenced with section names.
pub fn build_reference_map(
    inf_entries: &std::collections::HashMap<PathBuf, Vec<inf2ron::InfEntry>>,
    s2s_files: &[PathBuf],
    input_root: &Path,
) -> (S2sRefMap, S2sSectionMap) {
    let mut all_refs: Vec<S2sReference> = Vec::new();

    // Scan embedded scripts from .inf entries
    for (inf_path, entries) in inf_entries {
        for entry in entries {
            // Script content is stored in blocks named "script"
            if let Some(script_blocks) = entry.blocks.get("script") {
                for block in script_blocks {
                    if let inf2ron::BlockContent::Text(content) = &block.content {
                        scan_references(content, inf_path, input_root, &mut all_refs);
                    }
                }
            }
        }
    }

    // Scan standalone .s2s files
    for script_path in s2s_files {
        if let Ok(content) = read_file_lossy(script_path) {
            scan_references(&content, script_path, input_root, &mut all_refs);
        }
    }

    // Build file-level map: target path → all RefTypes referencing it
    let mut file_map: S2sRefMap = HashMap::new();
    // Build section-level map: target path → section name → ref types
    let mut section_map: S2sSectionMap = HashMap::new();

    for r in &all_refs {
        file_map
            .entry(r.target.clone())
            .or_default()
            .push(r.ref_type.clone());

        // If this reference has a section name, also record it in the section map
        if let Some(ref section_name) = r._section {
            section_map
                .entry(r.target.clone())
                .or_default()
                .entry(section_name.clone())
                .or_default()
                .push(r.ref_type.clone());
        }
    }

    if !all_refs.is_empty() {
        eprintln!("    {} cross-references found", all_refs.len());
    }

    (file_map, section_map)
}

/// After building the reference map, resolve references whose target files
/// have a different extension than expected (e.g., referenced as `.s2s` in
/// `msgbox`/`button` calls but stored as `.txt` on disk).
///
/// Updates `ref_map` and `section_map` in place so their keys point to the
/// actual files. Returns the list of non-`.s2s` files that need processing,
/// and emits warnings for references that could not be resolved at all.
pub fn resolve_missing_script_refs(
    ref_map: &mut S2sRefMap,
    section_map: &mut S2sSectionMap,
) -> Vec<PathBuf> {
    let mut extra_files: Vec<PathBuf> = Vec::new();
    let mut redirects: Vec<(PathBuf, PathBuf)> = Vec::new(); // old → new

    let keys: Vec<PathBuf> = ref_map.keys().cloned().collect();
    for key in &keys {
        if key.exists() {
            continue;
        }
        // Try swapping .s2s ↔ .txt
        if let Some(ext) = key.extension().and_then(|s| s.to_str()) {
            let alt_ext = match ext {
                "s2s" => "txt",
                "txt" => "s2s",
                _ => {
                    eprintln!("  Warning: referenced script not found: {:?}", key);
                    continue;
                }
            };
            let alt = key.with_extension(alt_ext);
            if alt.exists() {
                redirects.push((key.clone(), alt.clone()));
                extra_files.push(alt);
            } else {
                eprintln!("  Warning: referenced script not found: {:?}", key);
            }
        } else {
            eprintln!("  Warning: referenced script has no extension: {:?}", key);
        }
    }

    // Apply redirects
    for (old_key, new_key) in &redirects {
        if let Some(refs) = ref_map.remove(old_key) {
            ref_map.entry(new_key.clone()).or_insert(refs);
        }
        if let Some(sections) = section_map.remove(old_key) {
            section_map.entry(new_key.clone()).or_insert(sections);
        }
    }

    // Also include any referenced file that is not an .s2s file
    // (e.g., .txt files referenced directly via their real extension).
    // These need to be processed in step 3 alongside .s2s files.
    for key in ref_map.keys() {
        let is_s2s = key.extension().is_some_and(|e| e == "s2s");
        if !is_s2s && key.exists() {
            extra_files.push(key.clone());
        }
    }

    extra_files.sort();
    extra_files.dedup();
    extra_files
}

pub fn classify_s2s(path: &Path, ref_map: &S2sRefMap) -> S2sClass {
    let refs = match ref_map.get(path) {
        Some(r) => r,
        None => return S2sClass::Unknown,
    };

    // Prioritise: dialogue > mixed (both msgbox + script) > msgbox > script
    let mut dialogue_ref = None;
    let mut msgbox_ref = None;
    let mut is_script = false;

    for r in refs {
        match r {
            S2sRefType::Dialogue { start_page } => {
                dialogue_ref = Some(start_page.clone());
            }
            S2sRefType::Msgbox { title } => {
                msgbox_ref = Some(title.clone());
            }
            _ => {
                is_script = true;
            }
        }
    }

    if let Some(start_page) = dialogue_ref {
        // Dialogue takes highest priority
        S2sClass::Dialogue { start_page }
    } else if is_script && msgbox_ref.is_some() {
        // Mixed file: referenced by both msgbox AND button/addscript/etc.
        // Transpile as script (best-effort) rather than plain text.
        S2sClass::Script
    } else if let Some(title) = msgbox_ref {
        // Pure msgbox: plain text
        S2sClass::Msgbox { title }
    } else if is_script {
        S2sClass::Script
    } else {
        S2sClass::Unknown
    }
}
