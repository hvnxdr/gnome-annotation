//! The install script, exercised with a temporary HOME and XDG tree plus a fake gsettings that
//! records every call and keeps the custom-keybindings array.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

mod support;

use support::TempDir;

const OUR_PATH: &str =
    "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/gnome-annotation/";
const CUSTOM0: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/";

struct Installer {
    _dir: TempDir,
    home: PathBuf,
    data: PathBuf,
    config: PathBuf,
    bin: PathBuf,
    fake: PathBuf,
    source: PathBuf,
}

impl Installer {
    fn new(name: &str, initial: &str) -> Self {
        Self::with_bin_dir(name, initial, "bin")
    }

    fn with_bin_dir(name: &str, initial: &str, bin_dir: &str) -> Self {
        let dir = TempDir::new(name);
        let home = dir.path().join("home");
        let data = dir.path().join("data");
        let config = dir.path().join("config");
        let bin = dir.path().join(bin_dir);
        let fake_dir = dir.path().join("fake-gsettings");
        for path in [&home, &data, &config, &bin, &fake_dir] {
            fs::create_dir_all(path).expect("create fixture directory");
        }
        fs::write(fake_dir.join("initial"), format!("{initial}\n")).unwrap();
        let fake = fake_dir.join("gsettings");
        fs::write(&fake, FAKE_GSETTINGS).unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        let source = dir.path().join("built-gnome-annotation");
        fs::write(&source, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
        Self {
            _dir: dir,
            home,
            data,
            config,
            bin,
            fake,
            source,
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/install.sh");
        Command::new("bash")
            .arg(script)
            .args(args)
            .env("HOME", &self.home)
            .env("XDG_DATA_HOME", &self.data)
            .env("XDG_CONFIG_HOME", &self.config)
            .env("XDG_BIN_HOME", &self.bin)
            .env("GSETTINGS", &self.fake)
            .env("GNOME_ANNOTATION_BIN", &self.source)
            .env("FAKE_DIR", self.fake.parent().unwrap())
            .output()
            .expect("run the install script")
    }

    fn bin_path(&self) -> PathBuf {
        self.bin.join("gnome-annotation")
    }

    fn desktop(&self) -> PathBuf {
        self.data
            .join("applications/dev.hvnxdr.GnomeAnnotation.desktop")
    }

    fn autostart(&self) -> PathBuf {
        self.config
            .join("autostart/dev.hvnxdr.GnomeAnnotation.desktop")
    }

    fn current(&self) -> String {
        fs::read_to_string(self.fake.parent().unwrap().join("current")).unwrap_or_default()
    }

    fn log(&self) -> String {
        fs::read_to_string(self.fake.parent().unwrap().join("log")).unwrap_or_default()
    }
}

const FAKE_GSETTINGS: &str = r#"#!/usr/bin/env bash
printf '%s\n' "$*" >> "$FAKE_DIR/log"
case "$1" in
  get)
    case "$2:$3" in
      org.gnome.settings-daemon.plugins.media-keys:custom-keybindings)
        if [ -f "$FAKE_DIR/current" ]; then
          cat "$FAKE_DIR/current"
        else
          cat "$FAKE_DIR/initial"
        fi
        ;;
      *)
        printf '@as []\n'
        ;;
    esac
    ;;
  set)
    case "$2:$3" in
      org.gnome.settings-daemon.plugins.media-keys:custom-keybindings)
        printf '%s\n' "$4" > "$FAKE_DIR/current"
        ;;
    esac
    ;;
esac
exit 0
"#;

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "installer failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn install_writes_the_binary_entries_and_keybinding() {
    let fixture = Installer::new("install-basic", &format!("['{CUSTOM0}']"));
    let output = fixture.run(&[]);
    assert_success(&output);

    let bin = fixture.bin_path();
    let mode = fs::metadata(&bin).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o755, "the binary is installed executable");

    let desktop = fs::read_to_string(fixture.desktop()).expect("desktop entry");
    assert!(
        desktop.contains(&format!("Exec={}", bin.display())),
        "the desktop entry runs the installed binary: {desktop}"
    );
    assert!(desktop.contains("Name=gnome-annotation"));
    assert!(desktop.contains("Comment=Annotate a GNOME screenshot"));
    assert!(desktop.contains("StartupWMClass=dev.hvnxdr.GnomeAnnotation"));
    assert!(desktop.contains("Categories=Graphics"));

    let autostart = fs::read_to_string(fixture.autostart()).expect("autostart entry");
    assert!(
        autostart.contains(&format!("Exec={} --daemon", bin.display())),
        "the autostart entry runs this binary: {autostart}"
    );
    assert!(autostart.contains("X-GNOME-Autostart-enabled=true"));

    assert_eq!(
        fixture.current().trim(),
        format!("['{CUSTOM0}', '{OUR_PATH}']"),
        "the existing keybinding keeps its place and ours is appended"
    );

    let log = fixture.log();
    assert!(log.contains("binding Print"), "{log}");
    assert!(
        log.contains(&format!(
            "set org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:{OUR_PATH} command {} --trigger",
            bin.display()
        )),
        "the Print binding runs the trigger client: {log}"
    );
}

#[test]
fn a_bin_directory_with_a_space_keeps_the_print_command_quoted() {
    let fixture = Installer::with_bin_dir("install-space", &format!("['{CUSTOM0}']"), "bin dir");
    assert_success(&fixture.run(&[]));

    let bin = fixture.bin_path();
    assert!(
        bin.display().to_string().contains(' '),
        "the fixture bin path has a space: {}",
        bin.display()
    );
    let desktop = fs::read_to_string(fixture.desktop()).expect("desktop entry");
    assert!(
        desktop.contains(&format!("Exec=\"{}\"", bin.display())),
        "the desktop entry quotes the binary path: {desktop}"
    );
    let log = fixture.log();
    assert!(
        log.contains(&format!("command \"{}\" --trigger", bin.display())),
        "the Print command quotes the binary path: {log}"
    );
    assert!(
        !log.contains(&format!("command {} --trigger", bin.display())),
        "the unquoted path never reaches the keybinding: {log}"
    );
}

#[test]
fn installing_twice_does_not_duplicate_the_keybinding() {
    let fixture = Installer::new("install-twice", &format!("['{CUSTOM0}']"));
    assert_success(&fixture.run(&[]));
    assert_success(&fixture.run(&[]));

    let current = fixture.current();
    assert_eq!(
        current.matches(OUR_PATH).count(),
        1,
        "one entry after two installs: {current}"
    );
    assert_eq!(
        current.trim(),
        format!("['{CUSTOM0}', '{OUR_PATH}']"),
        "custom0 is still first"
    );
}

#[test]
fn install_works_with_an_empty_keybinding_array() {
    let fixture = Installer::new("install-empty", "@as []");
    assert_success(&fixture.run(&[]));
    assert_eq!(fixture.current().trim(), format!("['{OUR_PATH}']"));
}

#[test]
fn uninstall_removes_only_what_was_installed() {
    let fixture = Installer::new("uninstall", &format!("['{CUSTOM0}']"));
    assert_success(&fixture.run(&[]));
    let other = fixture.bin.join("other-tool");
    fs::write(&other, "keep me").unwrap();

    assert_success(&fixture.run(&["--uninstall"]));

    assert!(!fixture.bin_path().exists(), "the installed binary is gone");
    assert!(!fixture.desktop().exists(), "the desktop entry is gone");
    assert!(!fixture.autostart().exists(), "the autostart entry is gone");
    assert_eq!(
        fs::read_to_string(&other).unwrap(),
        "keep me",
        "an unrelated binary in the same directory stays"
    );
    assert_eq!(
        fixture.current().trim(),
        format!("['{CUSTOM0}']"),
        "the unrelated keybinding stays and ours is removed"
    );
    let log = fixture.log();
    assert!(
        log.contains(&format!(
            "reset org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:{OUR_PATH} name"
        )),
        "the installer resets only our keybinding keys: {log}"
    );
}

#[test]
fn uninstall_is_harmless_when_nothing_was_installed() {
    let fixture = Installer::new("uninstall-fresh", &format!("['{CUSTOM0}']"));
    let output = fixture.run(&["--uninstall"]);
    assert_success(&output);
    assert!(
        fixture.current().is_empty(),
        "no write reaches the keybinding array when ours is absent"
    );
    assert!(
        !fixture
            .log()
            .contains("set org.gnome.settings-daemon.plugins.media-keys custom-keybindings"),
        "the array was left untouched: {}",
        fixture.log()
    );
}
