#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
source_dir=$(CDPATH= cd -- "$script_dir/.." && pwd -P)
invocation_dir=$(pwd -P)
install_root=
check_only=false

while [ "$#" -gt 0 ]; do
    case "$1" in
        --source)
            [ "$#" -ge 2 ] || { echo "--source requires a path" >&2; exit 2; }
            source_dir=$(CDPATH= cd -- "$2" && pwd -P)
            shift 2
            ;;
        --root)
            [ "$#" -ge 2 ] || { echo "--root requires a path" >&2; exit 2; }
            install_root=$2
            shift 2
            ;;
        --check)
            check_only=true
            shift
            ;;
        *)
            echo "unknown option: $1" >&2
            exit 2
            ;;
    esac
done

if [ -n "$install_root" ]; then
    case "$install_root" in
        /*) ;;
        *) install_root=$invocation_dir/$install_root ;;
    esac
fi
if [ -n "${CARGO_HOME:-}" ]; then
    case "$CARGO_HOME" in
        /*) ;;
        *) CARGO_HOME=$invocation_dir/$CARGO_HOME ;;
    esac
    export CARGO_HOME
fi
if [ -z "$install_root" ]; then
    if [ -n "${CARGO_INSTALL_ROOT:-}" ]; then
        install_root=$CARGO_INSTALL_ROOT
    elif [ -n "${CARGO_HOME:-}" ]; then
        install_root=$CARGO_HOME
    elif [ -n "${HOME:-}" ]; then
        install_root=$HOME/.cargo
    else
        echo "An installation root could not be determined; pass --root explicitly." >&2
        exit 1
    fi
    case "$install_root" in
        /*) ;;
        *) install_root=$invocation_dir/$install_root ;;
    esac
fi

manifest=$source_dir/Cargo.toml
[ -f "$manifest" ] || { echo "Noter source manifest not found at '$manifest'." >&2; exit 1; }
command -v cargo >/dev/null 2>&1 || {
    echo "Cargo is required. Install the Rust toolchain from https://rustup.rs, then retry." >&2
    exit 1
}

metadata=$(cd "$source_dir" && cargo metadata --locked --no-deps --format-version 1 --manifest-path "$manifest")
case "$metadata" in
    *'"name":"noter"'*) ;;
    *) echo "The workspace at '$source_dir' does not contain the Noter package." >&2; exit 1 ;;
esac
expected_version=$(printf '%s\n' "$metadata" | sed -n 's/.*"name":"noter","version":"\([^"]*\)".*/\1/p')
[ -n "$expected_version" ] || {
    echo "Cargo metadata did not contain the Noter package version." >&2
    exit 1
}

if [ "$check_only" = true ]; then
    printf "Validated Noter source at '%s'.\n" "$source_dir"
    exit 0
fi

(cd "$source_dir" && cargo install --path "$source_dir" --locked --force --root "$install_root")

installed_binary=$install_root/bin/noter
[ -x "$installed_binary" ] || {
    echo "Cargo reported success, but '$installed_binary' was not found." >&2
    exit 1
}
installed_version=$("$installed_binary" --version)
[ "$installed_version" = "noter $expected_version" ] || {
    echo "The installed executable did not report the expected Noter version $expected_version." >&2
    exit 1
}
printf "Installed Noter at '%s'.\n" "$installed_binary"
