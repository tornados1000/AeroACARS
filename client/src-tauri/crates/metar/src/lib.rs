//! METAR fetch + parse.
//!
//! Snapshots weather at the moment of takeoff and touchdown for the PIREP
//! landing analysis. See requirements spec §21–§22.
//!
//! Source: aviationweather.gov (NOAA), which exposes a free JSON API and
//! doesn't require an API key. Requests are rate-limited by the upstream;
//! we don't expose this through CloudeAcars more than twice per flight
//! (departure + arrival), so we'll never come close to the limits.
//!
//! Status: Phase J.2 — production-ready for the standard "snapshot once
//! at takeoff, snapshot once at touchdown" use case. The full TAF /
//! forecast set is out of scope.

#![allow(dead_code)]

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Minimal subset of the aviationweather.gov METAR JSON we care about.
/// Field names match the upstream payload (`/api/data/metar`).
#[derive(Debug, Clone, Deserialize)]
struct ApiMetar {
    #[serde(default, rename = "icaoId")]
    icao_id: Option<String>,
    /// Raw observation text — what the pilot expects to read.
    #[serde(default, rename = "rawOb")]
    raw_ob: Option<String>,
    /// Observation time as Unix timestamp (seconds since epoch).
    #[serde(default, rename = "obsTime")]
    obs_time: Option<i64>,
    /// Wind direction in degrees true. Can be `"VRB"` for variable —
    /// upstream encodes that as a string in the same field, hence the
    /// untagged enum below.
    #[serde(default, rename = "wdir")]
    wdir: Option<WindDir>,
    /// Wind speed in knots.
    #[serde(default, rename = "wspd")]
    wspd: Option<f32>,
    /// Gust in knots, when reported.
    #[serde(default, rename = "wgst")]
    wgst: Option<f32>,
    /// Temperature in °C.
    #[serde(default, rename = "temp")]
    temp: Option<f32>,
    /// Dewpoint in °C.
    #[serde(default, rename = "dewp")]
    dewp: Option<f32>,
    /// Altimeter / QNH in hPa.
    #[serde(default, rename = "altim")]
    altim: Option<f32>,
    /// Visibility — "10+" for unlimited or a number in statute miles.
    /// Upstream uses both string and integer, so accept both.
    #[serde(default, rename = "visib")]
    visib: Option<Visibility>,
}

/// Wind-direction field can be either a numeric heading or the literal
/// string `"VRB"`. Treated as `None` when variable.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum WindDir {
    Numeric(f32),
    Variable(String),
}

/// Visibility is reported either as a number (statute miles) or as the
/// string `"10+"` for "10 sm or more".
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum Visibility {
    Numeric(f32),
    Text(String),
}

/// Cleaned, simulator-friendly weather snapshot we hand back to the
/// rest of the app. All optional because some METARs omit fields
/// (auto stations, missing sensors) — the activity log + PIREP simply
/// skip what we don't have.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetarSnapshot {
    pub icao: String,
    pub raw: String,
    pub time: DateTime<Utc>,
    pub wind_direction_deg: Option<f32>,
    pub wind_speed_kt: Option<f32>,
    pub gust_kt: Option<f32>,
    /// Visibility in metres. We convert from statute miles (NOAA's
    /// native unit) so the rest of CloudeAcars can stay metric.
    pub visibility_m: Option<u32>,
    pub temperature_c: Option<f32>,
    pub dewpoint_c: Option<f32>,
    pub qnh_hpa: Option<f32>,
}

#[derive(Debug, Error)]
pub enum MetarError {
    #[error("network error: {0}")]
    Network(String),
    #[error("upstream returned status {0}")]
    Status(u16),
    #[error("malformed METAR response: {0}")]
    Parse(String),
    #[error("no METAR available for {0}")]
    NotFound(String),
}

const STATUTE_MILE_M: f32 = 1609.344;

/// Fetch the latest METAR for an airport from aviationweather.gov.
/// Returns `MetarError::NotFound` if upstream has no current report
/// (stale stations, military fields). Times out after ~10 s — we
/// don't want a flaky weather server to hang the takeoff banner.
pub async fn fetch_metar(icao: &str) -> Result<MetarSnapshot, MetarError> {
    let icao = icao.trim().to_uppercase();
    let url = format!(
        "https://aviationweather.gov/api/data/metar?ids={icao}&format=json&hours=2"
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent(concat!("CloudeAcars/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| MetarError::Network(e.to_string()))?;
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| MetarError::Network(e.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(MetarError::Status(status.as_u16()));
    }
    let list: Vec<ApiMetar> = response
        .json()
        .await
        .map_err(|e| MetarError::Parse(e.to_string()))?;
    let raw = list
        .into_iter()
        .next()
        .ok_or_else(|| MetarError::NotFound(icao.clone()))?;

    let time = raw
        .obs_time
        .and_then(|s| Utc.timestamp_opt(s, 0).single())
        .unwrap_or_else(Utc::now);

    let wind_direction_deg = match raw.wdir {
        Some(WindDir::Numeric(n)) => Some(n),
        _ => None,
    };
    let visibility_m = match raw.visib {
        Some(Visibility::Numeric(n)) => Some((n * STATUTE_MILE_M) as u32),
        Some(Visibility::Text(t)) => {
            // "10+" → at least 10 sm. Treat as exactly 10 sm.
            if t.starts_with("10") {
                Some((10.0 * STATUTE_MILE_M) as u32)
            } else if let Ok(n) = t.parse::<f32>() {
                Some((n * STATUTE_MILE_M) as u32)
            } else {
                None
            }
        }
        None => None,
    };

    Ok(MetarSnapshot {
        icao: raw.icao_id.unwrap_or(icao),
        raw: raw.raw_ob.unwrap_or_default(),
        time,
        wind_direction_deg,
        wind_speed_kt: raw.wspd,
        gust_kt: raw.wgst,
        visibility_m,
        temperature_c: raw.temp,
        dewpoint_c: raw.dewp,
        qnh_hpa: raw.altim,
    })
}
