//! Local history: SQLite index plus content-addressed PNG blobs.
//!
//! The schema is the spec file compiled in with `include_str!`, so
//! `spec/schema/001_init.sql` and the database can never drift.
//!
//! Write order is fixed: blob first (temp file → rename), index second. A crash can
//! therefore leave an orphan blob (collected later) but never a row pointing at a
//! missing file (`spec/07-history-storage.md` §4).

use pinshot_core::{Error, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SCHEMA: &str = include_str!("../../../spec/schema/001_init.sql");

pub struct Store {
    connection: Connection,
    root: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct NewCapture {
    /// `snip` | `scroll` | `pin` | `import`
    pub kind: String,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    /// Shared so recording a history row never copies the (potentially 5 MP) PNG.
    pub png: Arc<[u8]>,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    pub display_id: Option<String>,
    pub frame_count: u32,
}

impl NewCapture {
    pub fn snip(width: u32, height: u32, scale: f32, png: Arc<[u8]>) -> Self {
        Self {
            kind: "snip".into(),
            width,
            height,
            scale,
            png,
            app_name: None,
            window_title: None,
            display_id: None,
            frame_count: 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CaptureRow {
    pub id: String,
    pub created_at: i64,
    pub kind: String,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub blob_hash: Option<String>,
    pub thumb_hash: Option<String>,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    pub favorite: bool,
    pub frame_count: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    pub captures: u32,
    pub favorites: u32,
    pub blobs: u32,
    pub bytes: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct RetainPolicy {
    pub keep_items: u32,
    pub keep_days: u32,
}

impl Default for RetainPolicy {
    fn default() -> Self {
        Self {
            keep_items: 1000,
            keep_days: 30,
        }
    }
}

impl Store {
    /// Opens (creating if needed) `<root>/pinshot.db` and `<root>/blobs/`.
    pub fn open(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root)
            .map_err(|error| Error::io(format!("cannot create {}: {error}", root.display())))?;
        let connection = Connection::open(root.join("pinshot.db"))
            .map_err(|error| Error::io(format!("cannot open the database: {error}")))?;
        let store = Self::prepare(connection, Some(root.to_owned()))?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self> {
        let connection = Connection::open_in_memory()
            .map_err(|error| Error::io(format!("cannot open the database: {error}")))?;
        Self::prepare(connection, None)
    }

    fn prepare(connection: Connection, root: Option<PathBuf>) -> Result<Self> {
        connection
            .execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
            .map_err(db)?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(db)?;
        if version == 0 {
            connection.execute_batch(SCHEMA).map_err(db)?;
        } else if version != 1 {
            return Err(Error::internal(format!(
                "database schema version {version} is newer than this build"
            )));
        }
        Ok(Self { connection, root })
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    // ── blobs ───────────────────────────────────────────────────────────────

    /// Stores the PNG under its content hash. Identical images share one file.
    pub fn save_blob(&self, png: &[u8]) -> Result<String> {
        let hash = blake3::hash(png).to_hex().to_string();
        let Some(root) = &self.root else {
            // In-memory stores keep the hash but no file; tests that need bytes use
            // a temporary directory.
            return Ok(hash);
        };
        let bucket = root.join("blobs").join(&hash[..2]);
        std::fs::create_dir_all(&bucket)
            .map_err(|error| Error::io(format!("cannot create {}: {error}", bucket.display())))?;
        let path = bucket.join(format!("{hash}.png"));
        if path.exists() {
            return Ok(hash);
        }
        // The hash alone is not enough: two concurrent writes of the same image in one
        // process would share this .part path and truncate each other.
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let temporary = bucket.join(format!(
            "{hash}.{}.{}.part",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::write(&temporary, png)
            .map_err(|error| Error::io(format!("cannot write {}: {error}", temporary.display())))?;
        std::fs::rename(&temporary, &path)
            .map_err(|error| Error::io(format!("cannot finalise {}: {error}", path.display())))?;
        Ok(hash)
    }

    pub fn blob_path(&self, hash: &str) -> Option<PathBuf> {
        self.root.as_ref().map(|root| {
            root.join("blobs")
                .join(&hash[..2])
                .join(format!("{hash}.png"))
        })
    }

    pub fn read_blob(&self, hash: &str) -> Result<Vec<u8>> {
        let path = self
            .blob_path(hash)
            .ok_or_else(|| Error::internal("this store has no blob directory"))?;
        std::fs::read(&path)
            .map_err(|error| Error::io(format!("cannot read {}: {error}", path.display())))
    }

    // ── captures ────────────────────────────────────────────────────────────

    pub fn insert_capture(&mut self, capture: NewCapture) -> Result<String> {
        let hash = self.save_blob(&capture.png)?;
        // Same image twice in the same second must still produce two rows, so the
        // seed carries a nanosecond stamp and a process-local counter.
        let id = stable_id(&format!(
            "{}|{}|{}|{}|{}",
            now_nanos(),
            next_sequence(),
            capture.kind,
            capture.width,
            hash
        ));
        self.connection
            .execute(
                "INSERT INTO captures
                    (id, created_at, kind, width, height, scale, blob_hash,
                     app_name, window_title, display_id, frame_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    id,
                    now(),
                    capture.kind,
                    capture.width,
                    capture.height,
                    capture.scale,
                    hash,
                    capture.app_name,
                    capture.window_title,
                    capture.display_id,
                    capture.frame_count
                ],
            )
            .map_err(db)?;
        Ok(id)
    }

    pub fn recent(&self, limit: u32) -> Result<Vec<CaptureRow>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, created_at, kind, width, height, scale, blob_hash, thumb_hash,
                        app_name, window_title, favorite, frame_count
                 FROM captures
                 WHERE deleted_at IS NULL
                 ORDER BY created_at DESC, rowid DESC
                 LIMIT ?1",
            )
            .map_err(db)?;
        let rows = statement
            .query_map(params![limit as i64], capture_row)
            .map_err(db)?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(db)
    }

    pub fn capture(&self, id: &str) -> Result<Option<CaptureRow>> {
        self.connection
            .query_row(
                "SELECT id, created_at, kind, width, height, scale, blob_hash, thumb_hash,
                        app_name, window_title, favorite, frame_count
                 FROM captures WHERE id = ?1 AND deleted_at IS NULL",
                params![id],
                capture_row,
            )
            .optional()
            .map_err(db)
    }

    pub fn set_favorite(&mut self, id: &str, favorite: bool) -> Result<bool> {
        let changed = self
            .connection
            .execute(
                "UPDATE captures SET favorite = ?2 WHERE id = ?1",
                params![id, favorite as i64],
            )
            .map_err(db)?;
        Ok(changed > 0)
    }

    /// Deletes a capture and, when nothing else references it, its blob file.
    pub fn delete_capture(&mut self, id: &str) -> Result<bool> {
        let hash: Option<Option<String>> = self
            .connection
            .query_row(
                "SELECT blob_hash FROM captures WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()
            .map_err(db)?;
        let Some(hash) = hash else { return Ok(false) };
        self.connection
            .execute("DELETE FROM captures WHERE id = ?1", params![id])
            .map_err(db)?;
        if let Some(hash) = hash {
            self.collect_blob(&hash)?;
        }
        Ok(true)
    }

    /// Empties the history, favourites included. Returns how many captures went away.
    pub fn clear_all(&mut self) -> Result<u32> {
        let removed = self
            .connection
            .execute("DELETE FROM captures", [])
            .map_err(db)? as u32;
        if removed > 0 {
            self.collect_orphan_blobs()?;
        }
        Ok(removed)
    }

    /// Applies the retention policy. Favourites are never removed.
    pub fn prune(&mut self, policy: RetainPolicy, now: i64) -> Result<u32> {
        let mut removed = 0;
        // 1) too old
        let cutoff = now - policy.keep_days as i64 * 86_400;
        removed += self
            .connection
            .execute(
                "DELETE FROM captures WHERE favorite = 0 AND created_at < ?1",
                params![cutoff],
            )
            .map_err(db)? as u32;

        // 2) too many: drop the oldest until the cap is met
        let total: u32 = self
            .connection
            .query_row("SELECT COUNT(*) FROM captures", [], |row| row.get(0))
            .map_err(db)?;
        let excess = total.saturating_sub(policy.keep_items);
        if excess > 0 {
            removed += self
                .connection
                .execute(
                    "DELETE FROM captures WHERE id IN (
                         SELECT id FROM captures WHERE favorite = 0
                         ORDER BY created_at ASC, rowid ASC LIMIT ?1
                     )",
                    params![excess as i64],
                )
                .map_err(db)? as u32;
        }

        if removed > 0 {
            self.collect_orphan_blobs()?;
        }
        Ok(removed)
    }

    /// Removes blob files no capture references (crash leftovers, pruned rows).
    pub fn collect_orphan_blobs(&self) -> Result<u32> {
        let Some(root) = &self.root else { return Ok(0) };
        let mut referenced = std::collections::HashSet::new();
        let mut statement = self
            .connection
            .prepare("SELECT blob_hash FROM captures WHERE blob_hash IS NOT NULL")
            .map_err(db)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(db)?;
        for hash in rows {
            referenced.insert(hash.map_err(db)?);
        }

        let mut removed = 0;
        let buckets = root.join("blobs");
        let Ok(entries) = std::fs::read_dir(&buckets) else {
            return Ok(0);
        };
        for bucket in entries.flatten() {
            if !bucket.path().is_dir() {
                continue;
            }
            let Ok(files) = std::fs::read_dir(bucket.path()) else {
                continue;
            };
            for file in files.flatten() {
                let name = file.file_name().to_string_lossy().into_owned();
                let hash = name.trim_end_matches(".png");
                // Keep files whose hash is still referenced; also drop stale `.part`.
                if !referenced.contains(hash) && std::fs::remove_file(file.path()).is_ok() {
                    removed += 1;
                }
            }
        }
        Ok(removed)
    }

    fn collect_blob(&self, hash: &str) -> Result<()> {
        let referenced: u32 = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM captures WHERE blob_hash = ?1",
                params![hash],
                |row| row.get(0),
            )
            .map_err(db)?;
        if referenced == 0 {
            if let Some(path) = self.blob_path(hash) {
                let _ = std::fs::remove_file(path);
            }
        }
        Ok(())
    }

    pub fn stats(&self) -> Result<Stats> {
        let captures: u32 = self
            .connection
            .query_row("SELECT COUNT(*) FROM captures", [], |row| row.get(0))
            .map_err(db)?;
        let favorites: u32 = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM captures WHERE favorite = 1",
                [],
                |row| row.get(0),
            )
            .map_err(db)?;
        let blobs = match &self.root {
            Some(root) => count_files(&root.join("blobs")).unwrap_or(0),
            None => 0,
        };
        Ok(Stats {
            captures,
            favorites,
            blobs,
            bytes: 0,
        })
    }
}

fn count_files(directory: &Path) -> std::io::Result<u32> {
    let mut total = 0;
    for entry in std::fs::read_dir(directory)?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            total += count_files(&path)?;
        } else {
            total += 1;
        }
    }
    Ok(total)
}

fn capture_row(row: &Row<'_>) -> rusqlite::Result<CaptureRow> {
    Ok(CaptureRow {
        id: row.get(0)?,
        created_at: row.get(1)?,
        kind: row.get(2)?,
        width: row.get(3)?,
        height: row.get(4)?,
        scale: row.get(5)?,
        blob_hash: row.get(6)?,
        thumb_hash: row.get(7)?,
        app_name: row.get(8)?,
        window_title: row.get(9)?,
        favorite: row.get::<_, i64>(10)? != 0,
        frame_count: row.get(11)?,
    })
}

fn stable_id(seed: &str) -> String {
    blake3::hash(seed.as_bytes()).to_hex().to_string()[..32].to_owned()
}

fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

fn next_sequence() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(1);
    SEQUENCE.fetch_add(1, Ordering::Relaxed)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn db(error: rusqlite::Error) -> Error {
    Error::io(format!("database error: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(tag: u8) -> Arc<[u8]> {
        Arc::from(vec![0x89, 0x50, 0x4e, 0x47, tag, tag, tag, tag])
    }

    #[test]
    fn schema_applies_and_starts_empty() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.recent(10).unwrap().is_empty());
        assert_eq!(store.stats().unwrap().captures, 0);
    }

    #[test]
    fn captures_come_back_newest_first() {
        let mut store = Store::open_in_memory().unwrap();
        let first = store
            .insert_capture(NewCapture::snip(100, 50, 2.0, png(1)))
            .unwrap();
        let second = store
            .insert_capture(NewCapture::snip(200, 60, 1.0, png(2)))
            .unwrap();

        let rows = store.recent(10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, second);
        assert_eq!(rows[1].id, first);
        assert_eq!(rows[0].width, 200);
        assert!((rows[1].scale - 2.0).abs() < f32::EPSILON);
    }

    #[test]
    fn identical_images_share_one_blob() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        let a = store
            .insert_capture(NewCapture::snip(10, 10, 1.0, png(7)))
            .unwrap();
        let b = store
            .insert_capture(NewCapture::snip(10, 10, 1.0, png(7)))
            .unwrap();
        let row_a = store.capture(&a).unwrap().unwrap();
        let row_b = store.capture(&b).unwrap().unwrap();
        assert_eq!(row_a.blob_hash, row_b.blob_hash);
        assert_eq!(store.stats().unwrap().blobs, 1);
        let bytes = store
            .read_blob(row_a.blob_hash.as_deref().unwrap())
            .unwrap();
        assert_eq!(bytes.as_slice(), &*png(7));
    }

    #[test]
    fn deleting_the_last_reference_removes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        let id = store
            .insert_capture(NewCapture::snip(10, 10, 1.0, png(3)))
            .unwrap();
        let hash = store.capture(&id).unwrap().unwrap().blob_hash.unwrap();
        let path = store.blob_path(&hash).unwrap();
        assert!(path.exists());

        assert!(store.delete_capture(&id).unwrap());
        assert!(!path.exists());
        assert!(store.recent(10).unwrap().is_empty());
        assert!(!store.delete_capture(&id).unwrap());
    }

    #[test]
    fn clearing_removes_rows_and_blob_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        let kept = store
            .insert_capture(NewCapture::snip(10, 10, 1.0, png(1)))
            .unwrap();
        store
            .insert_capture(NewCapture::snip(10, 10, 1.0, png(2)))
            .unwrap();
        store.set_favorite(&kept, true).unwrap();

        assert_eq!(store.clear_all().unwrap(), 2);
        assert!(store.recent(10).unwrap().is_empty());
        assert_eq!(store.stats().unwrap().blobs, 0);
        // Nothing left to clear, so it reports zero instead of lying.
        assert_eq!(store.clear_all().unwrap(), 0);
    }

    #[test]
    fn pruning_drops_the_oldest_but_keeps_favourites() {
        let mut store = Store::open_in_memory().unwrap();
        let mut ids = Vec::new();
        for tag in 0..5u8 {
            ids.push(
                store
                    .insert_capture(NewCapture::snip(10, 10, 1.0, png(tag)))
                    .unwrap(),
            );
        }
        // The middle one is a favourite and must survive.
        store.set_favorite(&ids[2], true).unwrap();

        let removed = store
            .prune(
                RetainPolicy {
                    keep_items: 2,
                    keep_days: 30,
                },
                now(),
            )
            .unwrap();
        assert_eq!(removed, 3);

        let remaining: Vec<String> = store
            .recent(10)
            .unwrap()
            .into_iter()
            .map(|row| row.id)
            .collect();
        assert_eq!(remaining.len(), 2);
        assert!(remaining.contains(&ids[2]), "favourite survived");
        assert!(remaining.contains(&ids[4]), "newest survived");
    }

    #[test]
    fn pruning_by_age_uses_the_supplied_clock() {
        let mut store = Store::open_in_memory().unwrap();
        store
            .insert_capture(NewCapture::snip(10, 10, 1.0, png(1)))
            .unwrap();
        // Pretend 60 days have passed.
        let removed = store
            .prune(
                RetainPolicy {
                    keep_items: 100,
                    keep_days: 30,
                },
                now() + 60 * 86_400,
            )
            .unwrap();
        assert_eq!(removed, 1);
        assert!(store.recent(10).unwrap().is_empty());
    }

    #[test]
    fn orphans_are_collected() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        let id = store
            .insert_capture(NewCapture::snip(10, 10, 1.0, png(9)))
            .unwrap();
        let hash = store.capture(&id).unwrap().unwrap().blob_hash.unwrap();
        // Simulate a crash between writing the blob and committing the row.
        let orphan = dir.path().join("blobs").join("zz").join("deadbeef.png");
        std::fs::create_dir_all(orphan.parent().unwrap()).unwrap();
        std::fs::write(&orphan, b"junk").unwrap();

        let removed = store.collect_orphan_blobs().unwrap();
        assert_eq!(removed, 1);
        assert!(!orphan.exists());
        assert!(store.blob_path(&hash).unwrap().exists());
    }
}
