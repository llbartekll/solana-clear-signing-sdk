#!/usr/bin/env bash
# Single source of truth for the Apple build (mirrors the reference lib):
# builds all slices AND regenerates the Swift bindings in the same run, so
# generated code and binary never drift.
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ "$(uname -s)" != Darwin ]]; then
  echo "Apple builds require macOS with full Xcode installed." >&2
  exit 1
fi
for tool in cargo rustup xcodebuild xcrun lipo; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "Missing $tool. See README.md: Build and test / Prerequisites." >&2
    exit 1
  fi
done
if ! xcrun --sdk iphoneos --show-sdk-path >/dev/null 2>&1 ||
   ! xcrun --sdk iphonesimulator --show-sdk-path >/dev/null 2>&1; then
  echo "Select full Xcode in Xcode > Settings > Locations > Command Line Tools and install its iOS platform." >&2
  exit 1
fi
installed_targets="$(rustup target list --installed)"
missing_targets=()
for target in aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios; do
  if ! grep -Fxq "$target" <<< "$installed_targets"; then
    missing_targets+=("$target")
  fi
done
if (( ${#missing_targets[@]} )); then
  echo "Missing Rust targets. Run: rustup target add ${missing_targets[*]}" >&2
  exit 1
fi

LIB=libsolana_clearsign.a
DYLIB=libsolana_clearsign.dylib
GEN=bindings/swift/generated
OUT=target/ios

echo "▸ rust release builds (feature: uniffi)"
cargo build --release --features uniffi --target aarch64-apple-ios
cargo build --release --features uniffi --target aarch64-apple-ios-sim
cargo build --release --features uniffi --target x86_64-apple-ios
cargo build --release --features uniffi   # host: bindgen input + macOS slice

echo "▸ swift bindings (uniffi library mode)"
rm -rf "$GEN" && mkdir -p "$GEN"
cargo run --release --features bindgen --bin uniffi-bindgen -- \
  generate --library "target/release/$DYLIB" --language swift --out-dir "$GEN"

echo "▸ headers + modulemap"
HDR=target/ios-headers
rm -rf "$HDR" && mkdir -p "$HDR"
cp "$GEN"/*.h "$HDR"/
cat "$GEN"/*.modulemap > "$HDR/module.modulemap"   # xcframework expects module.modulemap

echo "▸ universal simulator lib"
mkdir -p target/universal-sim
lipo -create \
  "target/aarch64-apple-ios-sim/release/$LIB" \
  "target/x86_64-apple-ios/release/$LIB" \
  -output "target/universal-sim/$LIB"

echo "▸ xcframework (ios + ios-sim + macos)"
rm -rf "$OUT/SolanaClearsignFFI.xcframework"
mkdir -p "$OUT"
xcodebuild -create-xcframework \
  -library "target/aarch64-apple-ios/release/$LIB" -headers "$HDR" \
  -library "target/universal-sim/$LIB" -headers "$HDR" \
  -library "target/release/$LIB" -headers "$HDR" \
  -output "$OUT/SolanaClearsignFFI.xcframework"

echo "✓ $OUT/SolanaClearsignFFI.xcframework + $GEN bindings"
