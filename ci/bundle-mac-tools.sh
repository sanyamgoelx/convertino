#!/bin/bash
# Builds the converters that ship inside Convertino for Mac (no official Mac
# downloads exist for them): ImageMagick, Poppler and Ghostscript, taken from
# Homebrew and made self-contained. Every library they load is copied next to
# them and re-pointed with @rpath, data files are copied, and everything is
# signed ad hoc (Apple silicon won't run unsigned code).
#
#   ci/bundle-mac-tools.sh [output folder]   (default: src-tauri/mac-tools)
#
# Layout (tools.rs finds them; a .bundled file makes tools.rs set the data paths):
#   imagemagick/magick, lib/, modules/{coders,filters}/, etc/
#   poppler/bin/{pdftoppm,pdftotext,pdfseparate,pdfunite}, lib/, share/poppler/
#   ghostscript/bin/gs, lib/, share/ghostscript/
# Each also gets etc/fonts/fonts.conf (the Mac's own font folders).
set -euo pipefail

OUT="${1:-src-tauri/mac-tools}"
BREW="$(brew --prefix)"
brew list dylibbundler >/dev/null 2>&1 || brew install dylibbundler
for f in imagemagick poppler ghostscript; do brew list "$f" >/dev/null 2>&1 || brew install "$f"; done

rm -rf "$OUT"
mkdir -p "$OUT"

fonts_conf() {
  mkdir -p "$1/etc/fonts"
  cat > "$1/etc/fonts/fonts.conf" <<'XML'
<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "fonts.dtd">
<fontconfig>
  <dir>/System/Library/Fonts</dir>
  <dir>/Library/Fonts</dir>
  <dir prefix="xdg">fonts</dir>
  <dir>~/Library/Fonts</dir>
  <cachedir>~/Library/Caches/Convertino/fontconfig</cachedir>
</fontconfig>
XML
}

# copy <file> <dest dir>: a writable real copy
copy() { mkdir -p "$2"; cp -L "$1" "$2/"; chmod u+w "$2/$(basename "$1")"; }

# relocate <lib dir> <rpath for the programs> <files...>
relocate() {
  local lib="$1" rp="$2"; shift 2
  local args=()
  for f in "$@"; do args+=(-x "$f"); done
  local search=(-s "$BREW/lib")
  for d in "$BREW"/opt/*/lib; do search+=(-s "$d"); done
  # stdin closed: dylibbundler asks on the terminal when it can't find a library; fail instead.
  dylibbundler -of -cd -b "${args[@]}" -d "$lib" -p @rpath/ "${search[@]}" </dev/null >/tmp/dylibbundler.log 2>&1 \
    || { tail -40 /tmp/dylibbundler.log; echo "::error::dylibbundler failed for $lib"; exit 1; }
  for f in "$@"; do install_name_tool -add_rpath "$rp" "$f" 2>/dev/null || true; done
  for d in "$lib"/*.dylib; do
    [ -e "$d" ] || continue
    chmod u+w "$d"
    install_name_tool -add_rpath @loader_path "$d" 2>/dev/null || true
  done
}

# ---------- ImageMagick ----------
IM="$OUT/imagemagick"
IMP="$(brew --prefix imagemagick)"
copy "$IMP/bin/magick" "$IM"
mods=()
MODDIR="$(find "$IMP/lib" -maxdepth 3 -type d -name 'modules-*' | head -1 || true)"
if [ -n "$MODDIR" ]; then
  for kind in coders filters; do
    if [ -d "$MODDIR/$kind" ]; then
      mkdir -p "$IM/modules/$kind"
      cp "$MODDIR/$kind"/*.so "$IM/modules/$kind/" 2>/dev/null || true
      cp "$MODDIR/$kind"/*.la "$IM/modules/$kind/" 2>/dev/null || true
      chmod u+w "$IM/modules/$kind"/* || true
      for m in "$IM/modules/$kind"/*.so; do [ -e "$m" ] && mods+=("$m"); done
    fi
  done
fi
relocate "$IM/lib" @executable_path/lib "$IM/magick" ${mods[@]+"${mods[@]}"}
for m in ${mods[@]+"${mods[@]}"}; do install_name_tool -add_rpath @loader_path/../../lib "$m" 2>/dev/null || true; done
mkdir -p "$IM/etc"
cp "$IMP"/etc/ImageMagick-*/*.xml "$IM/etc/" 2>/dev/null || true
cp "$IMP"/share/ImageMagick-*/*.xml "$IM/etc/" 2>/dev/null || true
fonts_conf "$IM"
touch "$IM/.bundled"

# ---------- Poppler ----------
PO="$OUT/poppler"
POP="$(brew --prefix poppler)"
progs=()
for p in pdftoppm pdftotext pdfseparate pdfunite; do copy "$POP/bin/$p" "$PO/bin"; progs+=("$PO/bin/$p"); done
relocate "$PO/lib" @executable_path/../lib "${progs[@]}"
if [ -d "$BREW/share/poppler" ]; then mkdir -p "$PO/share"; cp -R "$BREW/share/poppler" "$PO/share/"; fi
fonts_conf "$PO"
touch "$PO/.bundled"

# ---------- Ghostscript ----------
GS="$OUT/ghostscript"
GSP="$(brew --prefix ghostscript)"
copy "$GSP/bin/gs" "$GS/bin"
relocate "$GS/lib" @executable_path/../lib "$GS/bin/gs"
mkdir -p "$GS/share"
cp -R "$GSP/share/ghostscript" "$GS/share/"
fonts_conf "$GS"
touch "$GS/.bundled"

# ---------- sign, and check that nothing still points into Homebrew ----------
chmod -R u+w "$OUT"
find "$OUT" -type f \( -perm -u+x -o -name '*.dylib' -o -name '*.so' \) -print0 | while IFS= read -r -d '' f; do
  if file "$f" | grep -q Mach-O; then
    codesign --force --sign - "$f" >/dev/null 2>&1 || echo "couldn't sign $f"
    if otool -L "$f" | tail -n +2 | grep -E "$BREW|/usr/local/(opt|Cellar)|/opt/homebrew" >/dev/null; then
      echo "STILL LINKED TO HOMEBREW: $f"; otool -L "$f"; exit 1
    fi
  fi
done

du -sh "$OUT"/*
echo "Bundled converters ready in $OUT"
