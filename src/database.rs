use crate::plates::{Error, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{Arc, Mutex, atomic::AtomicBool},
};

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    pub name: String,
    pub host: String,
    pub serial: String,
    #[serde(default, skip_serializing)]
    pub access_code: String,
    #[serde(default, skip_serializing)]
    pub tls_certificate: String,
    pub machine_profile_key: String,
    pub default_process_profile_key: String,
    pub bed_type: String,
    pub nozzle_material: String,
    #[serde(default = "mqtt_port")]
    pub mqtt_port: u16,
    #[serde(default = "ftps_port")]
    pub ftps_port: u16,
    #[serde(default = "start_timeout")]
    pub start_timeout_secs: u16,
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty()
            || self.name.len() > 120
            || self.name.chars().any(char::is_control)
        {
            return Err(Error::Invalid(
                "Printer name must contain 1..120 bytes without control characters",
            ));
        }
        if !["stainless_steel", "hardened_steel", "unknown"]
            .contains(&self.nozzle_material.as_str())
        {
            return Err(Error::Invalid("Unknown nozzle material"));
        }
        if !crate::profiles::BEDS.contains(&self.bed_type.as_str()) {
            return Err(Error::Invalid("Unknown bed type"));
        }
        Ok(())
    }
}
fn mqtt_port() -> u16 {
    8883
}
fn ftps_port() -> u16 {
    990
}
fn start_timeout() -> u16 {
    600
}

#[derive(Clone, Serialize)]
pub(crate) struct Device {
    pub id: String,
    #[serde(flatten)]
    pub settings: Settings,
}

#[derive(Clone)]
pub(crate) struct Database {
    // ponytail: serialize the small registry; use a pool only if measured contention warrants it.
    connection: Arc<Mutex<Connection>>,
    pub(crate) notifications_enabled: Arc<AtomicBool>,
}

impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        if error.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation) {
            Error::Conflict("The record is referenced, duplicated or violates the schema")
        } else {
            tracing::error!(code=?error.sqlite_error_code(), "SQLite operation failed");
            Error::Unavailable("Database operation failed; check storage and server logs")
        }
    }
}
fn private_connection(root: &Path) -> Result<Connection> {
    std::fs::create_dir_all(root)?;
    let path = root.join("orca.sqlite3");
    // The database contains access codes. Create with private permissions before SQLite opens it.
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(&path)?;
    let connection = Connection::open(&path)?;
    connection
        .busy_timeout(std::time::Duration::from_secs(5))
        .map_err(Error::from)?;
    connection
        .pragma_update(None, "foreign_keys", false)
        .map_err(Error::from)?;
    Ok(connection)
}
fn check_references(c: &Connection) -> Result<()> {
    if c.prepare("PRAGMA foreign_key_check")?
        .query([])?
        .next()?
        .is_some()
    {
        return Err(Error::Unavailable(
            "Database migration found broken references; restore or repair the saved database",
        ));
    }
    Ok(())
}
impl Database {
    #[allow(clippy::too_many_lines)] // Schema migrations share one rollback boundary.
    pub fn open(root: &Path, initial: impl FnOnce() -> Result<Option<Device>>) -> Result<Self> {
        let mut connection = private_connection(root)?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let version: i64 = tx
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(Error::from)?;
        match version {
            0 => {
                tx.execute_batch("CREATE TABLE printers (
                    id TEXT PRIMARY KEY, name TEXT NOT NULL, host TEXT NOT NULL,
                    serial TEXT NOT NULL UNIQUE, access_code TEXT NOT NULL, tls_certificate TEXT NOT NULL,
                    machine_profile_key TEXT NOT NULL, default_process_profile_key TEXT NOT NULL,
                    bed_type TEXT NOT NULL, nozzle_material TEXT NOT NULL CHECK(nozzle_material IN ('stainless_steel','hardened_steel','unknown')),
                    mqtt_port INTEGER NOT NULL CHECK(mqtt_port BETWEEN 1 AND 65535),
                    ftps_port INTEGER NOT NULL CHECK(ftps_port BETWEEN 1 AND 65535),
                    start_timeout_secs INTEGER NOT NULL CHECK(start_timeout_secs BETWEEN 1 AND 3600)
                );").map_err(Error::from)?;
                if let Some(device) = initial()? {
                    save(&tx, &device)?;
                }
                tx.pragma_update(None, "user_version", 1)
                    .map_err(Error::from)?;
            }
            1..=17 => {}
            _ => {
                return Err(Error::Unavailable(
                    "Database schema is newer than this server; use a compatible version",
                ));
            }
        }
        if version < 2 {
            tx.execute_batch("CREATE TABLE filaments (
                id TEXT PRIMARY KEY, name TEXT NOT NULL, vendor TEXT NOT NULL,
                material TEXT NOT NULL, color TEXT NOT NULL, bambu_filament_id TEXT
            );
            CREATE TABLE filament_settings (
                id TEXT PRIMARY KEY, filament_id TEXT NOT NULL REFERENCES filaments(id) ON DELETE CASCADE,
                machine_profile_key TEXT NOT NULL, base_profile_key TEXT NOT NULL,
                overrides_json TEXT NOT NULL, UNIQUE(filament_id,machine_profile_key)
            );
            CREATE TABLE ams_slots (
                id TEXT PRIMARY KEY, printer_id TEXT NOT NULL REFERENCES printers(id) ON DELETE CASCADE,
                ams_id INTEGER NOT NULL, slot_index INTEGER NOT NULL,
                filament_id TEXT REFERENCES filaments(id), mapping_source TEXT NOT NULL DEFAULT 'unassigned',
                reported_tag_uid TEXT, reported_profile_id TEXT, reported_type TEXT, reported_color TEXT,
                reported_brand TEXT, reported_temp_min INTEGER, reported_temp_max INTEGER,
                present INTEGER, remaining_percent INTEGER, detect_on_insert INTEGER, detect_on_power_up INTEGER,
                last_seen_at INTEGER, revision INTEGER NOT NULL DEFAULT 1,
                UNIQUE(printer_id,ams_id,slot_index),
                CHECK(ams_id BETWEEN 0 AND 255), CHECK(slot_index BETWEEN 0 AND 3),
                CHECK(mapping_source IN ('unassigned','manual','automatic'))
            );
            PRAGMA user_version=2;").map_err(Error::from)?;
        }
        if version < 3 {
            tx.execute_batch("ALTER TABLE printers ADD COLUMN queue_generation INTEGER NOT NULL DEFAULT 0;
                ALTER TABLE printers ADD COLUMN queue_request TEXT;
                CREATE TABLE plates (id TEXT PRIMARY KEY, name TEXT NOT NULL, version INTEGER NOT NULL DEFAULT 1 CHECK(version>0));
                CREATE TABLE plate_items (
                    id TEXT PRIMARY KEY, plate_id TEXT NOT NULL REFERENCES plates(id) ON DELETE CASCADE,
                    position INTEGER NOT NULL, name TEXT NOT NULL, source_kind TEXT NOT NULL CHECK(source_kind IN ('scad','upload')),
                    model_key TEXT, original BLOB, quantity INTEGER NOT NULL CHECK(quantity BETWEEN 1 AND 64),
                    UNIQUE(plate_id,position),
                    CHECK((source_kind='scad' AND model_key IS NOT NULL AND original IS NULL) OR
                          (source_kind='upload' AND model_key IS NULL AND original IS NOT NULL))
                );
                CREATE UNIQUE INDEX ams_printer_slot ON ams_slots(printer_id,id);
                CREATE TABLE print_jobs (
                    id TEXT PRIMARY KEY, printer_id TEXT NOT NULL REFERENCES printers(id),
                    plate_id TEXT NOT NULL REFERENCES plates(id), name TEXT NOT NULL,
                    ams_slot_id TEXT NOT NULL, filament_id TEXT NOT NULL REFERENCES filaments(id),
                    required_machine_profile_key TEXT NOT NULL, process_profile_key TEXT NOT NULL, bed_type TEXT NOT NULL,
                    state TEXT NOT NULL CHECK(state IN ('queued','preparing','printing','awaiting_removal','completed','needs_attention','cancelled')),
                    position INTEGER NOT NULL, attempt_id TEXT, artifact_path TEXT, execution_json TEXT, attempt_json TEXT, last_error TEXT,
                    FOREIGN KEY(printer_id,ams_slot_id) REFERENCES ams_slots(printer_id,id),
                    FOREIGN KEY(filament_id,required_machine_profile_key) REFERENCES filament_settings(filament_id,machine_profile_key)
                );
                CREATE UNIQUE INDEX one_active_job ON print_jobs(printer_id)
                    WHERE state IN ('preparing','printing','awaiting_removal','needs_attention');")?;
            crate::plates::migrate(&tx, root)?;
            tx.pragma_update(None, "user_version", 3)?;
        }
        if version < 4 {
            crate::products::migrate(&tx)?;
        }
        if version < 5 {
            crate::ams::migrate(&tx)?;
        }
        if version < 6 {
            crate::plates::migrate_conditions(&tx)?;
        }
        if version < 7 {
            migrate_defaults(&tx)?;
        }
        if version < 8 {
            crate::notifications::migrate(&tx)?;
        }
        if version < 9 {
            tx.execute_batch("ALTER TABLE print_jobs ADD COLUMN estimate_json TEXT CHECK(estimate_json IS NULL OR json_valid(estimate_json)); PRAGMA user_version=9;")?;
        }
        if version < 10 {
            tx.execute_batch("ALTER TABLE plates ADD COLUMN sparse_infill_pattern TEXT;
                ALTER TABLE plates ADD COLUMN sparse_infill_density REAL CHECK(sparse_infill_density BETWEEN 0 AND 100);
                ALTER TABLE plates ADD COLUMN wall_loops INTEGER CHECK(wall_loops BETWEEN 0 AND 1000 AND wall_loops=CAST(wall_loops AS INTEGER));
                ALTER TABLE default_settings ADD COLUMN sparse_infill_pattern TEXT NOT NULL DEFAULT 'adaptivecubic';
                ALTER TABLE default_settings ADD COLUMN sparse_infill_density REAL NOT NULL DEFAULT 15 CHECK(sparse_infill_density BETWEEN 0 AND 100);
                ALTER TABLE default_settings ADD COLUMN wall_loops INTEGER NOT NULL DEFAULT 2 CHECK(wall_loops BETWEEN 0 AND 1000 AND wall_loops=CAST(wall_loops AS INTEGER));
                PRAGMA user_version=10;")?;
        }
        if version < 11 {
            tx.execute_batch("ALTER TABLE plates ADD COLUMN deleted INTEGER NOT NULL DEFAULT 0 CHECK(deleted IN (0,1)); PRAGMA user_version=11;")?;
        }
        if version < 12 {
            tx.execute_batch("ALTER TABLE plates ADD COLUMN brim_enabled INTEGER NOT NULL DEFAULT 0 CHECK(brim_enabled IN (0,1)); PRAGMA user_version=12;")?;
        }
        reconcile_defaults(&tx)?;
        if version < 13 {
            tx.execute_batch("ALTER TABLE plates ADD COLUMN support_enabled INTEGER NOT NULL DEFAULT 0 CHECK(support_enabled IN (0,1));
                ALTER TABLE plates ADD COLUMN support_interface_filament_id TEXT REFERENCES filaments(id);
                PRAGMA user_version=13;")?;
        }
        if version < 14 {
            tx.execute_batch(
                "CREATE TABLE plate_imports (
                plate_id TEXT PRIMARY KEY REFERENCES plates(id) ON DELETE CASCADE,
                metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
                original BLOB NOT NULL CHECK(length(original) BETWEEN 1 AND 67108864)
            ); PRAGMA user_version=14;",
            )?;
        }
        if version < 15 {
            tx.execute_batch("ALTER TABLE plates ADD COLUMN secondary_filament_id TEXT REFERENCES filaments(id);
                ALTER TABLE plate_items ADD COLUMN roles_json TEXT NOT NULL DEFAULT '[\"primary\"]' CHECK(json_valid(roles_json));
                PRAGMA user_version=15;")?;
        }
        if version < 16 {
            // Older schemas have no completion timestamp. Notification retry times
            // and queue registration times cannot reconstruct past print history.
            tx.execute_batch(
                "CREATE TABLE print_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                attempt_id TEXT NOT NULL UNIQUE, job_id TEXT NOT NULL,
                plate_id TEXT NOT NULL, printer_id TEXT NOT NULL, name TEXT NOT NULL,
                completed_at INTEGER NOT NULL CHECK(completed_at >= 0)
            );
            CREATE INDEX print_history_completion ON print_history(completed_at DESC,id DESC);
            PRAGMA user_version=16;",
            )?;
        }
        if version < 17 {
            tx.execute_batch(include_str!("../migrations/017-plate-slices.sql"))?;
        }
        check_references(&tx)?;
        tx.commit().map_err(Error::from)?;
        connection.pragma_update(None, "foreign_keys", true)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            notifications_enabled: Arc::new(AtomicBool::new(false)),
        })
    }
    pub(crate) fn connection(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| Error::Unavailable("Database lock failed"))
    }
    pub(crate) fn filaments(&self) -> Result<Vec<crate::filament::Filament>> {
        load_filaments(&*self.connection()?)
    }

    pub(crate) fn save_filament(&self, f: &crate::filament::Filament) -> Result<()> {
        f.data.validate()?;
        let mut c = self.connection()?;
        let tx = c.transaction()?;
        let old: Option<(String,String,String,i64)> = tx.query_row("SELECT f.product_id,f.name,p.name,(SELECT count(*) FROM filaments WHERE product_id=p.id) FROM filaments f JOIN filament_products p ON p.id=f.product_id WHERE f.id=?1",[&f.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        if let Some((pid, old_name, product_name, count)) = old {
            let display = if old_name == product_name {
                old_name.clone()
            } else {
                format!("{product_name} · {old_name}")
            };
            let name = if f.data.name == display {
                &old_name
            } else {
                &f.data.name
            };
            let product_name = if count == 1 && old_name == product_name {
                &f.data.name
            } else {
                &product_name
            };
            tx.execute("UPDATE filament_products SET vendor=?1,material=?2,bambu_filament_id=?3,name=?4 WHERE id=?5",params![f.data.vendor,f.data.material,f.data.bambu_filament_id,product_name,pid])?;
            tx.execute(
                "UPDATE filaments SET name=?1,color=?2 WHERE id=?3",
                params![name, f.data.color, f.id],
            )?;
        } else {
            tx.execute(
                "INSERT INTO filament_products VALUES (?1,?2,?3,?4,?5)",
                params![
                    f.id,
                    f.data.name,
                    f.data.vendor,
                    f.data.material,
                    f.data.bambu_filament_id
                ],
            )?;
            tx.execute(
                "INSERT INTO filaments VALUES (?1,?1,?2,?3)",
                params![f.id, f.data.name, f.data.color],
            )?;
        }
        crate::ams::invalidate_automatic(&tx)?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn delete_filament(&self, id: &str) -> Result<()> {
        let mut c = self.connection()?;
        let tx = c.transaction()?;
        let pid = crate::products::product_id(&tx, id)?;
        tx.execute("DELETE FROM filaments WHERE id=?1", [id])?;
        tx.execute("DELETE FROM filament_products WHERE id=?1 AND NOT EXISTS(SELECT 1 FROM filaments WHERE product_id=?1)",[pid])?;
        tx.commit()?;
        Ok(())
    }
    pub fn list(&self) -> Result<Vec<Device>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| Error::Unavailable("Database lock failed"))?;
        let mut query = connection.prepare("SELECT id,name,host,serial,access_code,tls_certificate,machine_profile_key,default_process_profile_key,bed_type,nozzle_material,mqtt_port,ftps_port,start_timeout_secs FROM printers ORDER BY name,id").map_err(Error::from)?;
        query
            .query_map([], |r| {
                Ok(Device {
                    id: r.get(0)?,
                    settings: Settings {
                        name: r.get(1)?,
                        host: r.get(2)?,
                        serial: r.get(3)?,
                        access_code: r.get(4)?,
                        tls_certificate: r.get(5)?,
                        machine_profile_key: r.get(6)?,
                        default_process_profile_key: r.get(7)?,
                        bed_type: r.get(8)?,
                        nozzle_material: r.get(9)?,
                        mqtt_port: r.get(10)?,
                        ftps_port: r.get(11)?,
                        start_timeout_secs: r.get(12)?,
                    },
                })
            })
            .map_err(Error::from)?
            .collect::<std::result::Result<_, _>>()
            .map_err(Error::from)
    }
    pub fn save(&self, device: &Device) -> Result<()> {
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        let s = &device.settings;
        tx.execute("DELETE FROM ams_slots WHERE printer_id IN (SELECT id FROM printers WHERE id=?1 AND (host!=?2 OR serial!=?3 OR mqtt_port!=?4 OR access_code!=?5 OR tls_certificate!=?6))",
            params![device.id,s.host,s.serial,s.mqtt_port,s.access_code,s.tls_certificate])?;
        tx.execute("UPDATE printers SET queue_generation=queue_generation+1,queue_request=NULL WHERE id=?1 AND (host!=?2 OR serial!=?3 OR access_code!=?4 OR tls_certificate!=?5 OR machine_profile_key!=?6 OR nozzle_material!=?7 OR mqtt_port!=?8 OR ftps_port!=?9 OR start_timeout_secs!=?10)",params![device.id,s.host,s.serial,s.access_code,s.tls_certificate,s.machine_profile_key,s.nozzle_material,s.mqtt_port,s.ftps_port,s.start_timeout_secs])?;
        save(&tx, device)?;
        reconcile_defaults(&tx)?;
        tx.commit()?;
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let mut c = self.connection()?;
        let tx = c.transaction()?;
        if tx.execute("DELETE FROM printers WHERE id=?1", [id])? == 0 {
            return Err(Error::NotFound);
        }
        reconcile_defaults(&tx)?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn default_printer(&self) -> Result<Option<String>> {
        Ok(self.connection()?.query_row(
            "SELECT default_printer_id FROM default_settings WHERE id=1",
            [],
            |r| r.get(0),
        )?)
    }
    pub(crate) fn default_strength(&self) -> Result<crate::strength::Strength> {
        Ok(self.connection()?.query_row(
            "SELECT sparse_infill_pattern,sparse_infill_density,wall_loops FROM default_settings WHERE id=1",
            [], |r| Ok(crate::strength::Strength { sparse_infill_pattern:r.get(0)?, sparse_infill_density:r.get(1)?, wall_loops:r.get(2)? }),
        )?)
    }
    pub(crate) fn set_defaults(
        &self,
        id: Option<&str>,
        strength: &crate::strength::Strength,
    ) -> Result<()> {
        strength.validate()?;
        let mut c = self.connection()?;
        let tx = c.transaction()?;
        if let Some(id) = id
            && !tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM printers WHERE id=?1)",
                [id],
                |r| r.get::<_, bool>(0),
            )?
        {
            return Err(Error::NotFound);
        }
        tx.execute(
            "UPDATE default_settings SET default_printer_id=?1,sparse_infill_pattern=COALESCE(?2,sparse_infill_pattern),sparse_infill_density=COALESCE(?3,sparse_infill_density),wall_loops=COALESCE(?4,wall_loops) WHERE id=1",
            params![id,strength.sparse_infill_pattern,strength.sparse_infill_density,strength.wall_loops],
        )?;
        reconcile_defaults(&tx)?;
        tx.commit()?;
        Ok(())
    }
}
fn migrate_defaults(c: &Connection) -> Result<()> {
    c.execute_batch(
        "CREATE TABLE default_settings (
                id INTEGER PRIMARY KEY CHECK(id=1),
                default_printer_id TEXT REFERENCES printers(id) ON DELETE SET NULL
            ); INSERT INTO default_settings VALUES (1,NULL); PRAGMA user_version=7;",
    )?;
    Ok(())
}
fn reconcile_defaults(c: &Connection) -> Result<()> {
    c.execute("UPDATE default_settings SET default_printer_id=(SELECT id FROM printers) WHERE id=1 AND default_printer_id IS NULL AND (SELECT count(*) FROM printers)=1", [])?;
    Ok(())
}
fn save(connection: &Connection, device: &Device) -> Result<()> {
    let s = &device.settings;
    connection.execute("INSERT INTO printers(id,name,host,serial,access_code,tls_certificate,machine_profile_key,default_process_profile_key,bed_type,nozzle_material,mqtt_port,ftps_port,start_timeout_secs) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)
        ON CONFLICT(id) DO UPDATE SET name=excluded.name,host=excluded.host,serial=excluded.serial,
        access_code=excluded.access_code,tls_certificate=excluded.tls_certificate,
        machine_profile_key=excluded.machine_profile_key,default_process_profile_key=excluded.default_process_profile_key,
        bed_type=excluded.bed_type,nozzle_material=excluded.nozzle_material,mqtt_port=excluded.mqtt_port,
        ftps_port=excluded.ftps_port,start_timeout_secs=excluded.start_timeout_secs",
        params![device.id,s.name,s.host,s.serial,s.access_code,s.tls_certificate,s.machine_profile_key,s.default_process_profile_key,s.bed_type,s.nozzle_material,s.mqtt_port,s.ftps_port,s.start_timeout_secs]).map_err(Error::from)?;
    Ok(())
}

pub(crate) fn load_filaments(connection: &Connection) -> Result<Vec<crate::filament::Filament>> {
    use crate::filament::{Filament, FilamentData};
    let mut q = connection.prepare(
        "SELECT f.id,CASE WHEN f.name=p.name THEN f.name ELSE p.name || ' · ' || f.name END,p.vendor,p.material,f.color,p.bambu_filament_id FROM filaments f JOIN filament_products p ON p.id=f.product_id ORDER BY p.name,f.name,f.id",
    )?;
    Ok(q.query_map([], |r| {
        Ok(Filament {
            id: r.get(0)?,
            data: FilamentData {
                name: r.get(1)?,
                vendor: r.get(2)?,
                material: r.get(3)?,
                color: r.get(4)?,
                bambu_filament_id: r.get(5)?,
            },
        })
    })?
    .collect::<std::result::Result<_, _>>()?)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn device() -> Device {
        Device {
            id: "stable-id".into(),
            settings: Settings {
                name: "P1S".into(),
                host: "127.0.0.1".into(),
                serial: "TESTSERIAL".into(),
                access_code: "synthetic-secret".into(),
                tls_certificate: "synthetic-certificate".into(),
                machine_profile_key: crate::profiles::PRINTER.into(),
                default_process_profile_key: "0.20mm Standard @BBL X1C".into(),
                bed_type: crate::profiles::BEDS[0].into(),
                nozzle_material: "unknown".into(),
                mqtt_port: 8883,
                ftps_port: 990,
                start_timeout_secs: 600,
            },
        }
    }
    #[test]
    fn strength_defaults_are_persisted_without_backfilling_legacy_plates() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path(), || Ok(None)).unwrap();
        let c = db.connection().unwrap();
        let values: (String, f64, u32) = c.query_row("SELECT sparse_infill_pattern,sparse_infill_density,wall_loops FROM default_settings", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(values, ("adaptivecubic".into(), 15.0, 2));
        c.execute("INSERT INTO plates(id,name) VALUES ('old','Old')", [])
            .unwrap();
        drop(c);
        drop(db);
        let db = Database::open(dir.path(), || panic!("no reimport")).unwrap();
        let plate = crate::plates::load(&db.connection().unwrap(), "old").unwrap();
        assert!(
            serde_json::to_value(plate.conditions)
                .unwrap()
                .as_object()
                .unwrap()
                .iter()
                .all(|(key, value)| {
                    if ["brim_enabled", "support_enabled"].contains(&key.as_str()) {
                        value == false
                    } else {
                        value.is_null()
                    }
                })
        );
    }
    #[test]
    fn defaults_migrate_and_keep_one_reference_without_rewriting_plates() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path(), || Ok(Some(device()))).unwrap();
        crate::legacy_schema::queue_v16(&db.connection().unwrap());
        db.connection().unwrap().execute_batch("DROP TABLE print_history; ALTER TABLE plate_items DROP COLUMN roles_json; ALTER TABLE plates DROP COLUMN secondary_filament_id; DROP TABLE plate_imports; ALTER TABLE plates DROP COLUMN support_interface_filament_id; ALTER TABLE plates DROP COLUMN support_enabled; ALTER TABLE plates DROP COLUMN brim_enabled; ALTER TABLE plates DROP COLUMN deleted; ALTER TABLE plates DROP COLUMN sparse_infill_pattern; ALTER TABLE plates DROP COLUMN sparse_infill_density; ALTER TABLE plates DROP COLUMN wall_loops; DROP TABLE default_settings; DROP TABLE print_notifications; ALTER TABLE print_jobs DROP COLUMN estimate_json; PRAGMA user_version=6; INSERT INTO plates(id,name) VALUES ('legacy','Legacy');").unwrap();
        drop(db);
        let db = Database::open(dir.path(), || panic!("must not reimport")).unwrap();
        assert_eq!(db.default_printer().unwrap().as_deref(), Some("stable-id"));
        let c = db.connection().unwrap();
        assert_eq!(
            c.query_row(
                "SELECT required_machine_profile_key FROM plates WHERE id='legacy'",
                [],
                |r| r.get::<_, Option<String>>(0)
            )
            .unwrap(),
            None
        );
        drop(c);
        let mut second = device();
        second.id = "second".into();
        second.settings.serial = "SECOND".into();
        db.save(&second).unwrap();
        db.set_defaults(Some("second"), &crate::strength::Strength::default())
            .unwrap();
        assert!(
            db.set_defaults(Some("missing"), &crate::strength::Strength::default())
                .is_err()
        );
        drop(db);
        let db = Database::open(dir.path(), || panic!("must not reimport")).unwrap();
        assert_eq!(db.default_printer().unwrap().as_deref(), Some("second"));
        db.delete("second").unwrap();
        assert_eq!(db.default_printer().unwrap().as_deref(), Some("stable-id"));
        db.delete("stable-id").unwrap();
        assert_eq!(db.default_printer().unwrap(), None);
        db.save(&device()).unwrap();
        db.save(&second).unwrap();
        crate::legacy_schema::queue_v16(&db.connection().unwrap());
        db.connection()
            .unwrap()
            .execute_batch("DROP TABLE print_history; ALTER TABLE plate_items DROP COLUMN roles_json; ALTER TABLE plates DROP COLUMN secondary_filament_id; DROP TABLE plate_imports; ALTER TABLE plates DROP COLUMN support_interface_filament_id; ALTER TABLE plates DROP COLUMN support_enabled; ALTER TABLE plates DROP COLUMN brim_enabled; ALTER TABLE plates DROP COLUMN deleted; ALTER TABLE plates DROP COLUMN sparse_infill_pattern; ALTER TABLE plates DROP COLUMN sparse_infill_density; ALTER TABLE plates DROP COLUMN wall_loops; DROP TABLE default_settings; DROP TABLE print_notifications; ALTER TABLE print_jobs DROP COLUMN estimate_json; PRAGMA user_version=6;")
            .unwrap();
        drop(db);
        let db = Database::open(dir.path(), || panic!("must not reimport")).unwrap();
        assert_eq!(db.default_printer().unwrap(), None);
    }
    #[test]
    fn migration_imports_references_and_uploads_once_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let revision = uuid::Uuid::new_v4().to_string();
        let files = dir.path().join(&id).join("revisions").join(&revision);
        std::fs::create_dir_all(&files).unwrap();
        let bytes = include_bytes!("../tests/fixtures/triangle.stl");
        std::fs::write(files.join("0.stl"), bytes).unwrap();
        std::fs::write(files.join("1.stl"), bytes).unwrap();
        let metadata = dir.path().join(&id).join("plate.json");
        let plate = serde_json::json!({"format_version":1,"id":id,"revision":revision,"name":"Desk",
            "settings":{"old":"ignored"},"project":null,"print":null,"models":[
            {"name":"latest.stl","source":"parts/latest.stl","path":format!("revisions/{revision}/0.stl")},
            {"name":"original.stl","source":null,"path":format!("revisions/{revision}/1.stl")}]});
        std::fs::write(&metadata, serde_json::to_vec(&plate).unwrap()).unwrap();
        // A corrupt published plate must abort the whole migration, without dropping originals.
        let bad = dir.path().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir(&bad).unwrap();
        std::fs::write(bad.join("plate.json"), "broken").unwrap();
        assert!(Database::open(dir.path(), || Ok(None)).is_err());
        assert_eq!(std::fs::read(files.join("1.stl")).unwrap(), bytes);
        std::fs::remove_dir_all(bad).unwrap();
        let db = Database::open(dir.path(), || Ok(None)).unwrap();
        {
            let c = db.connection().unwrap();
            assert_eq!(
                c.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                15
            );
            assert_eq!(
                c.query_row("SELECT name FROM plates WHERE id=?1", [&id], |r| r
                    .get::<_, String>(0))
                    .unwrap(),
                "Desk"
            );
            assert_eq!(c.query_row("SELECT model_key FROM plate_items WHERE source_kind='scad' AND original IS NULL", [], |r|r.get::<_,String>(0)).unwrap(), "parts/latest.stl");
            assert_eq!(
                c.query_row(
                    "SELECT original FROM plate_items WHERE source_kind='upload'",
                    [],
                    |r| r.get::<_, Vec<u8>>(0)
                )
                .unwrap(),
                bytes
            );
            c.execute("UPDATE plates SET name='Edited' WHERE id=?1", [&id])
                .unwrap();
        }
        drop(db);
        std::fs::write(metadata, "old files must never be reimported").unwrap();
        let db = Database::open(dir.path(), || panic!("no reimport")).unwrap();
        let c = db.connection().unwrap();
        assert_eq!(
            c.query_row("SELECT name FROM plates", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "Edited"
        );
        assert_eq!(
            c.query_row("SELECT count(*) FROM plate_items", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            2
        );
    }
    #[test]
    fn schema_sixteen_keeps_uncertain_execution_inputs_and_history_outside_the_queue() {
        let dir = tempfile::tempdir().unwrap();
        let c = Connection::open(dir.path().join("orca.sqlite3")).unwrap();
        c.execute_batch(include_str!("../tests/fixtures/schema-v16.sql"))
            .unwrap();
        save(&c, &device()).unwrap();
        c.execute_batch(r#"INSERT INTO filament_products VALUES ('product','PLA','Fixture','PLA',NULL);
            INSERT INTO filaments VALUES ('color','product','White','FFFFFFFF');
            INSERT INTO filament_settings VALUES ('setting','product','machine','base','{}');
            INSERT INTO ams_slots(id,printer_id,ams_id,slot_index,filament_id,mapping_source) VALUES ('slot','stable-id',0,0,'color','manual');
            INSERT INTO plates(id,name) VALUES ('plate','Retained plate');
            INSERT INTO plate_items(id,plate_id,position,name,source_kind,original,quantity) VALUES ('model','plate',0,'model.stl','upload',x'010203',1);
            INSERT INTO print_jobs(id,printer_id,plate_id,name,ams_slot_id,filament_id,required_machine_profile_key,process_profile_key,bed_type,state,position,attempt_id,artifact_path,execution_json,attempt_json)
                VALUES ('job','stable-id','plate','Printed name','slot','color','machine','process','bed','needs_attention',0,'attempt','jobs/job/attempt','{"frozen":"input"}','{"phase":"unknown","sent":true}');
            INSERT INTO print_jobs(id,printer_id,plate_id,name,ams_slot_id,filament_id,required_machine_profile_key,process_profile_key,bed_type,state,position)
                VALUES ('waiting','stable-id','plate','Old copy','slot','color','machine','process','bed','queued',1);
            INSERT INTO print_history(attempt_id,job_id,plate_id,printer_id,name,completed_at) VALUES ('previous','previous','plate','stable-id','Previous name',123);"#).unwrap();
        drop(c);
        let db = Database::open(dir.path(), || panic!("no import or print")).unwrap();
        let c = db.connection().unwrap();
        let state: (String, String) = c
            .query_row(
                "SELECT state,attempt_id FROM print_jobs WHERE id='job'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, ("needs_attention".into(), "attempt".into()));
        let original: Vec<u8> = c
            .query_row(
                "SELECT original FROM plate_items WHERE id='model'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(original, [1, 2, 3]);
        let waiting: String = c
            .query_row(
                "SELECT plate_id FROM print_jobs WHERE id='waiting'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(waiting, "plate");
        c.execute("DELETE FROM print_jobs WHERE id='job'", [])
            .unwrap();
        let saved: (String,String,String) = c.query_row("SELECT execution_json,attempt_json,artifact_path FROM print_executions WHERE id='attempt'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(
            saved,
            (
                r#"{"frozen":"input"}"#.into(),
                r#"{"phase":"unknown","sent":true}"#.into(),
                "jobs/job/attempt".into()
            )
        );
        assert_eq!(
            c.query_row("SELECT name FROM print_history", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "Previous name"
        );
    }

    #[test]
    fn schema_three_product_migration_preserves_colors_settings_and_active_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let c = Connection::open(dir.path().join("orca.sqlite3")).unwrap();
        c.execute_batch(include_str!("../tests/fixtures/schema-v3.sql"))
            .unwrap();
        save(&c, &device()).unwrap();
        c.execute_batch("INSERT INTO filaments VALUES
            ('black','PLA Matte','Bambu Lab','PLA','000000FF','GFA01'),
            ('white','PLA Matte','Bambu Lab','PLA','FFFFFFFF','GFA01'),
            ('tuned','PLA Matte','Bambu Lab','PLA','FFFF00FF','GFA01'),
            ('different','PLA Basic','Bambu Lab','PLA','000000FF','GFA00');
            INSERT INTO filament_settings VALUES
            ('bs','black','machine','base','{\"nozzle_temperature\":215}'),
            ('ws','white','machine','base','{\"nozzle_temperature\":215}'),
            ('ts','tuned','machine','base','{\"nozzle_temperature\":220}');
            INSERT INTO ams_slots(id,printer_id,ams_id,slot_index,filament_id,mapping_source) VALUES ('slot','stable-id',0,0,'white','manual');
            INSERT INTO plates(id,name) VALUES ('plate','Legacy plate');
            INSERT INTO print_jobs(id,printer_id,plate_id,name,ams_slot_id,filament_id,required_machine_profile_key,process_profile_key,bed_type,state,position,attempt_id,execution_json,attempt_json)
            VALUES ('job','stable-id','plate','Legacy plate','slot','white','machine','quality','bed','printing',0,'attempt','{\"frozen\":true}','{\"sent_at\":123}');").unwrap();
        drop(c);
        let db = Database::open(dir.path(), || panic!("must not import environment")).unwrap();
        let c = db.connection().unwrap();
        assert_eq!(
            c.query_row("SELECT count(*) FROM filament_products", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            3
        );
        assert_eq!(
            c.query_row(
                "SELECT count(DISTINCT product_id) FROM filaments WHERE id IN ('black','white')",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            c.query_row(
                "SELECT filament_id FROM ams_slots WHERE id='slot'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "white"
        );
        let conditions: (Option<String>,Option<String>,Option<String>,Option<String>)=c.query_row("SELECT required_machine_profile_key,filament_id,process_profile_key,bed_type FROM plates WHERE id='plate'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(conditions, (None, None, None, None));
        assert_eq!(
            c.query_row(
                "SELECT attempt_id FROM print_jobs WHERE id='job'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "attempt"
        );
        let job:(String,String,String,String)=c.query_row("SELECT e.filament_id,j.state,e.execution_json,e.attempt_json FROM print_jobs j JOIN print_executions e ON e.id=j.attempt_id WHERE j.id='job'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(
            job,
            (
                "white".into(),
                "printing".into(),
                "{\"frozen\":true}".into(),
                "{\"sent_at\":123}".into()
            )
        );
        assert_eq!(
            c.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            0
        );
        drop(c);
        assert_eq!(
            db.filament_settings("white").unwrap()[0]
                .data
                .overrides_json
                .nozzle_temperature,
            Some(215)
        );
        assert_eq!(
            db.filament_settings("tuned").unwrap()[0]
                .data
                .overrides_json
                .nozzle_temperature,
            Some(220)
        );
        assert_eq!(db.filaments().unwrap().len(), 4);
        assert!(db.delete_setting("white", "bs").is_err());
        assert!(db.delete_filament("white").is_err());
        assert!(db.delete_product("black").is_err());
        drop(db);
        let db = Database::open(dir.path(), || panic!("must not import")).unwrap();
        assert_eq!(db.filaments().unwrap().len(), 4);
    }
    #[test]
    fn legacy_color_read_write_preserves_display_names() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path(), || Ok(None)).unwrap();
        db.save_product(
            "product",
            &crate::products::ProductData {
                name: "Matte".into(),
                vendor: "Bambu".into(),
                material: "PLA".into(),
                bambu_filament_id: None,
            },
        )
        .unwrap();
        for (id, color) in [("black", "000000FF"), ("white", "FFFFFFFF")] {
            db.save_color(
                "product",
                id,
                &crate::products::ColorData {
                    name: id.into(),
                    color: color.into(),
                },
            )
            .unwrap();
            let saved = db
                .filaments()
                .unwrap()
                .into_iter()
                .find(|f| f.id == id)
                .unwrap();
            db.save_filament(&saved).unwrap();
            let read = db
                .filaments()
                .unwrap()
                .into_iter()
                .find(|f| f.id == id)
                .unwrap();
            assert_eq!(read.data.name, saved.data.name);
        }
    }
    #[test]
    fn invalid_legacy_settings_roll_back_product_migration() {
        let dir = tempfile::tempdir().unwrap();
        let c = Connection::open(dir.path().join("orca.sqlite3")).unwrap();
        c.execute_batch(include_str!("../tests/fixtures/schema-v3.sql"))
            .unwrap();
        c.execute_batch("INSERT INTO filaments VALUES ('a','Matte','Bambu','PLA','000000FF',NULL),('z','Matte','Bambu','PLA','FFFFFFFF',NULL);
            INSERT INTO filament_settings VALUES ('bad','z','machine','base','invalid-json');").unwrap();
        drop(c);
        assert!(Database::open(dir.path(), || panic!("no import")).is_err());
        let c = Connection::open(dir.path().join("orca.sqlite3")).unwrap();
        assert_eq!(
            c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            3
        );
        assert_eq!(
            c.query_row("SELECT count(*) FROM filaments", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            c.query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='filament_products'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            c.query_row(
                "SELECT overrides_json FROM filament_settings WHERE id='bad'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "invalid-json"
        );
    }
    #[test]
    fn new_ambiguous_catalog_entry_invalidates_automatic_mapping_without_a_report() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path(), || Ok(Some(device()))).unwrap();
        let mut f = crate::filament::Filament {
            id: "black".into(),
            data: crate::filament::FilamentData {
                name: "Matte black".into(),
                vendor: "Bambu Lab".into(),
                material: "PLA".into(),
                color: "000000FF".into(),
                bambu_filament_id: Some("GFA01".into()),
            },
        };
        db.save_filament(&f).unwrap();
        let mut state = crate::printer_state::State::new(true);
        state.connected();
        state.apply(br#"{"print":{"command":"push_status","msg":0,"gcode_state":"IDLE","print_error":0,"ams":{"tray_exist_bits":"1","ams":[{"id":"0","tray":[{"id":"0","tray_type":"PLA","tray_info_idx":"GFA01","tag_uid":"1234","tray_color":"000000FF"}]}]}}}"#,1);
        db.observe_ams(&device(), &state.status(1)).unwrap();
        let slot = db.ams_slots("stable-id").unwrap().remove(0);
        assert_eq!(slot.filament_id.as_deref(), Some("black"));
        f.id = "another".into();
        db.save_filament(&f).unwrap();
        let changed = db.ams_slots("stable-id").unwrap().remove(0);
        assert!(changed.filament_id.is_none());
        assert!(changed.revision > slot.revision);
    }
    #[test]
    fn version_one_migration_preserves_printers_and_rolls_back_failed_ddl() {
        fn legacy(root: &Path) -> Connection {
            let c = Connection::open(root.join("orca.sqlite3")).unwrap();
            let schema = include_str!("../tests/fixtures/schema-v3.sql");
            c.execute_batch(schema.split("CREATE TABLE filaments").next().unwrap())
                .unwrap();
            save(&c, &device()).unwrap();
            c.pragma_update(None, "user_version", 1).unwrap();
            c
        }
        let dir = tempfile::tempdir().unwrap();
        drop(legacy(dir.path()));
        let db = Database::open(dir.path(), || panic!("no import during migration")).unwrap();
        assert_eq!(db.list().unwrap()[0].id, "stable-id");
        assert!(db.filaments().unwrap().is_empty());
        assert_eq!(
            db.connection()
                .unwrap()
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            17
        );
        let bad = tempfile::tempdir().unwrap();
        legacy(bad.path())
            .execute_batch("CREATE TABLE ams_slots(conflict TEXT);")
            .unwrap();
        assert!(Database::open(bad.path(), || panic!("no import")).is_err());
        let c = Connection::open(bad.path().join("orca.sqlite3")).unwrap();
        assert_eq!(
            c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            c.query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='filaments'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            c.query_row("SELECT id FROM printers", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "stable-id"
        );
    }
    #[test]
    fn connection_changes_invalidate_inventory_and_reject_old_observers() {
        let dir = tempfile::tempdir().unwrap();
        let d = device();
        let db = Database::open(dir.path(), || Ok(Some(d.clone()))).unwrap();
        let mut state = crate::printer_state::State::new(true);
        state.connected();
        state.apply(br#"{"print":{"command":"push_status","msg":0,"gcode_state":"IDLE","print_error":0,"ams":{"tray_exist_bits":"1","ams":[{"id":"0","tray":[{"id":"0","tray_type":"PLA","tray_color":"FFFFFFFF"}]}]}}}"#,1);
        db.observe_ams(&d, &state.status(1)).unwrap();
        let mut edited = d.clone();
        edited.settings.name = "Renamed".into();
        db.save(&edited).unwrap();
        assert_eq!(db.ams_slots(&d.id).unwrap().len(), 1);
        db.observe_ams(&d, &state.status(1)).unwrap();
        edited.settings.host = "127.0.0.2".into();
        db.save(&edited).unwrap();
        assert!(db.ams_slots(&d.id).unwrap().is_empty());
        assert!(db.observe_ams(&d, &state.status(1)).is_err());
        db.observe_ams(&edited, &state.status(1)).unwrap();
        assert_eq!(db.ams_slots(&d.id).unwrap().len(), 1);
    }
    #[test]
    fn observed_slots_keep_manual_materials_until_identity_changes() {
        use crate::printer_state::State;
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path(), || Ok(Some(device()))).unwrap();
        let f = crate::filament::Filament {
            id: "gf".into(),
            data: crate::filament::FilamentData {
                name: "Glass PETG".into(),
                vendor: "Third party".into(),
                material: "PETG-GF".into(),
                color: "FFFFFFFF".into(),
                bambu_filament_id: None,
            },
        };
        db.save_filament(&f).unwrap();
        let mut state = State::new(true);
        state.connected();
        state.apply(br#"{"print":{"command":"push_status","msg":0,"gcode_state":"IDLE","print_error":0,"ams":{"tray_exist_bits":"1","ams":[{"id":"0","tray":[{"id":"0","tray_type":"PETG","tray_color":"FFFFFFFF","remain":-1}]}]}}}"#,1);
        db.observe_ams(&device(), &state.status(1)).unwrap();
        let slot = db.ams_slots("stable-id").unwrap().remove(0);
        db.map_slot("stable-id", &slot.id, slot.revision, Some("gf"))
            .unwrap();
        assert!(db.delete_filament("gf").is_err());
        state.apply(br#"{"print":{"command":"push_status","msg":1,"ams":{"ams":[{"id":"0","tray":[{"id":"0","remain":40}]}]}}}"#,2);
        db.observe_ams(&device(), &state.status(2)).unwrap();
        let slot = db.ams_slots("stable-id").unwrap().remove(0);
        assert_eq!(slot.filament_id.as_deref(), Some("gf"));
        assert_eq!(slot.mapping_source, "manual");
        assert_eq!(slot.reported.material.as_deref(), Some("PETG"));
        state.apply(br#"{"print":{"command":"push_status","msg":1,"ams":{"ams":[{"id":"0","tray":[{"id":"0","tray_color":"000000FF"}]}]}}}"#,3);
        db.observe_ams(&device(), &state.status(3)).unwrap();
        assert!(
            db.map_slot("stable-id", &slot.id, slot.revision, Some("gf"))
                .is_err()
        );
        let changed = db.ams_slots("stable-id").unwrap().remove(0);
        assert!(changed.filament_id.is_none());
        assert!(changed.revision > slot.revision);
        db.delete_filament("gf").unwrap();
    }
    #[test]
    fn materials_and_machine_settings_persist_and_protect_references() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path(), || Ok(Some(device()))).unwrap();
        let f = crate::filament::Filament {
            id: "material-1".into(),
            data: crate::filament::FilamentData {
                name: "My glass PETG".into(),
                vendor: "Third party".into(),
                material: "PETG-GF".into(),
                color: "FFFFFFFF".into(),
                bambu_filament_id: None,
            },
        };
        db.save_filament(&f).unwrap();
        let setting = crate::filament::Setting {
            id: "setting-1".into(),
            filament_id: f.id.clone(),
            data: crate::filament::SettingData {
                machine_profile_key: crate::profiles::PRINTER.into(),
                base_profile_key: "Generic PETG @BBL X1C".into(),
                overrides_json: crate::filament::Overrides {
                    nozzle_temperature_initial_layer: Some(250),
                    nozzle_temperature: Some(240),
                    bed_temperature_initial_layer: Some(65),
                    bed_temperature: Some(65),
                },
            },
        };
        db.save_setting(&setting).unwrap();
        let mut duplicate = setting.clone();
        duplicate.id = "duplicate".into();
        assert!(db.save_setting(&duplicate).is_err());
        drop(db);
        let db = Database::open(dir.path(), || panic!("no reimport")).unwrap();
        assert_eq!(db.filaments().unwrap()[0].data.material, "PETG-GF");
        assert_eq!(
            db.filament_settings(&f.id).unwrap()[0]
                .data
                .overrides_json
                .nozzle_temperature,
            Some(240)
        );
        assert!(
            db.filament_settings(&f.id).unwrap()[0].data.overrides_json
                == setting.data.overrides_json
        );
        db.delete_filament(&f.id).unwrap();
        assert!(db.filament_settings(&f.id).unwrap().is_empty());
    }
    #[test]
    fn invalid_registry_metadata_cannot_be_saved_as_usable_configuration() {
        let mut d = device();
        assert!(d.settings.validate().is_ok());
        d.settings.name = " ".into();
        assert!(d.settings.validate().is_err());
        d.settings.name = "P1S".into();
        d.settings.nozzle_material = "plastic".into();
        assert!(d.settings.validate().is_err());
        d.settings.nozzle_material = "hardened_steel".into();
        d.settings.bed_type = "unknown".into();
        assert!(d.settings.validate().is_err());
    }
    #[test]
    fn registry_persists_edits_and_deletes_without_reimporting_environment() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path(), || Ok(Some(device()))).unwrap();
        let mut d = db.list().unwrap().remove(0);
        d.settings.name = "Edited".into();
        db.save(&d).unwrap();
        drop(db);
        let db = Database::open(dir.path(), || panic!("must not read old environment")).unwrap();
        assert_eq!(db.list().unwrap()[0].settings.name, "Edited");
        let json = serde_json::to_value(&db.list().unwrap()[0]).unwrap();
        assert!(json.get("access_code").is_none());
        assert!(json.get("tls_certificate").is_none());
        db.delete(&d.id).unwrap();
        drop(db);
        let db = Database::open(dir.path(), || {
            panic!("deletion must not restore old environment")
        })
        .unwrap();
        assert!(db.list().unwrap().is_empty());
    }
    #[test]
    fn interrupted_schema_initialization_can_restart_without_changing_plate_files() {
        const CHILD_ROOT: &str = "ORCA_TEST_INTERRUPTED_DATABASE";
        if let Some(root) = std::env::var_os(CHILD_ROOT) {
            let _ = Database::open(Path::new(&root), || std::process::exit(63));
            panic!("initialization did not reach the import transaction");
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("existing-plate"), b"preserved input").unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "database::tests::interrupted_schema_initialization_can_restart_without_changing_plate_files"])
            .env(CHILD_ROOT, dir.path())
            .status().unwrap();
        assert_eq!(status.code(), Some(63));
        let db = Database::open(dir.path(), || Ok(Some(device()))).unwrap();
        assert_eq!(db.list().unwrap()[0].id, "stable-id");
        assert_eq!(
            std::fs::read(dir.path().join("existing-plate")).unwrap(),
            b"preserved input"
        );
    }
    #[test]
    fn failed_initialization_rolls_back_and_future_schema_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Database::open(dir.path(), || Err(Error::Invalid("bad initial config"))).is_err());
        let db = Database::open(dir.path(), || Ok(Some(device()))).unwrap();
        assert_eq!(db.list().unwrap().len(), 1);
        db.connection
            .lock()
            .unwrap()
            .pragma_update(None, "user_version", 99)
            .unwrap();
        drop(db);
        assert!(Database::open(dir.path(), || panic!("future DB")).is_err());
        let c = rusqlite::Connection::open(dir.path().join("orca.sqlite3")).unwrap();
        assert_eq!(
            c.query_row("SELECT name FROM printers", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "P1S"
        );
    }
}
