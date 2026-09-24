//! Turning the log up and down while the app is running.
//!
//! Every subsystem logs through `tracing`, and which of it reaches the
//! terminal is a filter: a default level, plus an override per target
//! (`warn,ac_client=info,ac_net=debug`). Setting that once at startup
//! through `RUST_LOG` is fine for a developer and useless to a player
//! watching a character do something odd, who wants to turn one part up
//! now and back down when they have seen it.
//!
//! So the filter is reloadable. [`init`] installs it, [`set`] replaces
//! it, and [`current`] says what it is. A file under the cache's `logs`
//! folder keeps its own fixed filter, so what happened can be read later.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use ac_plugin::logging::DEFAULT;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::layer::{Layer, SubscriberExt};
use tracing_subscriber::reload;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Registry;

type Handle = reload::Handle<EnvFilter, Registry>;

static RELOAD: OnceLock<Handle> = OnceLock::new();
static CURRENT: OnceLock<std::sync::Mutex<String>> = OnceLock::new();

/// Install the reloadable filter. `RUST_LOG` still wins when set, so a
/// developer's habits keep working; otherwise `saved` (from the
/// settings) or [`DEFAULT`].
pub fn init(saved: Option<&str>) {
    let text = std::env::var("RUST_LOG")
        .ok()
        .or_else(|| saved.map(str::to_string))
        .unwrap_or_else(|| DEFAULT.to_string());
    let filter = EnvFilter::try_new(&text).unwrap_or_else(|_| EnvFilter::new(DEFAULT));
    let (filter, handle) = reload::Layer::new(filter);
    let _ = RELOAD.set(handle);
    let _ = CURRENT.set(std::sync::Mutex::new(text));
    let file = open_file();
    let to_file = file.as_ref().map(|(w, _)| {
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .fmt_fields(PlainFields::default())
            .with_writer(w.clone())
            .with_filter(EnvFilter::new(FILE_FILTER))
    });
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(filter))
        .with(to_file)
        .init();
    if let Some((_, path)) = file {
        tracing::info!("logging to {}", path.display());
    }
}

/// What the file keeps whatever the terminal shows: why a character did what it did (its rules,
/// refusals, journeys, what it asked the server for), not chat, appraisals or the wire.
const FILE_FILTER: &str = "warn,acswarm=info,ac_plugin::sessions=info,ac_client::session=info,\
                           ac_client::session::chat=warn,ac_client::autoplay=info,\
                           ac_client::refused=info,ac_client::travel=info,ac_client::actions=info";

/// Log files kept in the folder, this process's included.
const KEEP_FILES: usize = 10;

/// Most one process writes to its file, in bytes; past it the file stops and the terminal goes on.
const FILE_CAP: u64 = 256 * 1024 * 1024;

/// This process's file, `acswarm-<unix secs>-<pid>.log` in the cache's `logs` folder, after
/// deleting all but the newest few. None when the folder cannot be written: the terminal still logs.
fn open_file() -> Option<(LogFile, PathBuf)> {
    let dir = ac_store::cache_dir().join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    let mut old: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("acswarm-") && n.ends_with(".log"))
        })
        .collect();
    // The names sort by the second they were opened in.
    old.sort();
    for stale in old.iter().rev().skip(KEEP_FILES - 1) {
        let _ = std::fs::remove_file(stale);
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let path = dir.join(format!("acswarm-{secs}-{}.log", std::process::id()));
    let file = std::fs::File::create(&path).ok()?;
    let log = LogFile(std::sync::Arc::new(Mutex::new(Capped {
        file,
        left: FILE_CAP,
    })));
    Some((log, path))
}

/// The default field format under a type of the file's own: span fields are formatted once per
/// formatter type and shared, so without it the file gets the terminal's colour codes.
#[derive(Default)]
struct PlainFields(tracing_subscriber::fmt::format::DefaultFields);

impl<'w> tracing_subscriber::fmt::FormatFields<'w> for PlainFields {
    fn format_fields<R: tracing_subscriber::field::RecordFields>(
        &self,
        writer: tracing_subscriber::fmt::format::Writer<'w>,
        fields: R,
    ) -> std::fmt::Result {
        self.0.format_fields(writer, fields)
    }
}

/// The log file, shared by every write; writing stops quietly at [`FILE_CAP`].
#[derive(Clone)]
struct LogFile(std::sync::Arc<Mutex<Capped>>);

struct Capped {
    file: std::fs::File,
    left: u64,
}

impl Write for LogFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let Ok(mut f) = self.0.lock() else {
            return Ok(buf.len());
        };
        if (buf.len() as u64) <= f.left {
            f.left -= buf.len() as u64;
            f.file.write_all(buf)?;
        } else {
            f.left = 0;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self.0.lock() {
            Ok(mut f) => f.file.flush(),
            Err(_) => Ok(()),
        }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogFile {
    type Writer = LogFile;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Replace the filter. Returns what is wrong with `text` rather than
/// applying it, so a half-typed line in a settings box does not blank
/// the log.
pub fn set(text: &str) -> Result<(), String> {
    let filter = EnvFilter::try_new(text).map_err(|e| e.to_string())?;
    let handle = RELOAD.get().ok_or("logging is not set up yet")?;
    handle.reload(filter).map_err(|e| e.to_string())?;
    if let Some(cur) = CURRENT.get() {
        if let Ok(mut c) = cur.lock() {
            *c = text.to_string();
        }
    }
    Ok(())
}

/// The filter in force.
pub fn current() -> String {
    CURRENT
        .get()
        .and_then(|c| c.lock().ok().map(|c| c.clone()))
        .unwrap_or_else(|| DEFAULT.to_string())
}
