use crate::{ams::AmsSlot, filament::Filament};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Default, Deserialize)]
pub(crate) struct Query {
    #[serde(default)]
    pub q: String,
    pub machine: Option<String>,
    pub selected_id: Option<String>,
    #[serde(default)]
    pub include_unloaded: bool,
}
pub(crate) struct Inventory {
    pub id: String,
    pub name: String,
    pub machine: String,
    pub current: Option<bool>,
    pub slots: Vec<AmsSlot>,
}
#[derive(Serialize)]
pub(crate) struct PrinterState {
    id: String,
    name: String,
    state: &'static str,
    unassigned: bool,
}
#[derive(Serialize)]
pub(crate) struct Candidates {
    pub filaments: Vec<Filament>,
    selected: Option<Filament>,
    selected_state: &'static str,
    printers: Vec<PrinterState>,
}
pub(crate) fn candidates(
    materials: Vec<Filament>,
    inventories: Vec<Inventory>,
    query: &Query,
) -> Candidates {
    let registered: BTreeSet<_> = materials.iter().map(|f| f.id.as_str()).collect();
    let mut loaded_ids = BTreeSet::new();
    let printers: Vec<_> = inventories
        .into_iter()
        .filter(|p| query.machine.as_ref().is_none_or(|m| m == &p.machine))
        .map(|p| {
            let mut unassigned = false;
            if p.current == Some(true) {
                for slot in &p.slots {
                    if slot.reported.present != Some(true) {
                        continue;
                    }
                    if let Some(id) = &slot.filament_id
                        && registered.contains(id.as_str())
                    {
                        loaded_ids.insert(id.clone());
                    } else {
                        unassigned = true;
                    }
                }
            }
            let state = match p.current {
                None => "error",
                Some(false) => "unconfirmed",
                Some(true) if p.slots.iter().any(|s| s.reported.present.is_none()) => "unconfirmed",
                Some(true) => "current",
            };
            PrinterState {
                id: p.id,
                name: p.name,
                state,
                unassigned,
            }
        })
        .collect();
    let selected = materials
        .iter()
        .find(|f| Some(&f.id) == query.selected_id.as_ref())
        .cloned();
    let selected_state = match &query.selected_id {
        None => "unset",
        Some(_) if selected.is_none() => "missing",
        Some(id) if loaded_ids.contains(id) => "loaded",
        Some(_) if printers.iter().any(|p| p.state != "current") => "unconfirmed",
        Some(_) => "unloaded",
    };
    let filaments = rank(
        materials
            .into_iter()
            .filter(|f| query.include_unloaded || loaded_ids.contains(&f.id))
            .collect(),
        &query.q,
    );
    Candidates {
        filaments,
        selected,
        selected_state,
        printers,
    }
}

// The all-material API and plate picker share the existing fuzzy ordering.
pub(crate) fn rank(materials: Vec<Filament>, query: &str) -> Vec<Filament> {
    let mut ranked: Vec<_> = materials
        .into_iter()
        .filter_map(|f| {
            let text = format!(
                "{} {} {} {}",
                f.data.name, f.data.vendor, f.data.material, f.data.color
            );
            crate::search::score(query.trim(), &text).map(|score| (score, f))
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.data.name.cmp(&b.1.data.name))
            .then_with(|| a.1.id.cmp(&b.1.id))
    });
    ranked.into_iter().map(|(_, f)| f).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{filament::FilamentData, printer_state::Tray};
    fn materials() -> Vec<Filament> {
        [
            ("white", "PLA 白", "FFFFFFFF"),
            ("blue", "PLA 青", "00FFFFFF"),
            ("future", "PETG-GF 黒", "000000FF"),
        ]
        .map(|(id, name, color)| Filament {
            id: id.into(),
            data: FilamentData {
                name: name.into(),
                vendor: "Bambu Lab".into(),
                material: if id == "future" { "PETG-GF" } else { "PLA" }.into(),
                color: color.into(),
                bambu_filament_id: None,
            },
        })
        .into()
    }
    fn slot(id: Option<&str>, present: Option<bool>) -> AmsSlot {
        AmsSlot {
            id: "slot".into(),
            printer_id: "printer".into(),
            ams_id: 7,
            slot_index: 5,
            filament_id: id.map(str::to_owned),
            mapping_source: "manual".into(),
            reported: Tray {
                present,
                ..Tray::default()
            },
            detect_on_insert: None,
            detect_on_power_up: None,
            revision: 1,
            load_order: None,
            priority_order: 0,
        }
    }
    fn inventory(machine: &str, current: Option<bool>, slots: Vec<AmsSlot>) -> Inventory {
        Inventory {
            id: machine.into(),
            name: machine.into(),
            machine: machine.into(),
            current,
            slots,
        }
    }
    fn ids(c: &Candidates) -> Vec<&str> {
        c.filaments.iter().map(|f| f.id.as_str()).collect()
    }
    #[test]
    fn matching_printers_union_current_mappings_without_count_or_remaining_assumptions() {
        let snapshots = || {
            vec![
                inventory(
                    "p1",
                    Some(true),
                    vec![
                        slot(Some("white"), Some(true)),
                        slot(Some("white"), Some(true)),
                        slot(Some("future"), Some(false)),
                        slot(None, Some(true)),
                    ],
                ),
                inventory(
                    "p1",
                    Some(true),
                    vec![
                        slot(Some("blue"), Some(true)),
                        slot(Some("deleted"), Some(true)),
                    ],
                ),
                inventory("mini", Some(true), vec![slot(Some("future"), Some(true))]),
                inventory("p1", Some(false), vec![slot(Some("future"), Some(true))]),
                inventory("p1", None, vec![slot(Some("future"), Some(true))]),
            ]
        };
        let result = candidates(
            materials(),
            snapshots(),
            &Query {
                machine: Some("p1".into()),
                ..Query::default()
            },
        );
        assert_eq!(ids(&result), vec!["white", "blue"]);
        assert_eq!(result.printers.len(), 4);
        assert!(result.printers[0].unassigned);
        assert_eq!(result.printers[2].state, "unconfirmed");
        assert_eq!(result.printers[3].state, "error");
        assert_eq!(
            candidates(materials(), snapshots(), &Query::default())
                .filaments
                .len(),
            3
        );
    }
    #[test]
    fn search_all_opt_in_and_selection_state_are_independent_of_visible_options() {
        let snapshots = || {
            vec![inventory(
                "p1",
                Some(true),
                vec![slot(Some("white"), Some(true))],
            )]
        };
        let mut q = Query {
            q: "bmb gf".into(),
            selected_id: Some("future".into()),
            ..Query::default()
        };
        let result = candidates(materials(), snapshots(), &q);
        assert!(result.filaments.is_empty());
        assert_eq!(result.selected.as_ref().unwrap().id, "future");
        assert_eq!(result.selected_state, "unloaded");
        q.include_unloaded = true;
        assert_eq!(
            ids(&candidates(materials(), snapshots(), &q)),
            vec!["future"]
        );
        q.q = "FFFFFFFF".into();
        assert_eq!(
            ids(&candidates(materials(), snapshots(), &q)),
            vec!["white"]
        );
        q.selected_id = Some("white".into());
        assert_eq!(
            candidates(materials(), snapshots(), &q).selected_state,
            "loaded"
        );
        q.selected_id = Some("deleted".into());
        let missing = candidates(materials(), snapshots(), &q);
        assert_eq!(missing.selected_state, "missing");
        assert!(missing.selected.is_none());
        q.selected_id = None;
        assert_eq!(
            candidates(materials(), snapshots(), &q).selected_state,
            "unset"
        );
    }
    #[test]
    fn unknown_inventory_is_not_empty_and_never_widens_search() {
        for current in [None, Some(false), Some(true)] {
            let q = Query {
                selected_id: Some("white".into()),
                ..Query::default()
            };
            let result = candidates(
                materials(),
                vec![inventory("p1", current, vec![slot(Some("white"), None)])],
                &q,
            );
            assert!(result.filaments.is_empty());
            assert_eq!(result.selected_state, "unconfirmed");
        }
        let q = Query {
            selected_id: Some("white".into()),
            ..Query::default()
        };
        assert_eq!(
            candidates(materials(), vec![], &q).selected_state,
            "unloaded"
        );
    }
}
