//! The right-click menu in sketch mode, as in SolidWorks: right-clicking first selects what is
//! under the pointer (keeping an existing selection if it is part of it), then offers what makes
//! sense for it. Logic only, no egui: the UI draws the entries and calls [`AppState::sketch_context_do`].

use crate::dimensioning::Pick;
use crate::relations::relation_label;
use crate::sketching::{is_datum, Relation, Tool};
use crate::state::AppState;
use dc_sketch::Entity;

#[derive(Clone, Debug, PartialEq)]
pub enum MenuAction {
    Select,
    ClearSelection,
    SelectAll,
    SelectChain,
    SelectTangency,
    Delete,
    Construction,
    SmartDimension,
    EndChain,
    Relate(Relation),
    RemoveRelation(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct MenuItem {
    pub label: String,
    pub action: MenuAction,
    /// Shown with a tick (a setting that is on).
    pub checked: Option<bool>,
    pub shortcut: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MenuEntry {
    Item(MenuItem),
    Separator,
}

fn item(label: impl Into<String>, action: MenuAction, shortcut: &'static str) -> MenuEntry {
    MenuEntry::Item(MenuItem {
        label: label.into(),
        action,
        checked: None,
        shortcut,
    })
}

impl AppState {
    /// Right-click at a screen point: selects what is there (unless it is already selected) and
    /// returns the menu for the situation.
    pub fn sketch_context_open(&mut self, px: f64, py: f64, w: f64, h: f64) -> Vec<MenuEntry> {
        let Some(tool) = self.sketching.as_ref().map(|s| s.tool) else {
            return Vec::new();
        };
        if tool != Tool::Select {
            return self.drawing_menu(tool);
        }
        // Pick what is under the pointer; a click on something already selected keeps the set.
        if let Some(rel) = self.relation_at(px, py, w, h) {
            let already = self
                .sketching
                .as_ref()
                .is_some_and(|s| s.relations.contains(&rel));
            if !already {
                self.select_relation(Some(rel), false);
            }
        } else if let Some(id) = self.select_pick_at(px, py, w, h) {
            // A line's middle handle stands for the line itself in the menu.
            let id = id
                .strip_prefix(crate::sketching::MID_PREFIX)
                .map_or(id.clone(), str::to_owned);
            if let Some(s) = &mut self.sketching {
                if !s.selection.contains(&id) {
                    s.selection = vec![id];
                    s.relations.clear();
                }
            }
        }
        self.view_version += 1;
        self.select_menu()
    }

    fn drawing_menu(&self, tool: Tool) -> Vec<MenuEntry> {
        let mut out = Vec::new();
        let drawing = self
            .sketching
            .as_ref()
            .is_some_and(|s| s.first.is_some() || s.second.is_some());
        if tool == Tool::Line && drawing {
            out.push(item("End chain", MenuAction::EndChain, "double-click"));
        }
        out.push(item("Select", MenuAction::Select, "Esc"));
        out
    }

    fn select_menu(&self) -> Vec<MenuEntry> {
        let Some(s) = &self.sketching else {
            return Vec::new();
        };
        let Some(sk) = self.sketch_data() else {
            return Vec::new();
        };
        let entities: Vec<&String> = s.selection.iter().filter(|i| !is_datum(i)).collect();
        let mut out: Vec<MenuEntry> = Vec::new();
        let chainable = entities
            .iter()
            .any(|i| matches!(sk.entity(i), Some(Entity::Line { .. } | Entity::Arc { .. })));
        if chainable {
            out.push(item("Select Chain", MenuAction::SelectChain, ""));
            out.push(item("Select Tangency", MenuAction::SelectTangency, ""));
        }
        if entities.len() == 1 {
            out.push(item("Smart Dimension", MenuAction::SmartDimension, "D"));
        }
        let shapes: Vec<&&String> = entities
            .iter()
            .filter(|i| !matches!(sk.entity(i), Some(Entity::Point { .. })))
            .collect();
        if !shapes.is_empty() {
            let all = shapes.iter().all(|i| sk.is_construction(i));
            out.push(MenuEntry::Item(MenuItem {
                label: "Construction Geometry".into(),
                action: MenuAction::Construction,
                checked: Some(all),
                shortcut: "X",
            }));
        }
        if !entities.is_empty() || !s.relations.is_empty() {
            let n = entities.len() + s.relations.len();
            let label = if n > 1 {
                format!("Delete ({n})")
            } else {
                "Delete".to_owned()
            };
            out.push(item(label, MenuAction::Delete, "Del"));
        }
        // Relations that can be added to the selection.
        let addable = self.available_relations();
        if !addable.is_empty() {
            out.push(MenuEntry::Separator);
            for r in addable {
                out.push(item(
                    format!("Add relation: {}", r.name()),
                    MenuAction::Relate(r),
                    "",
                ));
            }
        }
        // Relations and dimensions on a single selected thing, to delete one by one.
        if let [one] = entities.as_slice() {
            let mut touching: Vec<(String, String)> = sk
                .constraints()
                .iter()
                .filter(|(_, c)| c.refs().contains(&one.as_str()))
                .map(|(id, c)| (id.clone(), relation_label(c)))
                .collect();
            touching.truncate(8);
            if !touching.is_empty() {
                out.push(MenuEntry::Separator);
                for (id, label) in touching {
                    out.push(item(
                        format!("Delete relation: {label}"),
                        MenuAction::RemoveRelation(id),
                        "",
                    ));
                }
            }
        }
        if !out.is_empty() {
            out.push(MenuEntry::Separator);
        }
        out.push(item("Select All", MenuAction::SelectAll, "Ctrl+A"));
        if !s.selection.is_empty() || !s.relations.is_empty() {
            out.push(item("Clear Selections", MenuAction::ClearSelection, "Esc"));
        }
        out
    }

    /// Carries out a menu entry.
    pub fn sketch_context_do(&mut self, action: &MenuAction) -> bool {
        match action {
            MenuAction::Select => {
                self.set_tool(Tool::Select);
                true
            }
            MenuAction::ClearSelection => {
                if let Some(s) = &mut self.sketching {
                    s.selection.clear();
                    s.relations.clear();
                }
                self.view_version += 1;
                true
            }
            MenuAction::SelectAll => {
                self.sketch_select_all();
                true
            }
            MenuAction::SelectChain => {
                let ids: Vec<String> = self
                    .sketching
                    .as_ref()
                    .map(|s| s.selection.clone())
                    .unwrap_or_default();
                let mut any = false;
                for (i, id) in ids.iter().filter(|i| !is_datum(i)).enumerate() {
                    any |= self.sketch_select_chain(id, i > 0) > 0;
                }
                any
            }
            MenuAction::SelectTangency => {
                let ids: Vec<String> = self
                    .sketching
                    .as_ref()
                    .map(|s| s.selection.clone())
                    .unwrap_or_default();
                let mut any = false;
                for (i, id) in ids.iter().filter(|i| !is_datum(i)).enumerate() {
                    any |= self.sketch_select_tangency(id, i > 0) > 0;
                }
                any
            }
            MenuAction::Delete => self.sketch_delete_selected(),
            MenuAction::Construction => self.toggle_construction(),
            MenuAction::SmartDimension => {
                let id = self
                    .sketching
                    .as_ref()
                    .and_then(|s| s.selection.iter().find(|i| !is_datum(i)).cloned());
                let Some(id) = id else { return false };
                self.set_tool(Tool::Dimension);
                if let Some(s) = &mut self.sketching {
                    s.dim.picks = vec![Pick::Entity(id)];
                }
                true
            }
            MenuAction::EndChain => {
                if let Some(s) = &mut self.sketching {
                    s.first = None;
                    s.second = None;
                }
                self.view_version += 1;
                true
            }
            MenuAction::Relate(r) => self.apply_relation(*r),
            MenuAction::RemoveRelation(id) => {
                let sketch = self.sketching.as_ref().map(|s| s.sketch.clone());
                sketch.is_some_and(|sk| self.remove_constraint(&sk, id))
            }
        }
    }
}
