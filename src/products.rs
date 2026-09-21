use crate::{
    database::Database,
    filament::{FilamentData, Setting, SettingData},
    plates::{Error, Result},
};
use rmcp::schemars;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductData {
    pub name: String,
    pub vendor: String,
    pub material: String,
    pub bambu_filament_id: Option<String>,
}
impl ProductData {
    pub fn validate(&self) -> Result<()> {
        FilamentData {
            name: self.name.clone(),
            vendor: self.vendor.clone(),
            material: self.material.clone(),
            color: "FFFFFFFF".into(),
            bambu_filament_id: self.bambu_filament_id.clone(),
        }
        .validate()
    }
}
#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ColorData {
    pub name: String,
    pub color: String,
}
impl ColorData {
    pub fn validate(&self) -> Result<()> {
        FilamentData {
            name: self.name.clone(),
            vendor: "color".into(),
            material: "color".into(),
            color: self.color.clone(),
            bambu_filament_id: None,
        }
        .validate()
    }
}
#[derive(Serialize)]
pub(crate) struct Color {
    pub id: String,
    #[serde(flatten)]
    pub data: ColorData,
}
#[derive(Serialize)]
pub(crate) struct ProductSetting {
    pub id: String,
    #[serde(flatten)]
    pub data: SettingData,
}
#[derive(Serialize)]
pub(crate) struct Product {
    pub id: String,
    #[serde(flatten)]
    pub data: ProductData,
    pub colors: Vec<Color>,
    pub settings: Vec<ProductSetting>,
}

pub(crate) fn migrate(c: &Connection) -> Result<()> {
    let legacy = c
        .prepare(
            "SELECT id,name,vendor,material,color,bambu_filament_id FROM filaments ORDER BY id",
        )?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                ProductData {
                    name: r.get(1)?,
                    vendor: r.get(2)?,
                    material: r.get(3)?,
                    bambu_filament_id: r.get(5)?,
                },
                r.get::<_, String>(4)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    c.execute_batch("CREATE TABLE filament_products (id TEXT PRIMARY KEY, name TEXT NOT NULL, vendor TEXT NOT NULL, material TEXT NOT NULL, bambu_filament_id TEXT);
        CREATE TABLE colors_v4 (id TEXT PRIMARY KEY, product_id TEXT NOT NULL REFERENCES filament_products(id) ON DELETE CASCADE, name TEXT NOT NULL, color TEXT NOT NULL);
        CREATE TABLE settings_v4 (id TEXT PRIMARY KEY, product_id TEXT NOT NULL REFERENCES filament_products(id) ON DELETE CASCADE, machine_profile_key TEXT NOT NULL, base_profile_key TEXT NOT NULL, overrides_json TEXT NOT NULL, UNIQUE(product_id,machine_profile_key));")?;
    let mut groups = std::collections::BTreeMap::new();
    for (id, data, color) in legacy {
        let settings = c.prepare("SELECT id,machine_profile_key,base_profile_key,overrides_json FROM filament_settings WHERE filament_id=?1 ORDER BY machine_profile_key")?
            .query_map([&id], |r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?)))?
            .collect::<std::result::Result<Vec<_>,_>>()?;
        let normalized: Vec<_> = settings.iter().map(|(_,machine,base,raw)| {
            let overrides: crate::filament::Overrides = serde_json::from_str(raw).map_err(|_|Error::Unavailable("Invalid legacy material settings; restore or repair the saved database"))?;
            Ok(SettingData {machine_profile_key:machine.clone(),base_profile_key:base.clone(),overrides_json:overrides})
        }).collect::<Result<_>>()?;
        // Merge only exact product metadata and every machine setting, never color/name heuristics.
        let key = serde_json::to_string(&(&data, &normalized)).expect("product settings serialize");
        let product_id = if let Some(existing) = groups.get(&key) {
            existing
        } else {
            c.execute(
                "INSERT INTO filament_products VALUES (?1,?2,?3,?4,?5)",
                params![
                    id,
                    data.name,
                    data.vendor,
                    data.material,
                    data.bambu_filament_id
                ],
            )?;
            for (sid, machine, base, raw) in &settings {
                c.execute(
                    "INSERT INTO settings_v4 VALUES (?1,?2,?3,?4,?5)",
                    params![sid, id, machine, base, raw],
                )?;
            }
            groups.entry(key).or_insert_with(|| id.clone())
        };
        c.execute(
            "INSERT INTO colors_v4 VALUES (?1,?2,?3,?4)",
            params![id, product_id, data.name, color],
        )?;
    }
    c.execute_batch("CREATE TEMP TABLE saved_jobs AS SELECT * FROM print_jobs;
        DROP TABLE print_jobs; DROP TABLE filament_settings; DROP TABLE filaments;
        ALTER TABLE colors_v4 RENAME TO filaments; ALTER TABLE settings_v4 RENAME TO filament_settings;")?;
    c.execute_batch(include_str!("../migrations/004-print-jobs.sql"))?;
    c.execute_batch("INSERT INTO print_jobs SELECT * FROM saved_jobs; DROP TABLE saved_jobs; PRAGMA user_version=4;")?;
    Ok(())
}

pub(crate) fn product_id(c: &Connection, filament_id: &str) -> Result<String> {
    c.query_row(
        "SELECT product_id FROM filaments WHERE id=?1",
        [filament_id],
        |r| r.get(0),
    )
    .optional()?
    .ok_or(Error::NotFound)
}
pub(crate) fn load_setting(
    c: &Connection,
    filament: &str,
    machine: &str,
) -> Result<crate::filament::SettingData> {
    let (base,raw):(String,String)=c.query_row("SELECT s.base_profile_key,s.overrides_json FROM filament_settings s JOIN filaments f ON f.product_id=s.product_id WHERE f.id=?1 AND s.machine_profile_key=?2",params![filament,machine],|r|Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or(Error::Conflict("Configure this material for the required machine and nozzle first"))?;
    Ok(crate::filament::SettingData {
        machine_profile_key: machine.to_owned(),
        base_profile_key: base,
        overrides_json: serde_json::from_str(&raw).map_err(std::io::Error::other)?,
    })
}
pub(crate) fn settings(c: &Connection, id: &str) -> Result<Vec<ProductSetting>> {
    Ok(c.prepare("SELECT id,machine_profile_key,base_profile_key,overrides_json FROM filament_settings WHERE product_id=?1 ORDER BY machine_profile_key")?
        .query_map([id], |r| {
            let raw:String=r.get(3)?;
            Ok(ProductSetting {id:r.get(0)?,data:SettingData{machine_profile_key:r.get(1)?,base_profile_key:r.get(2)?,overrides_json:serde_json::from_str(&raw).map_err(|e|rusqlite::Error::FromSqlConversionFailure(3,rusqlite::types::Type::Text,Box::new(e)))?}})
        })?.collect::<std::result::Result<_,_>>()?)
}

impl Database {
    pub(crate) fn products(&self) -> Result<Vec<Product>> {
        let c = self.connection()?;
        let rows=c.prepare("SELECT id,name,vendor,material,bambu_filament_id FROM filament_products ORDER BY name,id")?
            .query_map([],|r|Ok((r.get::<_,String>(0)?,ProductData{name:r.get(1)?,vendor:r.get(2)?,material:r.get(3)?,bambu_filament_id:r.get(4)?})))?
            .collect::<std::result::Result<Vec<_>,_>>()?;
        rows.into_iter()
            .map(|(id, data)| {
                let colors = c
                    .prepare(
                        "SELECT id,name,color FROM filaments WHERE product_id=?1 ORDER BY name,id",
                    )?
                    .query_map([&id], |r| {
                        Ok(Color {
                            id: r.get(0)?,
                            data: ColorData {
                                name: r.get(1)?,
                                color: r.get(2)?,
                            },
                        })
                    })?
                    .collect::<std::result::Result<_, _>>()?;
                let settings = settings(&c, &id)?;
                Ok(Product {
                    id,
                    data,
                    colors,
                    settings,
                })
            })
            .collect()
    }
    pub(crate) fn product(&self, id: &str) -> Result<Product> {
        self.products()?
            .into_iter()
            .find(|p| p.id == id)
            .ok_or(Error::NotFound)
    }
    pub(crate) fn save_product(&self, id: &str, data: &ProductData) -> Result<()> {
        data.validate()?;
        let mut c = self.connection()?;
        let tx = c.transaction()?;
        tx.execute("INSERT INTO filament_products VALUES (?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET name=excluded.name,vendor=excluded.vendor,material=excluded.material,bambu_filament_id=excluded.bambu_filament_id",params![id,data.name,data.vendor,data.material,data.bambu_filament_id])?;
        crate::ams::invalidate_automatic(&tx)?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn delete_product(&self, id: &str) -> Result<()> {
        if self
            .connection()?
            .execute("DELETE FROM filament_products WHERE id=?1", [id])?
            == 0
        {
            return Err(Error::NotFound);
        }
        Ok(())
    }
    pub(crate) fn save_color(&self, pid: &str, id: &str, data: &ColorData) -> Result<()> {
        data.validate()?;
        let mut c = self.connection()?;
        let tx = c.transaction()?;
        if tx.execute("INSERT INTO filaments VALUES (?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET name=excluded.name,color=excluded.color WHERE product_id=excluded.product_id",params![id,pid,data.name,data.color.to_ascii_uppercase()])?==0 {return Err(Error::NotFound)}
        crate::ams::invalidate_automatic(&tx)?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn save_product_setting(
        &self,
        pid: &str,
        sid: &str,
        data: &SettingData,
    ) -> Result<()> {
        save_setting(&*self.connection()?, pid, sid, data)
    }
    pub(crate) fn adopt_color(&self, pid: &str, fid: &str) -> Result<()> {
        let mut c = self.connection()?;
        let tx = c.transaction()?;
        let old = product_id(&tx, fid)?;
        if old == pid {
            return Ok(());
        }
        let common = |id: &str| -> Result<(String, String, Option<String>)> {
            tx.query_row(
                "SELECT vendor,material,bambu_filament_id FROM filament_products WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .ok_or(Error::NotFound)
        };
        let configuration = |id: &str| -> Result<serde_json::Value> {
            Ok(serde_json::to_value(
                settings(&tx, id)?
                    .into_iter()
                    .map(|s| s.data)
                    .collect::<Vec<_>>(),
            )
            .expect("settings serialize"))
        };
        if common(&old)? != common(pid)? || configuration(&old)? != configuration(pid)? {
            return Err(Error::Conflict(
                "Product metadata and every machine setting must match before combining colors",
            ));
        }
        if tx.query_row("SELECT EXISTS(SELECT 1 FROM print_jobs WHERE filament_id=?1 AND state IN ('preparing','printing','awaiting_removal','needs_attention'))",[fid],|r|r.get::<_,bool>(0))? {
            return Err(Error::Conflict("Wait until this color's active print has been removed"));
        }
        tx.execute(
            "UPDATE filaments SET product_id=?1 WHERE id=?2",
            params![pid, fid],
        )?;
        tx.execute("DELETE FROM filament_products WHERE id=?1 AND NOT EXISTS(SELECT 1 FROM filaments WHERE product_id=?1)",[old])?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn delete_product_setting(&self, pid: &str, sid: &str) -> Result<()> {
        if self.connection()?.execute(
            "DELETE FROM filament_settings WHERE id=?1 AND product_id=?2",
            params![sid, pid],
        )? == 0
        {
            return Err(Error::NotFound);
        }
        Ok(())
    }
    pub(crate) fn filament_settings(&self, id: &str) -> Result<Vec<Setting>> {
        let c = self.connection()?;
        let Some(pid) = c
            .query_row("SELECT product_id FROM filaments WHERE id=?1", [id], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
        else {
            return Ok(vec![]);
        };
        Ok(settings(&c, &pid)?
            .into_iter()
            .map(|s| Setting {
                id: s.id,
                filament_id: id.into(),
                data: s.data,
            })
            .collect())
    }
    pub(crate) fn save_setting(&self, s: &Setting) -> Result<()> {
        let c = self.connection()?;
        let pid = product_id(&c, &s.filament_id)?;
        save_setting(&c, &pid, &s.id, &s.data)
    }
    pub(crate) fn delete_setting(&self, fid: &str, sid: &str) -> Result<()> {
        let c = self.connection()?;
        if c.execute("DELETE FROM filament_settings WHERE id=?1 AND product_id=(SELECT product_id FROM filaments WHERE id=?2)",params![sid,fid])?==0 {return Err(Error::NotFound)}
        Ok(())
    }
}

fn save_setting(c: &Connection, pid: &str, sid: &str, data: &SettingData) -> Result<()> {
    data.overrides_json.validate()?;
    if c.execute("INSERT INTO filament_settings(id,product_id,machine_profile_key,base_profile_key,overrides_json) VALUES (?1,?2,?3,?4,?5)
        ON CONFLICT(id) DO UPDATE SET machine_profile_key=excluded.machine_profile_key,base_profile_key=excluded.base_profile_key,overrides_json=excluded.overrides_json WHERE product_id=excluded.product_id",
        params![sid,pid,data.machine_profile_key,data.base_profile_key,serde_json::to_string(&data.overrides_json).expect("temperatures serialize")])? == 0 {return Err(Error::NotFound)}
    Ok(())
}
