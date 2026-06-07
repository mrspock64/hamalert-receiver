/**
 * HamAlert Receiver – server.js
 *
 * Tar emot DX-spots från HamAlert via HTTP POST/GET och pushar dem
 * i realtid till alla anslutna webbläsare via WebSocket.
 *
 * Konfigurera i HamAlert:
 *   Destinations → URL Notification → http://<din-publika-ip>:8181/spot
 *   Method: POST   Format: Form (URL-encoded)
 *
 * Starta: PORT=8181 node server.js
 */

const express   = require('express');
const http      = require('http');
const WebSocket = require('ws');
const path      = require('path');
const fs        = require('fs');

const app    = express();
// Vi skapar en vanlig HTTP-server och kopplar Express till den.
// Det låter oss dela samma port för HTTP och WebSocket.
const server = http.createServer(app);

// ── WebSocket-server i "noServer"-läge ────────────────────────────────────────
//
// VARFÖR noServer?
// Om man skapar ws med `new WebSocket.Server({ server })` lyssnar ws-biblioteket
// på HTTP-serverns "upgrade"-event och tar hand om ALLA uppgraderingsförfrågningar.
// Moderna Chrome-versioner skickar HTTP/2-förhandlingsheaders (Upgrade: h2c) vid
// varje vanlig sidladdning, vilket gjorde att ws svarade "426 Upgrade Required"
// *innan* Express hann servera index.html.
//
// Med noServer:true hanterar vi upgrade-eventet manuellt och skickar bara rena
// WebSocket-anslutningar (Upgrade: websocket, url: /ws) vidare till ws.
const wss = new WebSocket.Server({ noServer: true });

server.on('upgrade', (req, socket, head) => {
  const isWS = (req.headers.upgrade || '').toLowerCase() === 'websocket';
  // Ignorera alla upgrade-förfrågningar som inte är WebSocket på /ws
  if (!isWS || req.url !== '/ws') {
    socket.end('HTTP/1.1 400 Bad Request\r\n\r\n');
    return;
  }
  // Låt ws-biblioteket slutföra handskakningen och emittera "connection"
  wss.handleUpgrade(req, socket, head, ws => wss.emit('connection', ws, req));
});

// ── Middleware ─────────────────────────────────────────────────────────────────
app.use(express.urlencoded({ extended: true })); // HamAlert POST (Form)
app.use(express.json());                          // eventuell JSON-POST
app.use(express.static(path.join(__dirname, 'public'))); // servera index.html m.m.

// ── Persistens (spots.json) ────────────────────────────────────────────────────
//
// Spots sparas till disk efter varje ny spot. Vid omstart läses filen in igen
// så att webbläsaren direkt får historik utan att vänta på nya spots från HamAlert.
const SPOTS_FILE = path.join(__dirname, 'spots.json');

function loadSpots() {
  try {
    if (fs.existsSync(SPOTS_FILE)) {
      const data = JSON.parse(fs.readFileSync(SPOTS_FILE, 'utf8'));
      return Array.isArray(data) ? data : [];
    }
  } catch (e) {
    console.error('Kunde inte läsa spots.json:', e.message);
  }
  return [];
}

function saveSpots() {
  try {
    fs.writeFileSync(SPOTS_FILE, JSON.stringify(spots));
  } catch (e) {
    console.error('Kunde inte spara spots.json:', e.message);
  }
}

// ── State ──────────────────────────────────────────────────────────────────────
const MAX_SPOTS = 500; // Max antal spots i minnet (äldsta kastas bort)

let spots   = loadSpots();  // Array med alla buffrade spots (nyast först)
let lastDay = new Date().getUTCDate(); // Används för att nollställa dagräknaren vid midnatt UTC

// Räkna spots från idag ur den inlästa historiken (receivedAt = ISO-sträng)
let spotsToday = spots.filter(s =>
  s.receivedAt && s.receivedAt.startsWith(new Date().toISOString().slice(0, 10))
).length;

// ── Hjälpfunktioner ────────────────────────────────────────────────────────────

/**
 * Nollställer spotsToday vid midnatt UTC.
 * Anropas från addSpot() vid varje inkommande spot.
 */
function resetDailyCount() {
  const d = new Date().getUTCDate();
  if (d !== lastDay) {
    spotsToday = 0;
    lastDay = d;
  }
}

/**
 * Skickar ett JSON-meddelande till alla anslutna WebSocket-klienter.
 * Klienter som inte är i OPEN-tillstånd (t.ex. håller på att stängas) hoppas över.
 */
function broadcast(obj) {
  const msg = JSON.stringify(obj);
  wss.clients.forEach(c => {
    if (c.readyState === WebSocket.OPEN) c.send(msg);
  });
}

/**
 * Härleder amatörradiobandet från frekvensen i MHz.
 * Täcker 160m–70cm enligt IARU-bandplan.
 * Används som fallback om HamAlert inte skickar "band"-parametern.
 *
 * @param  {number|string} mhz  Frekvens i MHz
 * @returns {string}            Bandnamn, t.ex. "20m", eller "?" om okänt
 */
function bandFromMHz(mhz) {
  const f = parseFloat(mhz) || 0;
  if (f >= 1.8    && f < 2.0)    return '160m';
  if (f >= 3.5    && f < 4.0)    return '80m';
  if (f >= 5.3    && f < 5.5)    return '60m';
  if (f >= 7.0    && f < 7.3)    return '40m';
  if (f >= 10.1   && f < 10.15)  return '30m';
  if (f >= 14.0   && f < 14.35)  return '20m';
  if (f >= 18.068 && f < 18.168) return '17m';
  if (f >= 21.0   && f < 21.45)  return '15m';
  if (f >= 24.89  && f < 24.99)  return '12m';
  if (f >= 28.0   && f < 29.7)   return '10m';
  if (f >= 50     && f < 54)     return '6m';
  if (f >= 144    && f < 148)    return '2m';
  if (f >= 430    && f < 440)    return '70cm';
  return '?';
}

/**
 * Normaliserar en inkommande spot-payload (från GET-query eller POST-body)
 * till ett enhetligt spot-objekt med alla kända HamAlert-fält.
 *
 * HamAlert kan skicka en delmängd av fälten – saknade fält sätts till ''.
 * "band" beräknas ur frekvensen om HamAlert inte skickar det explicit.
 *
 * @param  {object} p  Raw request parameters
 * @returns {object}   Normaliserat spot-objekt
 */
function parseSpot(p) {
  resetDailyCount();
  return {
    // Unikt ID per spot – timestamp + slumpsuffix för att undvika kollisioner
    id:               `${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
    receivedAt:       new Date().toISOString(),

    // Callsign: fullCallsign kan innehålla /P, /M, QRP-suffix etc.
    fullCallsign:     p.fullCallsign || p.callsign || '?',
    callsign:         p.callsign     || '',

    // Frekvens i MHz; band beräknas ur frekvensen om det saknas
    frequency:        parseFloat(p.frequency) || 0,
    band:             p.band         || bandFromMHz(p.frequency),

    // mode = CW/SSB/FT8 etc.  modeDetail = mer specifikt, t.ex. FT8/FT4
    mode:             (p.mode        || '').toUpperCase(),
    modeDetail:       (p.modeDetail  || p.mode || '').toUpperCase(),

    // Spot-tid från HamAlert (HHMM-format). Används i UI om det finns.
    time:             p.time         || '',

    // DXCC-information om den spottade stationen
    dxcc:             p.dxcc         || '',
    continent:        p.continent    || '',
    entity:           p.entity       || '',       // DXCC-entitetens namn
    homeEntity:       p.homeEntity   || '',       // operatörens hemland
    cq:               p.cq           || '',       // CQ-zon

    // Information om spottaren
    spotter:          p.spotter      || '',
    spotterContinent: p.spotterContinent || '',
    spotterEntity:    p.spotterEntity    || '',
    spotterCq:        p.spotterCq        || '',

    // Fritext och metadata
    comment:          p.comment      || '',
    source:           p.source       || '',       // cluster/rbn/sotawatch/pota/…

    // Digitala modaliteter
    snr:              p.snr          || '',       // Signal/noise i dB (t.ex. FT8)
    speed:            p.speed        || '',       // CW-hastighet i WPM

    // QSL-information
    qsl:              p.qsl          || '',

    // SOTA-fält (Summits on the Air)
    summitRef:        p.summitRef    || '',
    summitName:       p.summitName   || '',
    summitHeight:     p.summitHeight || '',
    summitPoints:     p.summitPoints || '',

    // WWFF (World Wide Flora & Fauna)
    wwffRef:          p.wwffRef      || '',
    wwffName:         p.wwffName     || '',

    // POTA (Parks on the Air) och IOTA (Islands on the Air)
    potaRef:          p.potaRef      || '',
    iotaGroupRef:     p.iotaGroupRef || '',
    iotaGroupName:    p.iotaGroupName|| '',

    // Övrigt
    state:            p.state        || '',       // US-delstat
    title:            p.title        || '',
    rawText:          p.rawText      || '',       // Originalrad om tillgänglig
  };
}

/**
 * Lägger till en ny spot i bufferten, broadcastar till alla klienter
 * och sparar till disk.
 *
 * @param {object} params  Raw request parameters (från req.body eller req.query)
 */
function addSpot(params) {
  const spot = parseSpot(params);

  // Ny spot läggs överst (nyast först)
  spots.unshift(spot);

  // Håll bufferten under MAX_SPOTS – truncate tar bort de äldsta
  if (spots.length > MAX_SPOTS) spots.length = MAX_SPOTS;

  spotsToday++;

  // Pusha till alla anslutna webbläsare
  broadcast({ type: 'spot', data: spot, spotsToday });

  // Skriv till disk för persistens vid omstart
  saveSpots();

  // Konsollogg för felsökning
  console.log(
    `[${new Date().toISOString()}]`,
    spot.fullCallsign.padEnd(14),
    String(spot.frequency).padStart(8), 'MHz',
    spot.band.padEnd(4),
    spot.mode.padEnd(5),
    ' via', spot.spotter
  );
}

// ── HTTP-routes ────────────────────────────────────────────────────────────────

// HamAlert URL-notification: stödjer både POST (Form) och GET (URL-params)
app.post('/spot', (req, res) => { addSpot(req.body);  res.sendStatus(200); });
app.get( '/spot', (req, res) => { addSpot(req.query); res.sendStatus(200); });

// Initial dataladdning för webbläsaren (de 200 senaste spots)
// Webbläsaren anropar detta direkt vid sidladdning innan WebSocket är klar
app.get('/spots', (_req, res) => res.json({ spots: spots.slice(0, 200), spotsToday }));

// Hälsokontroll – användbart för att verifiera att servern är uppe
app.get('/health', (_req, res) => res.json({ ok: true, spotsToday, buffered: spots.length }));

// ── WebSocket-hantering ────────────────────────────────────────────────────────
wss.on('connection', ws => {
  // Skicka buffrade spots direkt vid anslutning så att webbläsaren
  // inte behöver vänta på nästa inkommande spot för att se data.
  ws.send(JSON.stringify({
    type:       'init',
    spots:      spots.slice(0, 200),
    spotsToday,
  }));
  // Från och med nu tar broadcast() hand om att skicka nya spots
});

// ── Starta servern ─────────────────────────────────────────────────────────────
// PORT kan överskridas med miljövariabeln, t.ex.: PORT=8181 node server.js
// Undvik port 6000 – Chrome blockerar den (ERR_UNSAFE_PORT).
const PORT = process.env.PORT || 8080;
server.listen(PORT, '0.0.0.0', () => {
  console.log('─────────────────────────────────────────');
  console.log(`  HamAlert Receiver  →  http://0.0.0.0:${PORT}`);
  console.log(`  HamAlert POST URL  →  http://<publik-ip>:${PORT}/spot`);
  console.log('─────────────────────────────────────────');
});
