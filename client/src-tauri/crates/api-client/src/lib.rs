//! phpVMS 7 HTTP API client.
//!
//! Talks to:
//!   * phpVMS Core API (users, bids, flights, fleet, PIREP file, ACARS positions)
//!   * CloudeAcars phpVMS module (config, version, heartbeat, landing extras) — Phase 4
//!
//! Authentication: phpVMS API key sent via the `X-API-Key` header (phpVMS standard).
//! All requests advertise `User-Agent: CloudeAcars/<version>` so the server can identify us.

#![allow(dead_code)] // Some endpoints land in later phases; their wrappers are stubbed.

use std::fmt;
use std::time::Duration;

use reqwest::{header, Client as HttpClient, Response, StatusCode};
use serde::de::{self, DeserializeOwned, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;
use url::Url;

/// phpVMS sometimes encodes string-ish fields (e.g. `flight_number`) as JSON
/// numbers when the value happens to be all digits. Accept either form.
fn de_str_or_int<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = String;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("string or integer")
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<String, E> {
            Ok(v.to_owned())
        }
        fn visit_string<E: de::Error>(self, v: String) -> Result<String, E> {
            Ok(v)
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<String, E> {
            Ok(v.to_string())
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<String, E> {
            Ok(v.to_string())
        }
        fn visit_f64<E: de::Error>(self, v: f64) -> Result<String, E> {
            Ok(v.to_string())
        }
    }
    d.deserialize_any(V)
}

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("invalid base URL: {0}")]
    InvalidUrl(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("authentication failed (HTTP 401) — check API key")]
    Unauthenticated,
    #[error("forbidden (HTTP 403) — account may lack permissions")]
    Forbidden,
    #[error("not found (HTTP 404) — endpoint missing on this phpVMS site")]
    NotFound,
    #[error("rate limited (HTTP 429), retry after {retry_after_seconds}s")]
    RateLimited { retry_after_seconds: u64 },
    #[error("server error (HTTP {status}): {body}")]
    Server { status: u16, body: String },
    #[error("unexpected response shape: {0}")]
    BadResponse(String),
}

impl ApiError {
    /// Stable identifier surfaced to the UI for i18n key lookup.
    pub fn code(&self) -> &'static str {
        match self {
            ApiError::InvalidUrl(_) => "invalid_url",
            ApiError::Network(_) => "network",
            ApiError::Unauthenticated => "unauthenticated",
            ApiError::Forbidden => "forbidden",
            ApiError::NotFound => "not_found",
            ApiError::RateLimited { .. } => "rate_limited",
            ApiError::Server { .. } => "server",
            ApiError::BadResponse(_) => "bad_response",
        }
    }
}

impl From<reqwest::Error> for ApiError {
    fn from(err: reqwest::Error) -> Self {
        ApiError::Network(err.to_string())
    }
}

/// Connection details for a phpVMS site.
#[derive(Clone, Debug)]
pub struct Connection {
    base_url: Url,
    api_key: String,
}

impl Connection {
    pub fn new(base_url: &str, api_key: impl Into<String>) -> Result<Self, ApiError> {
        let trimmed = base_url.trim().trim_end_matches('/');
        let url = Url::parse(trimmed).map_err(|_| ApiError::InvalidUrl(trimmed.into()))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(ApiError::InvalidUrl(format!(
                "URL must be http(s), got '{}'",
                url.scheme()
            )));
        }
        Ok(Self {
            base_url: url,
            api_key: api_key.into(),
        })
    }

    pub fn base_url(&self) -> &str {
        self.base_url.as_str()
    }
}

// ---- Resource types ----

/// Subset of `GET /api/user` we need.
///
/// phpVMS exposes `home_airport` / `curr_airport` (the ICAO strings) NOT a
/// `_id` suffix, despite the underlying DB columns being named that way. Took
/// us one debug round to spot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: i64,
    pub pilot_id: i64,
    /// Formatted pilot identifier, e.g. `GSG0001`.
    #[serde(default)]
    pub ident: Option<String>,
    pub name: String,
    pub email: Option<String>,
    pub airline_id: Option<i64>,
    /// ICAO of the airport the pilot is currently at.
    #[serde(default, alias = "curr_airport_id")]
    pub curr_airport: Option<String>,
    /// ICAO of the pilot's home airport.
    #[serde(default, alias = "home_airport_id")]
    pub home_airport: Option<String>,
    #[serde(default)]
    pub airline: Option<Airline>,
    #[serde(default)]
    pub rank: Option<Rank>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rank {
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Airline {
    pub id: i64,
    pub icao: String,
    pub iata: Option<String>,
    pub name: String,
    /// Optional URL to the airline's logo, exposed by phpVMS Airline resources.
    /// May be absent or empty depending on VA configuration.
    #[serde(default)]
    pub logo: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Airport {
    pub id: String,
    #[serde(default)]
    pub icao: Option<String>,
    #[serde(default)]
    pub iata: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub lat: Option<f64>,
    #[serde(default)]
    pub lon: Option<f64>,
    #[serde(default)]
    pub elevation: Option<f64>,
}

/// phpVMS exposes distance as a multi-unit object:
/// `{ "m": 483372, "km": 483.37, "mi": 300.35, "nmi": 261 }`.
/// Any of these may be missing depending on the serializer.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Distance {
    #[serde(default)]
    pub m: Option<f64>,
    #[serde(default)]
    pub mi: Option<f64>,
    #[serde(default)]
    pub km: Option<f64>,
    #[serde(default)]
    pub nmi: Option<f64>,
}

/// Subset of phpVMS's Flight resource we use in Phase 1. Permissive on optional fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Flight {
    pub id: String,
    /// phpVMS encodes this as a JSON number when the value is purely numeric
    /// (e.g. `284`) and as a string otherwise (e.g. `"VL12A"`). Accept both.
    #[serde(deserialize_with = "de_str_or_int")]
    pub flight_number: String,
    #[serde(default)]
    pub route_code: Option<String>,
    #[serde(default)]
    pub route_leg: Option<String>,
    #[serde(default)]
    pub callsign: Option<String>,
    pub dpt_airport_id: String,
    pub arr_airport_id: String,
    #[serde(default)]
    pub alt_airport_id: Option<String>,
    /// Scheduled flight time in minutes.
    #[serde(default)]
    pub flight_time: Option<i32>,
    /// Cruise level (e.g. 360 == FL360).
    #[serde(default)]
    pub level: Option<i32>,
    #[serde(default)]
    pub route: Option<String>,
    #[serde(default)]
    pub flight_type: Option<String>,
    #[serde(default)]
    pub distance: Option<Distance>,
    #[serde(default)]
    pub airline: Option<Airline>,
    #[serde(default)]
    pub dpt_airport: Option<Airport>,
    #[serde(default)]
    pub arr_airport: Option<Airport>,
    /// SimBrief OFP relation when the pilot has prepared this flight in SimBrief.
    #[serde(default)]
    pub simbrief: Option<SimBrief>,
}

/// SimBrief OFP record returned with a bid/flight when the pilot has prepared one.
/// `id` looks like `"1777622821_5F3E3B3842"` (a SimBrief-side identifier).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimBrief {
    pub id: String,
    /// JSON briefing endpoint exposed by phpVMS Core.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub aircraft_id: Option<i64>,
    /// SimBrief-derived subfleet info, including the fares (passenger / cargo
    /// counts) the OFP was generated against. We carry these forward into the
    /// final filed PIREP so the VA gets accurate load numbers.
    #[serde(default)]
    pub subfleet: Option<SimBriefSubfleet>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SimBriefSubfleet {
    #[serde(default)]
    pub type_: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub fares: Vec<Fare>,
}

/// Single fare-class entry. Fields are permissive — phpVMS varies which keys
/// it returns based on context (subfleet vs. PIREP file).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fare {
    pub id: i64,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub capacity: Option<i32>,
    /// Number of passengers (or cargo units) the OFP was generated for.
    #[serde(default)]
    pub count: Option<i32>,
    #[serde(default)]
    pub price: Option<f64>,
    /// 0 = passenger fare, 1 = cargo (phpVMS convention).
    #[serde(default, rename = "type")]
    pub fare_type: Option<i32>,
}

/// `GET /api/user/bids` returns a list of these.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bid {
    pub id: i64,
    pub user_id: i64,
    pub flight_id: String,
    pub flight: Flight,
}

// ---- PIREP lifecycle types ----

/// Body for `POST /api/pireps/prefile`.
/// phpVMS validates these; only the listed fields are required, the rest are
/// dropped if `None` (skipped via `skip_serializing_if`).
#[derive(Debug, Clone, Serialize, Default)]
pub struct PrefileBody {
    pub airline_id: i64,
    pub aircraft_id: String,
    pub flight_number: String,
    pub dpt_airport_id: String,
    pub arr_airport_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt_airport_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flight_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_leg: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planned_distance: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planned_flight_time: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    pub source_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Single position entry posted to `POST /api/pireps/{id}/acars/position`.
/// We map our internal `SimSnapshot` to this on the way out.
///
/// Field set tracks the phpVMS-Core `acars` model (lat/lon/heading/altitude/
/// altitude_agl/altitude_msl/gs/ias/vs/fuel/fuel_flow/transponder/autopilot/
/// distance/log/sim_time/source). Anything outside that schema (lights,
/// COM/NAV freqs, autopilot mode detail) goes into `log` as a compact JSON
/// blob so the live map / PIREP detail page can surface it without a custom
/// field per item.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PositionEntry {
    pub lat: f64,
    pub lon: f64,
    pub altitude: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub altitude_agl: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub altitude_msl: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading: Option<f32>,
    /// Groundspeed in knots.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gs: Option<f32>,
    /// Vertical speed in fpm.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vs: Option<f32>,
    /// Indicated airspeed in knots.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ias: Option<f32>,
    /// Total fuel on board, kilograms (phpVMS-Core column).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fuel: Option<f32>,
    /// Total fuel flow, kg/h (phpVMS-Core column).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fuel_flow: Option<f32>,
    /// 4-digit transponder / squawk code (phpVMS-Core column).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transponder: Option<u16>,
    /// Autopilot master on/off (phpVMS-Core column, stored as int 0/1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autopilot: Option<bool>,
    /// Distance to the destination in nautical miles (phpVMS-Core column).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance: Option<f64>,
    /// Free-form log line shown on the live map. We pack telemetry that
    /// phpVMS doesn't have first-class columns for (lights, com/nav,
    /// autopilot modes) here as compact JSON.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log: Option<String>,
    /// ISO-8601 UTC timestamp.
    pub sim_time: String,
}

/// Body for `POST /api/pireps/{id}/file` — final flight stats at submission.
#[derive(Debug, Clone, Serialize, Default)]
pub struct FileBody {
    /// Total flight time in minutes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flight_time: Option<i32>,
    /// Fuel used (units configured site-side).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fuel_used: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Final passenger / cargo loads per fare class.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fares: Option<Vec<FareEntry>>,
    /// Custom PIREP fields keyed by name. The VA admin configures the fields
    /// in their phpVMS / ACARS module; we send everything we can compute.
    /// Spec §24.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<std::collections::HashMap<String, String>>,
}

/// Minimal fare entry for filing — phpVMS uses `id` to look up the fare class
/// and `count` for the loaded amount.
#[derive(Debug, Clone, Serialize)]
pub struct FareEntry {
    pub id: i64,
    pub count: i32,
}

/// Body for `POST /api/pireps/{id}/update`. Used to advance the in-flight
/// status (Boarding, Pushback, Takeoff, Airborne, …) so the PIREP shows up
/// in the live-flight view.
#[derive(Debug, Clone, Serialize, Default)]
pub struct UpdateBody {
    /// phpVMS PirepState. 1 = IN_PROGRESS, 2 = PENDING, etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<i32>,
    /// phpVMS PirepStatus code (e.g. "BST" boarding, "OFB" pushback,
    /// "TKO" takeoff, "ENR" enroute, "APP" approach).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PirepCreated {
    pub id: String,
}

/// Lightweight PIREP record returned by `GET /api/user/pireps`. Used to find
/// an existing in-progress flight so we can resume it instead of doing a
/// fresh prefile (which would fail with aircraft-not-available because the
/// existing PIREP holds the aircraft "in use").
#[derive(Debug, Clone, Deserialize)]
pub struct PirepSummary {
    pub id: String,
    #[serde(default)]
    pub airline_id: Option<i64>,
    #[serde(default)]
    pub flight_number: Option<String>,
    #[serde(default)]
    pub aircraft_id: Option<i64>,
    /// phpVMS PirepState: 0=IN_PROGRESS, 1=PENDING, 2=ACCEPTED, 3=CANCELLED, …
    #[serde(default)]
    pub state: Option<i32>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub dpt_airport_id: Option<String>,
    #[serde(default)]
    pub arr_airport_id: Option<String>,
}

/// Subset of `GET /api/fleet/aircraft/{id}` we use for diagnostic purposes,
/// e.g. when phpVMS rejects a prefile with `aircraft-not-available`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AircraftDetails {
    pub id: i64,
    #[serde(default)]
    pub registration: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub icao: Option<String>,
    /// ICAO of the airport where the aircraft is currently parked.
    #[serde(default)]
    pub airport_id: Option<String>,
    /// 0 = parked, 1 = in use, 2 = in flight (phpVMS AircraftState).
    #[serde(default)]
    pub state: Option<i32>,
    /// "A" active, "S" stored, etc.
    #[serde(default)]
    pub status: Option<String>,
}

// phpVMS resource responses are wrapped: `{ "data": {...} }`.
#[derive(Deserialize)]
struct DataEnvelope<T> {
    data: T,
}

// ---- Client ----

/// A reusable client. `Clone` is cheap because the inner reqwest client is
/// `Arc`-backed and `Connection` only holds a URL + API key string.
#[derive(Clone)]
pub struct Client {
    http: HttpClient,
    conn: Connection,
}

impl Client {
    pub fn new(conn: Connection) -> Result<Self, ApiError> {
        let user_agent = format!("CloudeAcars/{}", env!("CARGO_PKG_VERSION"));
        let http = HttpClient::builder()
            .user_agent(user_agent)
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .map_err(ApiError::from)?;
        Ok(Self { http, conn })
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    fn endpoint(&self, path: &str) -> Result<Url, ApiError> {
        let path = path.trim_start_matches('/');
        let joined = format!(
            "{}/{}",
            self.conn.base_url.as_str().trim_end_matches('/'),
            path
        );
        Url::parse(&joined).map_err(|_| ApiError::InvalidUrl(joined))
    }

    /// GET a `{ "data": T }` resource and decode it.
    async fn get_data<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let url = self.endpoint(path)?;
        let response = self
            .http
            .get(url)
            .header("X-API-Key", &self.conn.api_key)
            .header(header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(ApiError::from)?;

        let response = check_status(response, path).await?;
        // Read the body as text so we can log a snippet on decode failure —
        // the default error from `response.json()` doesn't include the offending
        // JSON, which makes API-shape mismatches very hard to diagnose.
        let body = response
            .text()
            .await
            .map_err(|e| ApiError::BadResponse(format!("read body for {path}: {e}")))?;
        // Logged at DEBUG so it's silent by default but available for diagnosing
        // schema mismatches on a new VA via `RUST_LOG=api_client=debug`.
        if tracing::enabled!(tracing::Level::DEBUG) {
            let head: String = body.chars().take(2000).collect();
            tracing::debug!(path = %path, body_len = body.len(), head = %head, "response body");
        }
        match serde_json::from_str::<DataEnvelope<T>>(&body) {
            Ok(envelope) => Ok(envelope.data),
            Err(e) => {
                let snippet: String = body.chars().take(800).collect();
                tracing::warn!(
                    path = %path,
                    error = %e,
                    body_len = body.len(),
                    body_snippet = %snippet,
                    "JSON decode failed"
                );
                Err(ApiError::BadResponse(format!(
                    "JSON decode failed for {path}: {e}"
                )))
            }
        }
    }

    /// `GET /api/user`
    pub async fn get_profile(&self) -> Result<Profile, ApiError> {
        self.get_data("/api/user").await
    }

    /// `GET /api/user/bids`
    pub async fn get_bids(&self) -> Result<Vec<Bid>, ApiError> {
        self.get_data("/api/user/bids").await
    }

    /// `GET /api/user/pireps` — pilot's PIREPs (any state). Used during
    /// flight_start to find an existing in-progress PIREP we should resume
    /// rather than colliding with a fresh prefile.
    pub async fn get_user_pireps(&self) -> Result<Vec<PirepSummary>, ApiError> {
        self.get_data("/api/user/pireps").await
    }

    /// `GET /api/airports/{icao}` — single airport lookup with coordinates.
    pub async fn get_airport(&self, icao: &str) -> Result<Airport, ApiError> {
        let path = format!("/api/airports/{}", icao.trim().to_uppercase());
        self.get_data(&path).await
    }

    /// POST a JSON body and decode the response envelope `{ data: T }`.
    async fn post_data<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let url = self.endpoint(path)?;
        let response = self
            .http
            .post(url)
            .header("X-API-Key", &self.conn.api_key)
            .header(header::ACCEPT, "application/json")
            .json(body)
            .send()
            .await
            .map_err(ApiError::from)?;
        let response = check_status(response, path).await?;
        let text = response
            .text()
            .await
            .map_err(|e| ApiError::BadResponse(format!("read body for {path}: {e}")))?;
        if tracing::enabled!(tracing::Level::DEBUG) {
            let head: String = text.chars().take(2000).collect();
            tracing::debug!(path = %path, body_len = text.len(), head = %head, "post response");
        }
        match serde_json::from_str::<DataEnvelope<T>>(&text) {
            Ok(envelope) => Ok(envelope.data),
            Err(e) => {
                tracing::warn!(path = %path, error = %e, "JSON decode failed for POST response");
                Err(ApiError::BadResponse(format!(
                    "JSON decode failed for {path}: {e}"
                )))
            }
        }
    }

    /// POST and ignore the response body (status-check only).
    async fn post_void<B: Serialize>(&self, path: &str, body: &B) -> Result<(), ApiError> {
        let url = self.endpoint(path)?;
        let response = self
            .http
            .post(url)
            .header("X-API-Key", &self.conn.api_key)
            .header(header::ACCEPT, "application/json")
            .json(body)
            .send()
            .await
            .map_err(ApiError::from)?;
        let _ = check_status(response, path).await?;
        Ok(())
    }

    /// DELETE and ignore the response body (status-check only).
    async fn delete_void(&self, path: &str) -> Result<(), ApiError> {
        let url = self.endpoint(path)?;
        let response = self
            .http
            .delete(url)
            .header("X-API-Key", &self.conn.api_key)
            .header(header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(ApiError::from)?;
        let _ = check_status(response, path).await?;
        Ok(())
    }

    /// `DELETE /api/user/bids/{bid_id}` — drop a bid after its PIREP was filed
    /// (or to give it back). phpVMS does NOT auto-consume bids when the PIREP
    /// is filed unless we explicitly remove them.
    pub async fn delete_bid(&self, bid_id: i64) -> Result<(), ApiError> {
        let path = format!("/api/user/bids/{bid_id}");
        self.delete_void(&path).await
    }

    /// `POST /api/pireps/prefile` — create an in-flight PIREP.
    pub async fn prefile_pirep(&self, body: &PrefileBody) -> Result<PirepCreated, ApiError> {
        self.post_data("/api/pireps/prefile", body).await
    }

    /// `POST /api/pireps/{pirep_id}/acars/position` — push a batch of positions.
    pub async fn post_positions(
        &self,
        pirep_id: &str,
        positions: &[PositionEntry],
    ) -> Result<(), ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            positions: &'a [PositionEntry],
        }
        let path = format!("/api/pireps/{pirep_id}/acars/position");
        self.post_void(&path, &Body { positions }).await
    }

    /// `POST /api/pireps/{pirep_id}/file` — submit the PIREP.
    pub async fn file_pirep(&self, pirep_id: &str, body: &FileBody) -> Result<(), ApiError> {
        let path = format!("/api/pireps/{pirep_id}/file");
        self.post_void(&path, body).await
    }

    /// `POST /api/pireps/{pirep_id}/cancel` — cancel an in-flight PIREP.
    pub async fn cancel_pirep(&self, pirep_id: &str) -> Result<(), ApiError> {
        #[derive(Serialize)]
        struct Empty {}
        let path = format!("/api/pireps/{pirep_id}/cancel");
        self.post_void(&path, &Empty {}).await
    }

    /// `POST /api/pireps/{pirep_id}/update` — change PIREP status/state during flight.
    pub async fn update_pirep(&self, pirep_id: &str, body: &UpdateBody) -> Result<(), ApiError> {
        let path = format!("/api/pireps/{pirep_id}/update");
        self.post_void(&path, body).await
    }

    /// `GET /api/fleet/aircraft/{id}` — single aircraft, used for diagnostics.
    /// We also try `/api/aircraft/{id}` as a fallback because phpVMS deployments
    /// vary on this exact path.
    pub async fn get_aircraft(&self, id: i64) -> Result<AircraftDetails, ApiError> {
        match self
            .get_data::<AircraftDetails>(&format!("/api/fleet/aircraft/{id}"))
            .await
        {
            Ok(a) => Ok(a),
            Err(ApiError::NotFound) => {
                self.get_data(&format!("/api/aircraft/{id}")).await
            }
            Err(e) => Err(e),
        }
    }
}

async fn check_status(response: Response, path: &str) -> Result<Response, ApiError> {
    let status = response.status();
    if status == StatusCode::OK {
        return Ok(response);
    }
    match status {
        StatusCode::UNAUTHORIZED => Err(ApiError::Unauthenticated),
        StatusCode::FORBIDDEN => Err(ApiError::Forbidden),
        StatusCode::NOT_FOUND => Err(ApiError::NotFound),
        StatusCode::TOO_MANY_REQUESTS => {
            let retry_after_seconds = response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(60);
            Err(ApiError::RateLimited { retry_after_seconds })
        }
        s => {
            let body = response.text().await.unwrap_or_default();
            tracing::warn!(%s, %path, body_len = body.len(), "phpVMS returned non-OK");
            Err(ApiError::Server {
                status: s.as_u16(),
                body,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_http_scheme() {
        let err = Connection::new("ftp://example.com", "k").unwrap_err();
        assert!(matches!(err, ApiError::InvalidUrl(_)));
    }

    #[test]
    fn accepts_https() {
        Connection::new("https://example.com", "k").unwrap();
    }

    #[test]
    fn accepts_http_localhost() {
        Connection::new("http://localhost:8000", "k").unwrap();
    }

    #[test]
    fn strips_trailing_slash() {
        let c = Connection::new("https://example.com/", "k").unwrap();
        assert!(!c.base_url().ends_with("//"));
    }
}
