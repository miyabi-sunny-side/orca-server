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
            Error::Conflict("Printer serial is already registered or settings violate the schema")
        } else {
            tracing::error!(code=?error.sqlite_error_code(), "SQLite operation failed");
            Error::Unavailable("Database operation failed; check storage and server logs")
        }
    }
}
impl Database {
    pub fn open(root: &Path, initial: impl FnOnce() -> Result<Option<Device>>) -> Result<Self> {
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
        let mut connection = Connection::open(&path).map_err(Error::from)?;
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
            1 => {}
            _ => {
                return Err(Error::Unavailable(
                    "Database schema is newer than this server; use a compatible version",
                ));
            }
        }
        tx.commit().map_err(Error::from)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
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
        let connection = self
            .connection
            .lock()
            .map_err(|_| Error::Unavailable("Database lock failed"))?;
        save(&connection, device)
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
    connection.execute("INSERT INTO printers VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)
        ON CONFLICT(id) DO UPDATE SET name=excluded.name,host=excluded.host,serial=excluded.serial,
        access_code=excluded.access_code,tls_certificate=excluded.tls_certificate,
        machine_profile_key=excluded.machine_profile_key,default_process_profile_key=excluded.default_process_profile_key,
        bed_type=excluded.bed_type,nozzle_material=excluded.nozzle_material,mqtt_port=excluded.mqtt_port,
        ftps_port=excluded.ftps_port,start_timeout_secs=excluded.start_timeout_secs",
        params![device.id,s.name,s.host,s.serial,s.access_code,s.tls_certificate,s.machine_profile_key,s.default_process_profile_key,s.bed_type,s.nozzle_material,s.mqtt_port,s.ftps_port,s.start_timeout_secs]).map_err(Error::from)?;
    Ok(())
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
