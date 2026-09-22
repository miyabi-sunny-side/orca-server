use crate::plates::{Error, Result};
use roxmltree::{Document, Node};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read},
};

const CORE: &str = "http://schemas.microsoft.com/3dmanufacturing/core/2015/02";
const PRODUCTION: &str = "http://schemas.microsoft.com/3dmanufacturing/production/2015/06";
const MAX_FACES: usize = 1_000_000;

fn invalid() -> Error {
    Error::Invalid(
        "3MFの形状または部品参照が不正です。モデルを含むファイルを書き出し直してください。",
    )
}

#[derive(Clone)]
struct Reference {
    path: String,
    id: u32,
    transform: Transform,
    extruder: Option<String>,
}
struct Mesh {
    vertices: Vec<[f64; 3]>,
    faces: Vec<[usize; 3]>,
    materials: BTreeSet<String>,
    painted: bool,
}
struct Object {
    mesh: Option<Mesh>,
    components: Vec<Reference>,
    extruder: Option<String>,
}
struct ModelFile {
    unit: f64,
    objects: BTreeMap<u32, Object>,
    build: Vec<Reference>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SourceItem {
    pub build_index: usize,
    pub object_id: u32,
    pub instance_id: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Selection {
    pub name: String,
    pub root_model: String,
    pub plate_id: Option<String>,
    pub items: Vec<SourceItem>,
    pub print_reason: Option<String>,
}

pub(crate) struct Package {
    pub plates: Vec<Selection>,
    files: BTreeMap<String, ModelFile>,
    root: String,
}

impl Package {
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let entries = archive_entries(bytes)?;
        let relations = xml(entries.get("_rels/.rels").ok_or_else(invalid)?)?;
        let starts = relations
            .descendants()
            .filter(|n| {
                n.has_tag_name("Relationship")
                    && n.attribute("Type").is_some_and(|v| v.ends_with("/3dmodel"))
            })
            .collect::<Vec<_>>();
        if starts.len() != 1 || starts[0].attribute("TargetMode") == Some("External") {
            return Err(invalid());
        }
        let root = part_path("", starts[0].attribute("Target").ok_or_else(invalid)?)?;
        let mut files = BTreeMap::new();
        for (name, data) in &entries {
            if name.to_ascii_lowercase().ends_with(".model") {
                files.insert(name.clone(), model_file(name, data)?);
            }
        }
        if let Some(settings) = entries.get("Metadata/model_settings.config") {
            apply_settings(&mut files, &root, settings)?;
        }
        let model = files.get(&root).ok_or_else(invalid)?;
        if model.build.is_empty() || model.build.len() > 64 {
            return Err(invalid());
        }
        let mut occurrences = BTreeMap::new();
        let items = model
            .build
            .iter()
            .enumerate()
            .map(|(build_index, r)| {
                let instance_id = occurrences.entry(r.id).or_insert(0);
                let item = SourceItem {
                    build_index,
                    object_id: r.id,
                    instance_id: *instance_id,
                };
                *instance_id += 1;
                item
            })
            .collect::<Vec<_>>();
        let plates = selections(
            &root,
            &items,
            entries
                .get("Metadata/model_settings.config")
                .map(Vec::as_slice),
        )?;
        let mut package = Self {
            plates,
            files,
            root,
        };
        for index in 0..package.plates.len() {
            let parts = package.parts(index)?;
            let mut materials = BTreeSet::new();
            let mut painted = false;
            for part in parts {
                painted |= part.mesh.painted;
                if part.mesh.materials.is_empty() {
                    materials.insert(part.extruder.unwrap_or("default").to_owned());
                } else {
                    materials.extend(part.mesh.materials.iter().cloned());
                }
            }
            if painted || materials.len() > 1 {
                package.plates[index].print_reason = Some(
                    "多色・ペイントの印刷は未対応です。元の3MFはそのまま保存できます。".into(),
                );
            }
        }
        Ok(package)
    }
    fn parts(&self, plate: usize) -> Result<Vec<PlacedMesh<'_>>> {
        let selected = self.plates.get(plate).ok_or_else(invalid)?;
        let root = &self.files[&self.root];
        let mut parts = Vec::new();
        let mut visited = 0;
        for item in &selected.items {
            self.walk(
                &root.build[item.build_index],
                Transform::scale(root.unit),
                root.unit,
                None,
                &mut Vec::new(),
                &mut parts,
                &mut visited,
            )?;
        }
        let count: usize = parts.iter().map(|p| p.mesh.faces.len()).sum();
        if count == 0 || count > MAX_FACES {
            return Err(Error::Invalid(
                "展開した3MFの面数が上限を超えているか、形状がありません。モデルを減らして書き出し直してください。",
            ));
        }
        Ok(parts)
    }
    #[allow(clippy::cast_possible_truncation)] // Binary STL stores f32; range and finiteness are checked before conversion.
    pub fn mesh(&self, plate: usize) -> Result<Vec<u8>> {
        let mut triangles = Vec::new();
        for part in self.parts(plate)? {
            for face in &part.mesh.faces {
                let mut points = face.map(|i| part.transform.apply(part.mesh.vertices[i]));
                if points
                    .iter()
                    .flatten()
                    .any(|p| !p.is_finite() || p.abs() > f64::from(f32::MAX))
                {
                    return Err(invalid());
                }
                if part.transform.mirrored() {
                    points.swap(1, 2);
                }
                // Compute from the coordinates actually stored in STL, including reflected winding.
                let points = points.map(|p| p.map(|v| f64::from(v as f32)));
                let a: [f64; 3] = std::array::from_fn(|i| points[1][i] - points[0][i]);
                let b: [f64; 3] = std::array::from_fn(|i| points[2][i] - points[0][i]);
                let normal = [
                    a[1] * b[2] - a[2] * b[1],
                    a[2] * b[0] - a[0] * b[2],
                    a[0] * b[1] - a[1] * b[0],
                ];
                let length = normal[0].hypot(normal[1]).hypot(normal[2]);
                if !length.is_finite() || length == 0.0 {
                    return Err(invalid());
                }
                triangles.push(stl_io::Triangle {
                    normal: stl_io::Normal::new(normal.map(|v| (v / length) as f32)),
                    vertices: points.map(|p| stl_io::Vertex::new(p.map(|v| v as f32))),
                });
            }
        }
        let mut bytes = Vec::new();
        stl_io::write_stl(&mut bytes, triangles.iter())?;
        crate::plates::validate_stl(&bytes)?;
        Ok(bytes)
    }
    #[allow(clippy::too_many_arguments)] // Each recursive edge carries its transform, material and traversal budget.
    fn walk<'a>(
        &'a self,
        reference: &'a Reference,
        outer: Transform,
        parent_unit: f64,
        extruder: Option<&'a str>,
        stack: &mut Vec<(String, u32)>,
        parts: &mut Vec<PlacedMesh<'a>>,
        visited: &mut usize,
    ) -> Result<()> {
        *visited += 1;
        let key = (reference.path.clone(), reference.id);
        if stack.len() >= 32 || *visited > 4096 || stack.contains(&key) {
            return Err(invalid());
        }
        let file = self.files.get(&reference.path).ok_or_else(invalid)?;
        let object = file.objects.get(&reference.id).ok_or_else(invalid)?;
        let transform = Transform::scale(file.unit / parent_unit)
            .then(reference.transform)
            .then(outer);
        let extruder = reference
            .extruder
            .as_deref()
            .or(object.extruder.as_deref())
            .or(extruder);
        stack.push(key);
        if let Some(mesh) = &object.mesh {
            parts.push(PlacedMesh {
                mesh,
                transform,
                extruder,
            });
        }
        for child in &object.components {
            self.walk(child, transform, file.unit, extruder, stack, parts, visited)?;
        }
        stack.pop();
        Ok(())
    }
}
struct PlacedMesh<'a> {
    mesh: &'a Mesh,
    transform: Transform,
    extruder: Option<&'a str>,
}

fn part_path(base: &str, target: &str) -> Result<String> {
    if target.is_empty()
        || target.contains(['\\', ':', '?', '#'])
        || target.chars().any(char::is_control)
    {
        return Err(invalid());
    }
    let base = reqwest::Url::from_file_path(format!("/{base}")).map_err(|()| invalid())?;
    let url = base.join(target).map_err(|_| invalid())?;
    let path = url.to_file_path().map_err(|()| invalid())?;
    let path = path.to_str().ok_or_else(invalid)?.trim_start_matches('/');
    if path.is_empty() || path.contains('\\') {
        return Err(invalid());
    }
    Ok(path.to_owned())
}

fn archive_entries(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>> {
    if bytes.len() > crate::plates::MAX_UPLOAD {
        return Err(Error::Invalid("ファイルは64 MiB以内で選んでください。"));
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| invalid())?;
    if archive.len() > 512 {
        return Err(Error::Invalid("3MF内のファイル数が上限を超えています。"));
    }
    let mut total = 0_u64;
    let mut names = BTreeSet::new();
    let mut entries = BTreeMap::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|_| invalid())?;
        let name = file.name().to_owned();
        if name.len() > 1024
            || name.starts_with('/')
            || name.contains(['\\', ':'])
            || name.chars().any(char::is_control)
            || name.split('/').any(|p| matches!(p, "." | ".."))
            || !names.insert(name.clone())
        {
            return Err(invalid());
        }
        if name == "Metadata/Slic3r_PE_model.config" {
            return Err(Error::Invalid(
                "この3MFの旧形式の部品設定には未対応です。OrcaSlicerで形状を確認してプロジェクトを書き出し直してください。",
            ));
        }
        total = total.checked_add(file.size()).ok_or_else(invalid)?;
        if total > 128 * 1024 * 1024 || file.size() > 64 * 1024 * 1024 {
            return Err(Error::Invalid(
                "3MFの展開後サイズが上限を超えています。モデルを減らしてください。",
            ));
        }
        if name.to_ascii_lowercase().ends_with(".model")
            || name == "_rels/.rels"
            || name == "Metadata/model_settings.config"
        {
            let mut data = Vec::new();
            file.by_ref()
                .take(64 * 1024 * 1024 + 1)
                .read_to_end(&mut data)
                .map_err(|_| invalid())?;
            if data.len() > 64 * 1024 * 1024 {
                return Err(invalid());
            }
            entries.insert(name, data);
        }
    }
    Ok(entries)
}

fn xml(bytes: &[u8]) -> Result<Document<'_>> {
    Document::parse_with_options(
        std::str::from_utf8(bytes).map_err(|_| invalid())?,
        roxmltree::ParsingOptions {
            nodes_limit: 3_000_000,
            ..Default::default()
        },
    )
    .map_err(|_| invalid())
}
fn number<T: std::str::FromStr>(node: Node<'_, '_>, attr: &str) -> Result<T> {
    node.attribute(attr)
        .ok_or_else(invalid)?
        .parse()
        .map_err(|_| invalid())
}
fn reference(path: &str, node: Node<'_, '_>) -> Result<Reference> {
    Ok(Reference {
        path: node
            .attribute((PRODUCTION, "path"))
            .map_or_else(|| Ok(path.to_owned()), |target| part_path(path, target))?,
        id: number(node, "objectid")?,
        transform: Transform::parse(node.attribute("transform"))?,
        extruder: None,
    })
}
fn model_file(path: &str, bytes: &[u8]) -> Result<ModelFile> {
    let doc = xml(bytes)?;
    let root = doc.root_element();
    if !root.has_tag_name((CORE, "model")) {
        return Err(invalid());
    }
    for prefix in root
        .attribute("requiredextensions")
        .unwrap_or("")
        .split_whitespace()
    {
        if !matches!(
            root.lookup_namespace_uri(Some(prefix)),
            Some(PRODUCTION | "http://schemas.microsoft.com/3dmanufacturing/material/2015/02")
        ) {
            return Err(Error::Invalid(
                "この3MFの必須拡張には未対応です。対応するモデル形式で書き出し直してください。",
            ));
        }
    }
    let resources = root
        .children()
        .find(|n| n.has_tag_name((CORE, "resources")))
        .ok_or_else(invalid)?;
    // Index property groups once; a large palette must not be rescanned for every triangle.
    let properties = resources
        .children()
        .filter(Node::is_element)
        .filter(|n| !n.has_tag_name((CORE, "object")))
        .filter_map(|n| {
            n.attribute("id").map(|id| {
                (
                    id,
                    (
                        n.children().filter(Node::is_element).count(),
                        !n.has_tag_name((CORE, "basematerials"))
                            && !n.has_tag_name((
                                "http://schemas.microsoft.com/3dmanufacturing/material/2015/02",
                                "colorgroup",
                            )),
                    ),
                )
            })
        })
        .collect::<BTreeMap<_, _>>();
    let mut objects = BTreeMap::new();
    for node in resources
        .children()
        .filter(|n| n.has_tag_name((CORE, "object")))
    {
        let id: u32 = number(node, "id")?;
        if objects
            .insert(id, read_object(path, node, &properties)?)
            .is_some()
            || objects.len() > 4096
        {
            return Err(invalid());
        }
    }
    let build = root
        .children()
        .find(|n| n.has_tag_name((CORE, "build")))
        .map(|n| {
            n.children()
                .filter(|n| n.has_tag_name((CORE, "item")))
                .map(|n| {
                    if !matches!(n.attribute("printable"), None | Some("1" | "true")) {
                        return Err(Error::Invalid("印刷対象外の部品を含む3MFには未対応です。対象部品だけを書き出してください。"));
                    }
                    reference(path, n)
                })
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(ModelFile {
        unit: unit(root.attribute("unit"))?,
        objects,
        build,
    })
}
fn read_object(
    path: &str,
    node: Node<'_, '_>,
    properties: &BTreeMap<&str, (usize, bool)>,
) -> Result<Object> {
    if node.children().filter(Node::is_element).any(|n| {
        !["mesh", "components", "metadata"]
            .iter()
            .any(|name| n.has_tag_name((CORE, *name)))
    }) {
        return Err(Error::Invalid(
            "未対応の形状要素を含む3MFです。形状を確定して書き出し直してください。",
        ));
    }

    if !matches!(node.attribute("type"), None | Some("model")) {
        return Err(Error::Invalid(
            "補助形状やmodifierを含む3MFには未対応です。形状を確定して書き出し直してください。",
        ));
    }
    let meshes = node
        .children()
        .filter(|n| n.has_tag_name((CORE, "mesh")))
        .collect::<Vec<_>>();
    let components = node
        .children()
        .find(|n| n.has_tag_name((CORE, "components")));
    if meshes.len() > 1 || meshes.is_empty() == components.is_none() {
        return Err(invalid());
    }
    let mesh = meshes
        .first()
        .map(|n| read_mesh(*n, node, path, properties))
        .transpose()?;
    let components = components
        .map(|n| {
            n.children()
                .filter(|n| n.has_tag_name((CORE, "component")))
                .map(|n| reference(path, n))
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(Object {
        mesh,
        components,
        extruder: None,
    })
}
fn read_mesh(
    node: Node<'_, '_>,
    object: Node<'_, '_>,
    path: &str,
    properties: &BTreeMap<&str, (usize, bool)>,
) -> Result<Mesh> {
    if node
        .children()
        .filter(Node::is_element)
        .any(|n| !n.has_tag_name((CORE, "vertices")) && !n.has_tag_name((CORE, "triangles")))
    {
        return Err(Error::Invalid(
            "メッシュ以外の形状拡張には未対応です。形状をメッシュとして確定して書き出してください。",
        ));
    }
    let vertices = node
        .children()
        .find(|n| n.has_tag_name((CORE, "vertices")))
        .ok_or_else(invalid)?
        .children()
        .filter(|n| n.has_tag_name((CORE, "vertex")))
        .map(|n| Ok([number(n, "x")?, number(n, "y")?, number(n, "z")?]))
        .collect::<Result<Vec<[f64; 3]>>>()?;
    if vertices.is_empty()
        || vertices.len() > MAX_FACES * 3
        || vertices.iter().flatten().any(|v| !v.is_finite())
    {
        return Err(invalid());
    }
    let mut materials = BTreeSet::new();
    let mut painted = false;
    let faces = node
        .children()
        .find(|n| n.has_tag_name((CORE, "triangles")))
        .ok_or_else(invalid)?
        .children()
        .filter(|n| n.has_tag_name((CORE, "triangle")))
        .map(|n| {
            painted |= n.attributes().any(|a| {
                (a.name().contains("paint") || a.name() == "mmu_segmentation")
                    && !a.value().is_empty()
            });
            // Incomplete object defaults cannot be safely flattened into the current single-material path.
            painted |= object.attribute("pid").is_none()
                && ["pid", "p1", "p2", "p3"]
                    .iter()
                    .any(|key| n.attribute(*key).is_some());
            if let Some(pid) = n.attribute("pid").or_else(|| object.attribute("pid")) {
                let &(count, opaque) = properties.get(pid).ok_or_else(invalid)?;
                let first = n
                    .attribute("p1")
                    .or_else(|| object.attribute("pindex"))
                    .ok_or_else(invalid)?;
                for index in [Some(first), n.attribute("p2"), n.attribute("p3")]
                    .into_iter()
                    .flatten()
                {
                    let index: usize = index.parse().map_err(|_| invalid())?;
                    if index >= count {
                        return Err(invalid());
                    }
                    painted |= opaque;
                    // Printing needs only the single/multiple distinction; full assignments remain in the original.
                    if materials.len() < 2 {
                        materials.insert(format!("{path}:{pid}:{index}"));
                    }
                }
            }
            Ok([number(n, "v1")?, number(n, "v2")?, number(n, "v3")?])
        })
        .collect::<Result<Vec<[usize; 3]>>>()?;
    if faces.is_empty()
        || faces.len() > MAX_FACES
        || faces.iter().flatten().any(|i| *i >= vertices.len())
    {
        return Err(invalid());
    }
    Ok(Mesh {
        vertices,
        faces,
        materials,
        painted,
    })
}
fn apply_settings(files: &mut BTreeMap<String, ModelFile>, root: &str, bytes: &[u8]) -> Result<()> {
    let doc = xml(bytes)?;
    if !doc.root_element().has_tag_name("config") {
        return Err(Error::Invalid(
            "この3MFのプレート設定形式には未対応です。OrcaSlicerで書き出し直してください。",
        ));
    }
    for node in doc.descendants().filter(Node::is_element) {
        if node.has_tag_name("part")
            && !matches!(node.attribute("subtype"), None | Some("normal_part"))
            || node.has_tag_name("volume")
            || node.has_tag_name("metadata")
                && (matches!(node.attribute("key"), Some("part_type" | "volume_type"))
                    && node.attribute("value") != Some("normal_part")
                    || node.attribute("key") == Some("modifier")
                        && node.attribute("value") != Some("0"))
        {
            return Err(Error::Invalid(
                "modifier・切り抜き・補助形状を含む3MFには未対応です。形状を確定して書き出し直してください。",
            ));
        }
    }
    for node in doc
        .root_element()
        .children()
        .filter(|n| n.has_tag_name("object"))
    {
        let id: u32 = number(node, "id")?;
        let extruder = read_extruder(node)?;
        let object = files
            .get_mut(root)
            .and_then(|f| f.objects.get_mut(&id))
            .ok_or_else(invalid)?;
        object.extruder = extruder;
        for part in node.children().filter(|n| n.has_tag_name("part")) {
            let part_id: u32 = number(part, "id")?;
            if part_id == id && object.mesh.is_some() {
                object.extruder = read_extruder(part)?.or(object.extruder.take());
            } else {
                let mut found = false;
                let extruder = read_extruder(part)?;
                for component in object.components.iter_mut().filter(|c| c.id == part_id) {
                    component.extruder.clone_from(&extruder);
                    found = true;
                }
                if !found {
                    return Err(invalid());
                }
            }
        }
    }
    Ok(())
}
fn read_extruder(node: Node<'_, '_>) -> Result<Option<String>> {
    metadata(node, "extruder")
        .map(|v| {
            let value: u32 = v.parse().map_err(|_| invalid())?;
            Ok((value != 0).then(|| format!("extruder:{value}")))
        })
        .transpose()
        .map(Option::flatten)
}
fn metadata<'a>(node: Node<'a, 'a>, key: &str) -> Option<&'a str> {
    node.children()
        .find(|n| n.has_tag_name("metadata") && n.attribute("key") == Some(key))
        .and_then(|n| n.attribute("value"))
}
fn selections(root: &str, items: &[SourceItem], settings: Option<&[u8]>) -> Result<Vec<Selection>> {
    let mut plates = Vec::new();
    if let Some(settings) = settings {
        let doc = xml(settings)?;
        let nodes = doc
            .root_element()
            .children()
            .filter(|n| n.has_tag_name("plate"))
            .collect::<Vec<_>>();
        let has_plates = !nodes.is_empty();
        for node in nodes {
            let mut selected = Vec::new();
            for instance in node.children().filter(|n| n.has_tag_name("model_instance")) {
                let id = metadata(instance, "object_id")
                    .ok_or_else(invalid)?
                    .parse::<u32>()
                    .map_err(|_| invalid())?;
                let index = metadata(instance, "instance_id")
                    .ok_or_else(invalid)?
                    .parse::<usize>()
                    .map_err(|_| invalid())?;
                let item = items
                    .iter()
                    .find(|i| i.object_id == id && i.instance_id == index)
                    .ok_or_else(invalid)?
                    .clone();
                if selected.contains(&item) {
                    return Err(invalid());
                }
                selected.push(item);
            }
            if selected.is_empty() {
                continue;
            }
            plates.push(Selection {
                name: metadata(node, "plater_name")
                    .filter(|s| !s.trim().is_empty())
                    .map_or_else(|| format!("プレート {}", plates.len() + 1), str::to_owned),
                root_model: root.into(),
                plate_id: metadata(node, "plater_id").map(str::to_owned),
                items: selected,
                print_reason: None,
            });
        }
        if has_plates && plates.is_empty() {
            return Err(invalid());
        }
    }
    if plates.is_empty() {
        plates.push(Selection {
            name: "プレート 1".into(),
            root_model: root.into(),
            plate_id: None,
            items: items.to_vec(),
            print_reason: None,
        });
    }
    Ok(plates)
}

#[derive(Clone, Copy, Debug)]
struct Transform([f64; 12]);

impl Transform {
    fn parse(value: Option<&str>) -> Result<Self> {
        let Some(value) = value else {
            return Ok(Self::scale(1.0));
        };
        let values = value
            .split_whitespace()
            .map(str::parse::<f64>)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| {
                Error::Invalid("3MFの座標変換が不正です。ファイルを書き出し直してください。")
            })?;
        let matrix: [f64; 12] = values
            .try_into()
            .map_err(|_| Error::Invalid("3MFの座標変換には12個の数値が必要です。"))?;
        let result = Self(matrix);
        if matrix.iter().any(|v| !v.is_finite())
            || !result.determinant().is_finite()
            || result.determinant() == 0.0
        {
            return Err(Error::Invalid(
                "3MFの座標変換が不正です。ファイルを書き出し直してください。",
            ));
        }
        Ok(result)
    }
    fn scale(factor: f64) -> Self {
        Self([
            factor, 0.0, 0.0, 0.0, factor, 0.0, 0.0, 0.0, factor, 0.0, 0.0, 0.0,
        ])
    }
    fn then(self, outer: Self) -> Self {
        let mut result = [0.0; 12];
        for row in 0..4 {
            for column in 0..3 {
                result[row * 3 + column] = (0..3)
                    .map(|k| self.0[row * 3 + k] * outer.0[k * 3 + column])
                    .sum::<f64>()
                    + if row == 3 { outer.0[9 + column] } else { 0.0 };
            }
        }
        Self(result)
    }
    fn apply(self, point: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|column| {
            (0..3)
                .map(|k| point[k] * self.0[k * 3 + column])
                .sum::<f64>()
                + self.0[9 + column]
        })
    }
    fn determinant(self) -> f64 {
        let m = self.0;
        m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6])
            + m[2] * (m[3] * m[7] - m[4] * m[6])
    }
    fn mirrored(self) -> bool {
        self.determinant() < 0.0
    }
}

fn unit(value: Option<&str>) -> Result<f64> {
    match value.unwrap_or("millimeter") {
        "micron" => Ok(0.001),
        "millimeter" => Ok(1.0),
        "centimeter" => Ok(10.0),
        "inch" => Ok(25.4),
        "foot" => Ok(304.8),
        "meter" => Ok(1000.0),
        _ => Err(Error::Invalid(
            "3MFの単位に対応していません。mm指定で書き出し直してください。",
        )),
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::format_collect)] // Exact constants and small readable XML fixtures.
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    fn archive(model: &str, settings: Option<&str>) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, content) in [
            ("3D/3dmodel.model", model),
            (
                "_rels/.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="start" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel" Target="/3D/3dmodel.model"/></Relationships>"#,
            ),
            (
                "Metadata/model_settings.config",
                settings.unwrap_or("<config/>"),
            ),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn with_part(bytes: Vec<u8>, name: &str, data: &[u8]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new_append(Cursor::new(bytes)).unwrap();
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(data).unwrap();
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn production_parts_keep_units_mirrors_and_per_instance_materials() {
        let core = model().replace(r#"unit="inch""#, r#"unit="millimeter""#);
        let root = format!(
            r#"<model unit="millimeter" xmlns="{CORE}" xmlns:p="{PRODUCTION}" requiredextensions="p"><resources><object id="10"><components><component p:path="/3D/Objects/part.model" objectid="1" transform="-2 0 0 0 2 0 0 0 2 10 0 0"/></components></object><object id="20"><components><component p:path="/3D/Objects/part.model" objectid="1" transform="2 0 0 0 2 0 0 0 2 12 0 0"/></components></object></resources><build><item objectid="10"/><item objectid="20"/></build></model>"#
        );
        let settings = r#"<config><object id="10"><part id="1" subtype="normal_part"><metadata key="extruder" value="1"/></part></object><object id="20"><part id="1" subtype="normal_part"><metadata key="extruder" value="2"/></part></object></config>"#;
        let bytes = with_part(
            archive(&root, Some(settings)),
            "3D/Objects/part.model",
            core.as_bytes(),
        );
        let package = Package::read(&bytes).unwrap();
        assert!(package.plates[0].print_reason.is_some());
        let mesh = stl_io::read_stl(&mut Cursor::new(package.mesh(0).unwrap())).unwrap();
        let volume = mesh
            .faces
            .iter()
            .map(|f| {
                let [a, b, c] = f.vertices.map(|i| mesh.vertices[i]);
                (a[0] * (b[1] * c[2] - b[2] * c[1])
                    + a[1] * (b[2] * c[0] - b[0] * c[2])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]))
                    / 6.0
            })
            .sum::<f32>();
        assert!((volume - 16.0).abs() < 0.001, "{volume}");
        assert_eq!(bounds(package.mesh(0).unwrap()), ([6.0, 2.0, 2.0], 24));
        let external = root.replace("/3D/Objects/part.model", "https://example.com/part.model");
        assert!(Package::read(&archive(&external, None)).is_err());
        for path in [
            "../escaped.model",
            "/absolute.model",
            "3D/../escaped.model",
            "3D\\escaped.model",
        ] {
            assert!(
                Package::read(&with_part(archive(&model(), None), path, core.as_bytes())).is_err()
            );
        }
    }

    fn model() -> String {
        let vertices = [
            [0, 0, 0],
            [1, 0, 0],
            [1, 1, 0],
            [0, 1, 0],
            [0, 0, 1],
            [1, 0, 1],
            [1, 1, 1],
            [0, 1, 1],
        ]
        .into_iter()
        .map(|[x, y, z]| format!(r#"<vertex x="{x}" y="{y}" z="{z}"/>"#))
        .collect::<String>();
        let faces = [
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 1, 5],
            [0, 5, 4],
            [1, 2, 6],
            [1, 6, 5],
            [2, 3, 7],
            [2, 7, 6],
            [3, 0, 4],
            [3, 4, 7],
        ]
        .into_iter()
        .map(|[a, b, c]| format!(r#"<triangle v1="{a}" v2="{b}" v3="{c}"/>"#))
        .collect::<String>();
        format!(
            r#"<model unit="inch" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources><object id="1" type="model"><mesh><vertices>{vertices}</vertices><triangles>{faces}</triangles></mesh></object><object id="10" type="model"><components><component objectid="1" transform="2 0 0 0 2 0 0 0 2 1 0 0"/></components></object></resources><build><item objectid="10" transform="1 0 0 0 1 0 0 0 1 0 3 0"/><item objectid="10" transform="1 0 0 0 1 0 0 0 1 4 0 0"/></build></model>"#
        )
    }

    fn bounds(bytes: Vec<u8>) -> ([f32; 3], usize) {
        let mesh = stl_io::read_stl(&mut Cursor::new(bytes)).unwrap();
        let size = std::array::from_fn(|axis| {
            mesh.vertices
                .iter()
                .map(|p| p[axis])
                .fold(f32::NEG_INFINITY, f32::max)
                - mesh
                    .vertices
                    .iter()
                    .map(|p| p[axis])
                    .fold(f32::INFINITY, f32::min)
        });
        (size, mesh.faces.len())
    }

    #[test]
    fn imported_nested_components_and_repeated_instances_keep_their_mm_geometry() {
        let package = Package::read(&archive(&model(), None)).unwrap();
        assert_eq!(package.plates.len(), 1);
        assert_eq!(package.plates[0].items.len(), 2);
        let (size, faces) = bounds(package.mesh(0).unwrap());
        assert_eq!(faces, 24);
        let bytes = package.mesh(0).unwrap();
        for triangle in stl_io::create_stl_reader(&mut Cursor::new(bytes)).unwrap() {
            let normal = triangle.unwrap().normal;
            let length = (0..3).map(|i| normal[i] * normal[i]).sum::<f32>();
            assert!(
                (length - 1.0).abs() < 0.001,
                "STL lighting needs a unit face normal"
            );
        }
        for (actual, expected) in size.into_iter().zip([152.4, 127.0, 50.8]) {
            assert!((actual - expected).abs() < 0.001, "{size:?}");
        }
        assert!(package.mesh(1).is_err());
    }

    #[test]
    fn selected_vendor_plate_keeps_the_correct_object_instance_and_name() {
        let settings = r#"<config><plate><metadata key="plater_id" value="1"/><metadata key="plater_name" value="一つ目"/><model_instance><metadata key="object_id" value="10"/><metadata key="instance_id" value="0"/></model_instance></plate><plate><metadata key="plater_id" value="2"/><metadata key="plater_name" value="二つ目"/><model_instance><metadata key="object_id" value="10"/><metadata key="instance_id" value="1"/></model_instance></plate></config>"#;
        let package = Package::read(&archive(&model(), Some(settings))).unwrap();
        assert_eq!(package.plates.len(), 2);
        assert_eq!(package.plates[1].name, "二つ目");
        assert_eq!(package.plates[1].items[0].build_index, 1);
        let (size, faces) = bounds(package.mesh(1).unwrap());
        assert_eq!(faces, 12);
        for actual in size {
            assert!((actual - 50.8).abs() < 0.001);
        }
    }

    #[test]
    fn colors_and_paint_are_retained_but_not_silently_printed_as_one_material() {
        let material = model().replace("<resources>", r##"<resources><basematerials id="20"><base name="red" displaycolor="#FF0000"/><base name="blue" displaycolor="#0000FF"/></basematerials>"##)
            .replace(r#"id="1" type="model""#, r#"id="1" type="model" pid="20" pindex="0""#);
        assert!(
            Package::read(&archive(&material, None)).unwrap().plates[0]
                .print_reason
                .is_none()
        );
        let two = material.replacen(
            r#"<triangle v1="0""#,
            r#"<triangle pid="20" p1="1" v1="0""#,
            1,
        );
        let package = Package::read(&archive(&two, None)).unwrap();
        assert!(package.plates[0].print_reason.is_some());
        assert_eq!(bounds(package.mesh(0).unwrap()).1, 24);
        let painted = model().replacen("<triangle ", r#"<triangle paint_color="0C" "#, 1);
        assert!(
            Package::read(&archive(&painted, None)).unwrap().plates[0]
                .print_reason
                .is_some()
        );
        let settings = r#"<config><object id="10"><metadata key="extruder" value="2"/><part id="1" subtype="normal_part"><metadata key="extruder" value="1"/></part></object></config>"#;
        assert!(
            Package::read(&archive(&model(), Some(settings)))
                .unwrap()
                .plates[0]
                .print_reason
                .is_none()
        );
        let repeated = model().replace(
            "</components>",
            r#"<component objectid="1" transform="1 0 0 0 1 0 0 0 1 3 0 0"/></components>"#,
        );
        assert!(
            Package::read(&archive(&repeated, Some(settings)))
                .unwrap()
                .plates[0]
                .print_reason
                .is_none()
        );
        let partial = two.replace(r#" pid="20" pindex="0""#, "");
        assert!(
            Package::read(&archive(&partial, None)).unwrap().plates[0]
                .print_reason
                .is_some()
        );
    }

    #[test]
    fn invalid_or_shape_changing_inputs_never_produce_a_different_model() {
        for broken in [
            model().replace(r#"v3="1""#, r#"v3="99""#),
            model().replace(r#"objectid="1""#, r#"objectid="10""#),
            model().replace(
                r#"<item objectid="10""#,
                r#"<item printable="false" objectid="10""#,
            ),
            model().replace(
                "<model ",
                r#"<model xmlns:x="urn:unsupported" requiredextensions="x" "#,
            ),
        ] {
            assert!(
                Package::read(&archive(&broken, None))
                    .and_then(|p| p.mesh(0))
                    .is_err()
            );
        }
        for settings in [
            r#"<config><object id="10"><part id="1" subtype="negative_part"/></object></config>"#,
            r#"<config><object id="10"><part id="1"><metadata key="modifier" value="1"/></part></object></config>"#,
            r#"<config><plate><metadata key="plater_id" value="1"/></plate></config>"#,
        ] {
            assert!(
                Package::read(&archive(&model(), Some(settings))).is_err(),
                "{settings}"
            );
        }
        assert!(Package::read(b"not a zip").is_err());
    }

    #[test]
    fn geometry_extensions_and_legacy_modifiers_are_not_discarded() {
        let beam = model().replace("</mesh>", "<beamlattice/></mesh>");
        assert!(Package::read(&archive(&beam, None)).is_err());
        let modified = model().replace("</object>", "<shape_modifier/></object>");
        assert!(Package::read(&archive(&modified, None)).is_err());
        assert!(Package::read(&archive(&model(), Some("<unknown_config/>"))).is_err());
        let legacy = with_part(
            archive(&model(), None),
            "Metadata/Slic3r_PE_model.config",
            b"<config><object id='10'><volume modifier='1'/></object></config>",
        );
        assert!(Package::read(&legacy).is_err());
        let paint = model().replacen("<triangle ", r#"<triangle mmu_segmentation="0C" "#, 1);
        assert!(
            Package::read(&archive(&paint, None)).unwrap().plates[0]
                .print_reason
                .is_some()
        );
    }

    #[test]
    fn units_and_nested_row_major_transforms_preserve_physical_dimensions() {
        assert_eq!(unit(None).unwrap(), 1.0);
        assert_eq!(unit(Some("inch")).unwrap(), 25.4);
        assert_eq!(unit(Some("micron")).unwrap(), 0.001);
        assert_eq!(unit(Some("centimeter")).unwrap(), 10.0);
        assert_eq!(unit(Some("meter")).unwrap(), 1000.0);
        assert_eq!(unit(Some("foot")).unwrap(), 304.8);
        assert!(unit(Some("pixel")).is_err());
        let component = Transform::parse(Some("2 0 0 0 2 0 0 0 2 1 0 0")).unwrap();
        let item = Transform::parse(Some("1 0 0 0 1 0 0 0 1 0 3 0")).unwrap();
        let world = component.then(item).then(Transform::scale(25.4));
        let first = world.apply([0.0; 3]);
        let last = world.apply([1.0; 3]);
        for (actual, expected) in first.into_iter().zip([25.4, 76.2, 0.0]) {
            assert!((actual - expected).abs() < 1e-10);
        }
        for (actual, expected) in last.into_iter().zip([76.2, 127.0, 50.8]) {
            assert!((actual - expected).abs() < 1e-10);
        }
        let rotated = Transform::parse(Some("0 1 0 -1 0 0 0 0 1 10 20 0")).unwrap();
        assert_eq!(rotated.apply([2.0, 3.0, 4.0]), [7.0, 22.0, 4.0]);
        assert!(!rotated.mirrored());
        assert!(
            Transform::parse(Some("-1 0 0 0 1 0 0 0 1 0 0 0"))
                .unwrap()
                .mirrored()
        );
        assert_eq!(
            Transform::parse(None).unwrap().apply([2.0, 3.0, 4.0]),
            [2.0, 3.0, 4.0]
        );
    }

    #[test]
    fn malformed_nonfinite_and_flattening_transforms_are_not_imported() {
        for input in [
            "",
            "1 2",
            "1 0 0 0 1 0 0 0 0 0 0 0",
            "NaN 0 0 0 1 0 0 0 1 0 0 0",
            "1 0 0 0 1 0 0 0 1 inf 0 0",
        ] {
            assert!(Transform::parse(Some(input)).is_err(), "{input}");
        }
    }
}
