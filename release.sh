#!/usr/bin/env bash
# release.sh – bumpar version, committar och skapar release-tagg
# Användning: ./release.sh 0.2.0
set -e

cd "$(dirname "$0")"

VERSION="${1}"
if [ -z "$VERSION" ]; then
  echo "Användning: ./release.sh <version>  (t.ex. ./release.sh 0.2.0)"
  exit 1
fi

CONF="desktop/src-tauri/tauri.conf.json"

echo "→ Uppdaterar version till $VERSION i $CONF..."
# Ersätt "version": "x.y.z" med ny version
sed -i '' "s/\"version\": \"[^\"]*\"/\"version\": \"$VERSION\"/" "$CONF"

echo "→ Lägger till ändrade filer..."
git add "$CONF" .github/workflows/release.yml desktop/ sync-frontend.sh 2>/dev/null || true

echo "→ Committar version-bump..."
git commit -m "Release v$VERSION" || echo "  (inget nytt att committa)"

echo "→ Pushar till GitHub..."
git push origin tauri

echo "→ Tar bort gammal tagg (om den finns)..."
git tag -d "v$VERSION" 2>/dev/null || true
git push origin ":refs/tags/v$VERSION" 2>/dev/null || true

echo "→ Skapar och pushar tagg v$VERSION..."
git tag "v$VERSION"
git push origin "v$VERSION"

echo ""
echo "✓ Klart! Version $VERSION är på väg att byggas:"
echo "  https://github.com/mrspock64/hamalert-receiver/actions"
