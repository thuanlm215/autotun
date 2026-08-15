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
grep -q "Skipping GUI" <<<"$output"

# Same script also installs the GUI when that archive is in the release.
gui_target=x86_64-unknown-linux-gnu
gui_archive="autotun-gui-${gui_target}.tar.gz"
mkdir -p "$fixture/gui"
printf '#!/bin/sh\nprintf "autotun-gui fixture\\n"\n' >"$fixture/gui/autotun-gui"
chmod +x "$fixture/gui/autotun-gui"
printf '%s\n' '[Desktop Entry]' 'Exec=autotun --gui' >"$fixture/gui/autotun.desktop"
tar -C "$fixture/gui" -czf "$fixture/$gui_archive" autotun-gui autotun.desktop
(cd "$fixture" && sha256sum "$archive" "$gui_archive" >SHA256SUMS)

xdg="$fixture/xdg"
output=$(
    export PATH="$fixture/bin:$PATH"
    export AUTOTUN_BASE_URL="file://$fixture"
    export AUTOTUN_INSTALL_DIR="$fixture/install"
    export XDG_DATA_HOME="$xdg"
    sh "$repo_root/install.sh"
)

grep -q "Installed autotun GUI" <<<"$output"
test -x "$fixture/install/autotun-gui"
test "$("$fixture/install/autotun-gui")" = "autotun-gui fixture"
grep -q "Exec=$fixture/install/autotun --gui" "$xdg/applications/autotun.desktop"

printf 'installer test passed\n'
