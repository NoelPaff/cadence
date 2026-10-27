pub mod audio;
pub mod cli;
pub mod config;
pub mod desktop;
pub mod media;
pub mod model;
pub mod ui;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
