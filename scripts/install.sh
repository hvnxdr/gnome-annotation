#!/usr/bin/env bash
# Install or remove gnome-annotation for the current user: the binary, a desktop entry, an
# autostart entry for the daemon, and a Print custom keybinding.
#
#   scripts/install.sh              install
#   scripts/install.sh --uninstall  remove exactly what install added
#   scripts/install.sh --help       this text
#
# HOME, XDG_BIN_HOME, XDG_DATA_HOME, XDG_CONFIG_HOME and GSETTINGS are respected. The test
# suite points GSETTINGS at a fake and GNOME_ANNOTATION_BIN at a prepared file.

set -euo pipefail

APP_ID=dev.hvnxdr.GnomeAnnotation
BIN_NAME=gnome-annotation
BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}"
GSETTINGS="${GSETTINGS:-gsettings}"

MEDIA_SCHEMA=org.gnome.settings-daemon.plugins.media-keys
MEDIA_KEY=custom-keybindings
KEYBINDING_PATH=/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/gnome-annotation/
KEYBINDING_SCHEMA="org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:$KEYBINDING_PATH"

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd -- "$SCRIPT_DIR/.." && pwd)

usage() {
    cat <<EOF
Usage: scripts/install.sh [--uninstall]

Installs $BIN_NAME to $BIN_DIR, writes
$DATA_DIR/applications/$APP_ID.desktop and
$CONFIG_DIR/autostart/$APP_ID.desktop, and binds Print to the trigger.

--uninstall removes only those files and that keybinding.
EOF
}

warn() {
    printf 'install: %s\n' "$*" >&2
}

trim() {
    local value=$1
    value="${value#"${value%%[![:space:]]*}"}"
    value="${value%"${value##*[![:space:]]}"}"
    printf '%s' "$value"
}

have_gsettings() {
    command -v "$GSETTINGS" >/dev/null 2>&1
}

# Keep this in sync with the array gsettings prints. Entries are quoted object paths.
read_keybindings() {
    ENTRIES=()
    local raw item
    raw=$("$GSETTINGS" get "$MEDIA_SCHEMA" "$MEDIA_KEY" 2>/dev/null) || raw='@as []'
    raw=$(trim "$raw")
    case "$raw" in
        '' | '[]' | '@as []' | '@as[]') return 0 ;;
    esac
    raw=${raw#[}
    raw=${raw%]}
    local IFS=','
    for item in $raw; do
        item=$(trim "$item")
        item=${item#\'}
        item=${item%\'}
        item=${item#\"}
        item=${item%\"}
        [ -n "$item" ] && ENTRIES+=("$item")
    done
}

has_entry() {
    local needle=$1 entry
    for entry in "${ENTRIES[@]}"; do
        [ "$entry" = "$needle" ] && return 0
    done
    return 1
}

format_keybindings() {
    local out='' entry
    for entry in "${ENTRIES[@]}"; do
        if [ -z "$out" ]; then
            out="'$entry'"
        else
            out="$out, '$entry'"
        fi
    done
    printf '[%s]' "$out"
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
Exec=$BIN_NAME
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
Comment=Listen for the Print key and open GNOME's screenshot UI
Exec=$(desktop_exec "$BIN_DIR/$BIN_NAME") --daemon
Terminal=false
X-GNOME-Autostart-enabled=true
EOF
    printf 'install: %s\n' "$path"
}

install_keybinding() {
    if ! have_gsettings; then
        warn "gsettings is not available; Print was not bound"
        return 0
    fi
    read_keybindings
    if ! has_entry "$KEYBINDING_PATH"; then
        ENTRIES+=("$KEYBINDING_PATH")
        "$GSETTINGS" set "$MEDIA_SCHEMA" "$MEDIA_KEY" "$(format_keybindings)"
    fi
    "$GSETTINGS" set "$KEYBINDING_SCHEMA" name "$BIN_NAME"
    "$GSETTINGS" set "$KEYBINDING_SCHEMA" command "$BIN_DIR/$BIN_NAME --trigger"
    "$GSETTINGS" set "$KEYBINDING_SCHEMA" binding "Print"
    printf 'install: Print -> %s --trigger\n' "$BIN_DIR/$BIN_NAME"
}

remove_keybinding() {
    if ! have_gsettings; then
        warn "gsettings is not available; the Print binding was left alone"
        return 0
    fi
    read_keybindings
    if has_entry "$KEYBINDING_PATH"; then
        local kept=()
        for entry in "${ENTRIES[@]}"; do
            [ "$entry" = "$KEYBINDING_PATH" ] || kept+=("$entry")
        done
        ENTRIES=("${kept[@]}")
        "$GSETTINGS" set "$MEDIA_SCHEMA" "$MEDIA_KEY" "$(format_keybindings)"
    fi
    "$GSETTINGS" reset "$KEYBINDING_SCHEMA" name 2>/dev/null || true
    "$GSETTINGS" reset "$KEYBINDING_SCHEMA" command 2>/dev/null || true
    "$GSETTINGS" reset "$KEYBINDING_SCHEMA" binding 2>/dev/null || true
    printf 'install: removed the Print binding\n'
}

install_main() {
    install_binary
    install_desktop_entry
    install_autostart_entry
    install_keybinding
    printf 'install: done. Press Print to capture.\n'
}

uninstall_main() {
    remove_keybinding
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
