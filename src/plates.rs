use rmcp::schemars;
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
    Slicer(String),
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

pub(crate) fn read_limited(path: &Path, limit: usize) -> Result<Vec<u8>> {
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
    relative_path(name)
        && name.len() <= 1024
        && [".stl", ".3mf"]
            .iter()
            .any(|ext| name.to_ascii_lowercase().ends_with(ext))
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
    pub(crate) slice_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub source: Option<String>,
    pub quantity: u16,
    #[serde(default = "primary_role", skip_serializing_if = "is_primary")]
    pub roles: Vec<crate::model_import::Role>,
}
fn primary_role() -> Vec<crate::model_import::Role> {
    vec![crate::model_import::Role::Primary]
}
fn is_primary(roles: &[crate::model_import::Role]) -> bool {
    roles == [crate::model_import::Role::Primary]
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Conditions {
    pub required_machine_profile_key: Option<String>,
    pub filament_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_filament_id: Option<String>,
    pub process_profile_key: Option<String>,
    pub bed_type: Option<String>,
    pub brim_enabled: bool,
    pub support_enabled: bool,
    pub support_interface_filament_id: Option<String>,
    #[serde(flatten)]
    pub strength: crate::strength::Strength,
}
impl Conditions {
    pub(crate) fn role_id(&self, role: crate::model_import::Role) -> Result<&str> {
        use crate::model_import::Role;
        match role {
            Role::Primary => self
                .filament_id
                .as_deref()
                .ok_or(Error::Conflict("primaryの材料を設定してください。")),
            Role::Secondary => self
                .secondary_filament_id
                .as_deref()
                .ok_or(Error::Conflict("secondaryの材料を設定してください。")),
        }
    }
    pub(crate) fn interface_id(&self) -> Option<&str> {
        self.support_enabled
            .then(|| {
                self.support_interface_filament_id
                    .as_deref()
                    .or(self.filament_id.as_deref())
            })
            .flatten()
    }
    fn normalize(&mut self, first: crate::model_import::Role) {
        if self.support_enabled && self.support_interface_filament_id.is_none() {
            self.support_interface_filament_id = self.role_id(first).ok().map(str::to_owned);
        }
    }
    pub(crate) fn apply(
        &self,
        process: &mut serde_json::Map<String, serde_json::Value>,
    ) -> Result<()> {
        self.strength.apply(process)?;
        crate::support::configure(
            process,
            self.support_enabled,
            self.interface_id()
                .is_some_and(|id| Some(id) != self.filament_id.as_deref()),
        )?;
        if self.brim_enabled
            && !process
                .get("brim_width")
                .and_then(serde_json::Value::as_str)
                .and_then(|value| value.parse::<f64>().ok())
                .is_some_and(|width| width > 0.0 && width <= 100.0)
        {
            return Err(Error::Invalid(
                "Selected process has no valid brim width; choose another process or disable brim",
            ));
        }
        process.insert(
            "brim_type".into(),
            if self.brim_enabled {
                "outer_only"
            } else {
                "no_brim"
            }
            .into(),
        );
        Ok(())
    }
    fn validate(
        &self,
        c: &rusqlite::Connection,
        profiles: Option<&crate::profiles::Profiles>,
    ) -> Result<()> {
        self.strength.validate()?;
        if self
            .bed_type
            .as_ref()
            .is_some_and(|bed| !crate::profiles::BEDS.contains(&bed.as_str()))
        {
            return Err(Error::Invalid("Unknown bed type"));
        }
        for id in [
            &self.filament_id,
            &self.secondary_filament_id,
            &self.support_interface_filament_id,
        ]
        .into_iter()
        .flatten()
        {
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
                profiles.resolve_process(machine, process, self)?;
            }
            for id in self
                .filament_id
                .as_deref()
                .into_iter()
                .chain(self.interface_id())
            {
                let setting = crate::products::load_setting(c, id, machine)?;
                let filament = crate::database::load_filaments(c)?
                    .into_iter()
                    .find(|f| f.id == id)
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported: Option<Imported>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Imported {
    pub file_name: String,
    pub model_id: String,
    pub selection: crate::model_import::Selection,
}
impl Plate {
    pub(crate) fn roles(&self) -> std::collections::BTreeSet<crate::model_import::Role> {
        self.models
            .iter()
            .flat_map(|m| m.roles.iter().copied())
            .collect()
    }
    pub(crate) fn ensure_printable(&self) -> Result<()> {
        if self.imported.as_ref().is_some_and(|source| {
            source.selection.print_reason.is_some()
                && self.models.iter().any(|model| model.id == source.model_id)
        }) {
            return Err(Error::Conflict(
                "多色・ペイントの印刷は未対応です。保存した元の3MFを取得できます。",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ItemEdit {
    pub id: Option<String>,
    pub name: String,
    pub source: Option<String>,
    pub quantity: u16,
}
#[derive(Deserialize, Serialize, schemars::JsonSchema)]
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
pub(crate) fn validate_items(models: &[ItemEdit]) -> Result<()> {
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
            slice_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        })
    }
    /// Save uploaded STL originals or model references in `SQLite`.
    /// # Errors
    /// Rejects malformed STL, invalid metadata and unavailable storage.
    pub fn save(&self, input: Input) -> Result<Plate> {
        let quantities = vec![1; input.models.len()];
        self.save_upload(input, &quantities, None)
    }
    pub(crate) fn save_upload(
        &self,
        mut input: Input,
        quantities: &[u16],
        original: Option<crate::file_import::Original>,
    ) -> Result<Plate> {
        if quantities.len() != input.models.len() || original.is_some() && input.models.len() != 1 {
            return Err(Error::Invalid("モデルごとの個数を確認してください。"));
        }
        validate_metadata(&input.name)?;
        if input.models.is_empty() || input.models.len() > 64 {
            return Err(Error::Invalid("Select 1–64 STL models"));
        }
        let total: usize = input.models.iter().map(|m| m.data.len()).sum();
        if total > MAX_UPLOAD {
            return Err(Error::Invalid("Models exceed 64 MiB"));
        }
        let mut models = Vec::new();
        for (m, quantity) in input.models.into_iter().zip(quantities) {
            let roles = crate::model_import::roles(&m.data)?;
            let item = ItemEdit {
                id: Some(uuid::Uuid::new_v4().to_string()),
                name: m.name,
                source: m.source,
                quantity: *quantity,
            };
            models.push((item, m.data, roles));
        }
        validate_items(&models.iter().map(|(m, _, _)| m.clone()).collect::<Vec<_>>())?;
        let id = uuid::Uuid::new_v4().to_string();
        let mut c = self.db.connection()?;
        let tx = c.transaction()?;
        input.conditions.normalize(
            models
                .iter()
                .flat_map(|m| m.2.iter().copied())
                .min()
                .expect("validated roles"),
        );
        input.conditions.validate(&tx, self.profiles.as_deref())?;
        tx.execute(
            "INSERT INTO plates(id,name) VALUES (?1,?2)",
            rusqlite::params![id, input.name.trim()],
        )?;
        save_conditions(&tx, &id, &input.conditions)?;
        for (position, (m, data, roles)) in models.iter().enumerate() {
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
            save_roles(&tx, &id, position, roles)?;
        }
        if let Some(original) = original {
            let metadata = Imported {
                file_name: original.file_name,
                model_id: models[0].0.id.clone().expect("assigned id"),
                selection: original.selection,
            };
            let metadata = serde_json::to_string(&metadata).map_err(std::io::Error::other)?;
            tx.execute(
                "INSERT INTO plate_imports(plate_id,metadata_json,original) VALUES (?1,?2,?3)",
                rusqlite::params![id, metadata, original.data],
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
        self.edit_with_roles(id, edit, &std::collections::BTreeMap::new())
    }
    pub(crate) fn edit_with_roles(
        &self,
        id: Option<&str>,
        edit: Edit,
        fetched: &std::collections::BTreeMap<String, Vec<crate::model_import::Role>>,
    ) -> Result<Plate> {
        let Edit {
            mut conditions,
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
                "UPDATE plates SET name=?1,version=version+1 WHERE id=?2 AND version=?3 AND deleted=0",
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
        // Invalidate queue command fences when a waiting plate changes. Active snapshots stay intact.
        tx.execute("UPDATE printers SET queue_generation=queue_generation+1,queue_request=NULL WHERE id IN (SELECT printer_id FROM print_jobs WHERE plate_id=?1 AND state='queued')", [&id])?;
        let mut inputs = Vec::new();
        for m in &models {
            let original = if let Some(item_id) = &m.id {
                let stored: Option<(Option<String>,Option<Vec<u8>>,String)> = tx.query_row(
                    "SELECT model_key,original,roles_json FROM plate_items WHERE id=?1 AND plate_id=?2",
                    rusqlite::params![item_id,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
                let (source, original, roles) =
                    stored.ok_or(Error::Invalid("Item does not belong to this plate"))?;
                if source != m.source {
                    return Err(Error::Invalid("Replace the reference as a new item"));
                }
                (
                    original,
                    serde_json::from_str::<Vec<crate::model_import::Role>>(&roles)
                        .map_err(std::io::Error::other)?,
                )
            } else {
                (None, primary_role())
            };
            inputs.push(original);
        }
        tx.execute("DELETE FROM plate_items WHERE plate_id=?1", [&id])?;
        let mut used_roles = std::collections::BTreeSet::new();
        for (position, (m, (original, previous_roles))) in models.iter().zip(inputs).enumerate() {
            insert_item(&tx, &id, position, m, original.as_deref())?;
            let roles = m
                .source
                .as_ref()
                .and_then(|s| fetched.get(s))
                .unwrap_or(&previous_roles);
            used_roles.extend(roles.iter().copied());
            save_roles(&tx, &id, position, roles)?;
        }
        conditions.normalize(
            *used_roles
                .first()
                .ok_or(Error::Invalid("モデルの材料役割がありません。"))?,
        );
        save_conditions(&tx, &id, &conditions)?;
        let plate = load(&tx, &id)?;
        tx.commit()?;
        Ok(plate)
    }
    /// Copy a saved composition with independent ownership of its originals.
    /// # Errors
    /// Rejects invalid names, missing plates and unavailable storage.
    pub fn duplicate(&self, source: &str, name: &str) -> Result<Plate> {
        valid_id(source)?;
        validate_metadata(name)?;
        let mut c = self.db.connection()?;
        let tx = c.transaction()?;
        if is_deleted(&tx, source)? {
            return Err(Error::NotFound);
        }
        let plate = load(&tx, source)?;
        let id = uuid::Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO plates(id,name) VALUES (?1,?2)",
            rusqlite::params![id, name.trim()],
        )?;
        // Saved conditions are copied verbatim; defaults and current printer availability do not apply.
        save_conditions(&tx, &id, &plate.conditions)?;
        let mut imported = plate.imported;
        let imported_source = imported.as_ref().map(|value| value.model_id.clone());
        if let Some(value) = &mut imported {
            // An original can remain downloadable after its model was removed from the composition.
            value.model_id = uuid::Uuid::new_v4().to_string();
        }
        for model in plate.models {
            let item = uuid::Uuid::new_v4().to_string();
            tx.execute("INSERT INTO plate_items(id,plate_id,position,name,source_kind,model_key,original,quantity,roles_json) SELECT ?1,?2,position,name,source_kind,model_key,original,quantity,roles_json FROM plate_items WHERE id=?3 AND plate_id=?4", rusqlite::params![item,id,model.id,source])?;
            if imported_source.as_deref() == Some(&model.id)
                && let Some(metadata) = &mut imported
            {
                metadata.model_id = item;
            }
        }
        if let Some(metadata) = imported {
            let metadata = serde_json::to_string(&metadata).map_err(std::io::Error::other)?;
            tx.execute("INSERT INTO plate_imports(plate_id,metadata_json,original) SELECT ?1,?2,original FROM plate_imports WHERE plate_id=?3", rusqlite::params![id,metadata,source])?;
        }
        let copy = load(&tx, &id)?;
        tx.commit()?;
        Ok(copy)
    }
    /// Read a saved composition.
    /// # Errors
    /// Rejects invalid IDs, missing plates or unavailable storage.
    pub fn get(&self, id: &str) -> Result<Plate> {
        valid_id(id)?;
        let c = self.db.connection()?;
        if is_deleted(&c, id)? {
            return Err(Error::NotFound);
        }
        load(&c, id)
    }
    /// Hide a plate from new use without deleting originals or existing queue references.
    /// # Errors
    /// Rejects invalid or missing IDs and unavailable storage.
    pub fn delete(&self, id: &str) -> Result<()> {
        valid_id(id)?;
        let mut c = self.db.connection()?;
        let tx = c.transaction()?;
        if tx.execute("UPDATE plates SET deleted=1 WHERE id=?1", [id])? == 0 {
            return Err(Error::NotFound);
        }
        tx.execute("DELETE FROM plate_slices WHERE plate_id=?1", [id])?;
        tx.commit()?;
        Ok(())
    }
    /// Search saved names and model names, ordered by fuzzy relevance.
    /// # Errors
    /// Returns storage errors.
    pub fn list(&self, query: &str) -> Result<Vec<Plate>> {
        let c = self.db.connection()?;
        let ids = c
            .prepare("SELECT id FROM plates WHERE deleted=0")?
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
        self.db.connection()?.query_row("SELECT original FROM plate_items WHERE plate_id=?1 AND id=?2 AND source_kind='upload' AND EXISTS(SELECT 1 FROM plates WHERE id=?1 AND deleted=0)",rusqlite::params![id,item],|r|r.get(0)).optional()?.ok_or(Error::NotFound)
    }
    pub(crate) fn read_import(&self, id: &str) -> Result<Vec<u8>> {
        valid_id(id)?;
        self.db.connection()?.query_row("SELECT original FROM plate_imports WHERE plate_id=?1 AND EXISTS(SELECT 1 FROM plates WHERE id=?1 AND deleted=0)",[id],|r|r.get(0)).optional()?.ok_or(Error::NotFound)
    }
}
pub(crate) fn is_deleted(c: &rusqlite::Connection, id: &str) -> Result<bool> {
    c.query_row("SELECT deleted FROM plates WHERE id=?1", [id], |r| r.get(0))
        .optional()?
        .ok_or(Error::NotFound)
}
pub(crate) fn load(c: &rusqlite::Connection, id: &str) -> Result<Plate> {
    let (name, version, conditions) = c
        .query_row("SELECT name,version,required_machine_profile_key,filament_id,process_profile_key,bed_type,sparse_infill_pattern,sparse_infill_density,wall_loops,brim_enabled,support_enabled,support_interface_filament_id,secondary_filament_id FROM plates WHERE id=?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?, Conditions { required_machine_profile_key:r.get(2)?,filament_id:r.get(3)?,process_profile_key:r.get(4)?,bed_type:r.get(5)?,strength:crate::strength::Strength { sparse_infill_pattern:r.get(6)?,sparse_infill_density:r.get(7)?,wall_loops:r.get(8)? }, brim_enabled:r.get(9)?,support_enabled:r.get(10)?,support_interface_filament_id:r.get(11)?,secondary_filament_id:r.get(12)? }))
        })
        .optional()?
        .ok_or(Error::NotFound)?;
    let models=c.prepare("SELECT id,name,model_key,quantity,roles_json FROM plate_items WHERE plate_id=?1 ORDER BY position")?
        .query_map([id],|r|Ok((Model{id:r.get(0)?,name:r.get(1)?,source:r.get(2)?,quantity:r.get(3)?,roles: Vec::new()},r.get::<_,String>(4)?)))?
        .map(|row| { let (mut model, roles) = row?; model.roles = serde_json::from_str(&roles).map_err(std::io::Error::other)?; Ok(model) }).collect::<Result<Vec<_>>>()?;
    Ok(Plate {
        id: id.into(),
        name,
        version,
        conditions,
        models,
        imported: c
            .query_row(
                "SELECT metadata_json FROM plate_imports WHERE plate_id=?1",
                [id],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map(|json| serde_json::from_str(&json).map_err(std::io::Error::other))
            .transpose()?,
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
    c.execute("UPDATE plates SET required_machine_profile_key=?1,filament_id=?2,process_profile_key=?3,bed_type=?4,sparse_infill_pattern=?6,sparse_infill_density=?7,wall_loops=?8,brim_enabled=?9,support_enabled=?10,support_interface_filament_id=?11,secondary_filament_id=?12 WHERE id=?5", rusqlite::params![v.required_machine_profile_key,v.filament_id,v.process_profile_key,v.bed_type,id,v.strength.sparse_infill_pattern,v.strength.sparse_infill_density,v.strength.wall_loops,v.brim_enabled,v.support_enabled,v.support_interface_filament_id,v.secondary_filament_id])?;
    Ok(())
}
fn save_roles(
    c: &rusqlite::Connection,
    id: &str,
    position: usize,
    roles: &[crate::model_import::Role],
) -> Result<()> {
    c.execute(
        "UPDATE plate_items SET roles_json=?1 WHERE plate_id=?2 AND position=?3",
        rusqlite::params![
            serde_json::to_string(roles).map_err(std::io::Error::other)?,
            id,
            i64::try_from(position).expect("64 items")
        ],
    )?;
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
    #[test]
    fn duplicate_preserves_references_order_conditions_and_independent_versions() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let mut input = edit("天馬ルームケース: base前", 10);
        input.conditions.bed_type = Some("Textured PEI Plate".into());
        input.conditions.strength.wall_loops = Some(4);
        input.conditions.strength.sparse_infill_density = Some(30.0);
        input.conditions.brim_enabled = true;
        input.conditions.support_enabled = true;
        input.models.push(ItemEdit {
            name: "second.stl".into(),
            source: Some("second.stl".into()),
            quantity: 3,
            id: None,
        });
        let source = store.edit(None, input).unwrap();
        let copy = store
            .duplicate(&source.id, "  天馬ルームケース: base後  ")
            .unwrap();
        assert_eq!(copy.name, "天馬ルームケース: base後");
        assert_ne!(copy.id, source.id);
        assert_eq!(copy.version, 1);
        assert_eq!(copy.conditions, source.conditions);
        for (old, new) in source.models.iter().zip(&copy.models) {
            assert_ne!(old.id, new.id);
            assert_eq!(
                (&old.name, &old.source, old.quantity),
                (&new.name, &new.source, new.quantity)
            );
        }
        assert_eq!(copy.models.len(), 2);
        let mut update = edit("changed copy", 5);
        update.version = Some(copy.version);
        store.edit(Some(&copy.id), update).unwrap();
        assert_eq!(store.get(&source.id).unwrap(), source);
        store.delete(&source.id).unwrap();
        assert_eq!(store.get(&copy.id).unwrap().version, 2);
        let blank = store.edit(None, edit("nullable", 1)).unwrap();
        let copied_blank = store.duplicate(&blank.id, "nullable copy").unwrap();
        assert_eq!(copied_blank.conditions, Conditions::default());
        assert_eq!(
            Store::open(root.path())
                .unwrap()
                .get(&copied_blank.id)
                .unwrap(),
            copied_blank
        );
    }

    #[test]
    fn duplicate_owns_stl_and_3mf_originals_and_remaps_imported_model() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let stl = include_bytes!("../tests/fixtures/triangle.stl").to_vec();
        for imported in [false, true] {
            let selection = crate::model_import::Selection {
                name: "Plate 2".into(),
                root_model: "3D/3dmodel.model".into(),
                plate_id: Some("2".into()),
                items: vec![crate::model_import::SourceItem {
                    build_index: 1,
                    object_id: 42,
                    instance_id: 3,
                }],
                print_reason: Some("painted model".into()),
                roles: Vec::new(),
            };
            let original = b"source 3mf including untouched color and object metadata".to_vec();
            let source = store
                .save_upload(
                    Input {
                        name: "uploaded".into(),
                        conditions: Conditions::default(),
                        models: vec![ModelInput {
                            name: "bin.stl".into(),
                            source: None,
                            data: stl.clone(),
                        }],
                    },
                    &[10],
                    imported.then(|| crate::file_import::Original {
                        file_name: "bins.3mf".into(),
                        data: original.clone(),
                        selection: selection.clone(),
                    }),
                )
                .unwrap();
            let copy = store.duplicate(&source.id, "uploaded copy").unwrap();
            let model = &copy.models[0];
            assert_ne!(model.id, source.models[0].id);
            assert_eq!(model.quantity, 10);
            assert_eq!(store.read_file(&copy.id, &model.id).unwrap(), stl);
            assert!(store.read_file(&copy.id, &source.models[0].id).is_err());
            if imported {
                assert_eq!(
                    copy.imported,
                    Some(Imported {
                        file_name: "bins.3mf".into(),
                        model_id: model.id.clone(),
                        selection
                    })
                );
                assert_eq!(store.read_import(&copy.id).unwrap(), original);
                assert!(copy.ensure_printable().is_err());
            }
            store.delete(&source.id).unwrap();
            let restored = Store::open(root.path()).unwrap();
            assert_eq!(restored.get(&copy.id).unwrap(), copy);
            assert_eq!(restored.read_file(&copy.id, &model.id).unwrap(), stl);
            if imported {
                assert_eq!(restored.read_import(&copy.id).unwrap(), original);
            }
        }
    }

    #[test]
    fn material_roles_and_nullable_secondary_survive_edit_duplicate_and_reopen() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let edit: Edit = serde_json::from_value(serde_json::json!({
            "name":"Two roles", "models":[{"name":"sign.3mf","source":"sign.3mf","quantity":2}],
            "conditions":{"filament_id":null,"secondary_filament_id":null}
        }))
        .unwrap();
        let roles = std::collections::BTreeMap::from([(
            "sign.3mf".to_owned(),
            vec![
                crate::model_import::Role::Primary,
                crate::model_import::Role::Secondary,
            ],
        )]);
        let plate = store.edit_with_roles(None, edit, &roles).unwrap();
        assert_eq!(plate.models[0].roles, roles["sign.3mf"]);
        assert!(plate.conditions.secondary_filament_id.is_none());
        let copy = store.duplicate(&plate.id, "copy").unwrap();
        assert_eq!(copy.models[0].roles, plate.models[0].roles);
        assert_eq!(copy.models[0].quantity, 2);
        drop(store);
        let reopened = Store::open(root.path()).unwrap();
        assert_eq!(
            serde_json::to_value(reopened.get(&copy.id).unwrap()).unwrap(),
            serde_json::to_value(copy).unwrap()
        );
        let mut value = serde_json::to_value(&plate).unwrap();
        value.as_object_mut().unwrap().remove("id");
        value["models"][0].as_object_mut().unwrap().remove("roles");
        let mut edit: Edit = serde_json::from_value(value).unwrap();
        edit.conditions.secondary_filament_id = Some("missing".into());
        assert!(
            reopened
                .edit_with_roles(Some(&plate.id), edit, &roles)
                .is_err()
        );
        assert_eq!(reopened.get(&plate.id).unwrap().version, plate.version);
    }

    #[test]
    fn interface_default_follows_first_used_role_without_changing_explicit_assignment() {
        use crate::model_import::Role;
        let mut conditions: Conditions = serde_json::from_value(serde_json::json!({"filament_id":"unused-primary","secondary_filament_id":"body","support_enabled":true})).unwrap();
        conditions.normalize(Role::Secondary);
        assert_eq!(conditions.interface_id(), Some("body"));
        conditions.support_interface_filament_id = Some("explicit".into());
        conditions.normalize(Role::Secondary);
        assert_eq!(conditions.interface_id(), Some("explicit"));
    }

    #[test]
    fn duplicate_rejects_bad_sources_and_rolls_back_partial_copies() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let source = store.edit(None, edit("source", 10)).unwrap();
        for name in ["", " ", "bad\nname", &"あ".repeat(86)] {
            assert!(store.duplicate(&source.id, name).is_err());
        }
        for id in ["../outside", &uuid::Uuid::new_v4().to_string()] {
            assert!(store.duplicate(id, "copy").is_err());
        }
        store.db.connection().unwrap().execute_batch("CREATE TRIGGER fail_copy BEFORE INSERT ON plate_items BEGIN SELECT RAISE(ABORT, 'unavailable storage'); END;").unwrap();
        assert!(store.duplicate(&source.id, "failed copy").is_err());
        assert_eq!(store.list("").unwrap(), vec![source.clone()]);
        store
            .db
            .connection()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_copy")
            .unwrap();
        store.delete(&source.id).unwrap();
        assert!(matches!(
            store.duplicate(&source.id, "deleted copy"),
            Err(Error::NotFound)
        ));
        assert!(store.list("").unwrap().is_empty());
    }
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
    fn strength_conditions_round_trip_and_reject_unknown_input() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let mut body = serde_json::to_value(edit("Strength", 1)).unwrap();
        body["conditions"] = serde_json::json!({"sparse_infill_pattern":"gyroid","sparse_infill_density":22.5,"wall_loops":4});
        let saved = store
            .edit(None, serde_json::from_value(body.clone()).unwrap())
            .unwrap();
        let loaded = load(&store.db.connection().unwrap(), &saved.id).unwrap();
        let values = serde_json::to_value(loaded.conditions).unwrap();
        for key in [
            "sparse_infill_pattern",
            "sparse_infill_density",
            "wall_loops",
        ] {
            assert_eq!(values[key], body["conditions"][key]);
        }
        body["conditions"]["unknown"] = true.into();
        assert!(serde_json::from_value::<Edit>(body).is_err());
    }
    #[test]
    fn support_is_opt_in_and_uses_distinct_ids_not_material_names() {
        use serde_json::json;
        for (input, enabled, index) in [
            (json!({}), "0", None),
            (
                json!({"support_enabled":true,"filament_id":"main"}),
                "1",
                Some("1"),
            ),
            (
                json!({"support_enabled":true,"filament_id":"main","support_interface_filament_id":"main"}),
                "1",
                Some("1"),
            ),
            (
                json!({"support_enabled":true,"filament_id":"main","support_interface_filament_id":"other"}),
                "1",
                Some("2"),
            ),
        ] {
            let conditions: Conditions = serde_json::from_value(input).unwrap();
            let mut process = json!({"enable_support":"1"}).as_object().unwrap().clone();
            conditions.apply(&mut process).unwrap();
            assert_eq!(process["enable_support"], enabled);
            if let Some(index) = index {
                assert_eq!(process["support_filament"], "1");
                assert_eq!(process["support_interface_filament"], index);
            }
        }
        for invalid in [
            json!({"support_enabled":null}),
            json!({"support_enabled":"true"}),
        ] {
            assert!(serde_json::from_value::<Conditions>(invalid).is_err());
        }
    }

    #[test]
    fn brim_is_an_explicit_plate_opt_in_and_keeps_process_dimensions() {
        use serde_json::json;
        let inherited = json!({"brim_type":"auto_brim","brim_width":"5","brim_object_gap":"0.1",
            "skirt_loops":"2","raft_layers":"3","enable_support":"0","enable_prime_tower":"1","wall_loops":"2"});
        for (input, expected) in [
            (json!({}), "no_brim"),
            (json!({"brim_enabled":true}), "outer_only"),
            (json!({"brim_enabled":false}), "no_brim"),
        ] {
            let conditions: Conditions = serde_json::from_value(input).unwrap();
            let mut process = inherited.as_object().unwrap().clone();
            conditions.apply(&mut process).unwrap();
            assert_eq!(process["brim_type"], expected);
            for key in [
                "brim_width",
                "brim_object_gap",
                "skirt_loops",
                "raft_layers",
                "enable_support",
                "enable_prime_tower",
                "wall_loops",
            ] {
                assert_eq!(process[key], inherited[key]);
            }
        }
    }

    #[test]
    fn brim_round_trips_off_on_off_and_old_inputs_stay_off() {
        use serde_json::json;
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let first = store.edit(None, edit("Brim", 1)).unwrap();
        assert_eq!(
            serde_json::to_value(&first).unwrap()["conditions"]["brim_enabled"],
            false
        );
        let mut value = serde_json::to_value(edit("Brim", 1)).unwrap();
        for enabled in [true, false] {
            let current = store.get(&first.id).unwrap();
            value["version"] = json!(current.version);
            value["conditions"]["brim_enabled"] = json!(enabled);
            store
                .edit(
                    Some(&first.id),
                    serde_json::from_value(value.clone()).unwrap(),
                )
                .unwrap();
            let reread = Store::open(root.path()).unwrap().get(&first.id).unwrap();
            assert_eq!(
                serde_json::to_value(reread).unwrap()["conditions"]["brim_enabled"],
                enabled
            );
        }
        value["conditions"]["brim_enabled"] = json!("true");
        assert!(serde_json::from_value::<Edit>(value).is_err());
    }
    #[test]
    fn brim_on_requires_a_positive_inherited_width() {
        use serde_json::json;
        let on: Conditions = serde_json::from_value(json!({"brim_enabled":true})).unwrap();
        for width in [
            json!("0"),
            json!("-1"),
            json!("NaN"),
            json!("101"),
            serde_json::Value::Null,
        ] {
            let mut profile = json!({"brim_width":width}).as_object().unwrap().clone();
            assert!(on.apply(&mut profile).is_err());
            Conditions::default().apply(&mut profile).unwrap();
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
        assert_eq!(value["conditions"].as_object().unwrap().len(), 10);
        assert!(
            value["conditions"]
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
