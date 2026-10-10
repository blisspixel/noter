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
  --uninstall        Remove the installed binary. On Linux, also remove the
                     desktop entry and icon when they belong to that binary
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

# A private stage is only private if no other user can replace its directory
# or an ancestor while the installer copies and executes it. Check the
# requested spelling before resolving links as well as the resolved path.
trusted_install_directory() {
    current_uid=$(id -u)
    platform=$(uname -s)
    directory=$1
    while :; do
        case "$platform" in
            Darwin) metadata=$(stat -L -f '%u %Lp' "$directory" 2>/dev/null) || fail "cannot inspect install directory permissions." ;;
            *) metadata=$(stat -L -c '%u %a' "$directory" 2>/dev/null) || fail "cannot inspect install directory permissions." ;;
        esac
        set -- $metadata
        owner=$1
        mode=$2
        case "$owner:$mode" in
            *[!0-9:]* | :* | *:) fail "cannot inspect install directory permissions." ;;
        esac
        [ "$owner" = "$current_uid" ] || [ "$owner" = 0 ] ||
            fail "install directory is owned by another user."
        if [ "$platform" = Darwin ]; then
            listing=$(LC_ALL=C ls -Llde "$directory" 2>/dev/null) ||
                fail "cannot inspect install directory permissions."
            acl_entries=$(printf '%s\n' "$listing" | sed '1d')
            if [ -n "$acl_entries" ] &&
                printf '%s\n' "$acl_entries" | grep -v ' deny ' >/dev/null; then
                fail "install directory has an unsupported access control list."
            fi
        fi
        # On Linux the stat group bits are the ACL mask, so a named ACL
        # cannot grant write when the group write bit is clear.
        permissions=$((0$mode))
        if [ "$((permissions & 022))" -ne 0 ]; then
            fail "install directory is writable by another user."
        fi
        [ "$directory" = / ] && break
        directory=$(dirname -- "$directory")
    done
}

prepare_trusted_bin_dir() {
    bin_dir=$install_root/bin
    existing_dir=$bin_dir
    while [ ! -e "$existing_dir" ]; do
        [ ! -L "$existing_dir" ] || fail "install path contains a broken symbolic link."
        existing_dir=$(dirname -- "$existing_dir")
    done
    [ -d "$existing_dir" ] || fail "install path contains a non-directory entry."
    trusted_install_directory "$existing_dir"
    mkdir -p "$bin_dir"
    bin_dir=$(CDPATH='' cd -- "$bin_dir" 2>/dev/null && pwd -P) ||
        fail "cannot resolve the install directory."
    trusted_install_directory "$bin_dir"
}

# Reviewable copies live in packaging/linux. These heredocs are the bytes a
# piped --binary install can write, and a test requires the two to match.
desktop_template() {
    cat <<'EOF'
[Desktop Entry]
Type=Application
Name=Noter
GenericName=Text Editor
Comment=Edit plain text and Markdown files
Exec=@EXEC@
Icon=noter
Terminal=false
Categories=Utility;TextEditor;
MimeType=text/markdown;text/x-markdown;text/plain;
StartupWMClass=noter
Keywords=text;markdown;editor;
X-Noter-Installer=true
EOF
}

icon_svg() {
    cat <<'EOF'
<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 128 128">
  <!-- noter-installer -->
  <rect width="128" height="128" rx="28" fill="#1f2124"/>
  <path fill="#e6e8ec" d="M34 96V32h18.5l23 38.2V32H94v64H75.5L52.5 57.8V96H34z"/>
</svg>
EOF
}

# Absolute XDG_DATA_HOME, or $HOME/.local/share. A relative value is refused
# before any desktop file is created. Sets data_home. Not called in $(...).
resolve_linux_data_home() {
    if [ -n "${XDG_DATA_HOME:-}" ]; then
        case "$XDG_DATA_HOME" in
            /*) data_home=$XDG_DATA_HOME ;;
            *) fail "XDG_DATA_HOME must be an absolute path." ;;
        esac
    elif [ -n "${HOME:-}" ]; then
        data_home=$HOME/.local/share
    else
        fail "a data directory could not be determined; set HOME or XDG_DATA_HOME."
    fi
}

prepare_trusted_data_dir() {
    requested=$1
    existing_dir=$requested
    while [ ! -e "$existing_dir" ]; do
        [ ! -L "$existing_dir" ] || fail "desktop install path contains a broken symbolic link."
        existing_dir=$(dirname -- "$existing_dir")
    done
    [ -d "$existing_dir" ] || fail "desktop install path contains a non-directory entry."
    trusted_install_directory "$existing_dir"
    mkdir -p -- "$requested"
    resolved_data_dir=$(CDPATH='' cd -- "$requested" 2>/dev/null && pwd -P) ||
        fail "cannot resolve the desktop install directory."
    trusted_install_directory "$resolved_data_dir"
}

# Always quote Exec so a path containing spaces cannot be split.
desktop_quote() {
    case "$1" in
        *'
'*) return 1 ;;
    esac
    escaped=$(printf '%s' "$1" | sed 's/[\\"`$]/\\&/g')
    printf '"%s"' "$escaped"
}

canonical_installed_binary() {
    bin_dir=$install_root/bin
    if [ -d "$bin_dir" ] && [ ! -L "$bin_dir" ]; then
        resolved=$(CDPATH='' cd -- "$bin_dir" 2>/dev/null && pwd -P) || resolved=
        if [ -n "$resolved" ]; then
            printf '%s\n' "$resolved/noter"
            return 0
        fi
    fi
    printf '%s\n' "$install_root/bin/noter"
}

assert_regular_destination() {
    path=$1
    label=$2
    if [ -L "$path" ] || { [ -e "$path" ] && [ ! -f "$path" ]; }; then
        fail "the $label destination is not a regular file."
    fi
}

write_desktop_entry() {
    stage=$1
    quoted=$2
    template=$(mktemp "${TMPDIR:-/tmp}/noter-desktop.XXXXXXXX") || return 1
    desktop_template >"$template" || {
        rm -f "$template"
        return 1
    }
    while IFS= read -r line || [ -n "$line" ]; do
        case "$line" in
            'Exec=@EXEC@') printf 'Exec=%s\n' "$quoted" || {
                rm -f "$template"
                return 1
            } ;;
            *) printf '%s\n' "$line" || {
                rm -f "$template"
                return 1
            } ;;
        esac
    done <"$template" >"$stage"
    status=$?
    rm -f "$template"
    return "$status"
}

parse_desktop_exec() {
    awk '
        BEGIN { found = 0 }
        /^Exec=/ {
            value = substr($0, 6)
            if (substr(value, 1, 1) == "\"") {
                out = ""
                i = 2
                n = length(value)
                while (i <= n) {
                    c = substr(value, i, 1)
                    if (c == "\\") {
                        i++
                        if (i <= n) {
                            out = out substr(value, i, 1)
                        }
                    } else if (c == "\"") {
                        break
                    } else {
                        out = out c
                    }
                    i++
                }
                print out
            } else {
                split(value, fields, /[[:space:]]+/)
                print fields[1]
            }
            found = 1
            exit
        }
        END { if (found == 0) exit 1 }
    ' "$1"
}

refresh_desktop_database() {
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$1" >/dev/null 2>&1 || true
    fi
}

# Installs the Linux desktop entry and icon for one absolute binary path.
# No-op on other operating systems. --check prints the paths and creates nothing.
install_linux_desktop() {
    binary=$1
    [ "$(uname -s)" = Linux ] || return 0
    resolve_linux_data_home
    apps_dir=$data_home/applications
    icon_dir=$data_home/icons/hicolor/scalable/apps
    desktop_path=$apps_dir/noter.desktop
    icon_path=$icon_dir/noter.svg
    if [ "$check_only" = true ]; then
        printf "Would install desktop entry '%s'.\n" "$desktop_path"
        printf "Would install icon '%s'.\n" "$icon_path"
        return 0
    fi

    prepare_trusted_data_dir "$apps_dir"
    apps_dir=$resolved_data_dir
    prepare_trusted_data_dir "$icon_dir"
    icon_dir=$resolved_data_dir
    desktop_path=$apps_dir/noter.desktop
    icon_path=$icon_dir/noter.svg
    assert_regular_destination "$desktop_path" "desktop entry"
    assert_regular_destination "$icon_path" "icon"

    quoted=$(desktop_quote "$binary") || fail "cannot quote the installed executable path."
    stage=$(mktemp "$apps_dir/.noter.desktop.XXXXXXXX") ||
        fail "cannot reserve a desktop entry stage."
    if ! write_desktop_entry "$stage" "$quoted"; then
        rm -f "$stage"
        fail "cannot write the desktop entry."
    fi
    if ! grep -q '^X-Noter-Installer=true$' "$stage"; then
        rm -f "$stage"
        fail "desktop entry is incomplete."
    fi
    if grep -q '@EXEC@' "$stage"; then
        rm -f "$stage"
        fail "desktop entry is incomplete."
    fi
    chmod 644 "$stage" || {
        rm -f "$stage"
        fail "cannot set desktop entry permissions."
    }
    mv -f "$stage" "$desktop_path" || fail "cannot install the desktop entry."

    icon_stage=$(mktemp "$icon_dir/.noter.icon.XXXXXXXX") ||
        fail "cannot reserve an icon stage."
    if ! icon_svg >"$icon_stage"; then
        rm -f "$icon_stage"
        fail "cannot write the icon."
    fi
    grep -q 'noter-installer' "$icon_stage" || {
        rm -f "$icon_stage"
        fail "icon is incomplete."
    }
    chmod 644 "$icon_stage" || {
        rm -f "$icon_stage"
        fail "cannot set icon permissions."
    }
    mv -f "$icon_stage" "$icon_path" || fail "cannot install the icon."
    refresh_desktop_database "$apps_dir"
    printf "Installed desktop entry '%s'.\n" "$desktop_path"
}

# Removes the desktop entry only when it is a regular file, carries the
# installer marker, and its Exec path is this install. The icon goes only when
# it is a regular file carrying the installer comment. Symlinks are left alone.
remove_linux_desktop() {
    [ "$(uname -s)" = Linux ] || return 0
    resolve_linux_data_home
    desktop_path=$data_home/applications/noter.desktop
    icon_path=$data_home/icons/hicolor/scalable/apps/noter.svg
    expected=$(canonical_installed_binary)
    if [ "$check_only" = true ]; then
        printf "Would remove desktop entry '%s' when it belongs to '%s'.\n" "$desktop_path" "$expected"
        printf "Would remove icon '%s' when it belongs to this install.\n" "$icon_path"
        return 0
    fi
    if [ ! -L "$desktop_path" ] && [ -f "$desktop_path" ] &&
        grep -q '^X-Noter-Installer=true$' "$desktop_path"; then
        parsed=$(parse_desktop_exec "$desktop_path" || true)
        if [ -n "$parsed" ] && [ "$parsed" = "$expected" ]; then
            rm -f -- "$desktop_path"
            printf "Removed desktop entry '%s'.\n" "$desktop_path"
            refresh_desktop_database "$(dirname -- "$desktop_path")"
        fi
    fi
    if [ ! -L "$icon_path" ] && [ -f "$icon_path" ] &&
        grep -q 'noter-installer' "$icon_path"; then
        rm -f -- "$icon_path"
        printf "Removed icon '%s'.\n" "$icon_path"
    fi
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
        install_linux_desktop "$install_root/bin/noter"
        return
    fi

    build_root=$(mktemp -d "${TMPDIR:-/tmp}/noter-source.XXXXXX") ||
        fail "cannot reserve a private source build root."
    stage_dir=
    trap 'rm -rf "$build_root"; if [ -n "$stage_dir" ]; then rm -rf "$stage_dir"; fi' EXIT HUP INT TERM
    (cd "$source_dir" && cargo install --path "$source_dir" --locked --force --root "$build_root")

    built_binary=$build_root/bin/noter
    [ -x "$built_binary" ] || fail "Cargo reported success, but the staged Noter binary was not found."
    prepare_trusted_bin_dir
    stage_dir=$(mktemp -d "$bin_dir/.noter.install.XXXXXXXX") ||
        fail "cannot reserve a private install stage."
    cp "$built_binary" "$stage_dir/noter"
    chmod 755 "$stage_dir/noter"

    [ "$("$stage_dir/noter" --version)" = "noter $expected_version" ] ||
        fail "the staged executable did not report the expected Noter version $expected_version."

    invalid_stdout=$build_root/invalid.stdout
    invalid_stderr=$build_root/invalid.stderr
    if "$stage_dir/noter" --theme invalid >"$invalid_stdout" 2>"$invalid_stderr"; then
        invalid_status=0
    else
        invalid_status=$?
    fi
    [ "$invalid_status" -eq 2 ] &&
        [ ! -s "$invalid_stdout" ] &&
        grep -F 'unknown theme `invalid`; expected system, light, dark, green, or amber' "$invalid_stderr" >/dev/null &&
        grep -F 'Usage:' "$invalid_stderr" >/dev/null ||
        fail "the staged executable did not preserve the release command-line error contract."
    [ ! -d "$bin_dir/noter" ] || fail "the install destination is a directory."
    mv -f "$stage_dir/noter" "$bin_dir/noter"
    installed_binary=$bin_dir/noter
    printf "Installed Noter %s at '%s'.\n" "$expected_version" "$installed_binary"
    path_hint "$bin_dir"
    install_linux_desktop "$installed_binary"
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
        install_linux_desktop "$install_root/bin/noter"
        return
    fi

    command -v tar >/dev/null 2>&1 || fail "tar is required to unpack the release."
    command -v xz >/dev/null 2>&1 || fail "xz is required to unpack the release; install xz-utils or xz."

    bin_dir=$install_root/bin
    cli_temp_dir=$(mktemp -d "${TMPDIR:-/tmp}/noter-install.XXXXXX")
    stage_dir=
    trap 'rm -rf "$cli_temp_dir"; if [ -n "$stage_dir" ]; then rm -rf "$stage_dir"; fi' EXIT HUP INT TERM

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

    prepare_trusted_bin_dir
    # Stage beside the destination and rename, so the install is atomic and a
    # running copy keeps its file until it exits.
    stage_dir=$(mktemp -d "$bin_dir/.noter.install.XXXXXXXX") ||
        fail "cannot reserve a private install stage."
    staged=$stage_dir/noter
    cp "$found_binary" "$staged"
    chmod 755 "$staged"
    staged_version=$("$staged" --version) || {
        fail "the downloaded binary did not run on this system."
    }
    [ "$staged_version" = "noter ${tag#v}" ] || {
        fail "the downloaded binary reported '$staged_version', expected 'noter ${tag#v}'."
    }
    [ ! -d "$bin_dir/noter" ] || fail "the install destination is a directory."
    mv -f "$staged" "$bin_dir/noter"
    printf "Installed %s at '%s'.\n" "$staged_version" "$bin_dir/noter"
    path_hint "$bin_dir"
    install_linux_desktop "$bin_dir/noter"
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
        remove_linux_desktop
        return
    fi

    if [ "$mode" = source ]; then
        install_from_source
    else
        install_from_release
    fi
}

main "$@"
