use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::{self, Cursor, Read, Write},
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub enum Error {
    Invalid(&'static str),
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

#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Model {
    pub name: String,
    pub path: String,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Plate {
    pub format_version: u8,
    pub id: String,
    pub revision: String,
    pub name: String,
    pub models: Vec<Model>,
    pub settings: Value,
    pub project: Option<String>,
    pub print: Option<String>,
}

pub struct ModelInput {
    pub name: String,
    pub source: Option<String>,
    pub data: Vec<u8>,
}
pub struct Input {
    pub name: String,
    pub models: Vec<ModelInput>,
    pub settings: Value,
}

impl Store {
    /// Opens the application-owned storage directory.
    ///
    /// # Errors
    /// Returns an error if the directory cannot be created or resolved.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        fs::create_dir_all(root.as_ref())?;
        Ok(Self {
            root: fs::canonicalize(root)?,
        })
    }

    /// Saves a complete revision, retaining the last readable revision on failure.
    ///
    /// # Errors
    /// Rejects invalid input, unknown IDs, and inaccessible storage.
    pub fn save(&self, id: Option<&str>, input: Input) -> Result<Plate> {
        validate(&input)?;
        let id = if let Some(id) = id {
            self.get(id)?;
            id.to_owned()
        } else {
            uuid::Uuid::new_v4().to_string()
        };
        let plate_dir = self.root.join(&id);
        if !plate_dir.exists() {
            fs::create_dir(&plate_dir)?;
        }
        self.checked(&id)?;
        let revisions = plate_dir.join("revisions");
        if !revisions.exists() {
            fs::create_dir(&revisions)?;
        }
        self.checked(&format!("{id}/revisions"))?;
        let revision = uuid::Uuid::new_v4().to_string();
        let stage = tempfile::Builder::new()
            .prefix(".pending-")
            .tempdir_in(&revisions)?;
        let mut models = Vec::new();
        for (index, model) in input.models.into_iter().enumerate() {
            let filename = format!("{index}.stl");
            let mut file = fs::File::create(stage.path().join(&filename))?;
            file.write_all(&model.data)?;
            file.sync_all()?;
            models.push(Model {
                name: model.name,
                source: model.source,
                path: format!("revisions/{revision}/{filename}"),
            });
        }
        fs::File::open(stage.path())?.sync_all()?;
        fs::rename(stage.path(), revisions.join(&revision))?;
        fs::File::open(&revisions)?.sync_all()?;
        let plate = Plate {
            format_version: 1,
            id,
            revision,
            name: input.name.trim().into(),
            models,
            settings: input.settings,
            project: None,
            print: None,
        };
        let mut metadata = tempfile::NamedTempFile::new_in(&plate_dir)?;
        serde_json::to_writer(&mut metadata, &plate).map_err(io::Error::other)?;
        metadata.as_file().sync_all()?;
        // Models are immutable. Only this reference is atomically replaced.
        metadata
            .persist(plate_dir.join("plate.json"))
            .map_err(|e| e.error)?;
        fs::File::open(&plate_dir)?.sync_all()?;
        fs::File::open(&self.root)?.sync_all()?;
        Ok(plate)
    }

    /// Reads and validates the current metadata without trusting its paths.
    ///
    /// # Errors
    /// Rejects unknown IDs, corrupt metadata, and symlinks in stored paths.
    pub fn get(&self, id: &str) -> Result<Plate> {
        valid_id(id)?;
        let data = read_limited(&self.checked(&format!("{id}/plate.json"))?, MAX_METADATA)?;
        let plate: Plate = serde_json::from_slice(&data).map_err(io::Error::other)?;
        valid_id(&plate.revision)?;
        if plate.format_version != 1
            || plate.id != id
            || plate.models.is_empty()
            || plate.models.len() > 64
        {
            return Err(Error::Invalid("Invalid saved plate metadata"));
        }
        for (index, model) in plate.models.iter().enumerate() {
            if model.path != format!("revisions/{}/{index}.stl", plate.revision) {
                return Err(Error::Invalid("Invalid saved model path"));
            }
        }
        for (path, name) in [
            (&plate.project, "project.3mf"),
            (&plate.print, "print.gcode.3mf"),
        ] {
            if path
                .as_ref()
                .is_some_and(|path| *path != format!("revisions/{}/{name}", plate.revision))
            {
                return Err(Error::Invalid("Invalid saved artifact path"));
            }
        }
        Ok(plate)
    }

    /// Lists readable plates, with fuzzy matches ordered before weaker matches.
    ///
    /// # Errors
    /// Fails if the root cannot be listed. Individual corrupt entries are logged and skipped.
    pub fn list(&self, query: &str) -> Result<Vec<Plate>> {
        let query = query.trim();
        let mut matches = Vec::new();
        // ponytail: scan metadata per request; add an index only when this becomes slow.
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let id = entry.file_name().to_string_lossy().into_owned();
            if valid_id(&id).is_err() {
                continue;
            }
            match self.get(&id) {
                Ok(plate) => {
                    let score = std::iter::once(plate.name.as_str())
                        .chain(plate.models.iter().map(|m| m.name.as_str()))
                        .filter_map(|text| crate::search::score(query, text))
                        .max();
                    if let Some(score) = score {
                        matches.push((score, plate));
                    }
                }
                Err(Error::NotFound) => {} // An incomplete first save is not published.
                Err(error) => tracing::warn!(%id, %error, "skipping unreadable plate"),
            }
        }
        matches.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| a.1.name.cmp(&b.1.name))
                .then_with(|| a.1.id.cmp(&b.1.id))
        });
        Ok(matches.into_iter().map(|(_, plate)| plate).collect())
    }

    /// Reads a file explicitly referenced by the current plate.
    ///
    /// # Errors
    /// Rejects traversal, unlisted files, symlinks, oversized or missing files.
    pub fn read_file(&self, id: &str, path: &str) -> Result<Vec<u8>> {
        let plate = self.get(id)?;
        if !plate.models.iter().any(|model| model.path == path)
            && plate.project.as_deref() != Some(path)
            && plate.print.as_deref() != Some(path)
        {
            return Err(Error::NotFound);
        }
        read_limited(&self.checked(&format!("{id}/{path}"))?, MAX_UPLOAD)
    }

    fn checked(&self, relative: &str) -> Result<PathBuf> {
        if !relative_path(relative) {
            return Err(Error::Invalid("Invalid storage path"));
        }
        let mut path = self.root.clone();
        for part in relative.split('/') {
            path.push(part);
            if fs::symlink_metadata(&path)?.file_type().is_symlink() {
                return Err(Error::Invalid("Storage symlinks are not supported"));
            }
        }
        Ok(path)
    }
}

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

fn validate(input: &Input) -> Result<()> {
    if input.name.trim().is_empty()
        || input.name.len() > 256
        || input.name.chars().any(char::is_control)
    {
        return Err(Error::Invalid(
            "Name must contain 1–256 bytes without control characters",
        ));
    }
    if !input.settings.is_object() || input.settings.to_string().len() > 16 * 1024 {
        return Err(Error::Invalid(
            "Settings must be a JSON object no larger than 16 KiB",
        ));
    }
    if input.models.is_empty() || input.models.len() > 64 {
        return Err(Error::Invalid("A plate must contain 1–64 STL models"));
    }
    let mut total = 0usize;
    for model in &input.models {
        if !relative_path(&model.name)
            || model.name.len() > 1024
            || !model.name.to_ascii_lowercase().ends_with(".stl")
        {
            return Err(Error::Invalid("Each model needs a relative STL filename"));
        }
        if model
            .source
            .as_ref()
            .is_some_and(|s| s.len() > 2048 || s.chars().any(char::is_control))
        {
            return Err(Error::Invalid("Invalid model source"));
        }
        total = total.saturating_add(model.data.len());
        if total > MAX_UPLOAD {
            return Err(Error::Invalid("Models exceed 64 MiB"));
        }
        let mut cursor = Cursor::new(&model.data);
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
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    fn input(name: &str) -> Input {
        Input {
            name: name.into(),
            models: vec![ModelInput {
                name: "parts/box.stl".into(),
                source: Some("parts/box.stl".into()),
                data: include_bytes!("../tests/fixtures/triangle.stl").to_vec(),
            }],
            settings: json!({"material":"PLA"}),
        }
    }

    #[test]
    fn checks_binary_stl_finite_coordinates_and_upload_limits() {
        let mut candidate = input("binary");
        let mut data = vec![0u8; 134];
        data[..5].copy_from_slice(b"solid");
        data[80..84].copy_from_slice(&1u32.to_le_bytes());
        let values = [0f32, 0., 1., 0., 0., 0., 1., 0., 0., 0., 1., 0.];
        for (index, value) in values.into_iter().enumerate() {
            data[84 + index * 4..88 + index * 4].copy_from_slice(&value.to_le_bytes());
        }
        candidate.models[0].data = data.clone();
        assert!(validate(&candidate).is_ok());
        data[96..100].copy_from_slice(&f32::NAN.to_le_bytes());
        candidate.models[0].data = data;
        assert!(validate(&candidate).is_err());
        candidate.models[0].data = b"solid empty\nendsolid empty\n".to_vec();
        assert!(validate(&candidate).is_err());
        candidate.models[0].data = vec![0; MAX_UPLOAD + 1];
        assert!(validate(&candidate).is_err());
        candidate = input("source");
        candidate.models[0].source = Some("bad\nsource".into());
        assert!(validate(&candidate).is_err());
        candidate = input("settings");
        candidate.settings = json!({"oversized":"x".repeat(16 * 1024)});
        assert!(validate(&candidate).is_err());
    }

    #[test]
    fn saves_lists_searches_and_reopens_named_plates() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let first = store.save(None, input("机の箱")).unwrap();
        let second = store.save(None, input("Spare holder")).unwrap();
        assert_ne!(first.id, second.id);
        let store = Store::open(root.path()).unwrap();
        assert_eq!(store.get(&first.id).unwrap(), first);
        assert_eq!(store.list("").unwrap().len(), 2);
        assert_eq!(store.list("机箱").unwrap(), vec![first.clone()]);
        assert_eq!(store.list("prt/bx").unwrap().len(), 2);
        assert!(store.list("missing").unwrap().is_empty());
        assert_eq!(
            store.read_file(&first.id, &first.models[0].path).unwrap(),
            input("x").models[0].data
        );
    }

    #[test]
    fn invalid_inputs_and_failed_replacement_preserve_the_last_revision() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let before = store.save(None, input("original")).unwrap();
        for name in ["", "  ", "bad\nname"] {
            assert!(store.save(Some(&before.id), input(name)).is_err());
        }
        for filename in [
            "../outside.stl",
            "/etc/model.stl",
            "folder\\x.stl",
            "bad.txt",
        ] {
            let mut candidate = input("invalid");
            candidate.models[0].name = filename.into();
            assert!(store.save(Some(&before.id), candidate).is_err());
        }
        let mut candidate = input("broken");
        candidate.models[0].data = b"this is not an STL".to_vec();
        assert!(store.save(Some(&before.id), candidate).is_err());
        let mut candidate = input("empty");
        candidate.models.clear();
        assert!(store.save(Some(&before.id), candidate).is_err());
        let mut candidate = input("settings");
        candidate.settings = json!([]);
        assert!(store.save(Some(&before.id), candidate).is_err());
        // The destination is unusable, even for a root test runner.
        let revisions = root.path().join(&before.id).join("revisions");
        let backup = root.path().join("revisions-backup");
        fs::rename(&revisions, &backup).unwrap();
        fs::write(&revisions, "blocked").unwrap();
        assert!(store.save(Some(&before.id), input("replacement")).is_err());
        fs::remove_file(&revisions).unwrap();
        fs::rename(backup, revisions).unwrap();
        assert_eq!(store.get(&before.id).unwrap(), before);
        let after = store.save(Some(&before.id), input("renamed")).unwrap();
        assert_eq!(after.id, before.id);
        assert_ne!(after.revision, before.revision);
        assert_eq!(store.get(&before.id).unwrap(), after);
    }

    #[test]
    fn traversal_corrupt_metadata_and_symlinks_cannot_escape_the_store() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let plate = store.save(None, input("safe")).unwrap();
        for id in ["../outside", "/etc", ".", "x/y"] {
            assert!(store.get(id).is_err());
            assert!(store.save(Some(id), input("bad")).is_err());
        }
        assert!(store.read_file(&plate.id, "../../etc/passwd").is_err());
        let file = root.path().join(&plate.id).join(&plate.models[0].path);
        fs::write(outside.path().join("secret"), "private").unwrap();
        fs::remove_file(&file).unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), &file).unwrap();
        assert!(store.read_file(&plate.id, &plate.models[0].path).is_err());
        let metadata = root.path().join(&plate.id).join("plate.json");
        let mut tampered = plate.clone();
        tampered.models[0].path = "../../secret".into();
        fs::write(&metadata, serde_json::to_vec(&tampered).unwrap()).unwrap();
        assert!(store.get(&plate.id).is_err());
        fs::write(&metadata, "broken json").unwrap();
        assert!(store.get(&plate.id).is_err());
        assert!(store.list("").unwrap().is_empty());
        let other_id = uuid::Uuid::new_v4().to_string();
        std::os::unix::fs::symlink(outside.path(), root.path().join(&other_id)).unwrap();
        assert!(store.get(&other_id).is_err());
        assert!(store.save(Some(&other_id), input("bad")).is_err());
        assert_eq!(fs::read(outside.path().join("secret")).unwrap(), b"private");
    }
}
