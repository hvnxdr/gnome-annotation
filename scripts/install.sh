#!/usr/bin/env bash
# Install or remove gnome-annotation for the current user: the binary, a desktop entry, and an
# autostart entry for the daemon. GNOME keybinding settings are never read or changed.
#
#   scripts/install.sh              install
#   scripts/install.sh --uninstall  remove exactly what install added
#   scripts/install.sh --help       this text
#
# HOME, XDG_BIN_HOME, XDG_DATA_HOME and XDG_CONFIG_HOME are respected. The test suite points
# GNOME_ANNOTATION_BIN at a prepared file.

set -euo pipefail

APP_ID=dev.hvnxdr.GnomeAnnotation
BIN_NAME=gnome-annotation
BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}"

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd -- "$SCRIPT_DIR/.." && pwd)

usage() {
    cat <<EOF
Usage: scripts/install.sh [--uninstall]

Installs $BIN_NAME to $BIN_DIR, writes
$DATA_DIR/applications/$APP_ID.desktop
and $CONFIG_DIR/autostart/$APP_ID.desktop.

--uninstall removes only those files.
EOF
}

warn() {
    printf 'install: %s\n' "$*" >&2
}

# Quote an Exec value that contains spaces, as the desktop entry spec asks.
desktop_exec() {
    case "$1" in
        *[[:space:]]*) printf '"%s"' "$1" ;;
        *) printf '%s' "$1" ;;
    esac
}

install_binary() {
    local source=${GNOME_ANNOTATION_BIN:-}
    if [ -z "$source" ]; then
        source="$ROOT/target/release/$BIN_NAME"
        if [ ! -x "$source" ]; then
            printf 'install: building the release binary\n'
            (cd "$ROOT" && cargo build --release)
        fi
    fi
    if [ ! -x "$source" ]; then
        warn "no executable found at $source"
        exit 1
    fi
    mkdir -p "$BIN_DIR"
    install -m 755 "$source" "$BIN_DIR/$BIN_NAME"
    printf 'install: %s\n' "$BIN_DIR/$BIN_NAME"
}

install_desktop_entry() {
    local path="$DATA_DIR/applications/$APP_ID.desktop"
    mkdir -p "$(dirname -- "$path")"
    cat >"$path" <<EOF
[Desktop Entry]
Type=Application
Name=$BIN_NAME
Comment=Annotate a GNOME screenshot
Exec=$(desktop_exec "$BIN_DIR/$BIN_NAME")
Terminal=false
StartupWMClass=$APP_ID
Categories=Graphics;Utility;
EOF
    printf 'install: %s\n' "$path"
}

install_autostart_entry() {
    local path="$CONFIG_DIR/autostart/$APP_ID.desktop"
    mkdir -p "$(dirname -- "$path")"
    cat >"$path" <<EOF
[Desktop Entry]
Type=Application
Name=$BIN_NAME
Comment=Keep the annotation daemon running
Exec=$(desktop_exec "$BIN_DIR/$BIN_NAME") --daemon
Terminal=false
X-GNOME-Autostart-enabled=true
EOF
    printf 'install: %s\n' "$path"
}

install_main() {
    install_binary
    install_desktop_entry
    install_autostart_entry
    printf 'install: done\n'
}

uninstall_main() {
    rm -f "$BIN_DIR/$BIN_NAME"
    rm -f "$DATA_DIR/applications/$APP_ID.desktop"
    rm -f "$CONFIG_DIR/autostart/$APP_ID.desktop"
    printf 'install: removed the binary and the desktop entries\n'
}

mode=install
while [ $# -gt 0 ]; do
    case "$1" in
        --uninstall) mode=uninstall ;;
        --help | -h)
            usage
            exit 0
            ;;
        *)
            usage >&2
            exit 2
            ;;
    esac
    shift
done

if [ "$mode" = uninstall ]; then
    uninstall_main
else
    install_main
fi
