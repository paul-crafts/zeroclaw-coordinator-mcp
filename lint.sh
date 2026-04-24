#!/bin/bash
set -e

echo "🔍 Running Quality Checks..."

echo "📦 Running cargo fmt..."
cargo fmt --all

echo "📎 Running cargo clippy..."
cargo clippy -- -D warnings

echo "🧪 Running cargo test..."
cargo test

echo "✅ All checks passed!"
