//! `FsApplyRecordStore`: the real
//! `omnifrons_app::apply_record::ApplyRecordStore` (ADR-0005 sub-slice
//! 2c) under the product work area's `applies/<project identity hex>/`:
//! one `<plan id hex>-<nanoseconds>-<sequence>.json` per apply, owner-only
//! on unix (`0o600`, the directories `0o700`). A record first reserves
//! its final name with an exclusive create, so it never replaces an
//! earlier one: a rerun of one plan id at one instant takes the next
//! sequence number. The content is then written to a sibling temporary
//! file, synced, and renamed over the reservation (the reservation is
//! ours, so the overwrite is too), and the directory is synced where the
//! platform allows. No hard links are needed. A failure after the
//! reservation removes it; one left behind by a crash is an empty file,
//! which the listing skips as an unfinished record. A crash after the
//! temporary write leaves a `.apply.*.part` file; a later record removes
//! any older than one hour (a fresher one may belong to a live record). Any other file that
//! does not parse fails the whole listing closed as `Corrupt`.
//! Device-local, never a roaming payload.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use omnifrons_app::apply_record::{
    ApplyRecord, ApplyRecordError, ApplyRecordId, ApplyRecordStore, StoredApplyRecord,
    apply_record_name, parse_apply_record_name, sort_records_newest_first,
};
use omnifrons_app::work_area::WorkAreaRoot;
use omnifrons_domain::publication::ProjectIdentity;

use crate::apply_record_dto::ApplyRecordDto;
use crate::fs_snapshot_store::create_owner_only_dir;

/// The process-wide sequence behind the temporary names.
static PART_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// The name every temporary file starts with, and its extension.
const TEMPORARY_PREFIX: &str = ".apply.";
const TEMPORARY_EXTENSION: &str = "part";

/// How old a temporary file must be before a later record removes it.
const STALE_TEMPORARY_AGE: std::time::Duration = std::time::Duration::from_hours(1);

/// How many names one record tries before giving up: the name's sequence
/// is four digits wide.
const MAX_SEQUENCE: u32 = 10_000;

/// The file-per-apply [`ApplyRecordStore`] of one work area. Stateless
/// beyond its root: every call re-reads the directory.
#[derive(Debug, Clone)]
pub struct FsApplyRecordStore {
    root: PathBuf,
}

impl FsApplyRecordStore {
    /// The record file's extension.
    pub const EXTENSION: &'static str = "json";

    /// The store under `work_area`'s applies directory.
    #[must_use]
    pub fn open(work_area: &WorkAreaRoot) -> Self {
        Self {
            root: work_area.applies_dir(),
        }
    }

    fn project_dir(&self, project: &ProjectIdentity) -> PathBuf {
        self.root.join(project.to_hex())
    }

    fn record_path(dir: &Path, id: &ApplyRecordId) -> PathBuf {
        dir.join(format!("{id}.{}", Self::EXTENSION))
    }

    /// Write `bytes` to a fresh sibling temporary file, owner-only and
    /// synced; the caller publishes or removes it.
    fn write_temp(dir: &Path, bytes: &[u8]) -> std::io::Result<PathBuf> {
        let sequence = PART_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp = dir.join(format!(
            "{TEMPORARY_PREFIX}{}-{sequence}.{TEMPORARY_EXTENSION}",
            std::process::id()
        ));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        let written = file.write_all(bytes).and_then(|()| file.sync_data());
        drop(file);
        match written {
            Ok(()) => Ok(temp),
            Err(error) => {
                let _ = std::fs::remove_file(&temp);
                Err(error)
            }
        }
    }

    /// Reserve the first free name, then fill it through `seams`.
    fn record_with(
        dir: &Path,
        record: &ApplyRecord,
        bytes: &[u8],
        seams: &Seams,
    ) -> Result<ApplyRecordId, ApplyRecordError> {
        let (id, reserved) = Self::reserve(dir, record)?;
        let filled = (seams.write_temp)(dir, bytes).and_then(|temp| {
            let placed = (seams.rename)(&temp, &reserved);
            if placed.is_err() {
                let _ = std::fs::remove_file(&temp);
            }
            placed
        });
        if filled.is_err() {
            let _ = std::fs::remove_file(&reserved);
            return Err(ApplyRecordError::WriteFailed);
        }
        sync_dir(dir);
        Self::reclaim_stale_temporaries(dir);
        Ok(id)
    }

    /// Remove temporary files in `dir` last modified over
    /// [`STALE_TEMPORARY_AGE`] ago: a crash between the temporary write
    /// and the rename leaves one that nothing else reclaims. A fresh one
    /// may belong to a record being written, so it stays. Best effort: a
    /// file that cannot be removed is left for the next record.
    fn reclaim_stale_temporaries(dir: &Path) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let now = std::time::SystemTime::now();
        for entry in entries.flatten() {
            let path = entry.path();
            let is_temporary = path.extension().and_then(|extension| extension.to_str())
                == Some(TEMPORARY_EXTENSION)
                && entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(TEMPORARY_PREFIX);
            let stale = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|age| age > STALE_TEMPORARY_AGE);
            if is_temporary && stale {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    /// Create the first free record name exclusively, owner-only.
    fn reserve(
        dir: &Path,
        record: &ApplyRecord,
    ) -> Result<(ApplyRecordId, PathBuf), ApplyRecordError> {
        for sequence in 0..MAX_SEQUENCE {
            let id = apply_record_name(&record.plan_id, record.taken_at, sequence);
            let path = Self::record_path(dir, &id);
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(_) => return Ok((id, path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err(ApplyRecordError::WriteFailed),
            }
        }
        Err(ApplyRecordError::WriteFailed)
    }
}

/// The filesystem steps a record takes after its reservation, replaceable
/// in tests to fail them.
struct Seams {
    write_temp: fn(&Path, &[u8]) -> std::io::Result<PathBuf>,
    rename: fn(&Path, &Path) -> std::io::Result<()>,
}

impl Seams {
    const REAL: Self = Self {
        write_temp: FsApplyRecordStore::write_temp,
        rename: |from, to| std::fs::rename(from, to),
    };
}

/// Sync `dir` so the rename survives a crash. Best effort: the record is
/// already in place, and not every platform can open a directory.
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(handle) = std::fs::File::open(dir) {
        let _ = handle.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

impl ApplyRecordStore for FsApplyRecordStore {
    fn record(
        &self,
        project: &ProjectIdentity,
        record: &ApplyRecord,
    ) -> Result<ApplyRecordId, ApplyRecordError> {
        let text = serde_json::to_string(&ApplyRecordDto::from(record))
            .map_err(|_| ApplyRecordError::WriteFailed)?;
        let dir = self.project_dir(project);
        create_owner_only_dir(&dir).map_err(|_| ApplyRecordError::WriteFailed)?;
        Self::record_with(&dir, record, text.as_bytes(), &Seams::REAL)
    }

    fn list(&self, project: &ProjectIdentity) -> Result<Vec<StoredApplyRecord>, ApplyRecordError> {
        // Probe the applies path first: absent is an empty listing, but a
        // path that is there and is not a directory is unreadable, which
        // no platform may report as "not found" instead.
        match std::fs::symlink_metadata(&self.root) {
            Ok(metadata) if !metadata.is_dir() => return Err(ApplyRecordError::Unreadable),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err(ApplyRecordError::Unreadable),
        }
        let dir = self.project_dir(project);
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err(ApplyRecordError::Unreadable),
        };
        let mut records = Vec::new();
        for entry in entries {
            let path = entry.map_err(|_| ApplyRecordError::Unreadable)?.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some(Self::EXTENSION) {
                continue;
            }
            let name = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or(ApplyRecordError::Corrupt)?
                .to_string();
            // An empty file is a reservation a crash left unfinished.
            if std::fs::metadata(&path)
                .map_err(|_| ApplyRecordError::Unreadable)?
                .len()
                == 0
            {
                continue;
            }
            let text = std::fs::read_to_string(&path).map_err(|_| ApplyRecordError::Unreadable)?;
            let dto: ApplyRecordDto =
                serde_json::from_str(&text).map_err(|_| ApplyRecordError::Corrupt)?;
            let record = ApplyRecord::try_from(dto).map_err(|_| ApplyRecordError::Corrupt)?;
            // The name is bound to the content: a copy under another stem
            // would otherwise be listed as a record it is not.
            let parsed = parse_apply_record_name(&name).ok_or(ApplyRecordError::Corrupt)?;
            if parsed.plan_id != record.plan_id || parsed.taken_at != record.taken_at {
                return Err(ApplyRecordError::Corrupt);
            }
            records.push(StoredApplyRecord {
                id: ApplyRecordId(name),
                record,
            });
        }
        sort_records_newest_first(&mut records);
        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use super::{FsApplyRecordStore, Seams};
    use omnifrons_app::apply_record::{ApplyRecord, ApplyRecordError};
    use omnifrons_app::contract::apply_record::sample_apply_record;

    fn dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-apply-record-seam-{}-{label}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("fixture dir");
        dir
    }

    fn names(dir: &std::path::Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .expect("read dir")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }

    fn attempt(dir: &std::path::Path, seams: &Seams) -> Result<(), ApplyRecordError> {
        let record: ApplyRecord = sample_apply_record(1, 100);
        FsApplyRecordStore::record_with(dir, &record, b"{}", seams).map(|_| ())
    }

    #[test]
    fn a_failed_rename_removes_the_reservation_and_the_temporary_file() {
        let dir = dir("rename");
        let seams = Seams {
            rename: |_, _| Err(std::io::Error::other("injected")),
            ..Seams::REAL
        };
        assert_eq!(attempt(&dir, &seams), Err(ApplyRecordError::WriteFailed));
        assert!(names(&dir).is_empty(), "left behind: {:?}", names(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_temporary_write_removes_the_reservation() {
        let dir = dir("temp");
        let seams = Seams {
            write_temp: |_, _| Err(std::io::Error::other("injected")),
            ..Seams::REAL
        };
        assert_eq!(attempt(&dir, &seams), Err(ApplyRecordError::WriteFailed));
        assert!(names(&dir).is_empty(), "left behind: {:?}", names(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
