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

//! File format converters: `.inf → .ron`, `.b3d → .glb`, `.bmp → .png`,
//! `.bmpf → .fnt + .png`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use bmpf2fnt::{build_font_atlas, generate_bmfont, BmpfFont};

use crate::registry::register_ron;
use crate::script::{format_script_ref, write_script_lua};
use crate::util::{relative_path, write_ron};

/// Unified .inf → .ron conversion: extract scripts, write .ron, register.
///
/// Every .inf file is parsed the same way:
/// 1. Extract embedded scripts → `<ron_stem>.<N>.lua` (sequential numbering)
/// 2. Replace each script block's content with the reference path (now external)
/// 3. Write raw entries as `Vec<InfEntry>` to `.ron`
/// 4. Auto-register in manifest based on filename
///
/// Script blocks are identified by block name `"script"`. Each `script=start…end`
/// block is extracted, written as `<ron_stem>.<seq>.lua`, and the block's text
/// content is replaced with the reference path.
pub fn convert_inf_to_ron(
    inf_path: &Path,
    input_root: &Path,
    stage_root: &Path,
    entries: &mut [inf2ron::InfEntry],
    registry: &mut HashMap<String, Vec<String>>,
    debug: bool,
) -> Result<()> {
    let ron_rel = relative_path(inf_path, input_root).with_extension("ron");
    let ron_path = stage_root.join(&ron_rel);

    if let Some(parent) = ron_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Extract embedded scripts with sequential numbering
    // and replace each script block's content with the reference path.
    let mut seq = 0usize;
    for entry in entries.iter_mut() {
        if let Some(script_blocks) = entry.blocks.get_mut("script") {
            for block in script_blocks.iter_mut() {
                if let inf2ron::BlockContent::Text(script_s2s) = &mut block.content {
                    seq += 1;
                    let s2s = std::mem::take(script_s2s);
                    // Write the extracted script
                    write_script_lua(inf_path, input_root, stage_root, seq, &s2s, debug)?;
                    // Replace inline script with reference path
                    block.content = inf2ron::BlockContent::Text(format_script_ref(&ron_rel, seq));
                }
            }
        }
    }

    // Write .ron — cast to shared slice for Serialize
    let entries_slice: &[inf2ron::InfEntry] = &*entries;
    write_ron(&ron_path, entries_slice)?;

    // Auto-register in manifest based on filename
    register_ron(&ron_rel, registry);

    if debug {
        eprintln!("    {:?} → {:?} ({} entries, {} scripts)", inf_path, ron_path, entries.len(), seq);
    }

    Ok(())
}

/// Convert a `.b3d` file to `.glb` using the b3d2glb library.
pub fn convert_b3d_to_glb(
    b3d_path: &Path,
    glb_path: &Path,
    game_root: &Path,
    _stem: &str,
) -> Result<()> {
    let data = fs::read(b3d_path).with_context(|| format!("reading {:?}", b3d_path))?;

    let b3d = b3d2glb::b3d_parser::B3D::read(&data)
        .map_err(|e| anyhow::anyhow!("B3D parse error in {:?}: {}", b3d_path, e))?;

    let mesh = b3d2glb::b3d::collect_mesh(&b3d);
    let mut joints = Vec::new();
    let mut vertex_joint = vec![None; mesh.positions.len()];
    b3d2glb::b3d::collect_joints(&b3d.node, None, &mut joints, &mut vertex_joint, mesh.positions.len(), true);
    let clips = b3d2glb::b3d::collect_anims(&b3d.node);

    // Create a texture cache directory alongside the .glb
    let tex_cache = glb_path.parent().unwrap_or(Path::new(".")).join(".tex_cache");
    fs::create_dir_all(&tex_cache)?;

    b3d2glb::writer::write_glb(
        &mesh,
        &joints,
        &clips,
        &b3d.textures,
        &b3d.brushes,
        _stem,
        game_root,
        &tex_cache,
        glb_path,
        None, // material_params
        None, // color_override
    )
    .map_err(|e| anyhow::anyhow!("GLB conversion error in {:?}: {}", b3d_path, e))?;

    Ok(())
}

/// Convert a `.bmp` texture to `.png`, converting the magenta colour key
/// (255, 0, 255 ± tolerance) to fully transparent.
///
/// Blitz3D/Stranded II uses 24-bit BMP without alpha; magenta pixels serve
/// as the transparency colour key.  We convert them to `(0,0,0,0)` so the
/// output PNG has a proper alpha channel.
pub fn convert_bmp_to_png(bmp_path: &Path, png_path: &Path) -> Result<()> {
    let img = image::ImageReader::open(bmp_path)
        .with_context(|| format!("opening {:?}", bmp_path))?
        .decode()
        .with_context(|| format!("decoding {:?}", bmp_path))?;

    // Convert to RGBA so we can set alpha on colour-key pixels
    let mut rgba = img.to_rgba8();
    let tolerance = 10u8;

    for pixel in rgba.pixels_mut() {
        // magenta colour key: R=255, G=0, B=255
        let dr = pixel[0].abs_diff(255);
        let dg = pixel[1].abs_diff(0);
        let db = pixel[2].abs_diff(255);
        if dr <= tolerance && dg <= tolerance && db <= tolerance {
            pixel[3] = 0; // fully transparent
        }
    }

    if let Some(parent) = png_path.parent() {
        fs::create_dir_all(parent)?;
    }
    rgba.save(png_path)
        .with_context(|| format!("saving {:?}", png_path))?;

    Ok(())
}

/// Convert a `.bmpf` bitmap font + its `.bmp` texture into `.fnt` + `.png`.
pub fn convert_bmpf_to_fnt(
    bmpf_path: &Path,
    bmp_path: &Path,
    fnt_path: &Path,
    png_path: &Path,
    stem: &str,
    _input_root: &Path,
) -> Result<()> {
    // Read and parse .bmpf
    let bmpf_data = fs::read(bmpf_path)
        .with_context(|| format!("reading {:?}", bmpf_path))?;
    let bmpf = BmpfFont::parse(&bmpf_data)
        .map_err(|e| anyhow::anyhow!("invalid .bmpf {:?}: {e}", bmpf_path))?;

    // Load the matching .bmp as RGBA
    let img = image::ImageReader::open(bmp_path)
        .with_context(|| format!("opening {:?}", bmp_path))?
        .decode()
        .with_context(|| format!("decoding {:?}", bmp_path))?;
    let rgba = img.to_rgba8();
    let (img_w, img_h) = rgba.dimensions();
    let pixels = rgba.into_raw();

    // Build font atlas (scan glyphs, match to bmpf chars)
    let atlas = build_font_atlas(&pixels, img_w, img_h, &bmpf)?;

    // Always convert the BMP to PNG (with magenta→transparent)
    convert_bmp_to_png(bmp_path, png_path)?;

    // Compute relative path from .fnt's directory to .png for the .fnt reference
    let png_rel = if let Some(fnt_parent) = fnt_path.parent() {
        pathdiff::diff_paths(png_path, fnt_parent)
            .unwrap_or_else(|| PathBuf::from(stem).with_extension("png"))
    } else {
        PathBuf::from(stem).with_extension("png")
    };

    let fnt_content = generate_bmfont(&atlas, stem, png_rel.to_string_lossy().as_ref());
    fs::write(fnt_path, &fnt_content)
        .with_context(|| format!("writing {:?}", fnt_path))?;

    Ok(())
}
