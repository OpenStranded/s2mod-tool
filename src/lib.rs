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

//! Convert original Stranded II mods to `.s2mod` Content Pack format.
//!
//! # Library
//!
//! Use the public modules to access individual conversion steps:
//!
//! - [`convert`] — file format converters (`.inf→.ron`, `.b3d→.glb`, `.bmp→.png`, `.bmpf→.fnt`)
//! - [`registry`] — `.ron` filename → registry key derivation
//! - [`scanner`] — s2s cross-reference scanning and classification
//! - [`script`] — s2s script transpilation, dialogue parsing, sectioned files
//! - [`util`] — file I/O, path helpers, RON serialisation
//!
//! # CLI
//!
//! Enable the default `cli` feature to build the `openstranded-s2mod-tool` binary:
//!
//! ```bash
//! cargo install openstranded-s2mod-tool
//! openstranded-s2mod-tool --input /path/to/Stranded\ II --output ./content.s2mod
//! ```

pub mod convert;
pub mod registry;
pub mod scanner;
pub mod script;
pub mod util;
