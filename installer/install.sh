#!/bin/sh
# Teramind installer (Unix). Idempotent.
#
# Behavior:
#   1. Detect OS + arch.
#   2. Download the release archive for that target.
#   3. Verify the SHA-256 against SHA256SUMS.
#   4. Extract to $INSTALL_ROOT/bin/.
#   5. Symlink the `teramind` binary into ~/.local/bin/.
#   6. Print the next-steps line.
#
# Environment overrides (all optional):
#   TERAMIND_VERSION         — version tag to install (default: latest from releases.json)
#   TERAMIND_RELEASE_BASE    — base URL for releases (default: https://get.teramind.dev)
#   TERAMIND_INSTALL_ROOT    — where binaries go (default: ~/.local/share/teramind)
#   TERAMIND_BIN_DIR         — where the `teramind` symlink goes (default: ~/.local/bin)
#   TERAMIND_NO_MODIFY_PATH  — set to skip PATH printing (default: unset)

set -eu

BASE_URL="${TERAMIND_RELEASE_BASE:-https://get.teramind.dev}"
INSTALL_ROOT="${TERAMIND_INSTALL_ROOT:-${HOME}/.local/share/teramind}"
BIN_DIR="${TERAMIND_BIN_DIR:-${HOME}/.local/bin}"

die() { echo "install.sh: error: $*" >&2; exit 1; }
info() { echo "install.sh: $*"; }

need() { command -v "$1" >/dev/null 2>&1 || die "missing required tool: $1"; }

detect_os() {
    case "$(uname -s)" in
        Linux)  echo "unknown-linux-gnu" ;;
        Darwin) echo "apple-darwin" ;;
        *) die "unsupported OS: $(uname -s) (use install.ps1 on Windows)" ;;
    esac
}

detect_arch() {
    case "$(uname -m)" in
        x86_64|amd64)  echo "x86_64" ;;
        aarch64|arm64) echo "aarch64" ;;
        *) die "unsupported arch: $(uname -m)" ;;
    esac
}

detect_triple() {
    arch=$(detect_arch)
    os=$(detect_os)
    echo "${arch}-${os}"
}

fetch_to() {
    src="$1"; dest="$2"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL --output "${dest}" "${src}"
    elif command -v wget >/dev/null 2>&1; then
        wget -q -O "${dest}" "${src}"
    else
        die "need curl or wget on PATH"
    fi
}

resolve_version() {
    if [ -n "${TERAMIND_VERSION:-}" ]; then
        echo "${TERAMIND_VERSION}"
        return
    fi
    tmp=$(mktemp)
    fetch_to "${BASE_URL}/releases.json" "${tmp}"
    # Lightweight JSON pluck. Avoids a jq dep.
    sed -n 's/.*"latest"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "${tmp}" | head -1
    rm -f "${tmp}"
}

verify_sha256() {
    file="$1"; expected="$2"
    if command -v sha256sum >/dev/null 2>&1; then
        actual=$(sha256sum "${file}" | awk '{print $1}')
    elif command -v shasum >/dev/null 2>&1; then
        actual=$(shasum -a 256 "${file}" | awk '{print $1}')
    else
        die "need sha256sum or shasum on PATH"
    fi
    [ "${actual}" = "${expected}" ] || die "checksum mismatch (${file}): expected ${expected}, got ${actual}"
}

main() {
    need uname
    need tar
    need mktemp

    triple=$(detect_triple)
    version=$(resolve_version)
    [ -n "${version}" ] || die "could not determine latest version from ${BASE_URL}/releases.json"
    info "installing teramind ${version} for ${triple}"

    archive_name="teramind-${version}-${triple}.tar.gz"
    archive_url="${BASE_URL}/${version}/${archive_name}"
    sums_url="${BASE_URL}/${version}/teramind-${version}-SHA256SUMS"

    tmpdir=$(mktemp -d)
    trap 'rm -rf "${tmpdir}"' EXIT

    info "downloading ${archive_url}"
    fetch_to "${archive_url}" "${tmpdir}/${archive_name}"

    info "downloading SHA256SUMS"
    fetch_to "${sums_url}" "${tmpdir}/SHA256SUMS"

    expected=$(awk -v a="${archive_name}" '$2==a || $2=="*"a {print $1}' "${tmpdir}/SHA256SUMS")
    [ -n "${expected}" ] || die "no SHA256 entry for ${archive_name} in SHA256SUMS"
    verify_sha256 "${tmpdir}/${archive_name}" "${expected}"

    mkdir -p "${INSTALL_ROOT}/bin"
    tar -xzf "${tmpdir}/${archive_name}" -C "${INSTALL_ROOT}/bin" --strip-components=1
    chmod +x "${INSTALL_ROOT}/bin/"*

    mkdir -p "${BIN_DIR}"
    ln -sfn "${INSTALL_ROOT}/bin/teramind" "${BIN_DIR}/teramind"

    info "installed to ${INSTALL_ROOT}/bin/"
    info "symlinked   ${BIN_DIR}/teramind -> ${INSTALL_ROOT}/bin/teramind"
    if [ -z "${TERAMIND_NO_MODIFY_PATH:-}" ]; then
        case ":${PATH}:" in
            *":${BIN_DIR}:"*) ;;
            *) info "NOTE: ${BIN_DIR} is not on your PATH. Add it to ~/.bashrc / ~/.zshrc:"
               info "      export PATH=\"${BIN_DIR}:\$PATH\"" ;;
        esac
    fi
    info ""
    info "next:  teramind init && teramind claude install"
}

main "$@"
