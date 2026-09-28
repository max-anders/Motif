#!/usr/bin/env bash
set -euo pipefail

# ship-linux (Motif)
# Build release binary + install under ~/.local/opt + write a .desktop launcher.
#
# Usage:
#   ./scripts/ship-linux.sh [options]
#   ship-linux --project /path/to/motif
#
# Options:
#   --project <dir>     Project directory (default: parent of scripts/)
#   --id <app-id>       Desktop file basename (default: motif)
#   --name <label>      Launcher display name (default: Motif)
#   --comment <text>    Desktop comment
#   --icon <path>       Icon= line in the .desktop file
#   --wm-class <class>  StartupWMClass (default: motif)
#   --data-dir <dir>    Path= in .desktop (default: project dir; holds settings.json)
#   --restart           Kill running install and start the new binary
#   --no-restart        Skip restart even if .ship-linux has restart=true
#
# Project file .ship-linux (key=value, # comments ok):
#   id=motif
#   name=Motif
#   comment=Music sketchpad
#   icon=/path/to/icon.png
#   wm-class=motif
#   data-dir=/home/you/dev/Rust/motif
#   restart=true

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

APP_ID=""
APP_NAME=""
APP_COMMENT=""
ICON_PATH=""
WM_CLASS=""
DATA_DIR=""
RESTART=""

_load_config() {
  local cfg="$1/.ship-linux"
  [[ -f "$cfg" ]] || return 0
  while IFS='=' read -r key val; do
    [[ "$key" =~ ^[[:space:]]*# ]] && continue
    [[ -z "${key// }" ]] && continue
    key="${key// /}"
    val="${val%%#*}"
    val="${val#"${val%%[![:space:]]*}"}"
    val="${val%"${val##*[![:space:]]}"}"
    case "$key" in
      id|app-id) [[ -z "$APP_ID" ]] && APP_ID="$val" ;;
      name) [[ -z "$APP_NAME" ]] && APP_NAME="$val" ;;
      comment) [[ -z "$APP_COMMENT" ]] && APP_COMMENT="$val" ;;
      wm-class|wmclass) [[ -z "$WM_CLASS" ]] && WM_CLASS="$val" ;;
      data-dir|datadir) [[ -z "$DATA_DIR" ]] && DATA_DIR="$val" ;;
      restart)
        if [[ -z "$RESTART" ]]; then
          case "${val,,}" in
            1|true|yes|on) RESTART="true" ;;
            0|false|no|off) RESTART="false" ;;
          esac
        fi
        ;;
      icon)
        val="${val/#\~/$HOME}"
        if [[ -n "$val" && "$val" != /* ]]; then val="$1/$val"; fi
        [[ -z "$ICON_PATH" ]] && ICON_PATH="$val"
        ;;
    esac
  done < "$cfg"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --project) PROJECT_DIR="$(cd "$2" && pwd)"; shift 2 ;;
    --id) APP_ID="$2"; shift 2 ;;
    --name) APP_NAME="$2"; shift 2 ;;
    --comment) APP_COMMENT="$2"; shift 2 ;;
    --icon) ICON_PATH="$2"; shift 2 ;;
    --wm-class) WM_CLASS="$2"; shift 2 ;;
    --data-dir) DATA_DIR="$2"; shift 2 ;;
    --restart) RESTART="true"; shift ;;
    --no-restart) RESTART="false"; shift ;;
    --help|-h)
      sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

_load_config "$PROJECT_DIR"

APP_ID="${APP_ID:-motif}"
APP_NAME="${APP_NAME:-Motif}"
APP_COMMENT="${APP_COMMENT:-Music sketchpad (playlist + piano roll)}"
WM_CLASS="${WM_CLASS:-motif}"

if [[ -z "$DATA_DIR" ]]; then
  DATA_DIR="$PROJECT_DIR"
fi
DATA_DIR="$(cd "$DATA_DIR" && pwd)"

CARGO_TOML="$PROJECT_DIR/Cargo.toml"
if [[ ! -f "$CARGO_TOML" ]]; then
  echo "No Cargo.toml in: $PROJECT_DIR" >&2
  exit 1
fi

EXE_NAME="$(awk -F'"' '/^name = /{print $2; exit}' "$CARGO_TOML")"
if [[ -z "$EXE_NAME" ]]; then
  echo "Could not read package name from Cargo.toml" >&2
  exit 1
fi

INSTALL_DIR="$HOME/.local/opt/$APP_ID"
DESKTOP_DIR="$HOME/.local/share/applications"
DESKTOP_FILE="$DESKTOP_DIR/$APP_ID.desktop"
BINARY_SRC="$PROJECT_DIR/target/release/$EXE_NAME"
BINARY_DST="$INSTALL_DIR/$EXE_NAME"

echo "Building release ($EXE_NAME)..."
(
  cd "$PROJECT_DIR"
  cargo build --release
)

if [[ ! -x "$BINARY_SRC" ]]; then
  echo "Missing release binary: $BINARY_SRC" >&2
  exit 1
fi

mkdir -p "$INSTALL_DIR" "$DESKTOP_DIR"

echo "Installing to $INSTALL_DIR"
cp -f "$BINARY_SRC" "$BINARY_DST"
chmod +x "$BINARY_DST"

echo "Writing launcher $DESKTOP_FILE"
{
  echo "[Desktop Entry]"
  echo "Version=1.0"
  echo "Type=Application"
  echo "Name=$APP_NAME"
  echo "Comment=$APP_COMMENT"
  echo "Exec=$BINARY_DST"
  echo "Path=$DATA_DIR"
  echo "Terminal=false"
  echo "Categories=AudioVideo;Audio;Music;"
  echo "StartupWMClass=$WM_CLASS"
  if [[ -n "$ICON_PATH" ]]; then
    echo "Icon=$ICON_PATH"
  fi
} > "$DESKTOP_FILE"

update-desktop-database "$DESKTOP_DIR" >/dev/null 2>&1 || true

echo "Done"
echo "  App:      $APP_NAME ($APP_ID)"
echo "  Binary:   $BINARY_DST"
echo "  Data CWD: $DATA_DIR (settings.json, plugin_cache.json)"
echo "  Menu:     $DESKTOP_FILE"

if [[ "$RESTART" == "true" ]]; then
  echo "Restarting: $BINARY_DST"
  pkill -f "$BINARY_DST" >/dev/null 2>&1 || true
  for _ in $(seq 1 30); do
    pgrep -f "$BINARY_DST" >/dev/null 2>&1 || break
    sleep 0.1
  done
  if pgrep -f "$BINARY_DST" >/dev/null 2>&1; then
    echo "Old process still running; sending SIGKILL" >&2
    pkill -9 -f "$BINARY_DST" >/dev/null 2>&1 || true
    sleep 0.2
  fi
  nohup "$BINARY_DST" >/dev/null 2>&1 &
  echo "  Started pid $!"
  disown $! 2>/dev/null || true
fi
