//! Configurable structured operational logging for `id` (#1791).
//!
//! This module is the configuration and formatting core for the tracing
//! setup in `main.rs`. It is deliberately distinct from the #1063
//! administrative audit records: these are the process's own operational
//! logs (the ones an operator reads while running a node), not audit data.
//!
//! # Configuration (environment variables)
//!
//! All knobs are opt-in; with none of them set the process logs exactly
//! one plain-text stream to stderr, as it always has.
//!
//! - `ID_LOG_FORMAT` — `text` (default) or `json`. `json` emits one
//!   JSON object per line (JSON-lines) including the event's fields and,
//!   for events inside a span, the current span's name and fields as
//!   trace context. An unrecognized value falls back to `text` and the
//!   caller warns about the fallback (as `ID_LOG_TZ` does).
//! - `ID_LOG_FILE` — path of a second destination. Events are appended
//!   to the file in the same format as the console destination, without
//!   ANSI escapes.
//! - `ID_LOG_FILE_LEVEL` — an independent filter for the file
//!   destination (anything `tracing_subscriber::EnvFilter` accepts,
//!   e.g. `debug` or `id=trace,iroh=info`). Without it the file uses the
//!   same resolved filter as the console, so e.g. the console can stay
//!   at `info` while the file takes `debug`.
//! - `ID_LOG_TZ` — timezone for log timestamps: an IANA name
//!   (`America/Chicago`), `UTC`, `local`, or a fixed offset (`+05:30`).
//!   Default is the host's local timezone; an unrecognized value falls
//!   back to UTC and the caller warns about the fallback.
//!
//! # Timestamps
//!
//! Timestamps are RFC 3339 with nanosecond precision in the configured
//! zone, e.g. `2026-11-03T22:00:00.123456789-06:00`. Applying the
//! host-local zone by default is the one deliberate change to the
//! pre-#1791 default output (which used UTC); levels, destinations, and
//! filtering are otherwise unchanged.

use std::{
    fs::OpenOptions,
    io,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::SystemTime,
};

use chrono::{DateTime, FixedOffset, Local, SecondsFormat, Utc};
use tracing_subscriber::{
    field::RecordFields,
    fmt::{
        format::{DefaultFields, FormatFields, Writer},
        time::FormatTime,
    },
};

/// Output format for a log destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    /// Human-readable plain text (the historical default).
    Text,
    /// One JSON object per line, with span/trace context when present.
    Json,
}

impl LogFormat {
    /// Parse the `ID_LOG_FORMAT` value; anything unrecognized is `Text`.
    pub fn parse(raw: Option<&str>) -> Self {
        match raw
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref()
        {
            Some("json") => Self::Json,
            _ => Self::Text,
        }
    }
}

/// A resolved timestamp zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogZone {
    /// The host's local timezone (the default).
    Local,
    /// Coordinated Universal Time.
    Utc,
    /// A fixed offset from UTC, e.g. `+05:30`.
    Fixed(FixedOffset),
    /// A named IANA timezone, with its DST rules.
    Named(chrono_tz::Tz),
}

impl LogZone {
    /// Format `at` as RFC 3339 with nanosecond precision in this zone.
    pub fn format_timestamp(&self, at: SystemTime) -> String {
        let utc: DateTime<Utc> = DateTime::from(at);
        match self {
            Self::Local => utc
                .with_timezone(&Local)
                .to_rfc3339_opts(SecondsFormat::Nanos, false),
            Self::Utc => utc.to_rfc3339_opts(SecondsFormat::Nanos, true),
            Self::Fixed(offset) => utc
                .with_timezone(offset)
                .to_rfc3339_opts(SecondsFormat::Nanos, false),
            Self::Named(tz) => utc
                .with_timezone(tz)
                .to_rfc3339_opts(SecondsFormat::Nanos, false),
        }
    }
}

/// The outcome of resolving an `ID_LOG_TZ` specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TzResolution {
    /// The zone to stamp log lines with.
    pub zone: LogZone,
    /// True when the requested zone was not recognized and UTC is
    /// being used instead; the caller surfaces this as a warning.
    pub fell_back_to_utc: bool,
}

/// Resolve an `ID_LOG_TZ` value: unset/`local` → host local zone,
/// `UTC`/`Z` → UTC, an IANA name → that zone, `+HH:MM`/`-HH:MM` (or
/// `+HHMM`) → a fixed offset, anything else → UTC with the fallback
/// flag set.
pub fn resolve_zone(spec: Option<&str>) -> TzResolution {
    let ok = |zone: LogZone| TzResolution {
        zone,
        fell_back_to_utc: false,
    };
    match spec.map(str::trim) {
        None | Some("") => ok(LogZone::Local),
        Some(value) if value.eq_ignore_ascii_case("local") => ok(LogZone::Local),
        Some(value) if value.eq_ignore_ascii_case("utc") || value.eq_ignore_ascii_case("z") => {
            ok(LogZone::Utc)
        }
        Some(value) => {
            if let Ok(tz) = value.parse::<chrono_tz::Tz>() {
                ok(LogZone::Named(tz))
            } else if let Some(offset) = parse_fixed_offset(value) {
                ok(LogZone::Fixed(offset))
            } else {
                TzResolution {
                    zone: LogZone::Utc,
                    fell_back_to_utc: true,
                }
            }
        }
    }
}

/// Parse a fixed UTC offset: `+05:30`, `-04:00`, or `+0530`.
fn parse_fixed_offset(value: &str) -> Option<FixedOffset> {
    let (sign, rest) = match value.as_bytes().first()? {
        b'+' => (1, &value[1..]),
        b'-' => (-1, &value[1..]),
        _ => return None,
    };
    // Accept only `HHMM` or `HH:MM`: a colon is valid solely as the single
    // separator between the hour and minute pairs (#5369). Stripping every
    // colon first let malformed shapes such as `+05::30` and `+0:5:30`
    // silently resolve as `+05:30`.
    let digits: String = if rest.len() == 4 {
        rest.to_string()
    } else if rest.len() == 5 && rest.as_bytes()[2] == b':' {
        format!("{}{}", &rest[..2], &rest[3..])
    } else {
        return None;
    };
    if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let hours: i32 = digits[..2].parse().ok()?;
    let minutes: i32 = digits[2..].parse().ok()?;
    if hours > 23 || minutes > 59 {
        return None;
    }
    FixedOffset::east_opt(sign * (hours * 3600 + minutes * 60))
}

/// A `tracing_subscriber` timer that stamps events with
/// [`LogZone::format_timestamp`].
#[derive(Debug, Clone, Copy)]
pub struct Rfc3339TzTimer {
    /// The zone timestamps are rendered in.
    pub zone: LogZone,
}

impl FormatTime for Rfc3339TzTimer {
    fn format_time(&self, writer: &mut Writer<'_>) -> std::fmt::Result {
        write!(writer, "{}", self.zone.format_timestamp(SystemTime::now()))
    }
}

/// Fully resolved logging configuration for one process start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogConfig {
    /// Format applied to every configured destination.
    pub format: LogFormat,
    /// Optional file destination (appended to).
    pub file: Option<PathBuf>,
    /// Optional independent filter for the file destination, verbatim
    /// as given (an `EnvFilter` string); `None` reuses the console's
    /// resolved filter.
    pub file_filter: Option<String>,
    /// Timestamp zone.
    pub zone: LogZone,
    /// True when `ID_LOG_TZ` was set but unrecognized (zone is UTC).
    pub tz_fell_back: bool,
    /// True when `ID_LOG_FORMAT` was set to a non-empty value that is
    /// neither `text` nor `json` (format is `Text`); the caller warns
    /// about the fallback, mirroring `tz_fell_back`.
    pub format_fell_back: bool,
}

impl LogConfig {
    /// Resolve a configuration from raw environment values. Pure: the
    /// caller reads the environment, so tests pin the whole table
    /// without mutating process state.
    pub fn resolve(
        format: Option<&str>,
        file: Option<&str>,
        file_level: Option<&str>,
        tz: Option<&str>,
    ) -> Self {
        let tz = resolve_zone(tz);
        Self {
            format: LogFormat::parse(format),
            format_fell_back: format.is_some_and(|value| {
                let value = value.trim().to_ascii_lowercase();
                !value.is_empty() && value != "text" && value != "json"
            }),
            file: file
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
            file_filter: file_level
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
            zone: tz.zone,
            tz_fell_back: tz.fell_back_to_utc,
        }
    }

    /// The filter string for the file destination: the independent
    /// override when one is configured, otherwise the console's
    /// resolved filter — this is what lets the two destinations run at
    /// different levels.
    pub fn file_filter_string(&self, base_filter: &str) -> String {
        self.file_filter
            .clone()
            .unwrap_or_else(|| base_filter.to_owned())
    }
}

/// Field formatter for the file destination: a distinct type
/// wrapping [`DefaultFields`].
///
/// `tracing_subscriber` caches each span's formatted fields in the
/// span's extensions, keyed by the formatter's type. If the file
/// layer used [`DefaultFields`] directly (like the console layer),
/// whichever layer formatted a span first would populate the shared
/// cache — and the console formats with ANSI styling, so escape
/// sequences would leak into the log file even with
/// `.with_ansi(false)` on the file layer. Giving the file layer its
/// own formatter type gives it its own cache slot, formatted without
/// ANSI. The rendered field text is otherwise identical to
/// [`DefaultFields`].
#[derive(Debug, Default)]
pub struct PlainFields {
    inner: DefaultFields,
}

impl PlainFields {
    /// Create a `PlainFields` formatter.
    pub fn new() -> Self {
        Self {
            inner: DefaultFields::new(),
        }
    }
}

impl<'writer> FormatFields<'writer> for PlainFields {
    fn format_fields<R: RecordFields>(
        &self,
        writer: Writer<'writer>,
        fields: R,
    ) -> std::fmt::Result {
        self.inner.format_fields(writer, fields)
    }
}

/// A cloneable, append-mode file writer for a log destination.
///
/// Each clone shares one file behind a mutex; `tracing_subscriber`
/// asks the writer factory for a writer per event, so clones write
/// whole events without interleaving.
#[derive(Clone, Debug)]
pub struct SharedFileWriter {
    inner: Arc<Mutex<std::fs::File>>,
}

impl SharedFileWriter {
    /// Open (creating if needed) `path` for appending, creating any
    /// missing parent directories first.
    pub fn open(path: &std::path::Path) -> io::Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(file)),
        })
    }
}

impl io::Write for SharedFileWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut file = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("log file lock poisoned"))?;
        file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut file = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("log file lock poisoned"))?;
        file.flush()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::time::Duration;

    /// 2026-11-04T04:00:00.123456789Z — after the US fall-back, so
    /// America/Chicago is on CST (-06:00).
    fn probe_time() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::new(1_793_764_800, 123_456_789)
    }

    #[test]
    fn structured_logging_default_config_is_text_stderr_only() {
        let config = LogConfig::resolve(None, None, None, None);
        assert_eq!(config.format, LogFormat::Text);
        assert_eq!(config.file, None);
        assert_eq!(config.file_filter, None);
        assert_eq!(config.zone, LogZone::Local);
        assert!(!config.tz_fell_back);
        assert!(!config.format_fell_back);
    }

    #[test]
    fn structured_logging_format_parse() {
        assert_eq!(LogFormat::parse(Some("json")), LogFormat::Json);
        assert_eq!(LogFormat::parse(Some(" JSON ")), LogFormat::Json);
        assert_eq!(LogFormat::parse(Some("text")), LogFormat::Text);
        assert_eq!(LogFormat::parse(Some("yaml")), LogFormat::Text);
        assert_eq!(LogFormat::parse(None), LogFormat::Text);
    }

    #[test]
    fn test_structured_logging_unrecognized_format_falls_back_with_flag() {
        // An unrecognized ID_LOG_FORMAT still resolves to Text (the
        // lenient parse is unchanged), but the config now records the
        // fallback so the caller can warn — mirroring tz_fell_back.
        for spec in ["josn", "yaml", "jsonl", "structured"] {
            let config = LogConfig::resolve(Some(spec), None, None, None);
            assert_eq!(config.format, LogFormat::Text, "spec {spec:?}");
            assert!(config.format_fell_back, "spec {spec:?}");
        }
        // Recognized values (any case, padded) and unset/blank values
        // never set the flag.
        for spec in [
            None,
            Some(""),
            Some("  "),
            Some("text"),
            Some("Text"),
            Some("json"),
            Some(" JSON "),
        ] {
            let config = LogConfig::resolve(spec, None, None, None);
            assert!(!config.format_fell_back, "spec {spec:?}");
        }
    }

    #[test]
    fn structured_logging_utc_timestamp_is_rfc3339_nanos() {
        assert_eq!(
            LogZone::Utc.format_timestamp(probe_time()),
            "2026-11-04T04:00:00.123456789Z"
        );
    }

    #[test]
    fn structured_logging_named_zone_applies_dst_aware_offset() {
        let zone = resolve_zone(Some("America/Chicago"));
        assert!(!zone.fell_back_to_utc);
        assert_eq!(
            zone.zone.format_timestamp(probe_time()),
            "2026-11-03T22:00:00.123456789-06:00"
        );
    }

    #[test]
    fn structured_logging_fixed_offset_timestamp() {
        let zone = resolve_zone(Some("+05:30"));
        assert!(!zone.fell_back_to_utc);
        assert_eq!(
            zone.zone.format_timestamp(probe_time()),
            "2026-11-04T09:30:00.123456789+05:30"
        );
        let compact = resolve_zone(Some("-0400"));
        assert_eq!(
            compact.zone.format_timestamp(probe_time()),
            "2026-11-04T00:00:00.123456789-04:00"
        );
    }

    #[test]
    fn test_structured_logging_malformed_fixed_offset_falls_back_to_utc() {
        // #5369: only signed `HHMM` or `HH:MM` are valid fixed offsets.
        // Doubled, shifted, or trailing colons (and out-of-range parts)
        // are malformed and must fall back to UTC with the flag set.
        for spec in [
            "+05::30", "+0:5:30", "+0530:", "+05:300", "+5:30", "+24:00", "+05:60",
        ] {
            let zone = resolve_zone(Some(spec));
            assert!(zone.fell_back_to_utc, "spec {spec:?}");
            assert_eq!(zone.zone, LogZone::Utc, "spec {spec:?}");
        }
        // Valid controls keep resolving without the fallback flag.
        for spec in ["+05:30", "-0400", "+0000"] {
            let zone = resolve_zone(Some(spec));
            assert!(!zone.fell_back_to_utc, "spec {spec:?}");
        }
    }

    #[test]
    fn structured_logging_bad_zone_falls_back_to_utc_with_flag() {
        let zone = resolve_zone(Some("Mars/Olympus_Mons"));
        assert!(zone.fell_back_to_utc);
        assert_eq!(zone.zone, LogZone::Utc);
        assert_eq!(
            zone.zone.format_timestamp(probe_time()),
            "2026-11-04T04:00:00.123456789Z"
        );
        // `local` is an explicit choice, never a fallback.
        let local = resolve_zone(Some("local"));
        assert!(!local.fell_back_to_utc);
        assert_eq!(local.zone, LogZone::Local);
    }

    #[test]
    fn structured_logging_lowercase_z_is_utc_without_fallback() {
        // RFC 3339 allows the lowercase designator, and the neighbouring
        // recognized words are already case-insensitive, so `z` resolves
        // to UTC without the fallback flag that warns the operator (#5395).
        for spec in ["z", "Z", "utc", "Utc", "UTC"] {
            let zone = resolve_zone(Some(spec));
            assert_eq!(zone.zone, LogZone::Utc, "spec {spec:?}");
            assert!(!zone.fell_back_to_utc, "spec {spec:?}");
        }
        for spec in ["local", "Local", "LOCAL"] {
            let zone = resolve_zone(Some(spec));
            assert_eq!(zone.zone, LogZone::Local, "spec {spec:?}");
            assert!(!zone.fell_back_to_utc, "spec {spec:?}");
        }
    }

    #[test]
    fn structured_logging_file_destination_filters_are_independent() {
        let config = LogConfig::resolve(
            Some("json"),
            Some("/var/log/id/id.log"),
            Some("debug"),
            Some("UTC"),
        );
        assert_eq!(
            config.file.as_deref(),
            Some(std::path::Path::new("/var/log/id/id.log"))
        );
        // File override wins over the console's resolved filter.
        assert_eq!(config.file_filter_string("info"), "debug");
        // Without an override the file reuses the console filter.
        let plain = LogConfig::resolve(None, Some("id.log"), None, None);
        assert_eq!(plain.file_filter_string("warn,iroh=info"), "warn,iroh=info");
        // Blank values count as unset.
        let blank = LogConfig::resolve(None, Some("  "), Some(""), None);
        assert_eq!(blank.file, None);
        assert_eq!(blank.file_filter, None);
    }

    #[test]
    fn structured_logging_shared_file_writer_appends_through_clones() {
        let dir =
            std::env::temp_dir().join(format!("id-structured-logging-test-{}", std::process::id()));
        let path = dir.join("nested").join("id.log");
        let mut first = SharedFileWriter::open(&path).expect("open log file");
        let mut second = first.clone();
        first.write_all(b"one\n").expect("write one");
        second.write_all(b"two\n").expect("write two");
        drop(first);
        drop(second);
        let contents = std::fs::read_to_string(&path).expect("read log file");
        assert_eq!(contents, "one\ntwo\n");
        std::fs::remove_dir_all(&dir).expect("clean up temp dir");
    }

    #[test]
    fn structured_logging_json_line_carries_timestamp_fields_and_span() {
        use tracing_subscriber::{Layer, layer::SubscriberExt};

        let buffer = Arc::new(Mutex::new(Vec::<u8>::new()));
        let writer = buffer.clone();
        let layer = tracing_subscriber::fmt::layer()
            .json()
            .with_writer(move || SharedVecWriter(writer.clone()))
            .with_timer(Rfc3339TzTimer { zone: LogZone::Utc })
            .with_current_span(true)
            .with_span_list(true)
            .with_filter(tracing_subscriber::EnvFilter::new("trace"));
        let subscriber = tracing_subscriber::registry().with(layer);

        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("probe_span", probe_id = 42);
            let _entered = span.enter();
            tracing::info!(answer = 42, "structured logging probe");
        });

        let bytes = buffer.lock().expect("buffer lock").clone();
        let line = String::from_utf8(bytes).expect("utf8 log line");
        let line = line.trim();
        let value: serde_json::Value = serde_json::from_str(line).expect("json log line");
        let timestamp = value["timestamp"].as_str().expect("timestamp field");
        assert!(timestamp.ends_with('Z'), "timestamp: {timestamp}");
        // RFC 3339 nanoseconds: exactly 9 fractional digits.
        let fraction = timestamp
            .split('.')
            .nth(1)
            .expect("fractional seconds in timestamp");
        assert_eq!(
            fraction.trim_end_matches('Z').len(),
            9,
            "timestamp: {timestamp}"
        );
        assert_eq!(value["level"], "INFO");
        assert_eq!(value["fields"]["answer"], 42);
        // Span context rides along as trace context.
        let span = &value["span"];
        let spans = &value["spans"];
        assert!(
            span["name"] == "probe_span" || spans.to_string().contains("probe_span"),
            "span context in line: {line}"
        );
    }

    #[test]
    fn structured_logging_file_layer_span_fields_stay_plain() {
        use tracing_subscriber::{Layer as _, fmt::format::FmtSpan, layer::SubscriberExt};

        let console_buffer = Arc::new(Mutex::new(Vec::<u8>::new()));
        let file_buffer = Arc::new(Mutex::new(Vec::<u8>::new()));

        // Same layer order as production (console first, file second):
        // with a shared formatter type the console's ANSI-styled span
        // fields would populate the cache the file layer reuses.
        let console_layer = tracing_subscriber::fmt::layer()
            .with_writer({
                let writer = console_buffer.clone();
                move || SharedVecWriter(writer.clone())
            })
            .with_ansi(true)
            .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
            .with_filter(tracing_subscriber::EnvFilter::new("trace"))
            .boxed();
        let file_layer = tracing_subscriber::fmt::layer()
            .fmt_fields(PlainFields::new())
            .with_writer({
                let writer = file_buffer.clone();
                move || SharedVecWriter(writer.clone())
            })
            .with_ansi(false)
            .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
            .with_filter(tracing_subscriber::EnvFilter::new("trace"))
            .boxed();
        let subscriber = tracing_subscriber::registry().with(vec![console_layer, file_layer]);

        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("ansi_probe", marker = "plainvalue");
            let _entered = span.enter();
            tracing::info!("inside the probe span");
        });

        let console_text = String::from_utf8(console_buffer.lock().expect("console lock").clone())
            .expect("console utf8");
        let file_text =
            String::from_utf8(file_buffer.lock().expect("file lock").clone()).expect("file utf8");
        assert!(
            console_text.contains('\u{1b}'),
            "console output should carry ANSI styling: {console_text:?}"
        );
        assert!(
            file_text.contains("plainvalue"),
            "file output should contain the span field: {file_text:?}"
        );
        assert!(
            !file_text.contains('\u{1b}'),
            "file output must not contain ANSI escapes: {file_text:?}"
        );
    }

    /// Minimal `io::Write` over a shared byte buffer, for the JSON
    /// probe above.
    #[derive(Clone)]
    struct SharedVecWriter(Arc<Mutex<Vec<u8>>>);

    impl io::Write for SharedVecWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .map_err(|_| io::Error::other("buffer lock poisoned"))?
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}
