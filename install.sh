#!/usr/bin/env bash
# Build and install dewey tuned for THIS machine's CPU.
#
# `-C target-cpu=native` lets the compiler use every instruction the local CPU
# supports (NEON/SIMD on ARM, AVX on x86), which is a real win on low-resource
# devices like the PineTab 2. The resulting binary is NOT portable — rebuild
# it on each machine. Internal use; not intended for distribution.
#
#   ./install.sh                    # build + install to ~/.cargo/bin and ~/.local/share
#   DEWEY_PREFIX=/opt/bin ./install.sh
#
set -euo pipefail
cd "$(dirname "$0")"

# Append native-target flags, preserving any user-supplied RUSTFLAGS.
export RUSTFLAGS="${RUSTFLAGS:-} -C target-cpu=native"

cargo build --release

prefix="${DEWEY_PREFIX:-$HOME/.cargo/bin}"
mkdir -p "$prefix"
install -m755 target/release/dewey "$prefix/dewey"

echo "Installed native dewey -> $prefix/dewey"

# Install Desktop entry and icon
xdg_data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
apps_dir="$xdg_data_dir/applications"
icons_dir="$xdg_data_dir/icons/hicolor/scalable/apps"

mkdir -p "$apps_dir" "$icons_dir"

if [ -f "assets/dewey.svg" ]; then
    install -m644 assets/dewey.svg "$icons_dir/dewey.svg"
    echo "Installed icon -> $icons_dir/dewey.svg"
fi

if [ -f "assets/dewey.desktop" ]; then
    # Generate desktop entry pointing to installed binary
    sed "s|Exec=foot -T \"Dewey\" -a dewey dewey|Exec=foot -T \"Dewey\" -a dewey $prefix/dewey|" assets/dewey.desktop > "$apps_dir/dewey.desktop"
    chmod 644 "$apps_dir/dewey.desktop"
    echo "Installed desktop shortcut -> $apps_dir/dewey.desktop"
fi

# Update desktop and icon caches if tools are available
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$apps_dir" 2>/dev/null || true
fi

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "$xdg_data_dir/icons/hicolor" 2>/dev/null || true
fi

echo "Installation complete!"
