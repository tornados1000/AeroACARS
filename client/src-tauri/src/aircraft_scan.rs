//! Aircraft-Scan — "Geladenes Flugzeug analysieren"-Feature.
//!
//! Der Pilot schickt einen GEFILTERTEN Auszug eines MSFS-Addon-Pakets
//! (nur cfg/json/xml/js + panel-WASM — nie Texturen/Modelle/Sounds) an
//! live.kant.ovh (`POST /api/ascan/submissions?source=client`). Der
//! Recorder analysiert LVar-Kandidaten/Engines/Titles fuer AeroACARS-
//! Profile; der Pilot sieht seine Einreichung (exakte Dateiliste, DSGVO)
//! unter https://live.kant.ovh/aircraft/.
//!
//! Transparenz-Regel: `ascan_collect` liefert die exakte Dateiliste an die
//! UI, die sie dem Piloten VOR dem Senden anzeigt. `ascan_submit` sendet
//! GENAU dieselbe Auswahl (gleiche Whitelist, gleicher Walk, gleiche Kappen
//! — deterministisch), es gibt keinen zweiten, verdeckten Sammel-Pfad.
//!
//! Sicherheits-Seam: Die Webview waehlt Pakete nur ueber einen INDEX in die
//! serverseitig (Rust) gehaltene Paketliste — es wandern keine freien Pfade
//! vom Frontend in Datei-Operationen. Der optionale `manual_dir` (Pilot
//! tippt seinen Community-Pfad, z.B. Addon-Linker-Setups) wird nur als
//! Wurzel fuer die manifest.json-Suche verwendet.

use serde::Serialize;
use std::io::{Cursor, Read, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Whitelist-Kappen — MUESSEN mit aircraft-webapp/src/scanner.ts und
/// recorder/src/aircraftAnalyzer.ts (isAllowedUploadPath) uebereinstimmen.
const MAX_TEXT_FILE: u64 = 5 * 1024 * 1024;
const MAX_WASM_FILE: u64 = 110 * 1024 * 1024;
const MAX_TOTAL: u64 = 400 * 1024 * 1024;
const MAX_FILES: usize = 3000;

const DEFAULT_ENDPOINT: &str = "https://live.kant.ovh/api/ascan/submissions";

/// Vom letzten `ascan_list_aircraft` gefundene Paket-Wurzeln. Die UI
/// referenziert Pakete ausschliesslich per Index in diese Liste.
#[derive(Default)]
pub struct AircraftScanState {
    packages: Mutex<Vec<ScanPackage>>,
}

#[derive(Clone)]
struct ScanPackage {
    dir: PathBuf,
    folder: String,
}

#[derive(Serialize, Clone)]
pub struct FoundAircraft {
    pub index: usize,
    pub folder: String,
    pub title: String,
    pub creator: Option<String>,
    /// Woher das Paket stammt (Community-Ordner-Pfad, gekuerzt fuer die UI)
    pub source_dir: String,
}

#[derive(Serialize)]
pub struct CollectedFile {
    pub path: String,
    pub size: u64,
}

#[derive(Serialize)]
pub struct CollectResult {
    pub files: Vec<CollectedFile>,
    pub total_bytes: u64,
    pub skipped_large: Vec<String>,
}

#[derive(Serialize)]
pub struct SubmitResult {
    pub ok: bool,
    pub id: Option<String>,
    pub status: Option<String>,
    pub zip_bytes: u64,
    /// Kompakter Report-Auszug fuer die UI (Rest sieht der Pilot im Web)
    pub icao: Option<String>,
    pub lvar_count: Option<i64>,
    pub external_process_suspected: Option<bool>,
    pub warnings: Vec<String>,
}

// ─── Community-Ordner finden ────────────────────────────────────────────

/// UserCfg.opt-Kandidaten fuer MSFS 2020/2024, Store + Steam.
fn usercfg_candidates() -> Vec<(PathBuf, &'static str)> {
    let mut out = Vec::new();
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        out.push((
            Path::new(&local)
                .join("Packages/Microsoft.FlightSimulator_8wekyb3d8bbwe/LocalCache/UserCfg.opt"),
            "MSFS 2020 (Microsoft Store)",
        ));
        out.push((
            Path::new(&local)
                .join("Packages/Microsoft.Limitless_8wekyb3d8bbwe/LocalCache/UserCfg.opt"),
            "MSFS 2024 (Microsoft Store)",
        ));
    }
    if let Ok(roaming) = std::env::var("APPDATA") {
        out.push((
            Path::new(&roaming).join("Microsoft Flight Simulator/UserCfg.opt"),
            "MSFS 2020 (Steam)",
        ));
        out.push((
            Path::new(&roaming).join("Microsoft Flight Simulator 2024/UserCfg.opt"),
            "MSFS 2024 (Steam)",
        ));
    }
    out
}

/// `InstalledPackagesPath "D:\MSFS"` aus einer UserCfg.opt ziehen.
fn parse_installed_packages_path(text: &str) -> Option<PathBuf> {
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("InstalledPackagesPath") {
            let rest = rest.trim();
            let unquoted = rest.trim_matches('"');
            if !unquoted.is_empty() {
                return Some(PathBuf::from(unquoted));
            }
        }
    }
    None
}

/// Alle auffindbaren Community-Ordner (dedupliziert).
fn community_dirs() -> Vec<(PathBuf, &'static str)> {
    let mut out: Vec<(PathBuf, &'static str)> = Vec::new();
    for (cfg_path, label) in usercfg_candidates() {
        let Ok(text) = std::fs::read_to_string(&cfg_path) else {
            continue;
        };
        let Some(base) = parse_installed_packages_path(&text) else {
            continue;
        };
        let community = base.join("Community");
        if community.is_dir() && !out.iter().any(|(p, _)| p == &community) {
            out.push((community, label));
        }
    }
    out
}

/// manifest.json eines Top-Level-Paket-Ordners lesen; nur AIRCRAFT zaehlt.
fn read_aircraft_manifest(pkg_dir: &Path) -> Option<(String, Option<String>)> {
    let manifest_path = pkg_dir.join("manifest.json");
    let meta = std::fs::metadata(&manifest_path).ok()?;
    if meta.len() > 1024 * 1024 {
        return None;
    }
    let text = std::fs::read_to_string(&manifest_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let ct = json.get("content_type")?.as_str()?;
    if !ct.eq_ignore_ascii_case("AIRCRAFT") {
        return None;
    }
    let title = json
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| pkg_dir.file_name().and_then(|n| n.to_str()).unwrap_or("?"))
        .to_string();
    let creator = json
        .get("creator")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some((title, creator))
}

// ─── Whitelist (Spiegel von scanner.ts / aircraftAnalyzer.ts) ───────────

fn is_text_ext(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        ".cfg", ".json", ".xml", ".js", ".mjs", ".html", ".htm", ".txt", ".ini", ".yaml", ".yml",
    ]
    .iter()
    .any(|e| lower.ends_with(e))
}

fn is_wasm_ext(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".wasm")
}

/// Verzeichnisse, in die wir nicht absteigen (die Gigabyte-Fresser).
/// `model*` wird betreten, aber nur .xml daraus mitgenommen (Behaviors).
fn is_excluded_dir(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let base = lower.split('.').next().unwrap_or(&lower);
    matches!(
        base,
        "texture" | "textures" | "sound" | "soundai" | "effects" | "autogen" | "scenery" | "cgl"
            | "font" | "fonts"
    )
}

fn is_model_dir(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let base = lower.split('.').next().unwrap_or(&lower);
    matches!(base, "model" | "models")
}

enum Verdict {
    Yes,
    No,
    TooLarge,
}

fn whitelist_verdict(rel_path: &str, size: u64, in_model_dir: bool) -> Verdict {
    let name = rel_path.rsplit('/').next().unwrap_or(rel_path);
    if in_model_dir && !name.to_ascii_lowercase().ends_with(".xml") {
        return Verdict::No;
    }
    if is_wasm_ext(name) {
        return if size <= MAX_WASM_FILE { Verdict::Yes } else { Verdict::TooLarge };
    }
    if is_text_ext(name) {
        return if size <= MAX_TEXT_FILE { Verdict::Yes } else { Verdict::TooLarge };
    }
    Verdict::No
}

/// Deterministischer Walk: sortierte Verzeichniseintraege, Whitelist,
/// Kappen. Wird von collect UND submit identisch benutzt (Transparenz).
fn collect_files(pkg_dir: &Path) -> Result<CollectResult, String> {
    let mut files: Vec<CollectedFile> = Vec::new();
    let mut skipped_large: Vec<String> = Vec::new();
    let mut total: u64 = 0;

    fn walk(
        dir: &Path,
        prefix: &str,
        in_model: bool,
        files: &mut Vec<CollectedFile>,
        skipped: &mut Vec<String>,
        total: &mut u64,
    ) -> Result<(), String> {
        if files.len() >= MAX_FILES || *total >= MAX_TOTAL {
            return Ok(());
        }
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| format!("Ordner nicht lesbar: {} ({e})", dir.display()))?
            .filter_map(|e| e.ok())
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            if files.len() >= MAX_FILES || *total >= MAX_TOTAL {
                return Ok(());
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                if is_excluded_dir(&name) {
                    continue;
                }
                let child_in_model = in_model || is_model_dir(&name);
                walk(&entry.path(), &rel, child_in_model, files, skipped, total)?;
            } else if ft.is_file() {
                let Ok(meta) = entry.metadata() else { continue };
                match whitelist_verdict(&rel, meta.len(), in_model) {
                    Verdict::Yes => {
                        *total += meta.len();
                        files.push(CollectedFile { path: rel, size: meta.len() });
                    }
                    Verdict::TooLarge => skipped.push(rel),
                    Verdict::No => {}
                }
            }
        }
        Ok(())
    }

    walk(pkg_dir, "", false, &mut files, &mut skipped_large, &mut total)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(CollectResult { files, total_bytes: total, skipped_large })
}

/// ZIP (deflate) aus der Collect-Auswahl bauen.
fn build_zip(pkg_dir: &Path, folder: &str, collected: &CollectResult) -> Result<Vec<u8>, String> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);
    for f in &collected.files {
        let abs = pkg_dir.join(&f.path);
        let mut buf = Vec::with_capacity(f.size.min(16 * 1024 * 1024) as usize);
        std::fs::File::open(&abs)
            .and_then(|mut fh| fh.read_to_end(&mut buf))
            .map_err(|e| format!("Datei nicht lesbar: {} ({e})", f.path))?;
        // Server erwartet Pfade mit Paket-Ordner als Wurzel (wie Web-Upload)
        writer
            .start_file(format!("{folder}/{}", f.path), options)
            .and_then(|()| writer.write_all(&buf).map_err(zip::result::ZipError::Io))
            .map_err(|e| format!("ZIP-Fehler bei {}: {e}", f.path))?;
    }
    let cursor = writer.finish().map_err(|e| format!("ZIP-Finish-Fehler: {e}"))?;
    Ok(cursor.into_inner())
}

// ─── Tauri-Commands ─────────────────────────────────────────────────────

/// Community-Ordner (auto + optional manuell) nach Flugzeug-Paketen
/// durchsuchen. Liest pro Paket NUR die manifest.json.
#[tauri::command]
pub async fn ascan_list_aircraft(
    state: tauri::State<'_, AircraftScanState>,
    manual_dir: Option<String>,
) -> Result<Vec<FoundAircraft>, String> {
    let roots: Vec<(PathBuf, String)> = {
        let mut roots: Vec<(PathBuf, String)> = community_dirs()
            .into_iter()
            .map(|(p, l)| (p, l.to_string()))
            .collect();
        if let Some(m) = manual_dir.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            let p = PathBuf::from(m);
            if !p.is_dir() {
                return Err(format!("Ordner nicht gefunden: {m}"));
            }
            if !roots.iter().any(|(r, _)| r == &p) {
                roots.push((p, "manuell gewaehlt".to_string()));
            }
        }
        roots
    };

    let scanned = tauri::async_runtime::spawn_blocking(move || {
        let mut packages: Vec<ScanPackage> = Vec::new();
        let mut found: Vec<FoundAircraft> = Vec::new();
        for (root, label) in &roots {
            let Ok(entries) = std::fs::read_dir(root) else { continue };
            let mut dirs: Vec<_> = entries
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .collect();
            dirs.sort_by_key(|e| e.file_name());
            for d in dirs {
                let pkg_dir = d.path();
                // Direkt-gewaehlter Paket-Ordner? (Pilot gibt das Paket statt
                // des Community-Ordners an): manifest im Root selbst pruefen.
                if let Some((title, creator)) = read_aircraft_manifest(&pkg_dir) {
                    let folder = d.file_name().to_string_lossy().to_string();
                    found.push(FoundAircraft {
                        index: packages.len(),
                        folder: folder.clone(),
                        title,
                        creator,
                        source_dir: format!("{} — {}", label, root.display()),
                    });
                    packages.push(ScanPackage { dir: pkg_dir, folder });
                }
            }
            // Root selbst als Paket (manual_dir zeigt direkt auf ein Addon)
            if let Some((title, creator)) = read_aircraft_manifest(root) {
                let folder = root
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "package".to_string());
                found.push(FoundAircraft {
                    index: packages.len(),
                    folder: folder.clone(),
                    title,
                    creator,
                    source_dir: format!("{} — {}", label, root.display()),
                });
                packages.push(ScanPackage { dir: root.clone(), folder });
            }
        }
        found.sort_by_key(|a| a.title.to_lowercase());
        // Indizes nach Sortierung NICHT umschreiben — index zeigt in die
        // packages-Liste, nicht in die Anzeige-Reihenfolge.
        (packages, found)
    })
    .await
    .map_err(|e| format!("Scan-Task abgebrochen: {e}"))?;

    let (packages, found) = scanned;
    *state.packages.lock().map_err(|_| "state poisoned".to_string())? = packages;
    Ok(found)
}

/// Exakte Dateiliste des gewaehlten Pakets — wird dem Piloten VOR dem
/// Senden angezeigt (DSGVO-Transparenz).
#[tauri::command]
pub async fn ascan_collect(
    state: tauri::State<'_, AircraftScanState>,
    index: usize,
) -> Result<CollectResult, String> {
    let pkg = {
        let guard = state.packages.lock().map_err(|_| "state poisoned".to_string())?;
        guard.get(index).cloned().ok_or("Unbekanntes Paket — bitte neu suchen")?
    };
    tauri::async_runtime::spawn_blocking(move || collect_files(&pkg.dir))
        .await
        .map_err(|e| format!("Collect-Task abgebrochen: {e}"))?
}

/// Paket zippen und an live.kant.ovh senden. Auth = phpVMS-API-Key aus dem
/// Keyring (derselbe Trust-Anker wie Provisioning). Sendet exakt die
/// Auswahl aus `collect_files` (deterministisch identisch zu ascan_collect).
#[tauri::command]
pub async fn ascan_submit(
    state: tauri::State<'_, AircraftScanState>,
    index: usize,
    endpoint: Option<String>,
) -> Result<SubmitResult, String> {
    let pkg = {
        let guard = state.packages.lock().map_err(|_| "state poisoned".to_string())?;
        guard.get(index).cloned().ok_or("Unbekanntes Paket — bitte neu suchen")?
    };
    let api_key = crate::secrets_load_phpvms_key()
        .ok_or("Kein phpVMS-API-Key hinterlegt — bitte zuerst anmelden")?;

    let folder = pkg.folder.clone();
    let zip_bytes = tauri::async_runtime::spawn_blocking(move || {
        let collected = collect_files(&pkg.dir)?;
        if collected.files.is_empty() {
            return Err("Keine analysierbaren Dateien im Paket gefunden".to_string());
        }
        build_zip(&pkg.dir, &pkg.folder, &collected)
    })
    .await
    .map_err(|e| format!("Zip-Task abgebrochen: {e}"))??;

    let url = endpoint.unwrap_or_else(|| DEFAULT_ENDPOINT.to_string());
    let zip_len = zip_bytes.len() as u64;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .map_err(|e| format!("HTTP-Client-Fehler: {e}"))?;
    let resp = client
        .post(&url)
        .query(&[("source", "client"), ("package", folder.as_str())])
        .header("X-API-Key", api_key)
        .header("Content-Type", "application/zip")
        .body(zip_bytes)
        .send()
        .await
        .map_err(|e| format!("Upload fehlgeschlagen: {e}"))?;

    let status = resp.status();
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Server-Antwort nicht lesbar: {e}"))?;
    if !status.is_success() {
        let msg = body
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unbekannter Fehler");
        return Err(format!("Server lehnte ab ({status}): {msg}"));
    }

    let report = body.get("report");
    Ok(SubmitResult {
        ok: body.get("ok").and_then(|v| v.as_bool()).unwrap_or(false),
        id: body.get("id").and_then(|v| v.as_str()).map(String::from),
        status: body.get("status").and_then(|v| v.as_str()).map(String::from),
        zip_bytes: zip_len,
        icao: report
            .and_then(|r| r.pointer("/aircraft/icao_type"))
            .and_then(|v| v.as_str())
            .map(String::from),
        lvar_count: report
            .and_then(|r| r.pointer("/lvars/explicit_total"))
            .and_then(|v| v.as_i64()),
        external_process_suspected: report
            .and_then(|r| r.get("external_process_suspected"))
            .and_then(|v| v.as_bool()),
        warnings: report
            .and_then(|r| r.get("warnings"))
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|w| w.as_str().map(String::from)).collect())
            .unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_installed_packages_path() {
        let cfg = "Version 1\nInstalledPackagesPath \"D:\\MSFS Packages\"\nOther 2\n";
        assert_eq!(
            parse_installed_packages_path(cfg),
            Some(PathBuf::from("D:\\MSFS Packages"))
        );
        assert_eq!(parse_installed_packages_path("nix"), None);
    }

    #[test]
    fn whitelist_mirrors_server_rules() {
        assert!(matches!(whitelist_verdict("aircraft.cfg", 100, false), Verdict::Yes));
        assert!(matches!(whitelist_verdict("panel/sys.wasm", 100, false), Verdict::Yes));
        assert!(matches!(
            whitelist_verdict("panel/sys.wasm", MAX_WASM_FILE + 1, false),
            Verdict::TooLarge
        ));
        assert!(matches!(whitelist_verdict("texture/a.dds", 100, false), Verdict::No));
        // model-Ordner: nur XML
        assert!(matches!(whitelist_verdict("model/behaviors.xml", 100, true), Verdict::Yes));
        assert!(matches!(whitelist_verdict("model/a320.bin.json", 100, true), Verdict::No));
    }

    #[test]
    fn excluded_dirs_match_web_scanner() {
        for d in ["texture", "TEXTURE.FLEET", "sound", "SoundAI", "effects", "cgl"] {
            assert!(is_excluded_dir(d), "{d} muss ausgeschlossen sein");
        }
        assert!(!is_excluded_dir("panel"));
        assert!(is_model_dir("model.LVFR"));
    }

    #[test]
    fn collect_and_zip_synthetic_package() {
        let tmp = std::env::temp_dir().join(format!("ascan-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("SimObjects/Airplanes/X/panel")).unwrap();
        std::fs::create_dir_all(tmp.join("texture")).unwrap();
        std::fs::write(tmp.join("manifest.json"), "{\"content_type\":\"AIRCRAFT\",\"title\":\"T\"}").unwrap();
        std::fs::write(tmp.join("SimObjects/Airplanes/X/aircraft.cfg"), "[GENERAL]\n").unwrap();
        std::fs::write(tmp.join("SimObjects/Airplanes/X/panel/sys.wasm"), vec![0u8; 128]).unwrap();
        std::fs::write(tmp.join("texture/big.dds"), vec![0u8; 4096]).unwrap();

        let collected = collect_files(&tmp).unwrap();
        let paths: Vec<&str> = collected.files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"manifest.json"));
        assert!(paths.contains(&"SimObjects/Airplanes/X/aircraft.cfg"));
        assert!(paths.contains(&"SimObjects/Airplanes/X/panel/sys.wasm"));
        assert!(!paths.iter().any(|p| p.ends_with(".dds")), "dds darf nie mit");

        let zip_bytes = build_zip(&tmp, "pkg", &collected).unwrap();
        assert!(zip_bytes.len() > 100);
        // ZIP wieder oeffnen und Pfad-Prefix pruefen
        let mut archive = zip::ZipArchive::new(Cursor::new(zip_bytes)).unwrap();
        let names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();
        assert!(names.contains(&"pkg/manifest.json".to_string()));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn manifest_filter_only_aircraft() {
        let tmp = std::env::temp_dir().join(format!("ascan-mani-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("manifest.json"), "{\"content_type\":\"SCENERY\",\"title\":\"S\"}").unwrap();
        assert!(read_aircraft_manifest(&tmp).is_none());
        std::fs::write(
            tmp.join("manifest.json"),
            "{\"content_type\":\"AIRCRAFT\",\"title\":\"Fenix A320\",\"creator\":\"Fenix\"}",
        )
        .unwrap();
        let (title, creator) = read_aircraft_manifest(&tmp).unwrap();
        assert_eq!(title, "Fenix A320");
        assert_eq!(creator.as_deref(), Some("Fenix"));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
