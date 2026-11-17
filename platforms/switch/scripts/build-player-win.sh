#!/bin/sh
# Windows/Git-Bash adaptation of build-player.sh (no Docker).
# Upstream sources are untouched; only path style and the linker wrapper differ.
set -eu
cd "$(dirname "$0")/../../.."
: "${DEVKITPRO:=C:/msys64/opt/devkitpro}"
: "${DEVKITA64:=$DEVKITPRO/devkitA64}"
export DEVKITPRO DEVKITA64
sh platforms/switch/scripts/prepare-libc.sh
for shader in scene_vsh:vert scene_fsh:frag present_vsh:vert present_fsh:frag; do
    name=${shader%%:*}
    "$DEVKITPRO/tools/bin/uam" -s "${shader##*:}" -o "platforms/switch/build/$name.dksh" "platforms/switch/runtime/shaders/$name.glsl"
done
"$DEVKITA64/bin/aarch64-none-elf-gcc" -c platforms/switch/runtime/shaders.S -o platforms/switch/build/shaders.o
for unit in window gpu audio system libc keyboard; do
    "$DEVKITA64/bin/aarch64-none-elf-gcc" -c "platforms/switch/runtime/$unit.c" -o "platforms/switch/build/$unit.o" \
        -Wall -Wextra -Werror -O2 -march=armv8-a+crc+crypto -mtune=cortex-a57 -mtp=soft -fPIE -I"$DEVKITPRO/libnx/include"
done
"$DEVKITA64/bin/aarch64-none-elf-gcc" -c platforms/switch/runtime/abi.c -o platforms/switch/build/abi.o
# Host build scripts/proc-macros: no MSVC on this machine. Use the windows-gnu host
# toolchain and link host artifacts with the MSYS2 ucrt64 GCC (Rust ships its own
# mingw CRT, so no Windows SDK is needed). The Switch cross-link is unaffected.
TOOLCHAIN=nightly-2026-01-20-x86_64-pc-windows-gnu
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="C:/msys64/ucrt64/bin/gcc.exe"
# ucrt64 ld.exe reads response files as ANSI: CJK bytes in paths get mangled.
# Keep all linker-visible artifacts (objects, rlibs, outputs) on an ASCII path.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-C:/cmvs-rs-target}"
cargo "+$TOOLCHAIN" rustc --config 'patch.crates-io.libc.path="platforms/switch/build/libc-0.2.178"' \
    --config 'profile.release.lto="fat"' --config 'profile.release.codegen-units=1' \
    -p cmvs-switch --release --target platforms/switch/toolchain/aarch64-switch.json -Zbuild-std=std,panic_abort -- \
    -C "linker=$PWD/platforms/switch/toolchain/linker.cmd" \
    -C "link-arg=-specs=$DEVKITPRO/libnx/switch.specs" \
    -C "link-arg=platforms/switch/build/window.o" \
    -C "link-arg=platforms/switch/build/gpu.o" \
    -C "link-arg=platforms/switch/build/shaders.o" \
    -C "link-arg=platforms/switch/build/audio.o" \
    -C "link-arg=platforms/switch/build/system.o" \
    -C "link-arg=platforms/switch/build/libc.o" \
    -C "link-arg=platforms/switch/build/keyboard.o" \
    -C "link-arg=-L$DEVKITPRO/libnx/lib" -C link-arg=-ldeko3d -C link-arg=-lnx -C link-arg=-lm
# devkitPro tools may also use ANSI argv: create NACP/NRO on the ASCII target dir,
# then copy into the repository with the (Unicode-safe) shell.
"$DEVKITPRO/tools/bin/nacptool" --create 'CMVS Switch' 'cmvs-rs' "${CMVS_VERSION:-0.1.0}" "$CARGO_TARGET_DIR/cmvs-switch.nacp"
"$DEVKITPRO/tools/bin/elf2nro" "$CARGO_TARGET_DIR/aarch64-switch/release/cmvs-switch" "$CARGO_TARGET_DIR/cmvs-switch.nro" --nacp="$CARGO_TARGET_DIR/cmvs-switch.nacp"
cp "$CARGO_TARGET_DIR/cmvs-switch.nro" platforms/switch/build/cmvs-switch.nro