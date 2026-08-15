use std::ffi::OsString;

use anyhow::Result;
use clap::Parser;

use autotun::cli::Cli;

fn main() -> Result<()> {
    // Destination is optional for the GUI; reuse the shared CLI by forcing --gui.
    let cli = Cli::parse_from(std::env::args_os().chain([OsString::from("--gui")]));
    autotun::gui::run(&cli)
}
