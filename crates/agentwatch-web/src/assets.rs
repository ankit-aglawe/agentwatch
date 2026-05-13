//! Embedded static assets. preact + chart.js bundle (~60KB total).
//!
//! `debug-embed = false` (set in workspace Cargo.toml) means dev mode reads
//! files from disk for live-reload, release mode embeds them in the binary.

use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets/"]
pub struct Assets;
