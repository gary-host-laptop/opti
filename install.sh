#!/usr/bin/env bash
# Install opti (Rust/GTK4 rewrite) into a user-local XDG prefix.
#
# Defaults to ~/.local (user install; no root needed). Override with
#   ./install.sh --prefix /usr/local
#
# Installs:
#   <prefix>/bin/opti
#   <prefix>/share/applications/io.github.gary-host-laptop.opti.desktop
#   <prefix>/share/icons/hicolor/{S}x{S}/apps/io.github.gary-host-laptop.opti.png
#
# Note the icon dirs use <size>x<size> (e.g. 48x48) per the freedesktop Icon
# Theme Spec; GNOME will not resolve ~/icons/hicolor/48/apps/...
set -euo pipefail

PREFIX="${PREFIX:-$HOME/.local}"

while [ $# -gt 0 ]; do
    case "$1" in
        --prefix) PREFIX="$2"; shift 2 ;;
        --prefix=*) PREFIX="${1#*=}"; shift ;;
        *) echo "usage: install.sh [--prefix DIR]" >&2; exit 2 ;;
    esac
done

cd "$(dirname "$0")"
BINNAME="opti"
APPID="io.github.gary-host-laptop.opti"
DESKTOP="packaging/$APPID.desktop"

echo "==> building release binary"
cargo build --release

echo "==> installing into $PREFIX"

# Binaries.
install -d "$PREFIX/bin"
install -m 0755 "target/release/$BINNAME" "$PREFIX/bin/$BINNAME"

# Desktop entry.
install -d "$PREFIX/share/applications"
sed "s|__PREFIX__|$PREFIX|g" "$DESKTOP" > "$PREFIX/share/applications/$APPID.desktop"

# Icons, in freedesktop hicolor <size>x<size>/apps layout. Source files are
# already named <size>x<size>.png (e.g. 48x48.png) so the dir is ${size}/apps.
hicolor="$PREFIX/share/icons/hicolor"
install -d "$hicolor"
opti_sizes=""
for f in build/icons/*.png; do
    size="$(basename "$f" .png)"        # e.g. 48x48
    dir="$hicolor/${size}/apps"
    install -d "$dir"
    install -m 0644 "$f" "$dir/$APPID.png"
    opti_sizes="$opti_sizes ${size}/apps"
done

# index.theme handling. NEVER overwrite an existing file — the hicolor dir is
# community-owned (other apps may already have installed icons and a valid
# index.theme there). A broken/empty Directories= list makes gtk-update-icon-cache
# build a near-empty cache, which then hides every app's icon (the cache is
# trusted over a live directory scan once it exists).
ensure_theme_index() {
    local idx="$1"                       # "$hicolor/index.theme"
    # Gather sizes that actually exist on disk with an apps/ folder now.
    local dirs=""
    for d in "$hicolor"/*/apps; do
        [ -d "$d" ] || continue
        local b="$(basename "$(dirname "$d")")"
        dirs="$dirs $b/apps"
    done
    [ -d "$hicolor/scalable/apps" ] && dirs="$dirs scalable/apps"
    # de-dup, keep stable order
    dirs="$(echo $dirs | tr ' ' '\n' | sort -u | tr '\n' ' ')"
    # We register the union of what's on disk plus opti's own sizes.
    dirs="$(echo $dirs $opti_sizes | tr ' ' '\n' | sort -u | tr '\n' ' ')"

    if [ ! -f "$idx" ]; then
        # Fresh theme: write full enumeration from disk.
        {
            echo "[Icon Theme]"
            echo "Name=Hicolor"
            echo "Comment=Fallback icon theme"
            printf 'Directories=%s\n' "$(echo $dirs | sed 's/ *$//' | tr ' ' ',')"
            for s in $dirs; do
                echo
                echo "[$s]"
                case "$s" in
                    scalable/apps)
                        echo "Type=Scalable"
                        echo "MinSize=1"
                        echo "MaxSize=512"
                        ;;
                    *)
                        num="${s%/apps}"; num="${num%%x*}"
                        echo "Size=$num"
                        ;;
                esac
                echo "Context=Applications"
            done
        } > "$idx"
        return
    fi

    # Existing file: append opti's size entries only if each specific size is
    # missing from Directories=. Never rewrite or empty pre-existing data.
    local have=""
    local list=""
    have="$(sed -n 's/^Directories=//p' "$idx" | head -1)"
    list="$have"
    for s in $opti_sizes; do
        # shell substring check: is "$s" already one of the comma-separated dirs?
        if ! printf '%s' ",$list," | grep -q ",$s,"; then
            if [ -n "$list" ]; then
                list="$list,$s"
            else
                list="$s"
            fi
            # append the size stanza if not already present
            if ! grep -q "^\[$s\]" "$idx"; then
                {
                    echo
                    echo "[$s]"
                    case "$s" in
                        scalable/apps)
                            echo "Type=Scalable"
                            echo "MinSize=1"
                            echo "MaxSize=512"
                            ;;
                        *)
                            num="${s%/apps}"; num="${num%%x*}"
                            echo "Size=$num"
                            ;;
                    esac
                    echo "Context=Applications"
                } >> "$idx"
            fi
        fi
    done
    # Update Directories= line only if we appended to it.
    if [ "$list" != "$have" ]; then
        sed -i "s|^Directories=.*|Directories=$list|" "$idx"
    fi
}
ensure_theme_index "$hicolor/index.theme"

echo "==> done. binary at $PREFIX/bin/$BINNAME"
echo "==> refreshing caches (best-effort)"

# Best-effort cache refresh; failures are non-fatal (may not be installed).
command -v gtk-update-icon-cache >/dev/null && \
    gtk-update-icon-cache -f "$PREFIX/share/icons/hicolor" || true
command -v update-desktop-database >/dev/null && \
    update-desktop-database "$PREFIX/share/applications" || true

echo "==> launch with: gtk-launch $APPID"
