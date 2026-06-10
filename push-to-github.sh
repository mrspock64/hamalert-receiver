#!/bin/bash
# Kör detta script i terminalen på din Mac eller hamserver.local
# för att skapa ett GitHub-repo och pusha koden.
#
# Förutsättningar:
#   1. Git installerat: sudo apt install git  (Linux) / ingår i Xcode tools (Mac)
#   2. GitHub CLI installerat: https://cli.github.com/
#      eller: sudo apt install gh  /  brew install gh
#   3. Inloggad med GitHub CLI: gh auth login

set -e  # Avsluta vid fel

REPO_NAME="hamalert-receiver"
DESCRIPTION="HamAlert DX spot receiver med realtidskarta – Node.js + WebSocket + Leaflet"

echo "=== Skapar GitHub-repo och pushar HamAlert Receiver ==="

# Gå till projektmappen (anpassa sökvägen om du kör på hamserver)
cd "$(dirname "$0")"

# Initiera git om det inte redan finns
if [ ! -d ".git" ]; then
  git init
  echo "Git-repo initierat."
fi

# Skapa .gitignore om den inte finns
if [ ! -f ".gitignore" ]; then
  cat > .gitignore << 'EOF'
node_modules/
spots.json
.env
*.log
EOF
  echo ".gitignore skapad."
fi

# Lägg till alla filer och committa
git add .
git commit -m "Initial commit: HamAlert DX Receiver v1.0

- HTTP POST/GET webhook endpoint för HamAlert URL notifications
- WebSocket realtidsuppdatering till webbläsare
- Leaflet.js världskarta med DXCC-entiteter
- Maidenhead-locator → bäring/avstånd
- Band-färgkodning (160m–70cm)
- Ljust och mörkt tema
- Spot-persistens via spots.json
- Resizebar karta/tabell och kolumner
- QRZ.com-länk per spot
- systemd-service för autostart
- Mobil-responsiv design" 2>/dev/null || echo "(Inga nya ändringar att committa)"

# Skapa GitHub-repo med GitHub CLI och pusha
gh repo create "$REPO_NAME" \
  --description "$DESCRIPTION" \
  --private \
  --source=. \
  --remote=origin \
  --push

echo ""
echo "=== Klart! ==="
echo "Repo: https://github.com/$(gh api user --jq .login)/$REPO_NAME"
