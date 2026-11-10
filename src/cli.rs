//! Command line modes. No arguments asks the resident daemon for a capture; `--daemon` is the
//! resident daemon itself; `--capture` is the one-shot portal flow for manual testing.

use std::path::PathBuf;

use crate::capture::{ScreenshotPortal, capture_path};

pub const USAGE: &str = "\
gnome-annotation — annotate a screenshot, copy it, and close

Usage:
  gnome-annotation                     Ask the daemon to capture (starts it if needed)
  gnome-annotation --trigger           Same as no arguments
  gnome-annotation --daemon            Run the resident daemon (started at login)
  gnome-annotation --editor <path.png> Open the editor on an image
  gnome-annotation --capture           One-shot capture with GNOME's UI, then edit
  gnome-annotation --help              Show this help

The installer binds Print to the trigger and adds the daemon to autostart.

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
    Daemon,
    Trigger,
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
        [] => Ok(CliCommand::Trigger),
        [flag] if flag == "--help" || flag == "-h" => Ok(CliCommand::Help),
        [flag] if flag == "--capture" => Ok(CliCommand::Capture),
        [flag] if flag == "--daemon" => Ok(CliCommand::Daemon),
        [flag] if flag == "--trigger" => Ok(CliCommand::Trigger),
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

/// Run one command. `launch` opens the one-shot editor and blocks until it closes; `daemon`
/// and `trigger` start the resident side or ask it for a capture.
pub fn run_with(
    args: impl IntoIterator<Item = String>,
    portal: &dyn ScreenshotPortal,
    launch: impl FnOnce(PathBuf) -> Result<(), String>,
    daemon: impl FnOnce() -> Result<(), String>,
    trigger: impl FnOnce() -> Result<(), String>,
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
        Ok(CliCommand::Daemon) => match daemon() {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("gnome-annotation: {error}");
                1
            }
        },
        Ok(CliCommand::Trigger) => match trigger() {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("gnome-annotation: {error}");
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
