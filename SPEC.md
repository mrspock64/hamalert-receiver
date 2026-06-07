# HamAlert DX Receiver — Technical Specification

**Server:** hamserver.local  
**Version:** 1.0  
**Date:** 2026-06-07

---

## 1. Syfte

En lokal webbsida på `hamserver.local` som tar emot DX-spots från HamAlert och visar dem i realtid med en interaktiv karta. Användaren anger sitt eget callsign och Maidenhead-locator — sidan beräknar då riktning och avstånd till varje spottad station och ritar ut dem på en världskarta.

---

## 2. HamAlert-anslutning — valt format: Telnet

### Varför Telnet (inte HTTP POST)?

| Format | Pro | Con |
|---|---|---|
| **Telnet** (port 7373) | Servern kopplar *ut* till HamAlert — fungerar bakom NAT utan portforward | Kräver alltid inloggning |
| HTTP POST webhook | Enkel integration | HamAlert.org når inte `hamserver.local` inifrån LAN utan tunnel |

**Valt: Telnet.** Servern öppnar en TCP-anslutning till `hamalert.org:7373`, autentiserar med HamAlert-username/lösenord, och tar emot spots som klassisk DX-kluster-text:

```
DX de SM5ABC:    14025.0  OH2BH         CW 599 73          1234Z
```

Fälten (space-separerade, fast format):
- `DX de <spotter>:` — den som spottade
- `<frequency kHz>` — frekvens
- `<callsign>` — den spottade stationen
- `<comment>` — fri text (mode, rapport, info)
- `<HHMM>Z` — UTC-tid

HamAlert skickar även ett JSON-objekt per spot på telnet-strömmen (nyare variant) — se avsnitt 4.

---

## 3. Arkitektur

```
HamAlert.org:7373 (Telnet)
        │  TCP outbound
        ▼
┌──────────────────────────────┐
│  hamserver.local             │
│                              │
│  Backend (Node.js / Python)  │
│  ┌──────────────────────┐    │
│  │  Telnet-klient       │    │
│  │  Parser + buffer     │    │
│  │  WebSocket-server    │    │
│  └──────────────────────┘    │
│  Port 8080 HTTP + WS         │
└──────────────────────────────┘
        │  WebSocket ws://hamserver.local:8080
        ▼
┌──────────────────────────────┐
│  Webbläsare                  │
│  Single-page app             │
│  Leaflet.js karta            │
│  Spot-tabell                 │
└──────────────────────────────┘
```

### Teknikval

- **Backend:** Node.js (enklast för WebSocket + Telnet i ett)
- **Frontend:** Vanilla HTML/CSS/JS — inga build-steg, enkel deploy
- **Karta:** [Leaflet.js](https://leafletjs.com/) + OpenStreetMap-tiles
- **Locator → koordinater:** Maidenhead-algoritm (JS, inbyggd)
- **DXCC-koordinater:** Inbyggd JSON-lookup (cty.dat / Country-filer) för att placera spottade stationer på kartan

---

## 4. Datamodell — ett spot-objekt

```json
{
  "id": "uuid",
  "timestamp": "2026-06-07T12:34:00Z",
  "spotter": "SM5ABC",
  "callsign": "OH2BH",
  "frequency": 14025.0,
  "band": "20m",
  "mode": "CW",
  "comment": "599 73",
  "dxcc": "Finland",
  "continent": "EU",
  "spotterLatLon": [59.3, 18.0],
  "dxLatLon": [60.2, 24.9],
  "bearingFromHome": 42.3,
  "distanceKm": 1830
}
```

Frekvens → band konverteras i backenden med en bandplan-tabell (160m–70cm).

---

## 5. Frontend — layout

Inspirerad av [HolyCluster](https://holycluster.iarc.org/): mörkt tema, tät informationsdensitet, realtids-känsla.

```
┌─────────────────────────────────────────────────────────┐
│  ⚡ HamAlert Receiver        [callsign] [locator] [Save] │
│  Status: Connected · 1 234 spots today    🔔 Audio ON   │
├─────────────────────────────────────────────────────────┤
│  Filters: [All bands ▾]  [All modes ▾]  [EU/DX ▾]      │
├──────────────────────┬──────────────────────────────────┤
│                      │  TIME   CALL     FREQ    BAND    │
│   LEAFLET MAP        │  1234Z  OH2BH   14025   20m CW  │
│   (världskarta,      │  1233Z  VK2XX   7043    40m CW  │
│    mörkt tema)       │  1232Z  W1AW    3525    80m CW  │
│                      │  1231Z  JA1ABC  21250   15m SSB │
│   Din station = ★    │  ...                            │
│   DX-station = dot   │                                 │
│   Linje = riktning   │                                 │
│                      │                                 │
└──────────────────────┴──────────────────────────────────┘
│  DX: OH2BH   14.025 MHz · CW · 1 830 km · Bearing 42°  │
└─────────────────────────────────────────────────────────┘
```

---

## 6. Funktionalitet i detalj

### 6.1 Setup-panel (första besöket)

- Inputfält: **Callsign** (t.ex. `SM5ABC`) och **Locator** (t.ex. `JO89WI`)
- Sparas i `localStorage` — visas igen i headern för snabb ändring
- Skickas till backenden via `POST /config` så servern kan lägga till locator-koordinater i alla spots

### 6.2 Kartan

- Mörkt basemap (t.ex. CartoDB Dark Matter, gratis OSM-baserad)
- Stor gult/orange stjärna = din hemstation (från locatorn)
- Färgade cirklar för DX-stationer, färg efter band (se 6.5)
- Klick på cirkel → visar popup med all spot-info
- Linje från din station till aktiv DX (vid hover/klick)
- Kartan auto-centrerar på din station vid setup
- Spots äldre än 30 min bleknar och försvinner automatiskt

### 6.3 Spot-tabellen

Kolumner: **UTC · Callsign · Freq (kHz) · Band · Mode · Spotter · DXCC · km · Bearing · Comment**

- Ny rad scrollar in högst upp med en kort highlight-animation
- Klick på rad markerar stationen på kartan och drar upp detaljpanelen
- Färgkodning per rad = band (se 6.5)
- Max 200 rader visas, äldre rensas

### 6.4 Filterfält

- **Band:** All / 160m / 80m / 40m / 30m / 20m / 17m / 15m / 12m / 10m / 6m
- **Mode:** All / CW / SSB / FT8 / FT4 / RTTY / PSK / Digi
- **Continent:** All / EU / NA / SA / AF / AS / OC
- Filter tillämpas live utan reload

### 6.5 Bandkärger (som HolyCluster)

| Band | Färg |
|---|---|
| 160m | Mörkröd `#8B0000` |
| 80m | Röd `#FF4444` |
| 40m | Orange `#FF8C00` |
| 30m | Gul `#FFD700` |
| 20m | Grön `#00C800` |
| 17m | Turkos `#00CED1` |
| 15m | Blå `#4169E1` |
| 12m | Lila `#8A2BE2` |
| 10m | Magenta `#FF00FF` |
| 6m | Vit `#FFFFFF` |

### 6.6 Audio-notiser

- Valfri toggle i headern
- Kort "pip"-ljud (Web Audio API, ingen extern fil) vid ny spot
- Stationärt ljud för DX (continent ≠ EU) om användaren vill

### 6.7 Status-rad

- Visas längst upp: `Connected | Disconnected | Reconnecting…`
- Antal spots idag (räknas i backenden, nollstälks vid midnatt UTC)
- UTC-klocka uppdateras varje sekund

---

## 7. Backend-detaljer

### 7.1 Telnet-parsern

```
Anslutning: hamalert.org:7373
Login: username + "\n", sedan password + "\n"
Keepalive: skicka "\n" var 60:e sekund

Rader som börjar med "DX de " parsas.
Övriga rader (login-banner, DXCC-lookup m.m.) loggas men ignoreras.
```

### 7.2 WebSocket-protokoll (server → klient)

```json
// Ny spot
{ "type": "spot", "data": { ...spot-objekt... } }

// Konfigurationsbekräftelse
{ "type": "config_ack", "callsign": "SM5ABC", "lat": 59.8, "lon": 17.6 }

// Status
{ "type": "status", "connected": true, "spotsToday": 142 }
```

### 7.3 Endpoints

| Method | Path | Syfte |
|---|---|---|
| `GET` | `/` | Serverar index.html |
| `POST` | `/config` | Sparar callsign + locator |
| `GET` | `/spots` | De senaste 200 spots (JSON, för initial sidladdning) |
| `WS` | `/ws` | WebSocket-ström |

---

## 8. Maidenhead → koordinater

Standard-algoritm, t.ex.:
```
JO89WI → lat 59.854°N, lon 17.625°E
```

Behövs för:
1. Placera din hemstation på kartan
2. Beräkna avstånd/riktning till spottade stationer

DXCC-koordinater hämtas från en inbyggd `countries.json` (baserad på cty.dat).

---

## 9. Filstruktur

```
hamalert-receiver/
├── server.js            # Node.js backend (Telnet + WebSocket + HTTP)
├── countries.json       # DXCC → { lat, lon, continent }
├── package.json
└── public/
    ├── index.html       # Hela frontend-appen
    ├── style.css        # Mörkt tema
    └── app.js           # Leaflet, WebSocket, tabell, filter
```

---

## 10. Installation på hamserver.local

```bash
git clone ... /opt/hamalert-receiver
cd /opt/hamalert-receiver
npm install
# Skapa .env med HamAlert-credentials:
echo "HA_USER=ditt_username" > .env
echo "HA_PASS=ditt_lösenord" >> .env
node server.js
# Eller som systemd-service
```

Öppna sedan `http://hamserver.local:8080` i webbläsaren.

---

## 11. Begränsningar / framtida förbättringar

- Telnet-anslutningen kräver ett HamAlert-konto med Telnet-destination aktiverat och rätt triggers konfigurerade — spots visas bara för de triggers du satt upp
- Offline-läge (ingen LAN-fallback) — vid avbrott försöker backenden återansluta var 30:e sekund
- Möjlig utökning: URL POST webhook via Cloudflare Tunnel om portforwarding inte är möjlig
- Möjlig utökning: ljudprofiler per band, push-notiser till webbläsaren (PWA)
- Möjlig utökning: loggintegration (ADIF-export av klickade spots)
