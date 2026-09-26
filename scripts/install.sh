#!/bin/sh
# Installs Noter on macOS or Linux.
#
# From a checkout, this builds and installs the locked source with Cargo.
# Elsewhere, or with --binary, it downloads a release archive, verifies its
# SHA-256 sidecar, and installs the binary. Everything runs from main, so a
# download of this script that stops partway executes nothing.
set -eu

usage() {
    cat <<'EOF'
Usage: sh install.sh [OPTIONS]

Options:
  --version VERSION  Install this release, for example 0.1.0-beta.1
                     (default: the newest release, including prereleases)
  --root PATH        Install into PATH/bin
  --binary           Download a release even when run from a checkout
  --from-source      Build from the checkout that contains this script
  --source PATH      Build from the checkout at PATH
  --uninstall        Remove the installed binary
  --check            Validate the plan without installing
  -h, --help         Show this help
EOF
}

fail() {
    echo "noter installer: $*" >&2
    exit 1
}

absolute() {
    case "$1" in
        /*) printf '%s\n' "$1" ;;
        *) printf '%s/%s\n' "$invocation_dir" "$1" ;;
    esac
}

# Prints the newest release tag, prereleases included. GitHub's
# releases/latest skips prereleases, so it cannot find one here.
newest_release_tag() {
    releases=$(download_to_stdout "https://api.github.com/repos/$repo/releases?per_page=1") ||
        fail "could not ask GitHub for the newest release. GitHub limits anonymous requests; wait, or pass --version 0.1.0-beta.1."
    printf '%s\n' "$releases" |
        sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' |
        head -n 1
}

download_to_stdout() {
    if command -v curl >/dev/null 2>&1; then
        curl --proto '=https' --tlsv1.2 -fsSL "$1"
    elif command -v wget >/dev/null 2>&1; then
        wget --https-only -qO- "$1"
    else
        fail "curl or wget is required to download Noter."
    fi
}

download_to_file() {
    download_to_stdout "$1" >"$2" || fail "could not download $1"
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        fail "sha256sum or shasum is required to verify the download."
    fi
}

path_hint() {
    case ":${PATH:-}:" in
        *":$1:"*) ;;
        *) printf "Add %s to PATH to run noter by name.\n" "$1" ;;
    esac
}

install_from_source() {
    manifest=$source_dir/Cargo.toml
    [ -f "$manifest" ] || fail "Noter source manifest not found at '$manifest'."
    command -v cargo >/dev/null 2>&1 ||
        fail "Cargo is required. Install the Rust toolchain from https://rustup.rs, then retry."

    metadata=$(cd "$source_dir" && cargo metadata --locked --no-deps --format-version 1 --manifest-path "$manifest")
    case "$metadata" in
        *'"name":"noter"'*) ;;
        *) fail "the workspace at '$source_dir' does not contain the Noter package." ;;
    esac
    expected_version=$(printf '%s\n' "$metadata" | sed -n 's/.*"name":"noter","version":"\([^"]*\)".*/\1/p')
    [ -n "$expected_version" ] || fail "Cargo metadata did not contain the Noter package version."

    if [ "$check_only" = true ]; then
        printf "Validated Noter %s at '%s'.\n" "$expected_version" "$source_dir"
        return
    fi

    (cd "$source_dir" && cargo install --path "$source_dir" --locked --force --root "$install_root")

    installed_binary=$install_root/bin/noter
    [ -x "$installed_binary" ] || fail "Cargo reported success, but '$installed_binary' was not found."
    [ "$("$installed_binary" --version)" = "noter $expected_version" ] ||
        fail "the installed executable did not report the expected Noter version $expected_version."

    cli_temp_dir=$(mktemp -d "${TMPDIR:-/tmp}/noter-install.XXXXXX")
    trap 'rm -rf "$cli_temp_dir"' EXIT HUP INT TERM
    invalid_stdout=$cli_temp_dir/invalid.stdout
    invalid_stderr=$cli_temp_dir/invalid.stderr
    if "$installed_binary" --theme invalid >"$invalid_stdout" 2>"$invalid_stderr"; then
        invalid_status=0
    else
        invalid_status=$?
    fi
    [ "$invalid_status" -eq 2 ] &&
        [ ! -s "$invalid_stdout" ] &&
        grep -F 'unknown theme `invalid`; expected system, light, dark, green, or amber' "$invalid_stderr" >/dev/null &&
        grep -F 'Usage:' "$invalid_stderr" >/dev/null ||
        fail "the installed executable did not preserve the release command-line error contract."
    printf "Installed Noter %s at '%s'.\n" "$expected_version" "$installed_binary"
    path_hint "$install_root/bin"
}

install_from_release() {
    os=$(uname -s)
    arch=$(uname -m)
    case "$os" in
        Darwin)
            case "$arch" in
                arm64 | aarch64) target=aarch64-apple-darwin ;;
                x86_64) target=x86_64-apple-darwin ;;
                *) fail "unsupported architecture $arch on macOS." ;;
            esac
            ;;
        Linux)
            case "$arch" in
                x86_64) target=x86_64-unknown-linux-gnu ;;
                *) fail "no release is published for $arch on Linux yet; use --from-source." ;;
            esac
            ;;
        *) fail "unsupported operating system $os." ;;
    esac

    if [ "$version" = latest ]; then
        tag=$(newest_release_tag)
        [ -n "$tag" ] || fail "could not find a published release."
    else
        case "$version" in
            v*) tag=$version ;;
            *) tag=v$version ;;
        esac
    fi
    archive=noter-$target.tar.xz
    archive_url=https://github.com/$repo/releases/download/$tag/$archive

    if [ "$check_only" = true ]; then
        printf "Validated release %s for %s from %s.\n" "$tag" "$target" "$archive_url"
        return
    fi

    command -v tar >/dev/null 2>&1 || fail "tar is required to unpack the release."
    command -v xz >/dev/null 2>&1 || fail "xz is required to unpack the release; install xz-utils or xz."

    bin_dir=$install_root/bin
    staged=$bin_dir/.noter.install.$$
    cli_temp_dir=$(mktemp -d "${TMPDIR:-/tmp}/noter-install.XXXXXX")
    trap 'rm -rf "$cli_temp_dir"; rm -f "$staged"' EXIT HUP INT TERM

    printf "Downloading Noter %s for %s...\n" "$tag" "$target"
    download_to_file "$archive_url" "$cli_temp_dir/$archive"
    download_to_file "$archive_url.sha256" "$cli_temp_dir/$archive.sha256"

    # The sidecar comes from the same release, so this proves the archive
    # arrived intact, not who built it. INSTALLATION.md shows how to verify
    # the GitHub build attestation as well.
    expected_hash=$(awk '{print $1}' "$cli_temp_dir/$archive.sha256")
    actual_hash=$(sha256_of "$cli_temp_dir/$archive")
    [ -n "$expected_hash" ] && [ "$expected_hash" = "$actual_hash" ] ||
        fail "checksum mismatch for $archive: expected $expected_hash, got $actual_hash."

    tar -xJf "$cli_temp_dir/$archive" -C "$cli_temp_dir"
    found_binary=$(find "$cli_temp_dir" -type f -name noter | head -n 1)
    [ -n "$found_binary" ] || fail "the release archive did not contain the noter binary."

    mkdir -p "$bin_dir"
    # Stage beside the destination and rename, so the install is atomic and a
    # running copy keeps its file until it exits.
    cp "$found_binary" "$staged"
    chmod 755 "$staged"
    staged_version=$("$staged" --version) || {
        rm -f "$staged"
        fail "the downloaded binary did not run on this system."
    }
    [ "$staged_version" = "noter ${tag#v}" ] || {
        rm -f "$staged"
        fail "the downloaded binary reported '$staged_version', expected 'noter ${tag#v}'."
    }
    mv -f "$staged" "$bin_dir/noter"
    printf "Installed %s at '%s'.\n" "$staged_version" "$bin_dir/noter"
    path_hint "$bin_dir"
}

main() {
    repo=blisspixel/noter
    invocation_dir=$(pwd -P)
    install_root=
    check_only=false
    mode=auto
    uninstall=false
    version=latest
    source_dir=

    while [ "$#" -gt 0 ]; do
        case "$1" in
            --source)
                [ "$#" -ge 2 ] || fail "--source requires a path"
                source_dir=$(CDPATH='' cd -- "$2" 2>/dev/null && pwd -P) ||
                    fail "--source path '$2' is not a directory."
                mode=source
                shift 2
                ;;
            --root)
                [ "$#" -ge 2 ] || fail "--root requires a path"
                install_root=$(absolute "$2")
                shift 2
                ;;
            --version)
                [ "$#" -ge 2 ] || fail "--version requires a version"
                version=$2
                shift 2
                ;;
            --binary) mode=binary; shift ;;
            --from-source) mode=source; shift ;;
            --uninstall) uninstall=true; shift ;;
            --check) check_only=true; shift ;;
            -h | --help) usage; return ;;
            *) usage >&2; exit 2 ;;
        esac
    done

    # This script lives in scripts/ of a checkout only when it runs from a
    # file there. Piped into sh, $0 is the shell, and no checkout is assumed.
    if [ -z "$source_dir" ] && [ -f "$0" ]; then
        candidate=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
        if grep -q '^name = "noter"$' "$candidate/Cargo.toml" 2>/dev/null; then
            source_dir=$candidate
        fi
    fi
    if [ "$mode" = auto ]; then
        if [ -n "$source_dir" ]; then mode=source; else mode=binary; fi
    fi
    [ "$mode" = binary ] || [ -n "$source_dir" ] ||
        fail "--from-source needs a checkout; run the script from one or pass --source PATH."
    [ "$mode" = binary ] || [ "$version" = latest ] ||
        fail "--version selects a release download; add --binary, or omit --version to build this checkout."

    if [ -n "${CARGO_HOME:-}" ]; then
        CARGO_HOME=$(absolute "$CARGO_HOME")
        export CARGO_HOME
    fi
    if [ -z "$install_root" ]; then
        if [ -n "${CARGO_INSTALL_ROOT:-}" ]; then
            install_root=$(absolute "$CARGO_INSTALL_ROOT")
        elif [ "$mode" = source ] && [ -n "${CARGO_HOME:-}" ]; then
            install_root=$CARGO_HOME
        elif [ -n "${HOME:-}" ]; then
            if [ "$mode" = source ]; then
                install_root=$HOME/.cargo
            else
                install_root=$HOME/.local
            fi
        else
            fail "an installation root could not be determined; pass --root."
        fi
    fi

    if [ "$uninstall" = true ]; then
        target_binary=$install_root/bin/noter
        if [ "$check_only" = true ]; then
            printf "Would remove '%s'.\n" "$target_binary"
        elif [ -e "$target_binary" ]; then
            rm -f "$target_binary"
            printf "Removed '%s'. Documents and settings were not touched.\n" "$target_binary"
        else
            printf "No Noter binary at '%s'.\n" "$target_binary"
        fi
        return
    fi

    if [ "$mode" = source ]; then
        install_from_source
    else
        install_from_release
    fi
}

main "$@"
