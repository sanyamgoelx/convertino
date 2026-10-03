#!/usr/bin/env bash
# Downloads camera RAW samples for the tests into test-files/raw (or $1).
# Public domain photos from raw.pixls.us, mirrored on GitHub by
# github.com/sdcb/Sdcb.LibRaw.TestData. Each file is checked against its SHA-256.
set -euo pipefail
dest="${1:-test-files/raw}"
mkdir -p "$dest"
base="https://raw.githubusercontent.com/sdcb/Sdcb.LibRaw.TestData/main/files"
sha() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }
while read -r sum name; do
  [ -z "$name" ] && continue
  f="$dest/$name"
  if [ -f "$f" ] && [ "$(sha "$f")" = "$sum" ]; then echo "have $name"; continue; fi
  for try in 1 2 3; do
    curl -fsSL --retry 2 -o "$f" "$base/$name" && [ "$(sha "$f")" = "$sum" ] && break
    echo "retrying $name ($try)"; rm -f "$f"; sleep 5
  done
  [ -f "$f" ] || { echo "couldn't download $name"; exit 1; }
  echo "got  $name"
done <<'LIST'
5d826ccdde8f31569c4f147f97610e423107cba3cbb26edcb89df86055824d97 lossless_dng_load_raw_M14-1451_000085_cDNG_compressed.dng
80ae0b8fdce3f58286fc194513c6ec0080ab07caca753408b00ca72c90e5ad0c packed_dng_load_raw_A002_653_20240229_000001_2k_8bit.DNG
74abb0a113d075ad9887a058082f40dd2a938c4813a08474d82356f11a027778 crxLoadRaw_Canon_EOS_R6_CRAW_ISO_100_crop_nodual.CR3
f39be5acf14057ce04527fd8d0574e57ec8ed5fed082d7bfaff763075a44ee8a canon_load_raw_crw_1693.crw
155edb938f884ea7372ce98d4ff5f965c3e413b43b95bc9923da6e92082cf914 nikon_load_raw_D2H_3957.NEF
a35ebb2fbec929daa5beb20d1ce5c15a8aac7b1a7a231455387f3df8a7442e07 sony_arw2_load_raw_DSC04126.ARW
d142a23aca836053ed53e9ce3cb3ed2d434541d734d71a94a6eefadcd08bd31b panasonic_load_raw_P1020372.RW2
9419d1408ebf850395e5dd563beb96309466546252caf541466034c9cb7e9724 fuji_compressed_load_raw_Fujifilm-X-S10-compressed.RAF
ba4605bbd9fe9b77971da444bcdb1698904c0ad4ebdf9dd42dd45ab255f3f6d4 olympus_load_raw_E_3__3099164_casco_antiguo_petrer.ORF
e35ae4154a468be3154f5f462e884ba5941f010d3e8f23d347fbec14809f44d3 pentax_load_raw__IGP7284.PEF
LIST
echo "RAW samples ready in $dest"
