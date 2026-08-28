#!/usr/bin/env bash
set -euo pipefail

cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
cargo build --release --locked "$@"
echo "built: $PWD/target/release/herdr"
