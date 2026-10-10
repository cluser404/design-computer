//! Organising the design tree: names shown for things, suppressing features, the rollback bar,
//! folders. Each is an ordinary command (`feature.set_label`, `feature.suppress`,
//! `document.rollback`, `folder.set`), so one undo step and an agent can do the same. Logic only.

use crate::state::AppState;
use dc_doc::{CommandKind, Query, QueryResult};

impl AppState {
    /// Gives a feature or sketch the name shown in the tree (empty goes back to its id).
    pub fn rename(&mut self, is_feature: bool, id: &str, label: &str) -> bool {
        let kind = if is_feature {
            CommandKind::FeatureSetLabel {
                feature: id.to_owned(),
                label: label.to_owned(),
            }
        } else {
            CommandKind::SketchSetLabel {
                sketch: id.to_owned(),
                label: label.to_owned(),
            }
        };
        self.submit(kind).is_ok()
    }

    /// Switches a feature (and what is built on it) off or on.
    pub fn set_suppressed(&mut self, feature: &str, suppressed: bool) -> bool {
        self.submit(CommandKind::FeatureSuppress {
            feature: feature.to_owned(),
            suppressed,
        })
        .is_ok()
    }

    /// Puts the rollback bar after a feature (`None`: at the end).
    pub fn set_rollback(&mut self, to: Option<&str>) -> bool {
        self.submit(CommandKind::DocumentRollback {
            to: to.map(str::to_owned),
        })
        .is_ok()
    }

    /// Moves a feature into a folder (made if new), or out of any folder when `folder` is `None`.
    pub fn move_to_folder(&mut self, feature: &str, folder: Option<&str>) -> bool {
        let folders = self.authority.state().folders();
        match folder {
            Some(name) => {
                let mut members = folders
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, m)| m.clone())
                    .unwrap_or_default();
                if !members.iter().any(|m| m == feature) {
                    members.push(feature.to_owned());
                }
                self.submit(CommandKind::FolderSet {
                    folder: name.to_owned(),
                    features: members,
                })
                .is_ok()
            }
            None => {
                let Some((name, members)) = folders
                    .into_iter()
                    .find(|(_, m)| m.iter().any(|x| x == feature))
                else {
                    return true;
                };
                self.submit(CommandKind::FolderSet {
                    folder: name,
                    features: members.into_iter().filter(|m| m != feature).collect(),
                })
                .is_ok()
            }
        }
    }

    /// A folder name not yet used.
    pub fn free_folder_name(&self) -> String {
        let used = self.authority.state().folders();
        (1u32..)
            .map(|n| format!("Folder{n}"))
            .find(|n| !used.iter().any(|(u, _)| u == n))
            .expect("a free name exists")
    }

    /// Exact mass properties of the model as it is now (volume, surface area, size), from the
    /// kernel, not from a mesh. `None` if there is no part.
    pub fn mass_properties(&self) -> Option<String> {
        let last = self.authority.state().result_features().last().cloned()?;
        let Ok(QueryResult::FeatureSummary { info }) = self
            .authority
            .query(&Query::FeatureSummary { feature: last })
        else {
            return None;
        };
        let s = info.summary;
        let size: Vec<String> = (0..3)
            .map(|i| format!("{:.3}", s.bounds_max[i] - s.bounds_min[i]))
            .collect();
        Some(format!(
            "Mass properties of '{}': volume {:.3} mm3, surface area {:.3} mm2, size {} mm, {} faces, {} edges",
            self.authority.state().label("feature", &info.id),
            s.volume,
            s.area,
            size.join(" x "),
            s.faces,
            s.edges
        ))
    }

    /// The last `n` committed commands, newest first, as `(revision, who, what)`.
    pub fn activity(&self, n: usize) -> Vec<(u64, String, String)> {
        self.authority
            .log()
            .iter()
            .rev()
            .take(n)
            .map(|e| {
                let json = serde_json::to_value(&e.command.kind).unwrap_or_default();
                let what = json
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("command")
                    .to_owned();
                (e.revision.0, e.command.actor.to_string(), what)
            })
            .collect()
    }
}
