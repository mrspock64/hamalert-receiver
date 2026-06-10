#!/usr/bin/env bash
# sync-frontend.sh
#
# Kopierar desktop/src/index.html (master) till:
#   server/public/index.html  – web server (Node.js + WebSocket)
#   shared/index.html         – referenskopia
#
# Kör efter ändringar i desktop/src/index.html:
#   ./sync-frontend.sh
#
# Frontenden hanterar båda kontexterna via window.__TAURI__-detection.
# Tauri-specifika UI-element (settings-panel, credentials-overlay) är
# ofarliga i webbläsarläge – de triggar inget utan Tauri runtime.

set -e
MASTER="desktop/src/index.html"
TARGETS=("server/public/index.html" "shared/index.html")

if [ ! -f "$MASTER" ]; then
  echo "Error: $MASTER not found. Run from repo root." >&2
  exit 1
fi

for target in "${TARGETS[@]}"; do
  cp "$MASTER" "$target"
  echo "✓ $target"
done

echo "Done – $(wc -l < "$MASTER") lines synced from $MASTER"
