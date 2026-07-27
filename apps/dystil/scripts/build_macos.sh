#!/bin/bash
set -e

# Clean up any existing bundle
rm -rf src-tauri/target/release/bundle

# Build without signing
bun tauri build --no-sign

# Strip extended attributes from all files in the bundle
APP_PATH="src-tauri/target/release/bundle/macos/Dystil - Development.app"
xattr -cr "$APP_PATH"

# Sign the app manually
IDENTITY="Apple Development: Jayvardhan Patil (ChinuManu)"
codesign --force --deep --sign "$IDENTITY" "$APP_PATH"

echo "Build completed successfully!"
