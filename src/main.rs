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

//! openstranded-s2mod-tool — CLI tool for converting original Stranded II mods.
//!
//! Transforms .inf / .b3d / .bmp / .bmpf / .s2s files into a .s2mod Content Pack.
//! Structure is 1:1 with the original — every file keeps its relative path,
//! only extensions change:
//!
//!   .inf  → .ron [+ .ron.<id>.lua for embedded scripts]
//!   .s2s  → .lua
//!   .b3d  → .glb
//!   .bmp  → .png  (magenta → transparent colour key)
//!   .bmpf → .fnt  (+ .png texture atlas)
//!
//! Pipeline:
//!   1. Walk input directory, classify files
//!   2. Parse .inf → .ron (same tree) + extract scripts → .ron.<id>.lua
//!      2.5 Scan all scripts for cross-references (dialogue/msgbox/button/addscript/etc.)
//!   3. Transpile .s2s → .lua / .txt (using reference map to classify each file)
//!   4. Convert .b3d → .glb (same tree)
//!   5. Convert .bmp → .png (same tree, magenta → transparent)
//!   6. Convert .bmpf → .fnt + .png (same tree)
//!   7. Copy remaining files as-is
//!   8. Update asset paths in .ron files (.b3d→.glb, .bmp→.png)
//!   9. Generate manifest.toml with [registry] section
//!  10. Pack to .s2mod (zip) or directory

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use s2mod_packer::{ContentPackManifest, S2ModPackage};

use openstranded_s2mod_tool::convert::{convert_b3d_to_glb, convert_bmp_to_png, convert_bmpf_to_fnt, convert_inf_to_ron};
use openstranded_s2mod_tool::registry::register_ron;
use openstranded_s2mod_tool::scanner::{build_reference_map, classify_s2s, resolve_missing_script_refs, S2sClass, S2sRefType};
use openstranded_s2mod_tool::script::{convert_sectioned_file, parse_dialogue_to_lua, InfRawSource};
use openstranded_s2mod_tool::util::{copy_to_staging, normalize_id, parse_inf_file, read_file_lossy, relative_path, write_ron};

#[derive(Parser)]
#[command(
    name = "openstranded-s2mod-tool",
    about = "Convert Stranded II mods to .s2mod format"
)]
struct Cli {
    /// Path to the original mod directory
    #[arg(short, long)]
    input: String,

    /// Output .s2mod file path (or directory if --dir)
    #[arg(short, long)]
    output: String,

    /// Keep output as a directory (don't zip)
    #[arg(long)]
    dir: bool,

    /// Skip .b3d → .glb model conversion
    #[arg(long)]
    skip_models: bool,

    /// Skip .bmp → .png texture conversion
    #[arg(long)]
    skip_textures: bool,

    /// Enable debug output
    #[arg(long, default_value_t = false)]
    debug: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let debug = cli.debug;

    println!("openstranded-s2mod-tool v{}", env!("CARGO_PKG_VERSION"));
    println!("Input:  {}", cli.input);
    println!("Output: {}", cli.output);
    println!();

    let input_path = Path::new(&cli.input);
    if !input_path.is_dir() {
        return Err(anyhow!("Input path is not a directory: {}", cli.input));
    }

    // ── Create staging directory ───────────────────────────
    let staging = tempfile::tempdir().context("failed to create temp dir")?;
    let stage_path = staging.path().to_path_buf();
    if debug {
        eprintln!("Staging at: {:?}", stage_path);
    }

    // Mapping: original relative path → new relative path (for extension updates)
    let mut path_mappings: HashMap<String, String> = HashMap::new();

    // Registry tracking: domain → list of .ron file paths (relative to pack root)
    // Key is derived from the .ron filename:
    //   items_edible.ron   → "items"
    //   vehicles_flying.ron → "vehicles"
    //   strings.ron        → "strings"
    //   game.ron           → "game"
    let mut registry: HashMap<String, Vec<String>> = HashMap::new();

    // Collect all files by type for bucketed processing
    let mut inf_files: HashMap<String, Vec<PathBuf>> = HashMap::new();
    let mut s2s_files: Vec<PathBuf> = Vec::new();
    let mut b3d_files: Vec<PathBuf> = Vec::new();
    let mut bmp_files: Vec<PathBuf> = Vec::new();
    let mut bmpf_files: Vec<PathBuf> = Vec::new();
    let mut other_files: Vec<PathBuf> = Vec::new();

    // All parsed .inf entries (keyed by .inf path), used for reference scanning
    let mut parsed_inf_entries: HashMap<PathBuf, Vec<inf2ron::InfEntry>> = HashMap::new();

    // ── Step 1: Walk & Classify ────────────────────────────
    println!("[1/10] Walking input directory...");

    for entry in walkdir::WalkDir::new(input_path)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        match path.extension().and_then(|s| s.to_str()) {
            Some("inf") => {
                // Derive domain from filename (first token before '_', or full stem)
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                let domain = stem.split('_').next().unwrap_or(stem).to_string();
                inf_files.entry(domain).or_default().push(path.to_path_buf());
            }
            Some("s2s") => s2s_files.push(path.to_path_buf()),
            Some("b3d") => b3d_files.push(path.to_path_buf()),
            Some("bmp") => bmp_files.push(path.to_path_buf()),
            Some("bmpf") => bmpf_files.push(path.to_path_buf()),
            _ => other_files.push(path.to_path_buf()),
        }
    }

    // ── Step 2: Parse .inf → .ron + extract scripts ───────
    println!("[2/10] Parsing .inf files...");

    for (domain, paths) in &inf_files {
        if debug {
            eprintln!("  Domain {:?}: {} file(s)", domain, paths.len());
        }
        for inf_path in paths {
            let entries = match parse_inf_file(inf_path) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("  Warning: failed to parse {:?}: {}", inf_path, e);
                    // Write .ron with raw source content instead of copying .inf
                    let raw = InfRawSource {
                        parse_error: format!("{:#}", e),
                        raw_content: read_file_lossy(inf_path).unwrap_or_default(),
                    };
                    let ron_rel = relative_path(inf_path, input_path).with_extension("ron");
                    let ron_path = stage_path.join(&ron_rel);
                    if let Some(parent) = ron_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    if let Err(we) = write_ron(&ron_path, &raw) {
                        eprintln!("  Warning: failed to write raw .ron for {:?}: {:#}", inf_path, we);
                    } else {
                        register_ron(&ron_rel, &mut registry);
                    }
                    continue;
                }
            };

            // Unified conversion: extract scripts, write .ron, register
            convert_inf_to_ron(inf_path, input_path, &stage_path, &mut entries.clone(), &mut registry, debug)
                .unwrap_or_else(|e| {
                    eprintln!("  Warning: failed to convert {:?}: {:#}", inf_path, e);
                });

            // Collect parsed entries for reference scanning (step 2.5)
            parsed_inf_entries.insert(inf_path.to_path_buf(), entries);
        }
    }

    println!("    {} .inf files processed, {} registry entries",
        parsed_inf_entries.len(),
        registry.len());

    // ── Step 2.5: Build S2S reference map ──────────────────
    println!("[2.5/10] Scanning script references...");
    let (mut s2s_ref_map, mut s2s_section_map) =
        build_reference_map(&parsed_inf_entries, &s2s_files, input_path);

    // Resolve references where the referenced extension doesn't match the
    // actual file on disk (e.g., msgbox points to `.s2s` but file is `.txt`).
    let extra_script_files = resolve_missing_script_refs(&mut s2s_ref_map, &mut s2s_section_map);
    if !extra_script_files.is_empty() {
        println!("    {} resolved with alternate extension", extra_script_files.len());
    }

    let dialogue_count = s2s_ref_map.values()
        .filter(|refs| refs.iter().any(|r| matches!(r, S2sRefType::Dialogue { .. })))
        .count();
    let msgbox_count = s2s_ref_map.values()
        .filter(|refs| refs.iter().any(|r| matches!(r, S2sRefType::Msgbox { .. })))
        .count();
    if dialogue_count > 0 || msgbox_count > 0 {
        println!("    {} dialogue files, {} msgbox files", dialogue_count, msgbox_count);
    }

    // ── Step 3: Transpile .s2s → .lua (also referenced .txt → .lua) ──
    println!("[3/10] Transpiling scripts...");
    let mut s2s_ok = 0u32;
    let mut s2s_err = 0u32;
    let mut s2s_dialogue = 0u32;
    let mut s2s_msgbox = 0u32;

    // Process both .s2s files and extra referenced files (.txt etc.)
    let all_script_files: Vec<&PathBuf> = s2s_files.iter()
        .chain(extra_script_files.iter())
        .collect();

    for script_path_ptr in &all_script_files {
        let script_path = *script_path_ptr;
        let content = match read_file_lossy(script_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("  Warning: {:#}", e);
                copy_to_staging(script_path, input_path, &stage_path)?;
                s2s_err += 1;
                continue;
            }
        };

        let s2s_rel = relative_path(script_path, input_path);
        let lua_rel = s2s_rel.with_extension("lua");
        let lua_path = stage_path.join(&lua_rel);

        // ── Sectioned files (contain //~ markers) ──────────
        if content.contains("//~") {
            let sections_refs = s2s_section_map
                .get(script_path)
                .cloned()
                .unwrap_or_default();
            let lua = convert_sectioned_file(&content, &sections_refs);

            if let Some(parent) = lua_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&lua_path, &lua)
                .with_context(|| format!("writing {:?}", lua_path))?;
            s2s_ok += 1;

            if debug {
                eprintln!("    {:?} → {:?} (sectioned)", script_path, lua_path);
            }
            continue;
        }

        // ── Non-sectioned files: classify and convert ──────
        let classification = classify_s2s(script_path, &s2s_ref_map);

        // For non-.s2s files (e.g., .txt), force msgbox-style text conversion.
        let is_plain_text = script_path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e != "s2s");

        if is_plain_text && !matches!(classification, S2sClass::Dialogue { .. }) {
            // Treat as msgbox-style text — wrap in return string
            let lua_content = format!(
                "-- Generated by openstranded-s2mod-tool (plain text ref)\nreturn [===[\n{}]===]\n",
                content
            );
            if let Some(parent) = lua_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&lua_path, &lua_content)
                .with_context(|| format!("writing {:?}", lua_path))?;
            s2s_msgbox += 1;

            if debug {
                eprintln!("    {:?} → {:?} (plain text, {})", script_path, lua_path, lua_rel.display());
            }
        } else {
            match classification {
                S2sClass::Dialogue { start_page: _ } => {
                    // Parse as dialog data → Lua table
                    match parse_dialogue_to_lua(&content) {
                        Ok(lua) => {
                            if let Some(parent) = lua_path.parent() {
                                fs::create_dir_all(parent)?;
                            }
                            fs::write(&lua_path, &lua)
                                .with_context(|| format!("writing {:?}", lua_path))?;
                            s2s_dialogue += 1;

                            if debug {
                                eprintln!("    {:?} → {:?} (dialogue)", script_path, lua_path);
                            }
                        }
                        Err(e) => {
                            eprintln!("  Warning: failed to parse dialog {:?}: {:#}", script_path, e);
                            copy_to_staging(script_path, input_path, &stage_path)?;
                            s2s_err += 1;
                        }
                    }
                }
                S2sClass::Msgbox { title: _ } => {
                    // Plain text — write as .lua returning the string
                    let lua_content = format!(
                        "-- Generated by openstranded-s2mod-tool (msgbox text)\nreturn [===[\n{}]===]\n",
                        content
                    );
                    if let Some(parent) = lua_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::write(&lua_path, &lua_content)
                        .with_context(|| format!("writing {:?}", lua_path))?;
                    s2s_msgbox += 1;

                    if debug {
                        eprintln!("    {:?} → {:?} (msgbox text)", script_path, lua_path);
                    }
                }
                S2sClass::Script | S2sClass::Unknown => {
                    // Transpile as s2s script → Lua
                    match s2s2lua::S2sParser::parse(&content) {
                        Ok(script) => {
                            match s2s2lua::LuaGenerator::generate(&script, s2s2lua::GenOptions::default()) {
                                Ok(lua) => {
                                    if let Some(parent) = lua_path.parent() {
                                        fs::create_dir_all(parent)?;
                                    }
                                    fs::write(&lua_path, &lua)
                                        .with_context(|| format!("writing {:?}", lua_path))?;
                                    s2s_ok += 1;

                                    if debug {
                                        eprintln!("    {:?} → {:?}", script_path, lua_path);
                                    }
                                }
                                Err(e) => {
                                    eprintln!("  Warning: failed to generate lua for {:?}: {:#}", script_path, e);
                                    copy_to_staging(script_path, input_path, &stage_path)?;
                                    s2s_err += 1;
                                }
                            }
                        }
                        Err(e) => {
                            if matches!(classification, S2sClass::Unknown) {
                                if debug {
                                    eprintln!("    {:?} → copy-as-is (not valid s2s)", script_path);
                                }
                                copy_to_staging(script_path, input_path, &stage_path)?;
                            } else {
                                eprintln!("  Warning: failed to parse {:?}: {}", script_path, e);
                                copy_to_staging(script_path, input_path, &stage_path)?;
                                s2s_err += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    println!("    {s2s_ok} transpiled, {s2s_dialogue} dialogue, {s2s_msgbox} msgbox, {s2s_err} failed");

    // ── Step 4: Convert .b3d → .glb ───────────────────────
    if cli.skip_models {
        if !b3d_files.is_empty() {
            println!("[4/10] {} .b3d files skipped (--skip-models)", b3d_files.len());
        }
        for b3d_path in &b3d_files {
            copy_to_staging(b3d_path, input_path, &stage_path)?;
        }
    } else {
        println!("[4/10] Converting .b3d models → .glb...");
        let mut ok = 0u32;
        let mut err = 0u32;

        for b3d_path in &b3d_files {
            let glb_rel = relative_path(b3d_path, input_path).with_extension("glb");
            let glb_path = stage_path.join(&glb_rel);
            if let Some(parent) = glb_path.parent() {
                fs::create_dir_all(parent)?;
            }

            let stem = b3d_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("model");

            match convert_b3d_to_glb(b3d_path, &glb_path, input_path, stem) {
                Ok(()) => {
                    ok += 1;
                    let orig_rel = relative_path(b3d_path, input_path).to_string_lossy().to_string();
                    let new_rel = glb_rel.to_string_lossy().to_string();
                    path_mappings.insert(orig_rel, new_rel);

                    if debug {
                        eprintln!("    {:?} → {:?}", b3d_path, glb_path);
                    }
                }
                Err(e) => {
                    eprintln!("  Warning: failed to convert {:?}: {:#}", b3d_path, e);
                    copy_to_staging(b3d_path, input_path, &stage_path)?;
                    err += 1;
                }
            }
        }
        if ok > 0 || err > 0 {
            println!("    {ok} converted, {err} failed");
        }
    }

    // ── Step 5: Convert .bmp → .png ───────────────────────
    if cli.skip_textures {
        if !bmp_files.is_empty() {
            println!("[5/10] {} .bmp files skipped (--skip-textures)", bmp_files.len());
        }
        for bmp_path in &bmp_files {
            copy_to_staging(bmp_path, input_path, &stage_path)?;
        }
    } else {
        println!("[5/10] Converting .bmp textures → .png...");
        let mut ok = 0u32;
        let mut err = 0u32;

        for bmp_path in &bmp_files {
            let png_rel = relative_path(bmp_path, input_path).with_extension("png");
            let png_path = stage_path.join(&png_rel);
            if let Some(parent) = png_path.parent() {
                fs::create_dir_all(parent)?;
            }

            match convert_bmp_to_png(bmp_path, &png_path) {
                Ok(()) => {
                    ok += 1;
                    let orig_rel = relative_path(bmp_path, input_path).to_string_lossy().to_string();
                    let new_rel = png_rel.to_string_lossy().to_string();
                    path_mappings.insert(orig_rel, new_rel);

                    if debug {
                        eprintln!("    {:?} → {:?}", bmp_path, png_path);
                    }
                }
                Err(e) => {
                    eprintln!("  Warning: failed to convert {:?}: {:#}", bmp_path, e);
                    copy_to_staging(bmp_path, input_path, &stage_path)?;
                    err += 1;
                }
            }
        }
        if ok > 0 || err > 0 {
            println!("    {ok} converted, {err} failed");
        }
    }

    // ── Step 6: Convert .bmpf → .fnt + .png ───────────────
    if !bmpf_files.is_empty() {
        println!("[6/10] Converting .bmpf bitmap fonts...");

        for bmpf_path in &bmpf_files {
            let stem = bmpf_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("font");

            // Look for corresponding .bmp
            let bmp_path = bmpf_path.with_extension("bmp");
            if !bmp_path.exists() {
                eprintln!("  Warning: no matching .bmp for {:?}, copying as-is", bmpf_path);
                copy_to_staging(bmpf_path, input_path, &stage_path)?;
                continue;
            }

            // .fnt goes alongside the .bmpf
            let fnt_rel = relative_path(bmpf_path, input_path).with_extension("fnt");
            let fnt_path = stage_path.join(&fnt_rel);
            if let Some(parent) = fnt_path.parent() {
                fs::create_dir_all(parent)?;
            }

            // .png from the .bmp goes alongside
            let png_rel = relative_path(&bmp_path, input_path).with_extension("png");
            let png_path = stage_path.join(&png_rel);
            if let Some(parent) = png_path.parent() {
                fs::create_dir_all(parent)?;
            }

            let bmp_rel = relative_path(&bmp_path, input_path).to_string_lossy().to_string();

            match convert_bmpf_to_fnt(bmpf_path, &bmp_path, &fnt_path, &png_path, stem, input_path) {
                Ok(()) => {
                    if debug {
                        eprintln!("    {stem}.fnt + .png generated");
                    }
                    // Record .bmp mapping if not already done
                    path_mappings.entry(bmp_rel).or_insert_with(|| {
                        png_rel.to_string_lossy().to_string()
                    });
                }
                Err(e) => {
                    eprintln!("  Warning: failed to convert {:?}: {:#}", bmpf_path, e);
                    copy_to_staging(bmpf_path, input_path, &stage_path)?;
                }
            }
        }
        println!("    {} fonts processed", bmpf_files.len());
    }

    // ── Step 7: Copy remaining files ───────────────────────
    println!("[7/10] Copying remaining files...");

    let extra_set: HashSet<&PathBuf> = extra_script_files.iter().collect();
    let mut copied = 0u32;
    for file_path in &other_files {
        if extra_set.contains(file_path) {
            continue; // Already converted to .lua in step 3
        }
        match copy_to_staging(file_path, input_path, &stage_path) {
            Ok(()) => copied += 1,
            Err(e) => eprintln!("  Warning: failed to copy {:?}: {:#}", file_path, e),
        }
    }
    println!("    {copied} files copied");

    // ── Step 8: Update asset paths in .ron files ───────────
    if !path_mappings.is_empty() {
        println!("[8/10] Updating asset paths in .ron files...");
        let mut updated_files = 0u32;
        let mut total_replacements = 0u32;

        for entry in walkdir::WalkDir::new(&stage_path)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("ron") {
                continue;
            }

            let content = match fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let original_content = content.clone();
            let mut new_content = content;
            let mut replaced = false;

            // ── Phase A: Normalize backslashes → forward slashes ──
            let raw_double_bs = "\\\\";
            if new_content.contains(raw_double_bs) {
                new_content = new_content.replace(raw_double_bs, "/");
                replaced = true;
            }

            // ── Phase B: Replace .b3d → .glb, .bmp → .png ──
            for (orig_path, new_path) in &path_mappings {
                let orig_fs = orig_path.replace('\\', "/");
                let new_fs = new_path.replace('\\', "/");

                if new_content.contains(&orig_fs) {
                    new_content = new_content.replace(&orig_fs, &new_fs);
                    replaced = true;
                }
            }

            if replaced && new_content != original_content {
                fs::write(path, &new_content)
                    .with_context(|| format!("writing updated {:?}", path))?;
                updated_files += 1;
                total_replacements += 1;
            }
        }
        println!("    {updated_files} .ron files updated ({total_replacements} replacements)");
    } else {
        println!("[8/10] No asset path updates needed");
    }

    // ── Step 9: Generate manifest ───────────────────────────
    println!("[9/10] Generating manifest...");

    let pack_name = input_path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "content".to_string());
    let pack_id = normalize_id(&pack_name);

    let mut manifest = ContentPackManifest::new(&pack_id, &pack_name);

    // Populate registry from the auto-registered HashMap — each key is
    // derived from the .ron filename and maps directly to a TOML domain.
    for (key, paths) in &registry {
        manifest.registry.domains.insert(key.clone(), paths.clone());
    }

    if debug {
        eprintln!("  Registry keys: {:?}", registry.keys().collect::<Vec<_>>());
    }

    let manifest_toml = manifest.to_toml()?;
    fs::write(stage_path.join("manifest.toml"), &manifest_toml)?;
    if debug {
        eprintln!("  manifest.toml:\n{}", manifest_toml);
    }

    // ── Step 10: Pack ───────────────────────────────────────
    println!("[10/10] Packing...");

    let output_path = Path::new(&cli.output);
    if cli.dir {
        if output_path.exists() {
            fs::remove_dir_all(output_path)?;
        }
        S2ModPackage::pack_to_dir(&stage_path, output_path)?;
        println!("  Written to directory: {}", cli.output);
    } else {
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }
        if output_path.exists() {
            fs::remove_file(output_path)?;
        }
        S2ModPackage::pack_to_zip(&stage_path, output_path)?;
        println!("  Written to archive: {}", cli.output);
    }

    println!();
    println!("Done!");

    Ok(())
}
