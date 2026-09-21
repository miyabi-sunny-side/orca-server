use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Cursor, Read},
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub enum Error {
    Invalid(&'static str),
    Upstream(&'static str),
    Unavailable(&'static str),
    Conflict(&'static str),
    Timeout,
    NotFound,
    Io(io::Error),
}
pub type Result<T> = std::result::Result<T, Error>;

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::NotFound {
            Self::NotFound
        } else {
            Self::Io(error)
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}

pub const MAX_UPLOAD: usize = 64 * 1024 * 1024;
const MAX_METADATA: usize = 256 * 1024;

fn read_limited(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(u64::try_from(limit).unwrap_or(u64::MAX) + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(Error::Invalid("Stored file exceeds the size limit"));
    }
    Ok(bytes)
}

fn valid_id(id: &str) -> Result<()> {
    if uuid::Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id) {
        Ok(())
    } else {
        Err(Error::Invalid("Invalid plate ID"))
    }
}

fn relative_path(path: &str) -> bool {
    !path.contains('\\')
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

pub(crate) fn valid_model_name(name: &str) -> bool {
    relative_path(name) && name.len() <= 1024 && name.to_ascii_lowercase().ends_with(".stl")
}

pub(crate) fn validate_stl(data: &[u8]) -> Result<()> {
    if data.len() > MAX_UPLOAD {
        return Err(Error::Invalid("Models exceed 64 MiB"));
    }
    let mut cursor = Cursor::new(data);
    let reader =
        stl_io::create_stl_reader(&mut cursor).map_err(|_| Error::Invalid("Invalid STL"))?;
    let mut count = 0;
    for triangle in reader {
        let triangle = triangle.map_err(|_| Error::Invalid("Invalid STL"))?;
        if triangle
            .vertices
            .iter()
            .chain(std::iter::once(&triangle.normal))
            .any(|v| (0..3).any(|i| !v[i].is_finite()))
        {
            return Err(Error::Invalid("STL coordinates must be finite"));
        }
        count += 1;
    }
    if count == 0 {
        return Err(Error::Invalid("STL must contain triangles"));
    }
    Ok(())
}

#[derive(Clone)]
pub struct Store {
    pub(crate) root: PathBuf,
    pub(crate) db: crate::database::Database,
    pub(crate) profiles: Option<std::sync::Arc<crate::profiles::Profiles>>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub source: Option<String>,
    pub quantity: u16,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Conditions {
    pub required_machine_profile_key: Option<String>,
    pub filament_id: Option<String>,
    pub process_profile_key: Option<String>,
    pub bed_type: Option<String>,
}
impl Conditions {
    fn validate(
        &self,
        c: &rusqlite::Connection,
        profiles: Option<&crate::profiles::Profiles>,
    ) -> Result<()> {
        if self
            .bed_type
            .as_ref()
            .is_some_and(|bed| !crate::profiles::BEDS.contains(&bed.as_str()))
        {
            return Err(Error::Invalid("Unknown bed type"));
        }
        if let Some(id) = &self.filament_id {
            crate::products::product_id(c, id)?;
        }
        if let Some(machine) = &self.required_machine_profile_key {
            if !c.query_row(
                "SELECT EXISTS(SELECT 1 FROM printers WHERE machine_profile_key=?1)",
                [machine],
                |r| r.get::<_, bool>(0),
            )? {
                return Err(Error::Conflict(
                    "Register a matching machine and nozzle before saving these conditions",
                ));
            }
            let profiles = profiles.ok_or(Error::Unavailable("OrcaSlicer is not configured"))?;
            profiles.machine(machine)?;
            if let Some(process) = &self.process_profile_key {
                profiles.validate_process(machine, process)?;
            }
            if let Some(id) = &self.filament_id {
                let setting = crate::products::load_setting(c, id, machine)?;
                let filament = crate::database::load_filaments(c)?
                    .into_iter()
                    .find(|f| f.id == *id)
                    .ok_or(Error::NotFound)?;
                profiles.resolve_filament(&setting, &filament.data.material)?;
            }
        } else if self.process_profile_key.is_some() {
            return Err(Error::Invalid(
                "Select the required machine before a process profile",
            ));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Plate {
    pub id: String,
    pub version: i64,
    pub name: String,
    #[serde(default)]
    pub conditions: Conditions,
    pub models: Vec<Model>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemEdit {
    pub id: Option<String>,
    pub name: String,
    pub source: Option<String>,
    pub quantity: u16,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    #[serde(default)]
    pub conditions: Conditions,
    pub name: String,
    pub version: Option<i64>,
    pub models: Vec<ItemEdit>,
}
pub struct ModelInput {
    pub name: String,
    pub source: Option<String>,
    pub data: Vec<u8>,
}
pub struct Input {
    pub conditions: Conditions,
    pub name: String,
    pub models: Vec<ModelInput>,
}

pub(crate) fn validate_metadata(name: &str) -> Result<()> {
    if name.trim().is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
        return Err(Error::Invalid(
            "Name must contain 1–256 bytes without control characters",
        ));
    }
    Ok(())
}
fn validate_items(models: &[ItemEdit]) -> Result<()> {
    let count: usize = models.iter().map(|m| usize::from(m.quantity)).sum();
    if models.is_empty()
        || models.len() > 64
        || count > 64
        || models.iter().any(|m| m.quantity == 0)
    {
        return Err(Error::Invalid("A plate must contain 1–64 model instances"));
    }
    let mut ids = std::collections::BTreeSet::new();
    for m in models {
        if !valid_model_name(&m.name) || m.source.as_ref().is_some_and(|s| !valid_model_name(s)) {
            return Err(Error::Invalid(
                "Each model needs a relative STL name and source",
            ));
        }
        if let Some(id) = &m.id {
            valid_id(id)?;
            if !ids.insert(id) {
                return Err(Error::Invalid("Duplicate plate item"));
            }
        }
        if m.id.is_none() && m.source.is_none() {
            return Err(Error::Invalid(
                "Upload the original STL before referring to it",
            ));
        }
    }
    Ok(())
}
impl Store {
    /// Opens `SQLite` and atomically imports legacy plates once, preserving their files.
    /// # Errors
    /// Rejects invalid legacy data, unavailable storage and unsupported schema versions.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let db = crate::database::Database::open(root.as_ref(), || {
            crate::printer::Config::from_env()
                .map_err(Error::Invalid)?
                .map(|config| {
                    config.import().map(|settings| crate::database::Device {
                        id: "p1".into(),
                        settings,
                    })
                })
                .transpose()
        })?;
        Ok(Self {
            root: fs::canonicalize(root)?,
            db,
            profiles: None,
        })
    }
    /// Save uploaded STL originals or model references in `SQLite`.
    /// # Errors
    /// Rejects malformed STL, invalid metadata and unavailable storage.
    pub fn save(&self, input: Input) -> Result<Plate> {
        validate_metadata(&input.name)?;
        if input.models.is_empty() || input.models.len() > 64 {
            return Err(Error::Invalid("Select 1–64 STL models"));
        }
        let total: usize = input.models.iter().map(|m| m.data.len()).sum();
        if total > MAX_UPLOAD {
            return Err(Error::Invalid("Models exceed 64 MiB"));
        }
        let mut models = Vec::new();
        for m in input.models {
            validate_stl(&m.data)?;
            let item = ItemEdit {
                id: Some(uuid::Uuid::new_v4().to_string()),
                name: m.name,
                source: m.source,
                quantity: 1,
            };
            models.push((item, m.data));
        }
        validate_items(&models.iter().map(|(m, _)| m.clone()).collect::<Vec<_>>())?;
        let id = uuid::Uuid::new_v4().to_string();
        let mut c = self.db.connection()?;
        let tx = c.transaction()?;
        input.conditions.validate(&tx, self.profiles.as_deref())?;
        tx.execute(
            "INSERT INTO plates(id,name) VALUES (?1,?2)",
            rusqlite::params![id, input.name.trim()],
        )?;
        save_conditions(&tx, &id, &input.conditions)?;
        for (position, (m, data)) in models.iter().enumerate() {
            insert_item(
                &tx,
                &id,
                position,
                m,
                if m.source.is_none() {
                    Some(data.as_slice())
                } else {
                    None
                },
            )?;
        }
        let plate = load(&tx, &id)?;
        tx.commit()?;
        Ok(plate)
    }
    /// Create or edit a composition, rejecting stale versions and foreign upload references.
    /// # Errors
    /// Rejects unknown items, invalid quantities and concurrent edits.
    pub fn edit(&self, id: Option<&str>, edit: Edit) -> Result<Plate> {
        let Edit {
            conditions,
            name,
            version,
            models,
        } = edit;
        validate_metadata(&name)?;
        validate_items(&models)?;
        let mut c = self.db.connection()?;
        let tx = c.transaction()?;
        conditions.validate(&tx, self.profiles.as_deref())?;
        let id = if let Some(id) = id {
            valid_id(id)?;
            if tx.execute(
                "UPDATE plates SET name=?1,version=version+1 WHERE id=?2 AND version=?3",
                rusqlite::params![name.trim(), id, version],
            )? != 1
            {
                return Err(Error::Conflict("Plate changed; reload before saving"));
            }
            id.to_owned()
        } else {
            if version.is_some() {
                return Err(Error::Invalid("New plates have no version"));
            }
            let id = uuid::Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO plates(id,name) VALUES (?1,?2)",
                rusqlite::params![id, name.trim()],
            )?;
            id
        };
        save_conditions(&tx, &id, &conditions)?;
        // Invalidate queue command fences when a waiting plate changes. Active snapshots stay intact.
        tx.execute("UPDATE printers SET queue_generation=queue_generation+1,queue_request=NULL WHERE id IN (SELECT printer_id FROM print_jobs WHERE plate_id=?1 AND state='queued')", [&id])?;
        let mut inputs = Vec::new();
        for m in &models {
            let original =
                if let Some(item_id) = &m.id {
                    let stored: Option<(Option<String>,Option<Vec<u8>>)> = tx.query_row(
                    "SELECT model_key,original FROM plate_items WHERE id=?1 AND plate_id=?2",
                    rusqlite::params![item_id,id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
                    let (source, original) =
                        stored.ok_or(Error::Invalid("Item does not belong to this plate"))?;
                    if source != m.source {
                        return Err(Error::Invalid("Replace the reference as a new item"));
                    }
                    original
                } else {
                    None
                };
            inputs.push(original);
        }
        tx.execute("DELETE FROM plate_items WHERE plate_id=?1", [&id])?;
        for (position, (m, original)) in models.iter().zip(inputs).enumerate() {
            insert_item(&tx, &id, position, m, original.as_deref())?;
        }
        let plate = load(&tx, &id)?;
        tx.commit()?;
        Ok(plate)
    }
    /// Read a saved composition.
    /// # Errors
    /// Rejects invalid IDs, missing plates or unavailable storage.
    pub fn get(&self, id: &str) -> Result<Plate> {
        valid_id(id)?;
        load(&*self.db.connection()?, id)
    }
    /// Search saved names and model names, ordered by fuzzy relevance.
    /// # Errors
    /// Returns storage errors.
    pub fn list(&self, query: &str) -> Result<Vec<Plate>> {
        let c = self.db.connection()?;
        let ids = c
            .prepare("SELECT id FROM plates")?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut matches = Vec::new();
        for id in ids {
            let plate = load(&c, &id)?;
            if let Some(score) = std::iter::once(plate.name.as_str())
                .chain(plate.models.iter().map(|m| m.name.as_str()))
                .filter_map(|s| crate::search::score(query.trim(), s))
                .max()
            {
                matches.push((score, plate));
            }
        }
        matches.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| a.1.name.cmp(&b.1.name))
                .then_with(|| a.1.id.cmp(&b.1.id))
        });
        Ok(matches.into_iter().map(|(_, p)| p).collect())
    }
    /// Download a directly uploaded original. SCAD originals belong to the upstream service.
    /// # Errors
    /// Rejects missing items and references without an uploaded original.
    pub fn read_file(&self, id: &str, item: &str) -> Result<Vec<u8>> {
        valid_id(id)?;
        valid_id(item).map_err(|_| Error::NotFound)?;
        self.db.connection()?.query_row("SELECT original FROM plate_items WHERE plate_id=?1 AND id=?2 AND source_kind='upload'",rusqlite::params![id,item],|r|r.get(0)).optional()?.ok_or(Error::NotFound)
    }
}
pub(crate) fn load(c: &rusqlite::Connection, id: &str) -> Result<Plate> {
    let (name, version, conditions) = c
        .query_row("SELECT name,version,required_machine_profile_key,filament_id,process_profile_key,bed_type FROM plates WHERE id=?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?, Conditions { required_machine_profile_key:r.get(2)?,filament_id:r.get(3)?,process_profile_key:r.get(4)?,bed_type:r.get(5)? }))
        })
        .optional()?
        .ok_or(Error::NotFound)?;
    let models=c.prepare("SELECT id,name,model_key,quantity FROM plate_items WHERE plate_id=?1 ORDER BY position")?
        .query_map([id],|r|Ok(Model{id:r.get(0)?,name:r.get(1)?,source:r.get(2)?,quantity:r.get(3)?}))?.collect::<std::result::Result<Vec<_>,_>>()?;
    Ok(Plate {
        id: id.into(),
        name,
        version,
        conditions,
        models,
    })
}
pub(crate) fn migrate_conditions(c: &rusqlite::Connection) -> Result<()> {
    c.execute_batch(
        "ALTER TABLE plates ADD COLUMN required_machine_profile_key TEXT;
        ALTER TABLE plates ADD COLUMN filament_id TEXT REFERENCES filaments(id);
        ALTER TABLE plates ADD COLUMN process_profile_key TEXT;
        ALTER TABLE plates ADD COLUMN bed_type TEXT;
        UPDATE printers SET queue_request=NULL,queue_generation=queue_generation+1;
        PRAGMA user_version=6;",
    )?;
    Ok(())
}
fn save_conditions(c: &rusqlite::Connection, id: &str, v: &Conditions) -> Result<()> {
    c.execute("UPDATE plates SET required_machine_profile_key=?1,filament_id=?2,process_profile_key=?3,bed_type=?4 WHERE id=?5", rusqlite::params![v.required_machine_profile_key,v.filament_id,v.process_profile_key,v.bed_type,id])?;
    Ok(())
}
fn insert_item(
    c: &rusqlite::Connection,
    id: &str,
    position: usize,
    m: &ItemEdit,
    original: Option<&[u8]>,
) -> Result<()> {
    c.execute("INSERT INTO plate_items(id,plate_id,position,name,source_kind,model_key,original,quantity) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        rusqlite::params![m.id.clone().unwrap_or_else(||uuid::Uuid::new_v4().to_string()),id,i64::try_from(position).expect("64 items"),m.name,if m.source.is_some(){"scad"}else{"upload"},m.source,original,m.quantity])?;
    Ok(())
}

// Only schema migration reads the old filesystem representation.
pub(crate) fn migrate(c: &rusqlite::Connection, root: &Path) -> Result<()> {
    #[derive(Deserialize)]
    struct Legacy {
        format_version: u8,
        id: String,
        revision: String,
        name: String,
        models: Vec<LegacyModel>,
    }
    #[derive(Deserialize)]
    struct LegacyModel {
        name: String,
        source: Option<String>,
        path: String,
    }
    for entry in fs::read_dir(root)? {
        let id = entry?.file_name().to_string_lossy().into_owned();
        if valid_id(&id).is_err() {
            continue;
        }
        let path = match checked(root, &format!("{id}/plate.json")) {
            Ok(p) => p,
            Err(Error::NotFound) => continue,
            Err(e) => return Err(e),
        };
        let plate: Legacy = serde_json::from_slice(&read_limited(&path, MAX_METADATA)?)
            .map_err(io::Error::other)?;
        valid_id(&plate.revision)?;
        validate_metadata(&plate.name)?;
        if plate.id != id
            || plate.format_version != 1
            || plate.models.is_empty()
            || plate.models.len() > 64
        {
            return Err(Error::Invalid("Invalid legacy plate"));
        }
        c.execute(
            "INSERT INTO plates(id,name) VALUES (?1,?2)",
            rusqlite::params![id, plate.name],
        )?;
        let mut total = 0;
        for (position, m) in plate.models.into_iter().enumerate() {
            if m.path != format!("revisions/{}/{position}.stl", plate.revision) {
                return Err(Error::Invalid("Invalid legacy model path"));
            }
            let item = ItemEdit {
                id: Some(uuid::Uuid::new_v4().to_string()),
                name: m.name,
                source: m.source,
                quantity: 1,
            };
            validate_items(std::slice::from_ref(&item))?;
            let original = if item.source.is_none() {
                let data = read_limited(&checked(root, &format!("{id}/{}", m.path))?, MAX_UPLOAD)?;
                total += data.len();
                if total > MAX_UPLOAD {
                    return Err(Error::Invalid("Legacy plate exceeds 64 MiB"));
                }
                validate_stl(&data)?;
                Some(data)
            } else {
                None
            };
            insert_item(c, &id, position, &item, original.as_deref())?;
        }
    }
    Ok(())
}
fn checked(root: &Path, relative: &str) -> Result<PathBuf> {
    if !relative_path(relative) {
        return Err(Error::Invalid("Invalid storage path"));
    }
    let mut path = root.to_owned();
    for part in relative.split('/') {
        path.push(part);
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Err(Error::Invalid("Storage symlinks are not supported"));
        }
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn edit(name: &str, quantity: u16) -> Edit {
        Edit {
            conditions: Conditions::default(),
            name: name.into(),
            version: None,
            models: vec![ItemEdit {
                id: None,
                name: "part.stl".into(),
                source: Some("parts/part.stl".into()),
                quantity,
            }],
        }
    }
    #[test]
    fn nullable_conditions_survive_save_reload_and_legacy_snapshots() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let raw = serde_json::json!({"name":"unconfigured", "models":[{"name":"part.stl","source":"part.stl","quantity":1}], "conditions":{"required_machine_profile_key":null,"filament_id":null,"process_profile_key":null,"bed_type":null}});
        let saved = store
            .edit(None, serde_json::from_value(raw).unwrap())
            .unwrap();
        let value = serde_json::to_value(&saved).unwrap();
        assert!(value["conditions"].is_object());
        assert_eq!(value["conditions"].as_object().unwrap().len(), 4);
        assert!(
            value["conditions"]
                .as_object()
                .unwrap()
                .values()
                .all(serde_json::Value::is_null)
        );
        assert_eq!(
            Store::open(root.path()).unwrap().get(&saved.id).unwrap(),
            saved
        );
        let mut legacy = value;
        legacy.as_object_mut().unwrap().remove("conditions");
        assert_eq!(serde_json::from_value::<Plate>(legacy).unwrap(), saved);
        let c = store.db.connection().unwrap();
        assert_eq!(
            c.query_row(
                "SELECT required_machine_profile_key,filament_id,bed_type FROM plates",
                [],
                |r| Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?
                ))
            )
            .unwrap(),
            (None, None, None)
        );
    }
    #[test]
    fn plate_storage_is_database_owned_and_does_not_freeze_scad_bytes() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let plate = store.edit(None, edit("reference", 2)).unwrap();
        let c = rusqlite::Connection::open(root.path().join("orca.sqlite3")).unwrap();
        assert_eq!(
            c.query_row(
                "SELECT count(*) FROM plate_items WHERE original IS NULL",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert!(!root.path().join(&plate.id).exists());
        assert_eq!(
            Store::open(root.path()).unwrap().get(&plate.id).unwrap(),
            plate
        );
        assert_eq!(store.list("prt").unwrap(), vec![plate]);
    }
    #[test]
    fn edits_are_atomic_and_uploads_survive_a_database_only_restore() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let bytes = include_bytes!("../tests/fixtures/triangle.stl").to_vec();
        let plate = store
            .save(Input {
                conditions: Conditions::default(),
                name: "original".into(),
                models: vec![ModelInput {
                    name: "part.stl".into(),
                    source: None,
                    data: bytes.clone(),
                }],
            })
            .unwrap();
        let request = |quantity| Edit {
            conditions: Conditions::default(),
            name: "edited".into(),
            version: Some(plate.version),
            models: vec![ItemEdit {
                id: Some(plate.models[0].id.clone()),
                name: "part.stl".into(),
                source: None,
                quantity,
            }],
        };
        for qty in [0, 65] {
            assert!(store.edit(Some(&plate.id), request(qty)).is_err());
        }
        assert_eq!(store.get(&plate.id).unwrap(), plate);
        let saved = store.edit(Some(&plate.id), request(3)).unwrap();
        assert!(store.edit(Some(&plate.id), request(1)).is_err());
        let other = store.edit(None, edit("other", 1)).unwrap();
        let mut theft = request(1);
        theft.version = Some(other.version);
        assert!(store.edit(Some(&other.id), theft).is_err());
        assert_eq!(store.get(&plate.id).unwrap(), saved);
        drop(store);
        let restored = tempfile::tempdir().unwrap();
        fs::copy(
            root.path().join("orca.sqlite3"),
            restored.path().join("orca.sqlite3"),
        )
        .unwrap();
        let store = Store::open(restored.path()).unwrap();
        assert_eq!(store.get(&plate.id).unwrap(), saved);
        assert_eq!(
            store.read_file(&plate.id, &plate.models[0].id).unwrap(),
            bytes
        );
        assert!(store.read_file(&plate.id, "../../etc/passwd").is_err());
    }
    #[test]
    fn rejects_invalid_compositions_and_stl_coordinates() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        for name in ["", " ", "bad\nname"] {
            assert!(store.edit(None, edit(name, 1)).is_err());
        }
        for path in ["../part.stl", "/etc/part.stl", "bad.txt", "a\\b.stl"] {
            let mut e = edit("x", 1);
            e.models[0].source = Some(path.into());
            assert!(store.edit(None, e).is_err());
        }
        let mut e = edit("x", 64);
        e.models.push(e.models[0].clone());
        assert!(store.edit(None, e).is_err());
        let mut data = vec![0u8; 134];
        data[80..84].copy_from_slice(&1u32.to_le_bytes());
        assert!(validate_stl(&data).is_ok());
        data[96..100].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(validate_stl(&data).is_err());
        assert!(validate_stl(b"solid empty\nendsolid empty\n").is_err());
        assert!(validate_stl(&vec![0; MAX_UPLOAD + 1]).is_err());
    }
}
