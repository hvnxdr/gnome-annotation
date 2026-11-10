//! Command line modes. `--capture` is the one-shot capture flow; the resident daemon is
//! HVN-30.

use std::path::PathBuf;

use crate::capture::{ScreenshotPortal, capture_path};

pub const USAGE: &str = "\
gnome-annotation — annotate a screenshot, copy it, and close

Usage:
  gnome-annotation --editor <path.png>   Open the editor on an image
  gnome-annotation --capture             Capture with GNOME's screenshot UI, then edit
  gnome-annotation --help                Show this help

In the editor:
  V  select tool
  A  arrow tool
  T  text tool
  Enter  copy the annotated PNG at source resolution and close
  Esc    close without writing anything

While typing: Enter commits the label, Shift+Enter adds a line, Esc discards it.
";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CliCommand {
    Help,
    Editor(PathBuf),
    Capture,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliError(pub String);

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CliError {}

pub fn parse(args: &[String]) -> Result<CliCommand, CliError> {
    match args {
        [] => Ok(CliCommand::Help),
        [flag] if flag == "--help" || flag == "-h" => Ok(CliCommand::Help),
        [flag] if flag == "--capture" => Ok(CliCommand::Capture),
        [flag, path] if flag == "--editor" => {
            if path.is_empty() {
                Err(CliError("--editor needs an image path".into()))
            } else {
                Ok(CliCommand::Editor(PathBuf::from(path)))
            }
        }
        [flag] if flag == "--editor" => Err(CliError("--editor needs an image path".into())),
        _ => Err(CliError(format!(
            "unrecognised arguments: {}",
            args.join(" ")
        ))),
    }
}

/// Run one command. `launch` opens the editor and blocks until it closes.
pub fn run_with(
    args: impl IntoIterator<Item = String>,
    portal: &dyn ScreenshotPortal,
    launch: impl FnOnce(PathBuf) -> Result<(), String>,
) -> i32 {
    let args: Vec<String> = args.into_iter().collect();
    match parse(&args) {
        Ok(CliCommand::Help) => {
            print!("{USAGE}");
            0
        }
        Ok(CliCommand::Editor(path)) => match launch(path) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("gnome-annotation: {error}");
                1
            }
        },
        Ok(CliCommand::Capture) => match capture_path(portal) {
            Ok(Some(path)) => match launch(path) {
                Ok(()) => 0,
                Err(error) => {
                    eprintln!("gnome-annotation: {error}");
                    1
                }
            },
            Ok(None) => 0,
            Err(error) => {
                eprintln!("gnome-annotation: capture failed: {error}");
                1
            }
        },
        Err(error) => {
            eprintln!("gnome-annotation: {error}");
            eprint!("{USAGE}");
            2
        }
    }
}
