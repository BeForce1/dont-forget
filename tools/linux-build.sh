#!/usr/bin/env bash
# Builds the Linux version inside WSL from the Windows checkout (copying it to the Linux filesystem,
# which builds much faster than /mnt/c). Usage from Windows:  wsl -d Ubuntu -u root -- bash tools/linux-build.sh
set -euo pipefail
src="$(cd "$(dirname "$0")/.." && pwd)"
work=/root/dont-forget
mkdir -p "$work"
cp -r "$src/Cargo.toml" "$src/Cargo.lock" "$work/"
rm -rf "$work/src" && cp -r "$src/src" "$work/src"
. /root/.cargo/env
cd "$work"
cargo build --release 2>&1 | grep -E "^(warning|error)|Finished" -A6 || true
ls -la target/release/DontForget
