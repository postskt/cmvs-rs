#!/bin/sh
# One-click NSP packer for the cmvs-rs Switch player.
# Follows a hardware-proven NSP packing manual strictly (not bundled):
#   golden NPDM (title id patched only) + elf2nso NSO + nacptool/UTF-8 NACP
#   + 256x256 JPEG icons + ASCII workdir (romfs junction) + hacBrewPack
#   + pack_check.py gate (0 hard failures required).
# The resulting NSP has placeholder signatures: the console needs sigpatches.
#
# Usage (Git Bash):
#   sh platforms/switch/scripts/pack-nsp.sh <游戏目录> [options]
#     --mode a|b        a = pack the game data into romfs (default, big NSP)
#                       b = romfs holds only target.txt; game data stays in
#                           sdmc:/switch/cmvs/<游戏目录名>/ (tiny NSP)
#     --title "标题"    home-menu name (default: game folder name)
#     --author "作者"   default: cmvs-rs
#     --titleid HEX     default: 0100C65000030000 (0100XXXXXXXX0000, avoid real games)
#     --icon <image>    default: platforms/switch/build/icon-default.jpg
#     --version X.Y.Z   default: 1.0.0
#     --no-build        reuse the existing cross-compiled ELF
#     --out <dir>       where the final .nsp is copied (default: platforms/switch/build/nsp)
#
# Environment overrides: DEVKITPRO, CARGO_TARGET_DIR, CMVS_PACK_DIR (ASCII!),
# CMVS_PYTHON, CMVS_KEYS, CMVS_GOLDEN_NPDM, CMVS_HACBREWPACK, CMVS_HANDOVER.
set -eu
cd "$(dirname "$0")/../../.."

: "${DEVKITPRO:=C:/msys64/opt/devkitpro}"
: "${CARGO_TARGET_DIR:=C:/cmvs-rs-target}"
: "${CMVS_PACK_DIR:=C:/cmvs-nsp-pack}"        # must stay ASCII: hacBrewPack argv is ANSI
: "${CMVS_PYTHON:=python}"                    # Python 3.8+ with Pillow, on PATH
# Bring your own NSP toolchain (dumped prod.keys, golden reference NPDM,
# hacBrewPack, pack_check.py + patch_npdm_titleid.py) — none of it is bundled
# with this repository. See NSP-PACKING.md for what each variable must point to.
: "${CMVS_KEYS:?set CMVS_KEYS to your prod.keys (dumped from your own console)}"
: "${CMVS_GOLDEN_NPDM:?set CMVS_GOLDEN_NPDM to the hardware-proven golden main.npdm}"
: "${CMVS_HACBREWPACK:?set CMVS_HACBREWPACK to your hacbrewpack binary}"
: "${CMVS_HANDOVER:?set CMVS_HANDOVER to the tool dir holding pack_check.py and patch_npdm_titleid.py}"
export CARGO_TARGET_DIR
# Keep Python's log output UTF-8 (the Windows console defaults to GBK).
export PYTHONIOENCODING=utf-8

die() { echo "pack-nsp: $*" >&2; exit 1; }

MODE=a
TITLE=
AUTHOR=cmvs-rs
TITLEID=0100C65000030000
ICON=platforms/switch/build/icon-default.jpg
VERSION=1.0.0
BUILD=1
OUT=platforms/switch/build/nsp
GAME=
while [ $# -gt 0 ]; do
    case "$1" in
        --mode) MODE=$2; shift 2 ;;
        --title) TITLE=$2; shift 2 ;;
        --author) AUTHOR=$2; shift 2 ;;
        --titleid) TITLEID=$2; shift 2 ;;
        --icon) ICON=$2; shift 2 ;;
        --version) VERSION=$2; shift 2 ;;
        --no-build) BUILD=0; shift ;;
        --out) OUT=$2; shift 2 ;;
        -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
        -*) die "unknown option $1" ;;
        *) [ -z "$GAME" ] || die "only one game directory allowed"; GAME=$1; shift ;;
    esac
done
[ -n "$GAME" ] || { sed -n '2,30p' "$0"; exit 1; }
[ -d "$GAME" ] || die "game directory not found: $GAME"
GAME=$(cd "$GAME" && pwd)
NAME=$(basename "$GAME")
[ -n "$TITLE" ] || TITLE=$NAME
case "$MODE" in a|b) ;; *) die "--mode must be a or b" ;; esac
case "$TITLEID" in
    0100????????0000) ;;
    *) die "title id must look like 0100XXXXXXXX0000 (application range), got $TITLEID" ;;
esac
[ -f "$ICON" ] || die "icon not found: $ICON"
[ -f "$CMVS_KEYS" ] || die "prod.keys not found: $CMVS_KEYS"
[ -f "$CMVS_GOLDEN_NPDM" ] || die "golden NPDM not found: $CMVS_GOLDEN_NPDM"
[ -f "$CMVS_HACBREWPACK" ] || die "hacBrewPack not found: $CMVS_HACBREWPACK"
[ -f "$CMVS_HANDOVER/pack_check.py" ] || die "pack_check.py not found in $CMVS_HANDOVER"

if [ "$MODE" = a ]; then
    # Same markers the player's Mode A detection uses (GameFiles::discover).
    [ -f "$GAME/data/pack/script.cpz" ] || die "mode a: $GAME/data/pack/script.cpz missing"
    n=0
    [ -f "$GAME/data/pack/start.ps3" ] && n=$((n+1))
    [ -f "$GAME/data/pack/start.ps2" ] && n=$((n+1))
    [ "$n" = 1 ] || die "mode a: need exactly one of data/pack/start.ps3 / start.ps2"
else
    case "$NAME" in
        *[\\/]*|.|..) die "mode b: folder name cannot be used as target.txt: $NAME" ;;
    esac
fi

# 1) Cross-compile (also refreshes the .nro). The NSO is made from the same ELF.
ELF="$CARGO_TARGET_DIR/aarch64-switch/release/cmvs-switch"
if [ "$BUILD" = 1 ]; then
    sh platforms/switch/scripts/build-player-win.sh
fi
[ -f "$ELF" ] || die "ELF not found: $ELF (run without --no-build)"

# 2) ASCII workdir. Never run the packers on a CJK path.
mkdir -p "$CMVS_PACK_DIR/exefs" "$CMVS_PACK_DIR/control"
rm -rf "$CMVS_PACK_DIR/nca" "$CMVS_PACK_DIR/nsp"
mkdir -p "$CMVS_PACK_DIR/nca" "$CMVS_PACK_DIR/nsp"
cp "$CMVS_KEYS" "$CMVS_PACK_DIR/prod.keys"

# 3) ExeFS: NSO + golden NPDM with only the title id changed (3 places).
"$DEVKITPRO/tools/bin/elf2nso" "$ELF" "$CMVS_PACK_DIR/exefs/main"
"$CMVS_PYTHON" - "$CMVS_GOLDEN_NPDM" <<'EOF'
import hashlib, sys
sha = hashlib.sha256(open(sys.argv[1], 'rb').read()).hexdigest()
if not sha.startswith('bbbc0e6d7f154ab2'):
    raise SystemExit(f'golden NPDM sha256 {sha[:16]}... is not the hardware-proven bbbc0e6d7f154ab2...')
print(f'golden NPDM verified: sha256 {sha[:16]}...')
EOF
"$CMVS_PYTHON" "$CMVS_HANDOVER/patch_npdm_titleid.py" \
    "$CMVS_GOLDEN_NPDM" "$CMVS_PACK_DIR/exefs/main.npdm" "$TITLEID" --name Application

# 4) Control: nacptool with an ASCII placeholder (devkitPro argv may be ANSI),
#    then the Unicode-safe patch writes the real title into all 16 slots.
#    --titleid is essential: hacBrewPack takes the NSP title id from the NACP
#    (0x3038); without it every NCA/CNMT gets id 0 and pack_check fails.
"$DEVKITPRO/tools/bin/nacptool" --create 'CMVS Game' 'cmvs-rs' "$VERSION" \
    "$CMVS_PACK_DIR/control/control.nacp" --titleid="$TITLEID"
"$CMVS_PYTHON" platforms/switch/scripts/patch_nacp.py "$CMVS_PACK_DIR/control/control.nacp" \
    --title "$TITLE" --author "$AUTHOR" --version "$VERSION" --titleid "$TITLEID"
"$CMVS_PYTHON" - "$ICON" "$CMVS_PACK_DIR/control/icon.jpg" <<'EOF'
import sys
from PIL import Image
im = Image.open(sys.argv[1]).convert('RGB')
if im.size != (256, 256):
    im = im.resize((256, 256), Image.LANCZOS)
im.save(sys.argv[2], 'JPEG', quality=90)
print('icon: 256x256 JPEG')
EOF
cp "$CMVS_PACK_DIR/control/icon.jpg" "$CMVS_PACK_DIR/control/icon_AmericanEnglish.dat"

# 5) RomFS. Mode a: junction to the real game data (no multi-GB copy; the
#    junction itself lives on the ASCII workdir). Mode b: just target.txt.
#    cmd/rmdir removes a junction WITHOUT touching its target; rm -rf must
#    never be used on a junction (Git Bash would recurse into the game data).
PACKW=$(cygpath -w "$CMVS_PACK_DIR")
GAMEW=$(cygpath -w "$GAME")
"$CMVS_PYTHON" - "$MODE" "$PACKW" "$GAMEW" "$NAME" <<'EOF'
import ctypes, os, shutil, subprocess, sys
mode, packw, gamew, name = sys.argv[1:5]
link = os.path.join(packw, 'romfs')
if os.path.lexists(link):
    # Plain rmdir removes a junction (without touching its target) or an empty
    # dir. If it fails, the only safe case left is a REAL non-empty dir: never
    # rmtree a reparse point, that would delete the game data through it.
    subprocess.run(['cmd', '/c', 'rmdir', link], check=False,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if os.path.lexists(link):
        attrs = ctypes.windll.kernel32.GetFileAttributesW(link)
        if attrs != -1 and attrs & 0x400:  # FILE_ATTRIBUTE_REPARSE_POINT
            raise SystemExit(f'{link} is a junction that rmdir could not remove; fix it manually')
        shutil.rmtree(link)
if mode == 'a':
    r = subprocess.run(['cmd', '/c', 'mklink', '/J', link, gamew])
    if r.returncode != 0:
        raise SystemExit('mklink /J failed')
    print(f'romfs junction -> {gamew}')
else:
    os.makedirs(link)
    with open(os.path.join(link, 'target.txt'), 'w', encoding='utf-8', newline='\n') as f:
        f.write(name + '\n')
    print(f'romfs target.txt -> sdmc:/switch/cmvs/{name}/')
EOF

# 6) Pack (hacBrewPack = the hardware-verified route). ASCII Windows paths only.
#    hacBrewPack leaves hacbrewpack_temp in its CWD and reuses stale files
#    (an old CNMT once ended up inside the meta NCA): run it from the workdir
#    and wipe the temp dir first.
rm -rf "$CMVS_PACK_DIR/hacbrewpack_temp"
(cd "$CMVS_PACK_DIR" && MSYS2_ARG_CONV_EXCL='*' "$CMVS_HACBREWPACK" -k "$PACKW\\prod.keys" \
    --exefsdir "$PACKW\\exefs" --romfsdir "$PACKW\\romfs" --controldir "$PACKW\\control" \
    --ncadir "$PACKW\\nca" --nspdir "$PACKW\\nsp" --nologo)

# 7) Gate: pack_check.py must report 0 hard failures (exit code 0).
set -- "$CMVS_PACK_DIR"/nsp/*.nsp
[ -f "$1" ] || die "hacBrewPack produced no NSP in $CMVS_PACK_DIR/nsp"
NSP=$1
if ! "$CMVS_PYTHON" "$CMVS_HANDOVER/pack_check.py" "$NSP" --keys "$CMVS_KEYS" --romfs; then
    die "pack_check.py reported hard failures; the NSP was NOT copied out"
fi

# 8) Copy out with a friendly name (the shell is Unicode-safe).
mkdir -p "$OUT"
FINAL="$OUT/${TITLE}[${TITLEID}][v0].nsp"
cp "$NSP" "$FINAL"
echo
echo "pack-nsp: done"
echo "  NSP : $FINAL"
echo "  size: $(wc -c < "$FINAL") bytes"
echo "  mode: $MODE  title id: $TITLEID"
echo "  NOTE: placeholder signatures -> the console must have sigpatches installed."
if [ "$MODE" = b ]; then
    echo "  NOTE: mode b needs the game data at sdmc:/switch/cmvs/$NAME/ on the SD card."
fi