# ⚡ HamAlert DX Receiver

A real-time DX spot desktop application powered by [HamAlert](https://hamalert.org). Displays incoming spots on a live world map with filtering, audio alerts, QRZ lookups, and bearing/distance calculations from your home QTH.

Built with [Tauri 2](https://tauri.app) (Rust + web frontend). Available for **macOS**, **Windows**, and **Linux**.

> **73 de SA0LEK**

---

## Screenshots

<!-- Add screenshots here -->

---

## Features

- 🗺️ **Live world map** — DX spots appear as color-coded markers by band
- 📡 **Real-time spots** via HamAlert Telnet connection
- 🔍 **Filter** by band, mode, continent, source, and age
- 📏 **Bearing & distance** from your home QTH to each DX station, with a beam line drawn on the map
- 🧑‍💻 **QRZ XML lookup** — exact callsign position, name and photo (requires QRZ subscription)
- 🔔 **Audio alerts** on new spots
- 🌙 **Dark / light theme**
- 🌐 **Language support** — English and Swedish
- 🔄 **Auto-updates** via GitHub Releases

---

## Requirements

- A free [HamAlert](https://hamalert.org) account
- At least one **trigger** configured in HamAlert (determines which spots you receive)
- Your HamAlert **Telnet password** (separate from your login password — found under Account → Telnet)

---

## Installation

### macOS

1. Download the `.dmg` file and `Installera macOS.command` from [Releases](../../releases/latest)
2. Open the `.dmg` and drag **HamAlert DX Receiver** to **Applications**
3. Double-click **`Installera macOS.command`** — it removes Gatekeeper's quarantine flag and launches the app

> **Note:** The app is not notarized by Apple. The helper script runs `xattr -cr` to bypass Gatekeeper. This is safe for open-source software you build yourself.

### Windows

Download the `.msi` installer from [Releases](../../releases/latest) and run it.

### Linux

Download the `.AppImage` from [Releases](../../releases/latest), make it executable and run:

```bash
chmod +x HamAlert_DX_Receiver_*.AppImage
./HamAlert_DX_Receiver_*.AppImage
```

---

## First-Run Setup

On first launch an onboarding wizard guides you through:

1. **Create a HamAlert account** at [hamalert.org/register](https://hamalert.org/register)
2. **Set up a trigger** at [hamalert.org/triggers](https://hamalert.org/triggers)
3. **Find your Telnet password** under Account → Telnet on hamalert.org
4. **Enter your callsign and locator** (Maidenhead grid square)
5. **Connect** — spots start flowing in immediately

---

## Building from Source

### Prerequisites

- [Node.js](https://nodejs.org) 20+
- [Rust](https://rustup.rs) (stable)
- On Linux: `libwebkit2gtk-4.1-dev`, `libappindicator3-dev`, `librsvg2-dev`, `patchelf`

### Run in development

```bash
cd desktop
npm install
npm run tauri dev
```

### Build a release

```bash
cd desktop
npm run tauri build
```

The built app is in `desktop/src-tauri/target/release/bundle/`.

---

## Releasing a New Version

Use the included release script from the repo root:

```bash
./release.sh 0.2.0
```

This will:
1. Bump the version in `tauri.conf.json`
2. Commit and push to the `tauri` branch
3. Create and push the `v0.2.0` tag
4. Trigger GitHub Actions to build for all platforms and publish a draft release

Sign your release in [GitHub Releases](../../releases) when the builds are complete.

---

## Auto-Updates

The app checks for updates via the GitHub Releases API. Updates are signed with [minisign](https://jedisct1.github.io/minisign/). Go to **Settings → Check for updates** inside the app.

---

## License

MIT © SA0LEK
