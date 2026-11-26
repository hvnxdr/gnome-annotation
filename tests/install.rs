//! The install script, exercised with a temporary HOME and XDG tree plus a fake gsettings first
//! on PATH that records every call. Install and uninstall never touch GNOME keybinding settings.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

mod support;

use support::TempDir;

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
    fn new(name: &str) -> Self {
        Self::with_bin_dir(name, "bin")
    }

    fn with_bin_dir(name: &str, bin_dir: &str) -> Self {
        let dir = TempDir::new(name);
        let home = dir.path().join("home");
        let data = dir.path().join("data");
        let config = dir.path().join("config");
        let bin = dir.path().join(bin_dir);
        let fake_dir = dir.path().join("fake-gsettings");
        for path in [&home, &data, &config, &bin, &fake_dir] {
            fs::create_dir_all(path).expect("create fixture directory");
        }
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
            .env("PATH", path_with(self.fake.parent().unwrap()))
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

    fn log(&self) -> String {
        fs::read_to_string(self.fake.parent().unwrap().join("log")).unwrap_or_default()
    }
}

/// `PATH` with the fake gsettings directory first, so any `gsettings` call reaches the fake.
fn path_with(dir: &Path) -> std::ffi::OsString {
    let mut paths = vec![dir.to_path_buf()];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    std::env::join_paths(paths).expect("join PATH")
}

const FAKE_GSETTINGS: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$FAKE_DIR/log"
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
fn install_writes_the_binary_and_entries_without_touching_keybindings() {
    let fixture = Installer::new("install-basic");
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

    assert_eq!(fixture.log(), "", "install never runs gsettings");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("Print"),
        "install does not mention Print: {stdout}"
    );
}

#[test]
fn a_bin_directory_with_a_space_keeps_the_exec_lines_quoted() {
    let fixture = Installer::with_bin_dir("install-space", "bin dir");
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
    let autostart = fs::read_to_string(fixture.autostart()).expect("autostart entry");
    assert!(
        autostart.contains(&format!("Exec=\"{}\" --daemon", bin.display())),
        "the autostart entry quotes the binary path: {autostart}"
    );
}

#[test]
fn uninstall_removes_only_what_was_installed() {
    let fixture = Installer::new("uninstall");
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
    assert_eq!(fixture.log(), "", "uninstall never runs gsettings");
}

#[test]
fn uninstall_is_harmless_when_nothing_was_installed() {
    let fixture = Installer::new("uninstall-fresh");
    assert_success(&fixture.run(&["--uninstall"]));
    assert_eq!(fixture.log(), "", "uninstall never runs gsettings");
}
