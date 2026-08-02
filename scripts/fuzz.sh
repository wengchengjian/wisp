#!/usr/bin/env bash
set -euo pipefail

case "$(uname -s)" in
  Linux|Darwin)
    ;;
  *)
    echo "cargo-fuzz is only supported on Unix (Linux/macOS)" >&2
    exit 1
    ;;
esac

exec cargo +nightly fuzz "$@"
