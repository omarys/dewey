#!/usr/bin/env bash
# Build and install dewey tuned for THIS machine's CPU.
#
# `-C target-cpu=native` lets the compiler use every instruction the local CPU
# supports (NEON/SIMD on ARM, AVX on x86), which is a real win on low-resource
# devices like the PineTab 2. The resulting binary is NOT portable — rebuild
# it on each machine. Internal use; not intended for distribution.
#
#   ./install.sh                    # build + install to ~/.cargo/bin
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
