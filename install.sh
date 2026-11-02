#!/bin/sh
# Installs Aztec 2009 for the current user (no root needed): the program, an icon, and an
# application-menu launcher so it starts with a double click instead of from a terminal.
# Run from the project folder or the unpacked release folder.
set -e
here=$(cd "$(dirname "$0")" && pwd)
bin_dir="$HOME/.local/bin"
share_dir="$HOME/.local/share/aztec2009"
apps_dir="$HOME/.local/share/applications"
mkdir -p "$bin_dir" "$share_dir" "$apps_dir"

if [ -x "$here/target/release/aztec2009" ]; then
  src="$here/target/release/aztec2009"
elif [ -x "$here/aztec2009" ]; then
  src="$here/aztec2009"
elif command -v cargo >/dev/null 2>&1; then
  (cd "$here" && cargo build --release)
  src="$here/target/release/aztec2009"
else
  echo "No prebuilt binary and no cargo found. Install Rust from https://rustup.rs and rerun." >&2
  exit 1
fi
cp "$src" "$bin_dir/aztec2009"
chmod +x "$bin_dir/aztec2009"
[ -d "$here/examples" ] && cp -r "$here/examples" "$share_dir/"
if [ -f "$here/assets/aztec2009.png" ]; then
  cp "$here/assets/aztec2009.png" "$share_dir/aztec2009.png"
else
  "$bin_dir/aztec2009" --write-icon "$share_dir/aztec2009.png" >/dev/null
fi

launcher="$apps_dir/aztec2009.desktop"
cat > "$launcher" <<DESKTOP
[Desktop Entry]
Type=Application
Version=1.0
Name=Aztec 2009
GenericName=3D Animation
Comment=Maya 2009 style 3D animation tool
Exec=$bin_dir/aztec2009 %f
Icon=$share_dir/aztec2009.png
Terminal=false
StartupWMClass=aztec2009
Categories=Graphics;3DGraphics;
MimeType=
DESKTOP
chmod +x "$launcher"
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$apps_dir" >/dev/null 2>&1 || true

# Desktop shortcut, when the user has a Desktop folder.
desk=$(xdg-user-dir DESKTOP 2>/dev/null || echo "$HOME/Desktop")
if [ -d "$desk" ]; then
  cp "$launcher" "$desk/Aztec 2009.desktop"
  chmod +x "$desk/Aztec 2009.desktop"
  command -v gio >/dev/null 2>&1 && gio set "$desk/Aztec 2009.desktop" metadata::trusted true >/dev/null 2>&1 || true
fi

echo "Installed: $bin_dir/aztec2009"
echo "Launcher:  $launcher  (find \"Aztec 2009\" in your application menu)"
case ":$PATH:" in *":$bin_dir:"*) ;; *) echo "Note: $bin_dir is not on PATH yet; log out and in once, or use the menu launcher.";; esac
