#!/usr/bin/env bash
set -e

echo "=== YouTube Live Kokoro TTS Uninstaller & Cleanup ==="
echo "This script cleans up build artifacts, downloaded models, and optionally uninstalls Homebrew packages."

read -p "Do you want to clean build artifacts and models? [y/N]: " clean_build
if [[ "$clean_build" =~ ^[Yy]$ ]]; then
    echo "Cleaning target/ and models/..."
    rm -rf target/
    rm -rf models/
    echo "Local build artifacts cleaned."
fi

read -p "Do you want to uninstall Homebrew packages (rust, espeak-ng)? [y/N]: " uninstall_brew
if [[ "$uninstall_brew" =~ ^[Yy]$ ]]; then
    echo "Uninstalling brew packages..."
    brew uninstall espeak-ng || true
    brew uninstall rust || true
    echo "Running brew autoremove to clean orphaned dependencies..."
    brew autoremove || true
    echo "Brew packages uninstalled and cleaned."
fi

echo "Cleanup complete."
