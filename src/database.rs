use crate::plates::{Error, Result};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{Arc, Mutex},
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
    Connection::open(&path).map_err(Error::from)
}
impl Database {
    pub fn open(root: &Path, initial: impl FnOnce() -> Result<Option<Device>>) -> Result<Self> {
        let mut connection = private_connection(root)?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(Error::from)?;
        connection
            .pragma_update(None, "foreign_keys", true)
            .map_err(Error::from)?;
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(Error::from)?;
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
            1..=3 => {}
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
        tx.commit().map_err(Error::from)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
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
        tx.execute("INSERT INTO filaments(id,name,vendor,material,color,bambu_filament_id) VALUES (?1,?2,?3,?4,?5,?6)
            ON CONFLICT(id) DO UPDATE SET name=excluded.name,vendor=excluded.vendor,material=excluded.material,color=excluded.color,bambu_filament_id=excluded.bambu_filament_id",
            params![f.id,f.data.name,f.data.vendor,f.data.material,f.data.color,f.data.bambu_filament_id])?;
        crate::ams::invalidate_automatic(&tx)?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn delete_filament(&self, id: &str) -> Result<()> {
        if self
            .connection()?
            .execute("DELETE FROM filaments WHERE id=?1", [id])?
            == 0
        {
            return Err(Error::NotFound);
        }
        Ok(())
    }
    pub(crate) fn filament_settings(
        &self,
        filament_id: &str,
    ) -> Result<Vec<crate::filament::Setting>> {
        use crate::filament::{Setting, SettingData};
        let connection = self.connection()?;
        let mut q=connection.prepare("SELECT id,filament_id,machine_profile_key,base_profile_key,overrides_json FROM filament_settings WHERE filament_id=?1 ORDER BY machine_profile_key")?;
        Ok(q.query_map([filament_id], |r| {
            let raw: String = r.get(4)?;
            let overrides_json = serde_json::from_str(&raw).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    4,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
            Ok(Setting {
                id: r.get(0)?,
                filament_id: r.get(1)?,
                data: SettingData {
                    machine_profile_key: r.get(2)?,
                    base_profile_key: r.get(3)?,
                    overrides_json,
                },
            })
        })?
        .collect::<std::result::Result<_, _>>()?)
    }
    pub(crate) fn save_setting(&self, s: &crate::filament::Setting) -> Result<()> {
        s.data.overrides_json.validate()?;
        self.connection()?.execute("INSERT INTO filament_settings(id,filament_id,machine_profile_key,base_profile_key,overrides_json) VALUES (?1,?2,?3,?4,?5)
            ON CONFLICT(id) DO UPDATE SET machine_profile_key=excluded.machine_profile_key,base_profile_key=excluded.base_profile_key,overrides_json=excluded.overrides_json",
            params![s.id,s.filament_id,s.data.machine_profile_key,s.data.base_profile_key,serde_json::to_string(&s.data.overrides_json).expect("temperatures serialize")])?;
        Ok(())
    }
    pub(crate) fn delete_setting(&self, filament_id: &str, id: &str) -> Result<()> {
        if self.connection()?.execute(
            "DELETE FROM filament_settings WHERE id=?1 AND filament_id=?2",
            params![id, filament_id],
        )? == 0
        {
            return Err(Error::NotFound);
        }
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
        tx.execute("UPDATE printers SET queue_generation=queue_generation+1,queue_request=NULL WHERE id=?1 AND (host!=?2 OR serial!=?3 OR access_code!=?4 OR tls_certificate!=?5 OR machine_profile_key!=?6 OR default_process_profile_key!=?7 OR bed_type!=?8 OR nozzle_material!=?9 OR mqtt_port!=?10 OR ftps_port!=?11 OR start_timeout_secs!=?12)",params![device.id,s.host,s.serial,s.access_code,s.tls_certificate,s.machine_profile_key,s.default_process_profile_key,s.bed_type,s.nozzle_material,s.mqtt_port,s.ftps_port,s.start_timeout_secs])?;
        save(&tx, device)?;
        tx.commit()?;
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let changed = self
            .connection
            .lock()
            .map_err(|_| Error::Unavailable("Database lock failed"))?
            .execute("DELETE FROM printers WHERE id=?1", [id])
            .map_err(Error::from)?;
        if changed == 0 {
            return Err(Error::NotFound);
        }
        Ok(())
    }
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
        "SELECT id,name,vendor,material,color,bambu_filament_id FROM filaments ORDER BY name,id",
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
                7
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
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path(), || Ok(Some(device()))).unwrap();
        db.connection().unwrap().execute_batch("DROP TABLE print_jobs;DROP TABLE plate_items;DROP TABLE plates;ALTER TABLE printers DROP COLUMN queue_request;ALTER TABLE printers DROP COLUMN queue_generation;DROP TABLE ams_slots;DROP TABLE filament_settings;DROP TABLE filaments;PRAGMA user_version=1;").unwrap();
        drop(db);
        let db = Database::open(dir.path(), || panic!("no import during migration")).unwrap();
        assert_eq!(db.list().unwrap()[0].id, "stable-id");
        assert!(db.filaments().unwrap().is_empty());
        let c = db.connection().unwrap();
        assert_eq!(
            c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            3
        );
        c.execute_batch("DROP TABLE print_jobs;DROP TABLE plate_items;DROP TABLE plates;ALTER TABLE printers DROP COLUMN queue_request;ALTER TABLE printers DROP COLUMN queue_generation;DROP TABLE ams_slots;DROP TABLE filament_settings;DROP TABLE filaments;CREATE TABLE ams_slots(conflict TEXT);PRAGMA user_version=1;").unwrap();
        drop(c);
        drop(db);
        assert!(Database::open(dir.path(), || panic!("no import")).is_err());
        let c = Connection::open(dir.path().join("orca.sqlite3")).unwrap();
        assert_eq!(
            c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name='filaments'",
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
