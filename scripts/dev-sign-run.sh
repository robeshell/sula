#!/bin/sh
# Cargo runner on macOS (see .cargo/config.toml). Before `cargo run` or
# `tauri dev` starts the Sula app, sign it with a fixed development identity:
# the keychain then keeps trusting it across rebuilds instead of asking for
# the saved API keys after every build.
#
# The identity comes from SULA_DEV_SIGN_IDENTITY, else from the git-ignored
# file .cargo/sula-dev-identity (one line, e.g. "Apple Development: Name (TEAMID)").
# Without either, the app runs unsigned as before. Test binaries are left alone.
set -e

bin="$1"
if [ "$(basename "$bin")" = "sula" ]; then
  identity="${SULA_DEV_SIGN_IDENTITY:-}"
  file="$(cd "$(dirname "$0")/.." && pwd)/.cargo/sula-dev-identity"
  if [ -z "$identity" ] && [ -f "$file" ]; then
    identity="$(head -n 1 "$file")"
  fi
  if [ -n "$identity" ]; then
    # Sign a copy and swap it in, so a failed signature never touches the build.
    signed="$bin.dev-signed"
    cp -p "$bin" "$signed"
    # macOS kills and deletes a binary signed by a revoked certificate, so check
    # the signature before it replaces the build.
    if output="$(codesign --force --sign "$identity" --identifier com.sula.app "$signed" 2>&1)" \
      && ! output="$(spctl --assess --type execute "$signed" 2>&1 | grep -i revoked)"; then
      mv -f "$signed" "$bin"
    else
      rm -f "$signed"
      echo "dev-sign-run: could not sign with \"$identity\": $output" >&2
      echo "dev-sign-run: running unsigned" >&2
    fi
  fi
fi

exec "$@"
