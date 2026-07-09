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

//! End-to-end tests for the openstranded-convert CLI.
//!
//! ## Non-ignored tests (run in CI, use bundled text fixtures)
//!
//! These test the registry parser, .s2s→.lua transpiler, manifest generation,
//! and skip flags using a small text-only fixture under `tests/fixtures/`.
//! No .b3d or .bmp files are needed — models/textures are skipped.
//!
//! ## Ignored tests (run with original game data)
//!
//! A second set of tests exercises the full pipeline including .b3d→.glb and
//! .bmp→.png conversion.  These require the original Stranded II game data at:
//!   `/home/admen/Games/umu/umu-default/drive_c/Games/StrandedII/mods/Stranded II`
//!
//! Run with: `cargo test --test e2e -- --ignored`

use std::path::{Path, PathBuf};
use std::process::Command;

/// Path to the bundled text fixture directory.
const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/minimal_mod");

/// Path to the reference-extension fixture (tests .s2s↔.txt resolution).
const REF_EXT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ref_ext");

/// Path to the original mod directory (for full pipeline tests).
const ORIGINAL_MOD_DIR: &str =
    "/home/admen/Games/umu/umu-default/drive_c/Games/StrandedII/mods/Stranded II";

/// File names (without extension) to check in the fixture output.
const FIXTURE_RON_STEMS: &[&str] = &[
    "items_test",
    "objects_test",
    "units",
    "buildings",
    "combinations_test",
];

/// Helper: run the convert binary and return (stdout, stderr).
fn run_convert(input: &str, output: &str, extra_args: &[&str]) -> Result<(String, String), String> {
    let bin = binary_path();

    let mut cmd = Command::new(&bin);
    cmd.arg("--input")
        .arg(input)
        .arg("--output")
        .arg(output)
        .args(extra_args);

    let output = cmd.output().map_err(|e| format!("failed to execute: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        return Err(format!(
            "exit code: {}\nstdout:\n{}\nstderr:\n{}",
            output.status, stdout, stderr
        ));
    }

    Ok((stdout, stderr))
}

/// Locate the built binary.
fn binary_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("openstranded-convert")
}

// ═══════════════════════════════════════════════════════════════
//  Non-ignored tests  (fixture-based, run in CI)
// ═══════════════════════════════════════════════════════════════

#[test]
fn fixture_registry_ron_is_valid() {
    let tmp = tempfile::tempdir().unwrap();
    let output_dir = tmp.path().join("out");

    let (stdout, _stderr) = run_convert(
        FIXTURE_DIR,
        output_dir.to_str().unwrap(),
        &["--dir", "--skip-models", "--skip-textures"],
    )
    .expect("fixture conversion");

    // Every domain present in stdout
    assert!(stdout.contains(".inf files processed"), "inf count: {stdout}");
    assert!(stdout.contains("registry entries"), "registry count: {stdout}");
    assert!(stdout.contains("Done!"), "done: {stdout}");

    // RON files exist alongside their original .inf paths (1:1 structure)
    for stem in FIXTURE_RON_STEMS {
        let path = output_dir.join(format!("{stem}.ron"));
        assert!(path.exists(), "{stem}.ron exists: {path:?}");
        let content = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("read {stem}"));
        let value: ron::Value =
            ron::from_str(&content).unwrap_or_else(|e| panic!("{stem} RON: {e}"));
        // Most RON files are Vec<InfEntry> → Seq; raw parse failures are InfRawSource → Map
        if !matches!(value, ron::Value::Seq(_)) {
            // Allow Map for InfRawSource (parse failed but still valid RON)
            assert!(matches!(value, ron::Value::Map(_)), "{stem} is Seq or Map, got {value:?}");
        }
    }
}

#[test]
fn fixture_scripts_valid_lua() {
    let tmp = tempfile::tempdir().unwrap();
    let output_dir = tmp.path().join("out");

    let (_stdout, _stderr) = run_convert(
        FIXTURE_DIR,
        output_dir.to_str().unwrap(),
        &["--dir", "--skip-models", "--skip-textures"],
    )
    .expect("fixture conversion");

    // In the 1:1 structure, .s2s → .lua keeps its original relative path
    let lua_path = output_dir.join("scripts").join("test.lua");
    assert!(lua_path.exists(), "scripts/test.lua should exist");

    let content = std::fs::read_to_string(&lua_path).expect("read scripts/test.lua");
    assert!(content.contains("core_api"), "core_api reference");
    assert!(content.contains("-- Generated by"), "header comment");
}

#[test]
fn fixture_manifest_has_content() {
    let tmp = tempfile::tempdir().unwrap();
    let output_dir = tmp.path().join("out");

    let (_stdout, _stderr) = run_convert(
        FIXTURE_DIR,
        output_dir.to_str().unwrap(),
        &["--dir", "--skip-models", "--skip-textures"],
    )
    .expect("fixture conversion");

    let content = std::fs::read_to_string(output_dir.join("manifest.toml"))
        .expect("read manifest.toml");

    let value: toml::Value = toml::from_str(&content).expect("valid TOML");

    assert!(value.get("pack").is_some(), "[pack]");
    assert!(value.get("registry").is_some(), "[registry]");

    let items = value["registry"]["items"].as_array();
    assert!(items.is_some(), "registry.items");
    assert!(!items.unwrap().is_empty(), "registry.items non-empty");

    assert!(value["pack"].get("name").is_some(), "pack.name");
}

#[test]
fn fixture_skip_flags_work() {
    let tmp = tempfile::tempdir().unwrap();
    let output_dir = tmp.path().join("out");

    let (stdout, _stderr) = run_convert(
        FIXTURE_DIR,
        output_dir.to_str().unwrap(),
        &["--dir", "--skip-models", "--skip-textures"],
    )
    .expect("fixture skip flags");

    // Registry RON files still exist (1:1 structure — alongside .inf files)
    assert!(
        output_dir.join("items_test.ron").exists(),
        "items_test.ron should still exist"
    );
    assert!(
        stdout.contains("scripts"),
        "scripts should still be processed: {stdout}"
    );

    // Models/textures were skipped, and in the 1:1 structure they follow
    // their original relative paths (usually inside gfx/). Since the fixture
    // has no gfx/ directory, there should be no .glb or .png files at all.
    let glb_count = count_files(&output_dir, "glb");
    assert_eq!(glb_count, 0, "no .glb files when models skipped");
    let png_count = count_files(&output_dir, "png");
    assert_eq!(png_count, 0, "no .png files when textures skipped");
}

#[test]
fn fixture_full_conversion_creates_s2mod() {
    let tmp = tempfile::tempdir().unwrap();
    let output_path = tmp.path().join("output.s2mod");

    let (stdout, _stderr) = run_convert(
        FIXTURE_DIR,
        output_path.to_str().unwrap(),
        &["--skip-models", "--skip-textures"],
    )
    .expect("fixture full conversion");

    assert!(output_path.exists(), "output .s2mod should exist");
    assert!(
        output_path.metadata().unwrap().len() > 100,
        "output should be > 100 bytes (was {} bytes)",
        output_path.metadata().unwrap().len()
    );

    assert!(stdout.contains(".inf files processed"), "inf count: {stdout}");
    assert!(stdout.contains("registry entries"), "registry count: {stdout}");
    assert!(stdout.contains("script"), "scripts: {stdout}");
    assert!(stdout.contains("Done!"), "done: {stdout}");
}

// ═══════════════════════════════════════════════════════════════
//  Reference extension resolution tests
// ═══════════════════════════════════════════════════════════════

#[test]
fn fixture_ref_ext_txt_as_s2s_converted_to_lua() {
    // Tests: .txt file referenced as .s2s in msgbox → gets converted to .lua
    let tmp = tempfile::tempdir().unwrap();
    let output_dir = tmp.path().join("out");

    let (stdout, _stderr) = run_convert(
        REF_EXT_DIR,
        output_dir.to_str().unwrap(),
        &["--dir", "--skip-models", "--skip-textures"],
    )
    .expect("ref_ext conversion");

    // butterflygarden is .txt referenced as .s2s → should become .lua
    let lua_path = output_dir.join("sys").join("scripts").join("butterflygarden.lua");
    assert!(lua_path.exists(), "butterflygarden.lua should exist");
    let content = std::fs::read_to_string(&lua_path).expect("read butterflygarden.lua");
    assert!(content.contains("plain text ref") || content.contains("msgbox text"),
        "should be text conversion, got: {content}");
    assert!(content.contains("butterflies"), "should contain original text");

    // The .txt should NOT exist in output (converted, not copied)
    let txt_path = output_dir.join("sys").join("scripts").join("butterflygarden.txt");
    assert!(!txt_path.exists(), "butterflygarden.txt should NOT exist (converted to .lua)");

    // Should report resolution
    assert!(stdout.contains("alternate extension"), "should report extension resolution");
}

#[test]
fn fixture_ref_ext_direct_txt_to_lua() {
    // Tests: .txt file referenced directly (not as .s2s) → still gets converted
    let tmp = tempfile::tempdir().unwrap();
    let output_dir = tmp.path().join("out");

    let (_stdout, _stderr) = run_convert(
        REF_EXT_DIR,
        output_dir.to_str().unwrap(),
        &["--dir", "--skip-models", "--skip-textures"],
    )
    .expect("ref_ext direct txt");

    let lua_path = output_dir.join("sys").join("scripts").join("directref.lua");
    assert!(lua_path.exists(), "directref.lua should exist (direct .txt ref)");
    let content = std::fs::read_to_string(&lua_path).expect("read directref.lua");
    assert!(content.contains("-- Generated"), "should be converted, got: {content}");
}

#[test]
fn fixture_ref_ext_warns_missing() {
    // Tests: reference to non-existent file → warning, no crash
    let tmp = tempfile::tempdir().unwrap();
    let output_dir = tmp.path().join("out");

    let (stdout, stderr) = run_convert(
        REF_EXT_DIR,
        output_dir.to_str().unwrap(),
        &["--dir", "--skip-models", "--skip-textures"],
    )
    .expect("ref_ext conversion (should not crash)");

    // Warning about missing file should appear in stderr
    assert!(stderr.contains("Warning"), "should emit warning for missing ref: {stderr}");
    assert!(stderr.contains("nonexistent"), "warning should mention nonexistent.s2s: {stderr}");
    assert!(stdout.contains("Done!"), "conversion should complete");
}

// ═══════════════════════════════════════════════════════════════
//  Stranded II full conversion (local data, generates tests/out/)
// ═══════════════════════════════════════════════════════════════
//
// These tests convert the original Stranded II mod from tests/stranded2/
// and write output to tests/out/. Both .s2mod (zip) and directory formats
// are generated so the output can be inspected.
//
// Only runs when the tests/stranded2/ directory exists (local data).
// The tests/out/ and tests/stranded2/ directories are .gitignored.

/// Path to the local Stranded II data directory.
const STRANDED2_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/stranded2");

/// Path to the persistent output directory.
const STRANDED2_OUT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/out");

/// Check if the Stranded II test data is available.
fn stranded2_data_available() -> bool {
    let path = std::path::Path::new(STRANDED2_DIR);
    path.exists() && path.is_dir()
}

#[test]
fn stranded2_converts_to_dir() {
    if !stranded2_data_available() {
        eprintln!("skipped: tests/stranded2/ not available");
        return;
    }

    let out_dir = format!("{}/dir", STRANDED2_OUT_DIR);
    // Clean the output directory
    let out_path = std::path::Path::new(&out_dir);
    if out_path.exists() {
        std::fs::remove_dir_all(out_path).expect("remove tests/out/dir/");
    }
    std::fs::create_dir_all(out_path).expect("create tests/out/dir/");

    let (stdout, _stderr) = run_convert(STRANDED2_DIR, &out_dir, &["--dir"])
        .expect("stranded2 conversion to dir");

    // Basic checks
    assert!(stdout.contains("Done!"), "done: {stdout}");
    assert!(stdout.contains("inf files processed"), "inf count: {stdout}");

    // Manifest exists and has [registry]
    let manifest_path = out_path.join("manifest.toml");
    assert!(manifest_path.exists(), "manifest.toml should exist");
    let manifest_content = std::fs::read_to_string(&manifest_path).expect("read manifest.toml");
    let manifest: toml::Value = toml::from_str(&manifest_content).expect("valid TOML");
    assert!(manifest.get("registry").is_some(), "[registry] in manifest");

    // RON files: items are split across multiple .inf files
    assert!(
        out_path.join("sys").join("items_material.ron").exists(),
        "sys/items_material.ron"
    );
    assert!(
        out_path.join("sys").join("objects_stone.ron").exists(),
        "sys/objects_stone.ron"
    );
    assert!(
        out_path.join("sys").join("units.ron").exists(),
        "sys/units.ron"
    );
    assert!(
        out_path.join("sys").join("buildings.ron").exists(),
        "sys/buildings.ron"
    );
    assert!(
        out_path.join("sys").join("combinations_basic.ron").exists(),
        "sys/combinations_basic.ron"
    );

    // Some Lua files from .s2s transpilation
    let lua_count = count_files(out_path, "lua");
    assert!(
        lua_count >= 150,
        "expected >= 150 .lua files, got {lua_count}"
    );

    // GLB models
    let glb_count = count_files(out_path, "glb");
    assert!(
        glb_count >= 300,
        "expected >= 300 .glb files, got {glb_count}"
    );

    // PNG textures
    let png_count = count_files(out_path, "png");
    assert!(
        png_count >= 400,
        "expected >= 400 .png files, got {png_count}"
    );
}

#[test]
fn stranded2_converts_to_s2mod() {
    if !stranded2_data_available() {
        eprintln!("skipped: tests/stranded2/ not available");
        return;
    }

    let out_dir = format!("{}/s2mod", STRANDED2_OUT_DIR);
    let out_path = std::path::Path::new(&out_dir);
    if out_path.exists() {
        std::fs::remove_dir_all(out_path).expect("remove tests/out/s2mod/");
    }
    std::fs::create_dir_all(out_path).expect("create tests/out/s2mod/");

    let s2mod_path = out_path.join("stranded2.s2mod");

    let (stdout, _stderr) = run_convert(STRANDED2_DIR, s2mod_path.to_str().unwrap(), &[])
        .expect("stranded2 conversion to s2mod");

    // Check basic output
    assert!(stdout.contains("Done!"), "done: {stdout}");
    assert!(stdout.contains("inf files processed"), "inf count: {stdout}");

    // .s2mod file exists and is non-trivial
    let meta = std::fs::metadata(&s2mod_path).expect(".s2mod should exist");
    assert!(
        meta.len() > 1_000_000,
        ".s2mod should be > 1 MB, was {} bytes",
        meta.len()
    );
}

#[test]
fn stranded2_manifest_is_valid() {
    if !stranded2_data_available() {
        eprintln!("skipped: tests/stranded2/ not available");
        return;
    }

    let dir_path = format!("{}/dir", STRANDED2_OUT_DIR);
    let dir = std::path::Path::new(&dir_path);

    // If the dir test hasn't run yet, generate output
    if !dir.join("manifest.toml").exists() {
        if dir.exists() {
            std::fs::remove_dir_all(dir).expect("remove tests/out/dir/");
        }
        std::fs::create_dir_all(dir).expect("create tests/out/dir/");
        let (stdout, _stderr) = run_convert(STRANDED2_DIR, &dir_path, &["--dir"])
            .expect("stranded2 conversion for manifest check");
        assert!(stdout.contains("Done!"), "done: {stdout}");
    }

    let manifest_path = dir.join("manifest.toml");
    let content = std::fs::read_to_string(&manifest_path).expect("read manifest.toml");
    let value: toml::Value = toml::from_str(&content).expect("valid TOML");

    // [pack] section
    assert!(value.get("pack").is_some(), "[pack]");
    assert!(value["pack"].get("name").is_some(), "pack.name");

    // [registry] section with all expected categories
    let registry = value.get("registry").expect("[registry]");
    for cat in &["items", "objects", "units", "buildings", "combinations", "infos", "groups", "states", "game"] {
        let arr = registry.get(*cat).and_then(|v| v.as_array());
        assert!(arr.is_some(), "registry.{} should be an array", cat);
        assert!(!arr.unwrap().is_empty(), "registry.{} non-empty", cat);
    }
}

// ═══════════════════════════════════════════════════════════════
//  Ignored tests  (require original game data)
// ═══════════════════════════════════════════════════════════════

#[test]
#[ignore]
fn original_conversion_all_outputs_exist() {
    let tmp = tempfile::tempdir().unwrap();
    let output_dir = tmp.path().join("output_dir");

    let (_stdout, _stderr) = run_convert(ORIGINAL_MOD_DIR, output_dir.to_str().unwrap(), &["--dir"])
        .expect("conversion with --dir");

    // Manifest
    assert!(output_dir.join("manifest.toml").exists(), "manifest.toml");

    // RON files are 1:1 alongside .inf files in sys/
    for fname in &["items", "items_edible", "items_material", "items_stuff",
                   "items_tools", "items_weapons", "objects", "objects_buildings",
                   "objects_bushes", "objects_flowers", "objects_gras", "objects_palms",
                   "objects_stone", "objects_stuff", "objects_trees", "units",
                   "buildings", "combinations", "combinations_actions", "combinations_ammo",
                   "combinations_basic", "combinations_potions", "combinations_stuff",
                   "combinations_tools", "combinations_weapons"] {
        let path = output_dir.join("sys").join(format!("{fname}.ron"));
        assert!(path.exists(), "sys/{fname}.ron");
    }

    // .lua scripts at original .s2s locations
    let sys_lua = count_files(&output_dir.join("sys").join("scripts"), "lua");
    assert!(sys_lua >= 3, "expected >= 3 .lua in sys/scripts, got {sys_lua}");
    let maps_lua = count_files(&output_dir.join("maps"), "lua");
    assert!(
        maps_lua >= 2,
        "expected >= 2 .lua in maps/, got {maps_lua}"
    );

    // GLB models in gfx/ (1:1 structure — same path as .b3d)
    let glb_count = count_files(&output_dir.join("gfx"), "glb");
    assert!(glb_count >= 300, "expected >= 300 .glb files, got {glb_count}");

    // PNG textures in gfx/ (1:1 structure — same path as .bmp)
    let png_count = count_files(&output_dir.join("gfx"), "png");
    assert!(png_count >= 400, "expected >= 400 .png files, got {png_count}");
}

#[test]
#[ignore]
fn original_registry_ron_is_valid() {
    let tmp = tempfile::tempdir().unwrap();
    let output_dir = tmp.path().join("registry_test");

    let (_stdout, _stderr) = run_convert(ORIGINAL_MOD_DIR, output_dir.to_str().unwrap(), &["--dir"])
        .expect("conversion");

    for fname in &["items", "items_edible", "items_material", "items_stuff",
                   "items_tools", "items_weapons", "objects", "objects_buildings",
                   "objects_bushes", "objects_flowers", "objects_gras", "objects_palms",
                   "objects_stone", "objects_stuff", "objects_trees", "units",
                   "buildings", "combinations", "combinations_actions", "combinations_ammo",
                   "combinations_basic", "combinations_potions", "combinations_stuff",
                   "combinations_tools", "combinations_weapons"] {
        let path = output_dir.join("sys").join(format!("{fname}.ron"));
        let content = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("read {fname}"));
        let value: ron::Value =
            ron::from_str(&content).unwrap_or_else(|e| panic!("{fname} valid RON: {e}"));
        assert!(matches!(value, ron::Value::Seq(_)), "{fname} should be Seq");
    }
}

#[test]
#[ignore]
fn original_scripts_contain_valid_lua() {
    let tmp = tempfile::tempdir().unwrap();
    let output_dir = tmp.path().join("lua_test");

    let (_stdout, _stderr) = run_convert(ORIGINAL_MOD_DIR, output_dir.to_str().unwrap(), &["--dir"])
        .expect("conversion");

    let mut lua_files = vec![];
    collect_files_rec(&output_dir.join("sys").join("scripts"), "lua", &mut lua_files);
    collect_files_rec(&output_dir.join("maps"), "lua", &mut lua_files);
    lua_files.sort();
    assert!(!lua_files.is_empty(), "at least one .lua file");
    assert!(
        lua_files.len() >= 5,
        "expected >= 5 lua, got {}",
        lua_files.len()
    );

    let content = std::fs::read_to_string(&lua_files[0]).expect("read first lua");
    assert!(content.contains("core_api"), "core_api reference");
    assert!(content.contains("-- Generated by"), "header comment");
}

#[test]
#[ignore]
fn original_glb_files_are_valid() {
    use std::io::Read;

    let tmp = tempfile::tempdir().unwrap();
    let output_dir = tmp.path().join("glb_test");

    let (_stdout, _stderr) = run_convert(ORIGINAL_MOD_DIR, output_dir.to_str().unwrap(), &["--dir"])
        .expect("conversion");

    let mut glb_files = vec![];
    collect_files_rec(&output_dir.join("gfx"), "glb", &mut glb_files);
    assert!(
        glb_files.len() >= 300,
        "expected >= 300 glb, got {}",
        glb_files.len()
    );

    let mut header = [0u8; 4];
    std::fs::File::open(&glb_files[0])
        .expect("open first glb")
        .read_exact(&mut header)
        .expect("read glb header");
    assert_eq!(&header, b"glTF", "first glb should have glTF magic header");
}

#[test]
#[ignore]
fn original_png_files_are_valid() {
    let tmp = tempfile::tempdir().unwrap();
    let output_dir = tmp.path().join("png_test");

    let (_stdout, _stderr) = run_convert(ORIGINAL_MOD_DIR, output_dir.to_str().unwrap(), &["--dir"])
        .expect("conversion");

    let mut png_files = vec![];
    collect_files_rec(&output_dir.join("gfx"), "png", &mut png_files);
    assert!(
        png_files.len() >= 400,
        "expected >= 400 png, got {}",
        png_files.len()
    );

    let img = image::ImageReader::open(&png_files[0])
        .expect("open first png")
        .decode()
        .expect("decode first png");
    assert!(img.width() > 0 && img.height() > 0, "valid image dimensions");
}

#[test]
#[ignore]
fn original_manifest_has_content() {
    let tmp = tempfile::tempdir().unwrap();
    let output_dir = tmp.path().join("manifest_test");

    let (_stdout, _stderr) = run_convert(ORIGINAL_MOD_DIR, output_dir.to_str().unwrap(), &["--dir"])
        .expect("conversion");

    let content = std::fs::read_to_string(output_dir.join("manifest.toml"))
        .expect("read manifest.toml");

    let value: toml::Value = toml::from_str(&content).expect("valid TOML");
    assert!(value.get("pack").is_some(), "[pack]");
    assert!(value.get("registry").is_some(), "[registry]");

    let items = value["registry"]["items"].as_array();
    assert!(items.is_some(), "registry.items");
    assert!(!items.unwrap().is_empty(), "registry.items non-empty");
    assert!(value["pack"].get("name").is_some(), "pack.name");
}

#[test]
#[ignore]
fn original_skip_flags_work() {
    let tmp = tempfile::tempdir().unwrap();
    let output_dir = tmp.path().join("skip_test");

    let (stdout, _stderr) = run_convert(
        ORIGINAL_MOD_DIR,
        output_dir.to_str().unwrap(),
        &["--dir", "--skip-models", "--skip-textures"],
    )
    .expect("skip flags");

    assert!(
        stdout.contains("skipped"),
        "should mention skipping: {stdout}"
    );
    assert!(
        output_dir.join("sys").join("items.ron").exists(),
        "sys/items.ron should still exist"
    );

    // No .glb or .png files when models/textures skipped
    let glb_count = count_files(&output_dir.join("gfx"), "glb");
    assert_eq!(glb_count, 0, "no .glb files when models skipped");
    let png_count = count_files(&output_dir.join("gfx"), "png");
    assert_eq!(png_count, 0, "no .png files when textures skipped");
}

// ═══════════════════════════════════════════════════════════════
//  Helpers
// ═══════════════════════════════════════════════════════════════

fn count_files(dir: &Path, ext: &str) -> usize {
    if !dir.exists() {
        return 0;
    }
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == ext))
        .count()
}

fn collect_files_rec(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    if !dir.exists() {
        return;
    }
    for entry in walkdir::WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        if entry.path().extension().is_some_and(|x| x == ext) {
            out.push(entry.path().to_owned());
        }
    }
}
