#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT

target=x86_64-unknown-linux-musl
archive="autotun-${target}.tar.gz"
mkdir -p "$fixture/payload" "$fixture/bin" "$fixture/install"
printf '#!/bin/sh\nprintf "autotun fixture 1.0.0\\n"\n' >"$fixture/payload/autotun"
chmod +x "$fixture/payload/autotun"
tar -C "$fixture/payload" -czf "$fixture/$archive" autotun
(cd "$fixture" && sha256sum "$archive" >SHA256SUMS)

output=$(
    export PATH="$fixture/bin:$PATH"
    export AUTOTUN_BASE_URL="file://$fixture"
    export AUTOTUN_INSTALL_DIR="$fixture/install"
    sh "$repo_root/install.sh"
)

grep -q "Installed autotun" <<<"$output"
test -x "$fixture/install/autotun"
test "$("$fixture/install/autotun")" = "autotun fixture 1.0.0"

printf 'installer test passed\n'
