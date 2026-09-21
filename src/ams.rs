use crate::{
    database::{Database, Device},
    filament,
    plates::{Error, Result},
    printer_state::{Status, Tray},
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

pub(crate) fn migrate(c: &Connection) -> Result<()> {
    c.execute_batch("ALTER TABLE ams_slots ADD COLUMN load_order INTEGER CHECK(load_order>0);
        ALTER TABLE ams_slots ADD COLUMN priority_order INTEGER NOT NULL DEFAULT 0 CHECK(priority_order>=0);
        WITH ordered AS (SELECT id,ROW_NUMBER() OVER(PARTITION BY printer_id ORDER BY ams_id,slot_index) AS n FROM ams_slots WHERE present=1)
        UPDATE ams_slots SET load_order=(SELECT n FROM ordered WHERE ordered.id=ams_slots.id),priority_order=COALESCE((SELECT n FROM ordered WHERE ordered.id=ams_slots.id),0);
        PRAGMA user_version=5;")?;
    Ok(())
}

fn known_change<T: PartialEq>(a: Option<&T>, b: Option<&T>) -> bool {
    a.zip(b).is_some_and(|(a, b)| a != b)
}
fn new_load(old: Option<&AmsSlot>, tray: &Tray) -> bool {
    old.is_none_or(|s| {
        s.load_order.is_none()
            || s.reported.present == Some(false)
            || known_change(s.reported.tag_uid.as_ref(), tray.tag_uid.as_ref())
            || known_change(s.reported.profile_id.as_ref(), tray.profile_id.as_ref())
            || known_change(s.reported.material.as_ref(), tray.material.as_ref())
            || known_change(s.reported.color.as_ref(), tray.color.as_ref())
            || known_change(s.reported.brand.as_ref(), tray.brand.as_ref())
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SlotRevision {
    pub id: String,
    pub revision: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Priority {
    pub filament_id: String,
    pub order: Vec<SlotRevision>,
}

fn resolve(c: &Connection, printer: &str, filament: &str, machine: &str) -> Result<Vec<AmsSlot>> {
    let product = crate::products::product_id(c, filament)?;
    let mut q=c.prepare("SELECT f.id FROM filaments f JOIN filaments requested ON requested.id=?1 WHERE f.product_id=?2 AND UPPER(f.color)=UPPER(requested.color) AND EXISTS(SELECT 1 FROM filament_settings s WHERE s.product_id=f.product_id AND s.machine_profile_key=?3)")?;
    let colors = q
        .query_map(params![filament, product, machine], |r| {
            r.get::<_, String>(0)
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut candidates: Vec<_> = slots(c, printer)?
        .into_iter()
        .filter(|s| {
            s.reported.present == Some(true)
                && s.load_order.is_some()
                && s.filament_id.as_ref().is_some_and(|id| colors.contains(id))
        })
        .collect();
    candidates.sort_by_key(|s| (s.priority_order, s.load_order, s.ams_id, s.slot_index));
    Ok(candidates)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Mapping {
    pub revision: i64,
    pub filament_id: Option<String>,
}
pub(crate) enum Change {
    Mapping(String, Mapping),
    Priority(Priority),
}

#[derive(Clone, Serialize)]
pub(crate) struct AmsSlot {
    pub id: String,
    pub printer_id: String,
    pub ams_id: u8,
    pub slot_index: u8,
    pub filament_id: Option<String>,
    pub mapping_source: String,
    pub reported: Tray,
    pub detect_on_insert: Option<bool>,
    pub detect_on_power_up: Option<bool>,
    pub revision: i64,
    pub load_order: Option<i64>,
    pub priority_order: i64,
}
fn slots(c: &Connection, printer_id: &str) -> Result<Vec<AmsSlot>> {
    let mut q=c.prepare("SELECT id,printer_id,ams_id,slot_index,filament_id,mapping_source,reported_tag_uid,reported_profile_id,reported_type,reported_color,reported_brand,reported_temp_min,reported_temp_max,present,remaining_percent,detect_on_insert,detect_on_power_up,last_seen_at,revision,load_order,priority_order FROM ams_slots WHERE printer_id=?1 ORDER BY ams_id,slot_index")?;
    Ok(q.query_map([printer_id], |r| {
        Ok(AmsSlot {
            id: r.get(0)?,
            printer_id: r.get(1)?,
            ams_id: r.get(2)?,
            slot_index: r.get(3)?,
            filament_id: r.get(4)?,
            mapping_source: r.get(5)?,
            reported: Tray {
                id: r.get(3)?,
                tag_uid: r.get(6)?,
                profile_id: r.get(7)?,
                material: r.get(8)?,
                color: r.get(9)?,
                brand: r.get(10)?,
                temperature_min: r.get(11)?,
                temperature_max: r.get(12)?,
                present: r.get(13)?,
                remaining_percent: r.get(14)?,
                last_seen_at: r
                    .get::<_, Option<i64>>(17)?
                    .and_then(|v| u64::try_from(v).ok()),
            },
            detect_on_insert: r.get(15)?,
            detect_on_power_up: r.get(16)?,
            revision: r.get(18)?,
            load_order: r.get(19)?,
            priority_order: r.get(20)?,
        })
    })?
    .collect::<std::result::Result<_, _>>()?)
}
fn save(c: &Connection, s: &AmsSlot) -> Result<()> {
    let t = &s.reported;
    let seen = t
        .last_seen_at
        .map(i64::try_from)
        .transpose()
        .map_err(|_| Error::Invalid("Invalid observation timestamp"))?;
    c.execute("INSERT INTO ams_slots(id,printer_id,ams_id,slot_index,filament_id,mapping_source,reported_tag_uid,reported_profile_id,reported_type,reported_color,reported_brand,reported_temp_min,reported_temp_max,present,remaining_percent,detect_on_insert,detect_on_power_up,last_seen_at,revision,load_order,priority_order)
        VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)
        ON CONFLICT(id) DO UPDATE SET filament_id=excluded.filament_id,mapping_source=excluded.mapping_source,reported_tag_uid=excluded.reported_tag_uid,reported_profile_id=excluded.reported_profile_id,reported_type=excluded.reported_type,reported_color=excluded.reported_color,reported_brand=excluded.reported_brand,reported_temp_min=excluded.reported_temp_min,reported_temp_max=excluded.reported_temp_max,present=excluded.present,remaining_percent=excluded.remaining_percent,detect_on_insert=excluded.detect_on_insert,detect_on_power_up=excluded.detect_on_power_up,last_seen_at=excluded.last_seen_at,revision=excluded.revision,load_order=excluded.load_order,priority_order=excluded.priority_order",
        params![s.id,s.printer_id,s.ams_id,s.slot_index,s.filament_id,s.mapping_source,t.tag_uid,t.profile_id,t.material,t.color,t.brand,t.temperature_min,t.temperature_max,t.present,t.remaining_percent,s.detect_on_insert,s.detect_on_power_up,seen,s.revision,s.load_order,s.priority_order])?;
    Ok(())
}
pub(crate) fn invalidate_automatic(c: &Connection) -> Result<()> {
    let filaments = crate::database::load_filaments(c)?;
    let ids = c
        .prepare("SELECT DISTINCT printer_id FROM ams_slots")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for id in ids {
        for mut slot in slots(c, &id)? {
            if slot.mapping_source != "automatic" {
                continue;
            }
            let choices = filament::candidates(&slot.reported, &filaments);
            if choices.len() != 1 || choices.first() != slot.filament_id.as_ref() {
                slot.filament_id = None;
                slot.mapping_source = "unassigned".into();
                slot.revision += 1;
                save(c, &slot)?;
            }
        }
    }
    Ok(())
}
impl Database {
    pub(crate) fn resolve_slots(
        &self,
        printer: &str,
        filament: &str,
        machine: &str,
    ) -> Result<Vec<AmsSlot>> {
        resolve(&*self.connection()?, printer, filament, machine)
    }
    pub(crate) fn prioritize_slots(
        &self,
        printer: &str,
        filament: &str,
        machine: &str,
        order: &[SlotRevision],
    ) -> Result<()> {
        let mut c = self.connection()?;
        let tx = c.transaction()?;
        let candidates = resolve(&tx, printer, filament, machine)?;
        let ids: std::collections::BTreeSet<_> = order.iter().map(|s| &s.id).collect();
        if candidates.len() < 2
            || candidates.len() != order.len()
            || ids.len() != order.len()
            || candidates.iter().any(|s| {
                !order
                    .iter()
                    .any(|o| o.id == s.id && o.revision == s.revision)
            })
        {
            return Err(Error::Conflict(
                "Loaded material or priority changed; refresh before reordering",
            ));
        }
        for (index, s) in order.iter().enumerate() {
            let rank = i64::try_from(index + 1).map_err(|_| Error::Invalid("Too many slots"))?;
            tx.execute(
                "UPDATE ams_slots SET priority_order=?1,revision=revision+1 WHERE id=?2",
                params![rank, s.id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn ams_slots(&self, printer_id: &str) -> Result<Vec<AmsSlot>> {
        slots(&*self.connection()?, printer_id)
    }
    pub(crate) fn observe_ams(&self, device: &Device, status: &Status) -> Result<()> {
        if !status.synchronized {
            return Ok(());
        }
        let mut c = self.connection()?;
        let tx = c.transaction()?;
        let printer_id = &device.id;
        let s = &device.settings;
        // Only the observer for the saved connection may replace this inventory.
        if !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM printers WHERE id=?1 AND host=?2 AND serial=?3 AND mqtt_port=?4 AND access_code=?5 AND tls_certificate=?6)",
            params![printer_id,s.host,s.serial,s.mqtt_port,s.access_code,s.tls_certificate],|r|r.get::<_,bool>(0)
        )? { return Err(Error::Conflict("Printer connection was replaced")); }
        let filaments = crate::database::load_filaments(&tx)?;
        let existing = slots(&tx, printer_id)?;
        let mut seen = std::collections::BTreeSet::new();
        if let Some(ams) = &status.ams {
            for unit in &ams.units {
                for tray in &unit.trays {
                    let old = existing
                        .iter()
                        .find(|s| s.ams_id == unit.id && s.slot_index == tray.id);
                    let mut current = old.cloned().unwrap_or_else(|| AmsSlot {
                        id: uuid::Uuid::new_v4().to_string(),
                        printer_id: printer_id.into(),
                        ams_id: unit.id,
                        slot_index: tray.id,
                        filament_id: None,
                        mapping_source: "unassigned".into(),
                        reported: tray.clone(),
                        detect_on_insert: ams.detect_on_insert,
                        detect_on_power_up: ams.detect_on_power_up,
                        revision: 0,
                        load_order: None,
                        priority_order: 0,
                    });
                    seen.insert(current.id.clone());
                    let identity_changed =
                        old.is_none_or(|s| !filament::same_identity(&s.reported, tray));
                    if identity_changed || current.mapping_source != "manual" {
                        let choices = filament::candidates(tray, &filaments);
                        current.filament_id = if choices.len() == 1 {
                            Some(choices[0].clone())
                        } else {
                            None
                        };
                        current.mapping_source = if current.filament_id.is_some() {
                            "automatic"
                        } else {
                            "unassigned"
                        }
                        .into();
                    }
                    if tray.present == Some(true) && new_load(old, tray) {
                        current.load_order = Some(tx.query_row("SELECT COALESCE(MAX(load_order),0)+1 FROM ams_slots WHERE printer_id=?1",[printer_id],|r|r.get(0))?);
                        current.priority_order = tx.query_row("SELECT COALESCE(MAX(priority_order),0)+1 FROM ams_slots WHERE printer_id=?1",[printer_id],|r|r.get(0))?;
                    }
                    let mapping_changed = old.is_none_or(|s| {
                        s.filament_id != current.filament_id
                            || s.mapping_source != current.mapping_source
                    });
                    if identity_changed || mapping_changed {
                        current.revision += 1;
                    }
                    if old.is_some_and(|s| {
                        s.reported == *tray
                            && s.detect_on_insert == ams.detect_on_insert
                            && s.detect_on_power_up == ams.detect_on_power_up
                    }) && !mapping_changed
                    {
                        continue;
                    }
                    current.reported = tray.clone();
                    current.detect_on_insert = ams.detect_on_insert;
                    current.detect_on_power_up = ams.detect_on_power_up;
                    save(&tx, &current)?;
                }
            }
        }
        for mut old in existing {
            if !seen.contains(&old.id)
                && (old.reported.present.is_some() || old.filament_id.is_some())
            {
                old.reported.present = None;
                old.filament_id = None;
                old.mapping_source = "unassigned".into();
                old.revision += 1;
                save(&tx, &old)?;
            }
        }
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn map_slot(
        &self,
        printer_id: &str,
        id: &str,
        revision: i64,
        filament_id: Option<&str>,
    ) -> Result<()> {
        let mut c = self.connection()?;
        let tx = c.transaction()?;
        let slot = slots(&tx, printer_id)?
            .into_iter()
            .find(|s| s.id == id)
            .ok_or(Error::NotFound)?;
        if slot.revision != revision
            || (filament_id.is_some() && slot.reported.present != Some(true))
        {
            return Err(Error::Conflict(
                "AMS observation changed; refresh before mapping",
            ));
        }
        // A manual empty choice remains empty until the reported identity changes.
        tx.execute("UPDATE ams_slots SET filament_id=?1,mapping_source='manual',revision=revision+1 WHERE id=?2",params![filament_id,id])?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "ams_tests.rs"]
mod tests;
