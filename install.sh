#!/bin/sh
set -eu

REPO=${AUTOTUN_REPO:-thuanlm215/autotun}
VERSION=${AUTOTUN_VERSION:-latest}
INSTALL_DIR=${AUTOTUN_INSTALL_DIR:-"${HOME}/.local/bin"}

die() {
    printf 'autotun installer: %s\n' "$*" >&2
    exit 1
}

for command in curl tar sha256sum mktemp install; do
    command -v "$command" >/dev/null 2>&1 || die "required command not found: $command"
done

[ "$(uname -s)" = Linux ] || die "only Linux is supported"
case "$(uname -m)" in
    x86_64 | amd64)
        target=x86_64-unknown-linux-musl
        gui_target=x86_64-unknown-linux-gnu
        ;;
    aarch64 | arm64)
        target=aarch64-unknown-linux-musl
        gui_target=aarch64-unknown-linux-gnu
        ;;
    *) die "unsupported CPU architecture: $(uname -m)" ;;
esac

archive="autotun-${target}.tar.gz"
gui_archive="autotun-gui-${gui_target}.tar.gz"
if [ -n "${AUTOTUN_BASE_URL:-}" ]; then
    base_url=${AUTOTUN_BASE_URL%/}
elif [ "$VERSION" = latest ]; then
    base_url="https://github.com/${REPO}/releases/latest/download"
else
    case "$VERSION" in v*) tag=$VERSION ;; *) tag="v${VERSION}" ;; esac
    base_url="https://github.com/${REPO}/releases/download/${tag}"
fi

tmp_dir=$(mktemp -d)
trap 'rm -rf "$tmp_dir"' EXIT HUP INT TERM

printf 'Downloading autotun for %s...\n' "$target"
curl --fail --location --silent --show-error \
    --output "$tmp_dir/$archive" "$base_url/$archive"
curl --fail --location --silent --show-error \
    --output "$tmp_dir/SHA256SUMS" "$base_url/SHA256SUMS"

expected=$(awk -v file="$archive" '$2 == file || $2 == "*" file { print; exit }' "$tmp_dir/SHA256SUMS")
[ -n "$expected" ] || die "release checksum does not include $archive"
printf '%s\n' "$expected" | (cd "$tmp_dir" && sha256sum --check --status -) \
    || die "checksum verification failed for $archive"

tar -xzf "$tmp_dir/$archive" -C "$tmp_dir"
[ -f "$tmp_dir/autotun" ] || die "release archive does not contain autotun"
mkdir -p "$INSTALL_DIR"
install -m 0755 "$tmp_dir/autotun" "$INSTALL_DIR/autotun"

printf 'Installed autotun to %s/autotun\n' "$INSTALL_DIR"

# GUI is optional so a TUI-only fixture / older release still installs.
expected_gui=$(awk -v file="$gui_archive" '$2 == file || $2 == "*" file { print; exit }' "$tmp_dir/SHA256SUMS" || true)
if [ -n "$expected_gui" ] \
    && curl --fail --location --silent --show-error \
        --output "$tmp_dir/$gui_archive" "$base_url/$gui_archive"; then
    if printf '%s\n' "$expected_gui" | (cd "$tmp_dir" && sha256sum --check --status -); then
        tar -xzf "$tmp_dir/$gui_archive" -C "$tmp_dir"
        if [ -f "$tmp_dir/autotun-gui" ]; then
            install -m 0755 "$tmp_dir/autotun-gui" "$INSTALL_DIR/autotun-gui"
            applications_dir="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
            mkdir -p "$applications_dir"
            if [ ! -f "$tmp_dir/autotun.desktop" ]; then
                cat >"$tmp_dir/autotun.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=autotun
Comment=Discover and toggle SSH port forwards
Exec=$INSTALL_DIR/autotun --gui
Icon=autotun
Terminal=false
Categories=Network;Utility;
StartupNotify=true
StartupWMClass=autotun
EOF
            fi
            # Point the launcher at this install even if PATH is incomplete.
            sed "s|^Exec=.*|Exec=$INSTALL_DIR/autotun --gui|" "$tmp_dir/autotun.desktop" \
                >"$applications_dir/autotun.desktop"
            icons_home="${XDG_DATA_HOME:-$HOME/.local/share}/icons"
            if [ -f "$tmp_dir/autotun.svg" ]; then
                mkdir -p "$icons_home/hicolor/scalable/apps"
                install -m 0644 "$tmp_dir/autotun.svg" \
                    "$icons_home/hicolor/scalable/apps/autotun.svg"
            fi
            if [ -f "$tmp_dir/autotun.png" ]; then
                mkdir -p "$icons_home/hicolor/256x256/apps"
                install -m 0644 "$tmp_dir/autotun.png" \
                    "$icons_home/hicolor/256x256/apps/autotun.png"
            fi
            printf 'Installed autotun GUI to %s/autotun-gui\n' "$INSTALL_DIR"
        fi
    else
        printf 'Skipping GUI: checksum verification failed for %s\n' "$gui_archive"
    fi
else
    printf 'Skipping GUI: %s is not in this release\n' "$gui_archive"
fi

case ":${PATH}:" in
    *:"$INSTALL_DIR":*) ;;
    *)
        printf '%s\n' "Add $INSTALL_DIR to PATH. For fish:"
        printf '  fish_add_path %s\n' "$INSTALL_DIR"
        ;;
esac
"$INSTALL_DIR/autotun" --version
