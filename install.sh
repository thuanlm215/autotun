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
    x86_64 | amd64) target=x86_64-unknown-linux-musl ;;
    aarch64 | arm64) target=aarch64-unknown-linux-musl ;;
    *) die "unsupported CPU architecture: $(uname -m)" ;;
esac

archive="autotun-${target}.tar.gz"
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
case ":${PATH}:" in
    *:"$INSTALL_DIR":*) ;;
    *)
        printf '%s\n' "Add $INSTALL_DIR to PATH. For fish:"
        printf '  fish_add_path %s\n' "$INSTALL_DIR"
        ;;
esac
"$INSTALL_DIR/autotun" --version
