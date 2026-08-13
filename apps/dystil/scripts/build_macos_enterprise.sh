#!/usr/bin/env bash

set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"
ENV_FILE="${ENV_FILE:-$APP_DIR/.env.build}"

warning() {
	printf '\033[1;33m[WARN]\033[0m %s\n' "$*" >&2
}

if [[ ! -f "$ENV_FILE" ]]; then
	warning "Build environment file not found: $ENV_FILE"
	warning "Copy apps/dystil/.env.build.example to .env.build or set ENV_FILE=/path/to/.env.build."
	exit 1
fi

printf 'Loading build environment from %s\n' "$ENV_FILE"

# .env.build is intentionally shell-compatible so private keys can be loaded
# from a local file with, for example, TAURI_SIGNING_PRIVATE_KEY_FILE="$HOME/.tauri/dystil.key".
unset DYSTIL_CLOUD_BASE_URL \
	DYSTIL_TELEMETRY_ENDPOINT \
	TAURI_SIGNING_PRIVATE_KEY \
	TAURI_SIGNING_PRIVATE_KEY_FILE \
	TAURI_SIGNING_PRIVATE_KEY_PASSWORD \
	APPLE_SIGNING_IDENTITY \
	APPLE_ID \
	APPLE_PASSWORD \
	APPLE_TEAM_ID
set -a
# shellcheck disable=SC1090
source "$ENV_FILE"
set +a

# The production Tauri config is selected explicitly below. Keep the generated
# frontend/Rust app config on the same channel as the bundle.
export DYSTIL_BUILD_CHANNEL=prod
export DYSTIL_RELEASE_TARGET=aarch64-apple-darwin

# A key file is safer and easier to maintain locally than putting the complete
# multiline private key directly in .env.build.
if [[ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" && -n "${TAURI_SIGNING_PRIVATE_KEY_FILE:-}" ]]; then
	key_file="$TAURI_SIGNING_PRIVATE_KEY_FILE"
	if [[ -r "$key_file" ]]; then
		export TAURI_SIGNING_PRIVATE_KEY="$(<"$key_file")"
	else
		warning "TAURI_SIGNING_PRIVATE_KEY_FILE is not readable: $key_file"
	fi
fi

if [[ "$(uname -s)" == "Darwin" && -z "${APPLE_SIGNING_IDENTITY:-}" ]]; then
	APPLE_SIGNING_IDENTITY="$(security find-identity -v -p codesigning |
		awk -F'"' '/Developer ID Application/ { print $2; exit }')"
	export APPLE_SIGNING_IDENTITY
fi

required_vars=(
	DYSTIL_CLOUD_BASE_URL
	DYSTIL_TELEMETRY_ENDPOINT
	TAURI_SIGNING_PRIVATE_KEY
	TAURI_SIGNING_PRIVATE_KEY_PASSWORD
	APPLE_SIGNING_IDENTITY
	APPLE_ID
	APPLE_PASSWORD
	APPLE_TEAM_ID
)
missing_vars=()

for variable in "${required_vars[@]}"; do
	if [[ -z "${!variable:-}" ]]; then
		missing_vars+=("$variable")
	fi
done

if (( ${#missing_vars[@]} > 0 )); then
	warning "Required build values are missing: ${missing_vars[*]}"
	warning "No build was started. Check $ENV_FILE and try again."
	exit 1
fi

if [[ "$(uname -s)" != "Darwin" ]]; then
	warning "This script must run on macOS; ARM64 macOS bundles cannot be built with the current Linux toolchain."
	exit 1
fi

if ! security find-identity -v -p codesigning | grep -Fq "\"$APPLE_SIGNING_IDENTITY\""; then
	warning "APPLE_SIGNING_IDENTITY was not found in the current Mac keychain: $APPLE_SIGNING_IDENTITY"
	warning "Run: security find-identity -v -p codesigning"
	exit 1
fi

cd "$APP_DIR"
printf 'Building production enterprise ARM64 macOS bundle...\n'

bunx tauri build \
	--config src-tauri/tauri.prod.conf.json \
	--target aarch64-apple-darwin \
	--features enterprise-client

printf 'Build completed. Bundles are under src-tauri/target/aarch64-apple-darwin/release/bundle/\n'
