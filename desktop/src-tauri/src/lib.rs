//! HamAlert DX Receiver – Tauri backend
//!
//! Kopplar upp mot HamAlert via Telnet (hamalert.org:7373),
//! autentiserar och tar emot DX-spots i realtid.
//! Spots parsas och skickas till frontend via Tauri-events.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// Generations-räknare: ökas varje gång connect_hamalert anropas.
/// Varje Telnet-tråd känner till sin generation och avslutar sig
/// om räknaren har ökat (= en ny anslutning har startats).
static CONNECTION_GEN: AtomicU64 = AtomicU64::new(0);

// ── Spot-datamodell ────────────────────────────────────────────────────────────
// Fältnamn i camelCase för att matcha frontend-JS direkt
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct Spot {
    id:                String,
    received_at:       String,
    full_callsign:     String,
    callsign:          String,
    frequency:         f64,
    band:              String,
    mode:              String,
    mode_detail:       String,
    spotter:           String,
    comment:           String,
    time:              String,
    entity:            String,
    continent:         String,
    dxcc:              String,
    cq:                String,
    source:            String,
    snr:               String,
    speed:             String,
    qsl:               String,
    summit_ref:        String,
    wwff_ref:          String,
    pota_ref:          String,
    iota_group_ref:    String,
    state_field:       String,
    home_entity:       String,
    spotter_continent: String,
    spotter_entity:    String,
    spotter_cq:        String,
    grid:              String,   // Maidenhead-locator för DX-stationen (t.ex. "JO89WI")
    raw_text:          String,
}

// ── Bandplan: frekvens (MHz) → bandnamn ───────────────────────────────────────
fn band_from_mhz(mhz: f64) -> &'static str {
    if      mhz >= 1.8   && mhz < 2.0   { "160m" }
    else if mhz >= 3.5   && mhz < 4.0   { "80m"  }
    else if mhz >= 5.3   && mhz < 5.5   { "60m"  }
    else if mhz >= 7.0   && mhz < 7.3   { "40m"  }
    else if mhz >= 10.1  && mhz < 10.15 { "30m"  }
    else if mhz >= 14.0  && mhz < 14.35 { "20m"  }
    else if mhz >= 18.068 && mhz < 18.168 { "17m" }
    else if mhz >= 21.0  && mhz < 21.45 { "15m"  }
    else if mhz >= 24.89 && mhz < 24.99 { "12m"  }
    else if mhz >= 28.0  && mhz < 29.7  { "10m"  }
    else if mhz >= 50.0  && mhz < 54.0  { "6m"   }
    else if mhz >= 144.0 && mhz < 148.0 { "2m"   }
    else if mhz >= 430.0 && mhz < 440.0 { "70cm" }
    else                                 { "?"    }
}

/// Försöker detektera mode från kommentarstexten.
fn detect_mode(comment: &str) -> String {
    let upper = comment.to_uppercase();
    for mode in &["FT8","FT4","RTTY","PSK31","PSK","WSPR","JS8","JT65","JT9","SSB","LSB","USB","AM","FM","CW"] {
        if upper.contains(mode) { return mode.to_string(); }
    }
    String::new()
}

/// Plockar ut basanropet – tar bort /P, /M suffix men behåller prefix (OH/SM5ABC → SM5ABC).
fn base_callsign(call: &str) -> String {
    if call.contains('/') {
        call.split('/').max_by_key(|s| s.len()).unwrap_or(call).to_string()
    } else {
        call.to_string()
    }
}

/// Genererar ett unikt spot-ID.
fn unique_id() -> String {
    let ts = Utc::now().timestamp_millis();
    let suffix = (ts as u64).wrapping_mul(6364136223846793005) >> 44;
    format!("{}-{:05x}", ts, suffix & 0xFFFFF)
}

/// Skapar en content-baserad dedup-nyckel för en spot.
/// HamAlert kan skicka samma spot flera gånger (en per matchande filter)
/// – vi använder callsign + frekvens + spotter + spotttid som unik nyckel.
fn dedup_key(callsign: &str, freq: f64, spotter: &str, time: &str) -> String {
    format!("{}-{:.3}-{}-{}", callsign, freq, spotter, time)
}

/// Parsar en klassisk DX-kluster-rad i HamAlert Telnet-format:
///   `DX de SM5ABC:    14025.0  OH2BH         CW 599 73          1234Z`
fn parse_dx_line(line: &str) -> Option<Spot> {
    let line = line.trim();
    if !line.starts_with("DX de ") { return None; }

    let rest = &line[6..];
    let colon = rest.find(':')?;
    let spotter = rest[..colon].trim().to_string();
    let rest = rest[colon + 1..].trim();

    let mut tokens: Vec<&str> = rest.split_whitespace().collect();
    if tokens.len() < 2 { return None; }

    // Frekvens i kHz (Telnet-format) → MHz
    let freq_khz: f64 = tokens[0].parse().ok()?;
    let freq_mhz = (freq_khz / 1000.0 * 1000.0).round() / 1000.0;
    let callsign = tokens[1].to_string();

    // UTC-tid: sista token om den slutar på Z och är 4–5 tecken (t.ex. "1234Z")
    let time = if tokens.last().map(|t| t.ends_with('Z') && t.len() <= 5).unwrap_or(false) {
        tokens.pop().unwrap_or("").to_string()
    } else { String::new() };

    let comment = if tokens.len() > 2 { tokens[2..].join(" ") } else { String::new() };
    let mode    = detect_mode(&comment);
    let band    = band_from_mhz(freq_mhz).to_string();

    Some(Spot {
        id:                unique_id(),
        received_at:       Utc::now().to_rfc3339(),
        full_callsign:     callsign.clone(),
        callsign:          base_callsign(&callsign),
        frequency:         freq_mhz,
        band,
        mode:              mode.clone(),
        mode_detail:       mode,
        spotter,
        comment,
        time,
        raw_text:          line.to_string(),
        source:            "telnet".to_string(),
        // Fält ej tillgängliga i Telnet-formatet
        entity:            String::new(),
        continent:         String::new(),
        dxcc:              String::new(),
        cq:                String::new(),
        snr:               String::new(),
        speed:             String::new(),
        qsl:               String::new(),
        summit_ref:        String::new(),
        wwff_ref:          String::new(),
        pota_ref:          String::new(),
        iota_group_ref:    String::new(),
        state_field:       String::new(),
        home_entity:       String::new(),
        spotter_continent: String::new(),
        spotter_entity:    String::new(),
        spotter_cq:        String::new(),
        grid:              String::new(),   // Ej tillgängligt i DX de-textformat
    })
}

/// Parsar en JSON-rad från HamAlert set/json-läge.
/// HamAlert skickar spots som ett JSON-objekt per rad med fält som matchar
/// deras webhook-format (camelCase).
fn parse_json_spot(raw: &str) -> Option<Spot> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;

    // Hjälpare: strängfält, tom sträng om saknas
    let s = |key: &str| v[key].as_str().unwrap_or("").to_string();

    // Hjälpare: numeriskt fält – hanterar både JSON-tal och JSON-sträng ("14.074")
    let n = |key: &str| -> f64 {
        if let Some(num) = v[key].as_f64()          { return num; }
        if let Some(str) = v[key].as_str()          { return str.parse().unwrap_or(0.0); }
        0.0
    };

    let freq_mhz = {
        // HamAlert skickar frekvens i MHz i sitt JSON-format (samma som webhook).
        // Om värdet mot förmodan är i kHz (>1000) konverterar vi.
        let raw = n("frequency");
        if raw > 1000.0 { (raw / 1000.0 * 1000.0).round() / 1000.0 }
        else            { (raw * 1000.0).round() / 1000.0 }  // runda till 3 decimaler
    };

    let full_callsign = s("callsign");
    if full_callsign.is_empty() { return None; }

    let band = if s("band").is_empty() {
        band_from_mhz(freq_mhz).to_string()
    } else {
        s("band")
    };

    Some(Spot {
        id:                unique_id(),
        received_at:       Utc::now().to_rfc3339(),
        callsign:          base_callsign(&full_callsign),
        full_callsign,
        frequency:         freq_mhz,
        band,
        mode:              s("mode"),
        mode_detail:       s("modeDetail"),
        spotter:           s("spotter"),
        comment:           s("comment"),
        time:              s("time"),
        entity:            s("entity"),
        continent:         s("continent"),
        dxcc:              s("dxcc"),
        cq:                s("cq"),
        source:            { let src = s("source"); if src.is_empty() { "telnet-json".to_string() } else { src } },
        snr:               s("snr"),
        speed:             s("speed"),
        qsl:               s("qsl"),
        summit_ref:        s("summitRef"),
        wwff_ref:          s("wwffRef"),
        pota_ref:          s("potaRef"),
        iota_group_ref:    s("iotaGroupRef"),
        state_field:       s("state"),
        home_entity:       s("homeEntity"),
        spotter_continent: s("spotterContinent"),
        spotter_entity:    s("spotterEntity"),
        spotter_cq:        s("spotterCq"),
        grid:              s("grid"),   // Maidenhead-locator om HamAlert skickar den
        raw_text:          raw.to_string(),
    })
}

// ── Spot-persistens ────────────────────────────────────────────────────────────
/// Returnerar sökvägen till spots.json i appens data-katalog.
fn spots_file(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("spots.json"))
}

/// Laddar spots från disk. Filtrerar bort spots äldre än max_hours direkt.
fn load_spots(app: &AppHandle, max_hours: i64) -> Vec<Spot> {
    let path = match spots_file(app) { Some(p) => p, None => return vec![] };
    let data = match std::fs::read_to_string(&path) { Ok(d) => d, Err(_) => return vec![] };
    let all: Vec<Spot> = serde_json::from_str(&data).unwrap_or_default();
    let cutoff = Utc::now() - chrono::Duration::hours(max_hours);
    all.into_iter()
        .filter(|s| s.received_at.parse::<DateTime<Utc>>().map(|t| t > cutoff).unwrap_or(false))
        .collect()
}

/// Sparar spots till disk (skriver hela bufferten).
/// Anropas efter varje ny spot – filen är liten nog att det går fort.
fn save_spots(app: &AppHandle, spots: &[Spot]) {
    if let Some(path) = spots_file(app) {
        // Skapa katalogen om den inte finns
        if let Some(dir) = path.parent() { let _ = std::fs::create_dir_all(dir); }
        if let Ok(json) = serde_json::to_string(spots) {
            let _ = std::fs::write(&path, json);
        }
    }
}

// ── QRZ-cache persistens ───────────────────────────────────────────────────────
/// Returnerar sökvägen till qrz_cache.json i appens data-katalog.
#[allow(dead_code)]
fn qrz_cache_file(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("qrz_cache.json"))
}

/// Laddar QRZ-cachen från disk.
/// Returnerar råa JSON-strängen – TTL-filtrering (30 dagar) sköts i frontend.
#[tauri::command]
fn load_qrz_cache(app: AppHandle) -> String {
    let path = match qrz_cache_file(&app) { Some(p) => p, None => return "{}".to_string() };
    std::fs::read_to_string(&path).unwrap_or_else(|_| "{}".to_string())
}

/// Sparar QRZ-cachen till disk.
/// Anropas från frontend (debounced) med hela cache-objektet som JSON-sträng.
#[tauri::command]
fn save_qrz_cache(app: AppHandle, data: String) {
    if let Some(path) = qrz_cache_file(&app) {
        if let Some(dir) = path.parent() { let _ = std::fs::create_dir_all(dir); }
        let _ = std::fs::write(&path, data);
    }
}

// ── Tauri-kommando ─────────────────────────────────────────────────────────────
/// Startar Telnet-anslutningen mot HamAlert i en bakgrundstråd.
/// Anropas från frontend med:
///   invoke('connect_hamalert', { username, password, maxHours })
///
/// max_hours: hur länge spots sparas (1–8 timmar)
/// Varje anrop ökar CONNECTION_GEN → gammal tråd avslutar sig
#[tauri::command]
fn connect_hamalert(app: AppHandle, username: String, password: String, max_hours: i64) {
    // Öka generationen – en eventuell pågående tråd märker detta och avslutar sin loop
    let my_gen = CONNECTION_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let max_hours = max_hours.clamp(1, 8);

    std::thread::spawn(move || {
        // Ladda historik från disk (filtreras till max_hours direkt)
        let mut spots: Vec<Spot> = load_spots(&app, max_hours);
        let mut spots_today: u32 = spots.iter().filter(|s| {
            s.received_at.parse::<DateTime<Utc>>()
                .map(|t| t.format("%d").to_string() == Utc::now().format("%d").to_string())
                .unwrap_or(false)
        }).count() as u32;
        let mut last_day = Utc::now().format("%d").to_string();

        loop {
            // Avsluta om en nyare anslutning har startats
            if CONNECTION_GEN.load(Ordering::Relaxed) != my_gen { break; }
            let _ = app.emit("ha-status", serde_json::json!({
                "connected": false, "text": "Connecting to HamAlert…"
            }));

            // HamAlert Telnet-port är 7300 (ej 7373)
            match TcpStream::connect("hamalert.org:7300") {
                Err(e) => {
                    let _ = app.emit("ha-status", serde_json::json!({
                        "connected": false,
                        "text": format!("Connection failed: {} – retry in 30s", e)
                    }));
                    std::thread::sleep(Duration::from_secs(30));
                    continue;
                }
                Ok(stream) => {
                    // Läs-timeout: 3 min. Keepalive-tråden pingar var 90s → bör aldrig triggas
                    // om inte kopplingen faktiskt dött.
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(180)));

                    // writer delas med keepalive-tråden via Arc<Mutex<>>
                    let writer = match stream.try_clone() {
                        Ok(w) => Arc::new(Mutex::new(w)),
                        Err(_) => { std::thread::sleep(Duration::from_secs(10)); continue; }
                    };

                    // HamAlert Telnet-login
                    // Vänta på välkomstbannern/login-prompten innan vi skickar credentials.
                    // Telnet-protokollet kräver \r\n som radavslutning.
                    {
                        let mut w = writer.lock().unwrap();
                        std::thread::sleep(Duration::from_millis(1500));
                        let _ = write!(w, "{}\r\n", username);
                        std::thread::sleep(Duration::from_millis(1000));
                        let _ = write!(w, "{}\r\n", password);
                        std::thread::sleep(Duration::from_millis(800));
                        // Begär JSON-format: en JSON-rad per spot, enklare att parsa.
                        let _ = write!(w, "set/json\r\n");
                    }

                    let _ = app.emit("ha-status", serde_json::json!({
                        "connected": true, "text": "Connected"
                    }));

                    // Skicka åldersfiltrad historik till frontend vid reconnect
                    let cutoff = Utc::now() - chrono::Duration::hours(max_hours);
                    spots.retain(|s| {
                        s.received_at.parse::<DateTime<Utc>>()
                            .map(|t| t > cutoff).unwrap_or(false)
                    });

                    // Bygg dedup-set från befintlig historik.
                    // HamAlert skickar samma spot flera gånger (en per matchande alert)
                    // – seen_keys förhindrar att samma spot lagras och sänds flera gånger.
                    let mut seen_keys: HashSet<String> = spots.iter()
                        .map(|s| dedup_key(&s.callsign, s.frequency, &s.spotter, &s.time))
                        .collect();

                    let _ = app.emit("ha-init", serde_json::json!({
                        "spots": spots.iter().take(200).collect::<Vec<_>>(),
                        "spotsToday": spots_today
                    }));

                    // ── Keepalive-tråd ────────────────────────────────────────────
                    // Skickar "echo ping\r\n" var 90:e sekund.
                    // HamAlert-servern är tyst när inga spots trillar in, annars
                    // triggar vår 180s read-timeout och vi kopplar ner i onödan.
                    let writer_ka = Arc::clone(&writer);
                    std::thread::spawn(move || {
                        loop {
                            std::thread::sleep(Duration::from_secs(90));
                            if writer_ka.lock().unwrap().write_all(b"echo ping\r\n").is_err() {
                                break; // kopplingen dog – main-tråden tar hand om reconnect
                            }
                        }
                    });

                    // ── Läs spots rad för rad ─────────────────────────────────────
                    // Med set/json är varje rad antingen en JSON-blob eller servertext.
                    let reader = BufReader::new(stream);
                    for line in reader.lines() {
                        match line {
                            Err(_) => break, // Timeout/frånkoppling → återanslut
                            Ok(raw) => {
                                let raw = raw.trim().to_string();
                                if raw.is_empty() { continue; }

                                let spot_opt = if raw.starts_with('{') {
                                    parse_json_spot(&raw)
                                } else {
                                    parse_dx_line(&raw) // fallback för icke-JSON-rader
                                };

                                if let Some(spot) = spot_opt {
                                    // Dedup: HamAlert skickar samma spot flera gånger
                                    // (en per matchande alert/filter). Hoppa över duplikat.
                                    let key = dedup_key(
                                        &spot.callsign, spot.frequency,
                                        &spot.spotter,  &spot.time
                                    );
                                    if !seen_keys.insert(key) { continue; }

                                    // Nollställ dagräknare vid midnatt UTC
                                    let day = Utc::now().format("%d").to_string();
                                    if day != last_day { spots_today = 0; last_day = day; }

                                    spots.insert(0, spot.clone());

                                    // Begränsa bufferten: max 500 spots OCH max 2h ålder
                                    if spots.len() > 500 { spots.truncate(500); }
                                    let cutoff = Utc::now()
                                        - chrono::Duration::hours(max_hours);
                                    spots.retain(|s| {
                                        s.received_at.parse::<DateTime<Utc>>()
                                            .map(|t| t > cutoff).unwrap_or(false)
                                    });

                                    spots_today += 1;
                                    save_spots(&app, &spots);
                                    let _ = app.emit("ha-spot", serde_json::json!({
                                        "data": spot,
                                        "spotsToday": spots_today
                                    }));
                                }
                            }
                        }
                    }

                    let _ = app.emit("ha-status", serde_json::json!({
                        "connected": false, "text": "Disconnected – reconnecting in 30s"
                    }));
                    std::thread::sleep(Duration::from_secs(30));
                }
            }
        }
    });
}

// ── QRZ XML API ────────────────────────────────────────────────────────────────
//
// QRZ XML API (kräver prenumeration) ger oss callsign-data inkl. lat/lon/grid.
// Autentisering: username + password → session key (giltig ~24h).
// Uppslag:       session key + callsign → XML med stationsdata.
//
// Vi implementerar detta som två separata async Tauri-kommandon:
//   qrz_login  – loggar in och returnerar session key
//   qrz_lookup – slår upp ett anrop, returnerar lat/lon/grid/namn/bild

/// Returnerat data per callsign-uppslag.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct QrzInfo {
    callsign: String,
    lat:      f64,
    lon:      f64,
    grid:     String,
    name:     String,
    image:    String,
}

/// Enkel XML-värdesextraktor: hittar <tag>value</tag> i en XML-sträng.
/// Ersätter full XML-parser för att undvika extra dependencies.
fn xml_val(xml: &str, tag: &str) -> Option<String> {
    let open  = format!("<{}>",  tag);
    let close = format!("</{}>", tag);
    let start = xml.find(&open)? + open.len();
    let end   = xml[start..].find(&close)? + start;
    Some(xml[start..end].trim().to_string())
}

/// Loggar in mot QRZ XML API med vanliga QRZ-credentials.
/// Returnerar session key vid success, felmeddelande vid fail.
#[tauri::command]
async fn qrz_login(username: String, password: String) -> Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .get("https://xmldata.qrz.com/xml/current/")
        .query(&[
            ("username", username.as_str()),
            ("password", password.as_str()),
            ("agent",    "HamAlertReceiver/1.0"),
        ])
        .send().await
        .map_err(|e| format!("Network error: {}", e))?
        .text().await
        .map_err(|e| format!("Read error: {}", e))?;

    if let Some(key) = xml_val(&resp, "Key") {
        Ok(key)
    } else if let Some(err) = xml_val(&resp, "Error") {
        Err(err)
    } else {
        Err("Login failed – unexpected response".to_string())
    }
}

/// Slår upp ett callsign mot QRZ XML API.
/// Returnerar lat/lon/grid/namn/bild, eller felmeddelande.
#[tauri::command]
async fn qrz_lookup(session: String, callsign: String) -> Result<QrzInfo, String> {
    let query = format!("s={};callsign={}", session, callsign);
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("https://xmldata.qrz.com/xml/current/?{}", query))
        .header("User-Agent", "HamAlertReceiver/1.0")
        .send().await
        .map_err(|e| format!("Network error: {}", e))?
        .text().await
        .map_err(|e| format!("Read error: {}", e))?;

    // Kolla om sessionen löpt ut
    if resp.contains("<Error>Session Timeout</Error>") || resp.contains("Invalid session key") {
        return Err("SESSION_EXPIRED".to_string());
    }

    let lat: f64 = xml_val(&resp, "lat").and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let lon: f64 = xml_val(&resp, "lon").and_then(|v| v.parse().ok()).unwrap_or(0.0);

    if lat == 0.0 && lon == 0.0 {
        return Err("Not found".to_string());
    }

    Ok(QrzInfo {
        callsign,
        lat,
        lon,
        grid:  xml_val(&resp, "grid").unwrap_or_default(),
        name:  xml_val(&resp, "name").unwrap_or_default(),
        image: xml_val(&resp, "image").unwrap_or_default(),
    })
}

// ── Uppdateringsstöd ───────────────────────────────────────────────────────────

/// Kollar om en ny version finns tillgänglig.
/// Returnerar { available: bool, version?: string, body?: string }
#[tauri::command]
async fn check_for_updates(app: AppHandle) -> Result<serde_json::Value, String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await.map_err(|e| e.to_string())? {
        Some(update) => Ok(serde_json::json!({
            "available": true,
            "version":   update.version,
            "body":      update.body.unwrap_or_default(),
        })),
        None => Ok(serde_json::json!({ "available": false })),
    }
}

/// Laddar ner och installerar uppdateringen, startar sedan om appen.
#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(|e| e.to_string())?;
    if let Some(update) = updater.check().await.map_err(|e| e.to_string())? {
        update
            .download_and_install(|_chunk, _total| {}, || {})
            .await
            .map_err(|e| e.to_string())?;
        app.restart();
    }
    Ok(())
}

// ── Tauri app entry point ──────────────────────────────────────────────────────
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            connect_hamalert,
            qrz_login,
            qrz_lookup,
            load_qrz_cache,
            save_qrz_cache,
            check_for_updates,
            install_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
