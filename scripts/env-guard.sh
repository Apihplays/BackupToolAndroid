#!/usr/bin/env bash
# env-guard.sh — Fail if the build environment could leak artifacts onto a device.
#
# Usage:
#   source scripts/env-guard.sh   # from repo root before running cargo
#   scripts/env-guard.sh          # standalone check (exit 1 = unsafe)
#
# Exits 0 when safe, 1 when the environment looks dangerous.

set -euo pipefail

FAIL=0

die() {
    echo "ENV-GUARD FAIL: $1" >&2
    FAIL=1
}

# --- Check CARGO_TARGET_DIR ---
if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
    case "$CARGO_TARGET_DIR" in
        */Android/media/*|*/sdcard/*|*/storage/emulated/*|*/storage/*)
            die "CARGO_TARGET_DIR='$CARGO_TARGET_DIR' points at a device-mounted path"
            ;;
    esac
fi

# --- Check CWD ---
CWD="$(pwd -P 2>/dev/null || echo "$PWD")"
case "$CWD" in
    */Android/media/*|*/sdcard/*|*/storage/emulated/*)
        die "PWD='$CWD' is inside a device-mounted path"
        ;;
esac

# --- Check TMPDIR ---
if [[ -n "${TMPDIR:-}" ]]; then
    case "$TMPDIR" in
        */Android/media/*|*/sdcard/*|*/storage/emulated/*)
            die "TMPDIR='$TMPDIR' points at a device-mounted path"
            ;;
    esac
fi

# --- Check HOME (app sandbox heuristic) ---
case "${HOME:-}" in
    */Android/data/*|*/storage/emulated/*)
        die "HOME='$HOME' looks like an Android app sandbox"
        ;;
esac

# --- Check for stray CARGO or TARGET overrides ---
for var in CARGO_HOME RUSTUP_HOME; do
    val="${!var:-}"
    if [[ -n "$val" ]]; then
        case "$val" in
            */Android/media/*|*/sdcard/*|*/storage/emulated/*)
                die "$var='$val' points at a device-mounted path"
                ;;
        esac
    fi
done

if [[ $FAIL -eq 0 ]]; then
    echo "ENV-GUARD OK: build environment looks safe"
    exit 0
else
    exit 1
fi
