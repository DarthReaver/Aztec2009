#!/usr/bin/env bash
# Build Aztec 2009 into a single native Linux binary (windowed workspace plus terminal modes).
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release
cp target/release/aztec2009 ./aztec2009
echo "Built ./aztec2009"
echo "Open the app:    ./aztec2009        (or double click 'Aztec 2009.desktop', or run ./install.sh)"
echo "Terminal UI:     ./aztec2009 --tui"
echo "Run a script:    ./aztec2009 -c script.mel"
