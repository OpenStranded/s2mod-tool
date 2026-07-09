# openstranded-convert

Convert original Stranded II mods to the `.s2mod` Content Pack format.

## Usage

```bash
# Install
cargo install openstranded-convert

# Convert a mod directory to .s2mod archive
openstranded-convert --input "/path/to/Stranded II" --output ./stranded2.s2mod

# Convert to directory instead of zip
openstranded-convert --input "/path/to/Stranded II" --output ./out --dir

# Skip model/texture conversion (text-only)
openstranded-convert --input "/path/to/Stranded II" --output ./text.s2mod --skip-models --skip-textures
```

## Pipeline

1. **Walk** — scan input directory, classify files by extension
2. **Parse .inf** — generic `key=value` parser → `.ron` + embedded scripts → `.lua`
3. **Scan references** — find `dialogue`/`msgbox`/`addscript` etc. across all scripts
4. **Transpile .s2s** — s2s → Lua (with reference-based classification)
5. **Convert .b3d** → `.glb` (3D models)
6. **Convert .bmp** → `.png` (magenta → transparent colour key)
7. **Convert .bmpf** → `.fnt` + `.png` (bitmap fonts)
8. **Update paths** — `.b3d`→`.glb`, `.bmp`→`.png` references in `.ron` files
9. **Generate manifest.toml** — `[registry]` section with domain→`.ron` mappings
10. **Pack** — `.s2mod` (zip) or directory

## Library

Use as a Rust library:

```toml
[dependencies]
openstranded-convert = { version = "0.2", default-features = false }
```

```rust
use openstranded_convert::convert::convert_bmp_to_png;
use openstranded_convert::scanner::{build_reference_map, S2sRefType};
use openstranded_convert::util::parse_inf_file;
```

## License

GPL-3.0-or-later
