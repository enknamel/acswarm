//! The files acswarm keeps between runs: where they live, and reading and
//! writing them whole.
//!
//! One crate so every caller agrees on the directories, so no write can
//! leave half a file behind ([`write_atomic`]: temp file, fsync, rename),
//! and so a file holding a password can say so ([`Visibility::Private`]).
//! It depends on serde alone, which is what lets the launcher share it
//! without pulling in the game crates.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;

/// The user's home, from `$HOME`; `.` when it is unset, so a relative
/// path is built rather than one hanging off the filesystem root.
pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The client's config directory: `$ACSWARM_CONFIG_DIR`, else
/// `~/.config/acswarm`. Settings, the login store and the loot profiles.
pub fn config_dir() -> PathBuf {
    match std::env::var_os("ACSWARM_CONFIG_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => home().join(".config").join("acswarm"),
    }
}

/// The launcher's own directory, `~/.acswarm`: its config, its logs and
/// the scripts. Kept apart from [`config_dir`]; merging the two would
/// need a migration.
pub fn app_dir() -> PathBuf {
    home().join(".acswarm")
}

/// Where the Rhai scripts are read from: `$ACSWARM_SCRIPTS`, else
/// [`app_dir`]`/scripts`.
pub fn scripts_dir() -> PathBuf {
    match std::env::var_os("ACSWARM_SCRIPTS") {
        Some(dir) => PathBuf::from(dir),
        None => app_dir().join("scripts"),
    }
}

/// Files that can be rebuilt from the archives (the world grid, holdings
/// snapshots): `$ACSWARM_CACHE_DIR`, else `~/.cache/acswarm`.
pub fn cache_dir() -> PathBuf {
    match std::env::var_os("ACSWARM_CACHE_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => home().join(".cache").join("acswarm"),
    }
}

/// Who may read a file this crate writes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Visibility {
    /// Whatever the umask allows, as for any config file.
    #[default]
    Normal,
    /// Its owner alone: mode 0600, and 0700 on its directory, on unix; a
    /// no-op elsewhere. For a file holding a password in the clear.
    Private,
}

/// Read `path` as JSON. `Ok(None)` when there is no file; a file that
/// cannot be read or will not parse is an error, so each caller decides
/// what that means to it rather than being handed an empty value.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Write `value` to `path` as pretty JSON with a closing newline -- the
/// shape the settings and login files already have -- through
/// [`write_atomic`].
pub fn write_json_atomic<T: Serialize + ?Sized>(
    path: &Path,
    value: &T,
    visibility: Visibility,
) -> io::Result<()> {
    write_with(path, visibility, |out| {
        serde_json::to_writer_pretty(&mut *out, value).map_err(io::Error::other)?;
        out.write_all(b"\n")
    })
}

/// Write `bytes` to `path`, creating its directory: into a temp file
/// beside it, flushed to the disk, then renamed over the target. What
/// was there is replaced whole or not at all, so a write that fails
/// partway leaves the old file readable rather than a truncated one.
pub fn write_atomic(path: &Path, bytes: &[u8], visibility: Visibility) -> io::Result<()> {
    write_with(path, visibility, |out| out.write_all(bytes))
}

/// The one write: fill a temp file beside `path`, then rename it over.
fn write_with(
    path: &Path,
    visibility: Visibility,
    fill: impl FnOnce(&mut dyn Write) -> io::Result<()>,
) -> io::Result<()> {
    let name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} names no file", path.display()),
        )
    })?;
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    std::fs::create_dir_all(dir)?;
    // Before anything is written, so a directory that cannot be closed
    // off fails the write instead of leaving the file open to read.
    if visibility == Visibility::Private {
        set_mode(dir, 0o700)?;
    }
    // Beside the target, so the rename stays on one filesystem; named
    // for the process, so two writing at once do not share a temp file.
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        name.to_string_lossy(),
        std::process::id()
    ));
    let written = (|| -> io::Result<()> {
        let mut file = create(&tmp, visibility)?;
        let mut out = io::BufWriter::new(&mut file);
        fill(&mut out)?;
        out.flush()?;
        drop(out);
        // On the disk before the rename: a crash in between leaves the
        // old file, never an empty new one.
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// The temp file. A private one is opened 0600, so it is owner-only from
/// its first byte and stays so through the rename, which carries the
/// mode with it.
fn create(path: &Path, visibility: Visibility) -> io::Result<std::fs::File> {
    let mut open = std::fs::OpenOptions::new();
    open.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        if visibility == Visibility::Private {
            open.mode(0o600);
        }
    }
    #[cfg(not(unix))]
    let _ = visibility;
    open.open(path)
}

/// Owner-only on unix, nothing elsewhere. Not encryption: anything
/// running as this user still reads the file; it closes the door on the
/// other accounts on the machine.
fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

/// A name safe to use as one path segment: letters and digits, and
/// `` -_+'. ``, are kept and anything else (a separator, a quote, a
/// control character) becomes `_`; leading and trailing dots and spaces
/// go, so the answer is never empty, `.` or `..`.
///
/// The loot profiles and the loot ledger keep sanitisers of their own:
/// they map to other characters, and renaming what a player already has
/// on disk would lose a profile, or a ledger's decisions.
pub fn file_safe(name: &str) -> String {
    let kept: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '+' | '\'' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let kept = kept.trim_matches(|c| c == '.' || c == ' ').to_string();
    if kept.is_empty() {
        "_".into()
    } else {
        kept
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::collections::BTreeMap;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Saved {
        name: String,
        count: u32,
    }

    /// A directory of this test's own, gone before it starts.
    fn scratch(what: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ac-store-{what}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[cfg(unix)]
    fn mode_of(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn the_directories_are_where_they_are_documented() {
        // The environment is read at every call and tests share one
        // process, so only the shape is checked here; `validate/run.sh`
        // exercises $ACSWARM_CONFIG_DIR and $ACSWARM_SCRIPTS for real.
        assert!(config_dir().ends_with("acswarm"));
        assert!(app_dir().ends_with(".acswarm") || std::env::var_os("HOME").is_none());
        assert!(
            scripts_dir().ends_with("scripts") || std::env::var_os("ACSWARM_SCRIPTS").is_some()
        );
        assert!(cache_dir().ends_with("acswarm"));
        assert!(!home().as_os_str().is_empty());
    }

    #[test]
    fn json_goes_out_and_comes_back() {
        let dir = scratch("json");
        let path = dir.join("nested").join("saved.json");
        // Nothing there yet is not an error: the caller decides.
        assert_eq!(read_json::<Saved>(&path).unwrap(), None);

        let saved = Saved {
            name: "Fletch".into(),
            count: 3,
        };
        write_json_atomic(&path, &saved, Visibility::Normal).unwrap();
        assert_eq!(read_json::<Saved>(&path).unwrap(), Some(saved));
        // Pretty, and newline-terminated.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("{\n  \"name\": \"Fletch\""), "{text}");
        assert!(text.ends_with("}\n"), "{text}");

        // A file that will not parse is an error, not an empty value.
        std::fs::write(&path, "{ not json").unwrap();
        let e = read_json::<Saved>(&path).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidData);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_private_write_is_readable_only_by_its_owner() {
        let dir = scratch("private");
        let path = dir.join("servers.json");
        let saved = Saved {
            name: "alice".into(),
            count: 1,
        };
        write_json_atomic(&path, &saved, Visibility::Private).unwrap();
        assert_eq!(mode_of(&path), 0o600, "the file");
        assert_eq!(mode_of(&dir), 0o700, "its directory");

        // Written again over a file that was left open to the world.
        set_mode(&path, 0o644).unwrap();
        set_mode(&dir, 0o755).unwrap();
        write_atomic(&path, b"{}", Visibility::Private).unwrap();
        assert_eq!(mode_of(&path), 0o600);
        assert_eq!(mode_of(&dir), 0o700);

        // A normal write beside it leaves that file to the umask and
        // does not loosen the directory again.
        write_atomic(&dir.join("ui.json"), b"{}", Visibility::Normal).unwrap();
        assert_eq!(mode_of(&dir), 0o700);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_write_that_fails_partway_leaves_the_old_file_alone() {
        let dir = scratch("interrupted");
        let path = dir.join("ledger.json");
        write_atomic(&path, b"{\"took\":{}}", Visibility::Normal).unwrap();

        // A map whose keys are not strings: serde_json opens the object
        // and then refuses the first key, so the temp file has been
        // written to by the time the write fails.
        let refused: BTreeMap<(u8, u8), u32> = BTreeMap::from([((1, 2), 3)]);
        let e = write_json_atomic(&path, &refused, Visibility::Normal).unwrap_err();
        assert!(e.to_string().contains("key must be a string"), "{e}");

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"took\":{}}");
        let left: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p != &path)
            .collect();
        assert!(left.is_empty(), "no temp file left behind: {left:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_safe_names_keep_the_ones_characters_have() {
        assert_eq!(file_safe("+Fletch"), "+Fletch");
        assert_eq!(file_safe("Al'Arqas of Yaraq"), "Al'Arqas of Yaraq");
        assert_eq!(file_safe("play.coldeve.ac"), "play.coldeve.ac");
        // No separator, no climbing out of the directory, never empty.
        assert_eq!(file_safe("../x/y"), "_x_y");
        assert_eq!(file_safe("  .. "), "_");
        assert_eq!(file_safe(""), "_");
        assert_eq!(file_safe("a\tb"), "a_b");
    }
}
