//! Whole-sketch tools: fully define, repair, check for problems, trim away inside / outside,
//! re-attach the sketch to another plane, turn and flip the view, hide the bodies, and undo
//! while drawing. Logic only, no egui.

use crate::sketching::{Anchor, Tool};
use crate::state::AppState;
use dc_doc::CommandKind;
use dc_sketch::PlaneSpec;
use std::collections::BTreeSet;

/// A problem the checker found: what it is and which entities it is about.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckProblem {
    pub message: String,
    pub entities: Vec<String>,
}

impl AppState {
    fn session_sketch_name(&self) -> Option<String> {
        self.sketching.as_ref().map(|s| s.sketch.clone())
    }

    /// Adds the relations and dimensions the sketch still needs to be fully defined.
    pub fn sketch_fully_define(&mut self) -> bool {
        let Some(sketch) = self.session_sketch_name() else {
            return false;
        };
        let before: BTreeSet<String> = self
            .sketch_data()
            .map(|sk| sk.constraints().keys().cloned().collect())
            .unwrap_or_default();
        if self
            .submit(CommandKind::SketchFullyDefine { sketch })
            .is_err()
        {
            return false;
        }
        let (added, dof) = self
            .sketch_data()
            .map(|sk| {
                (
                    sk.constraints()
                        .keys()
                        .filter(|k| !before.contains(*k))
                        .count(),
                    self.authority
                        .state()
                        .sketch(&self.session_sketch_name().unwrap_or_default())
                        .map_or(0, |s| s.report.dof),
                )
            })
            .unwrap_or((0, 0));
        self.info(if added == 0 {
            "nothing to add: the sketch is as defined as it can be".to_owned()
        } else if dof == 0 {
            format!("added {added} relation(s) and dimension(s): the sketch is fully defined")
        } else {
            format!(
                "added {added} relation(s) and dimension(s); {dof} degree(s) of freedom are left"
            )
        });
        true
    }

    /// Merges points in the same place and removes empty and repeated things.
    pub fn sketch_repair(&mut self) -> bool {
        let Some(sketch) = self.session_sketch_name() else {
            return false;
        };
        let before = self.sketch_data().cloned();
        if self.submit(CommandKind::SketchRepair { sketch }).is_err() {
            return false;
        }
        let changed = match (&before, self.sketch_data()) {
            (Some(b), Some(a)) => b != a,
            _ => false,
        };
        self.info(if changed {
            "repaired the sketch".to_owned()
        } else {
            "nothing needed repairing".to_owned()
        });
        true
    }

    /// Looks for open contours, branches, crossings and the like; remembers them so they are
    /// shown on the sketch, and returns them.
    pub fn sketch_check(&mut self) -> Vec<CheckProblem> {
        let problems: Vec<CheckProblem> = self
            .sketch_data()
            .map(|sk| {
                sk.profiles()
                    .problems
                    .into_iter()
                    .map(|p| CheckProblem {
                        message: p.message,
                        entities: p.entities,
                    })
                    .collect()
            })
            .unwrap_or_default();
        if let Some(s) = &mut self.sketching {
            s.check = Some(problems.clone());
        }
        if problems.is_empty() {
            self.info("no problems: every contour is closed and clean".to_owned());
        } else {
            self.error(
                format!(
                    "{} problem(s) found; they are marked on the sketch",
                    problems.len()
                ),
                Some("open ends and branches stop a profile from being extruded".into()),
            );
        }
        self.view_version += 1;
        problems
    }

    /// Trim away inside / outside the two bounding shapes picked in the Trim tool's
    /// "Between bounds" mode: everything they both cross loses the part between them (or beyond).
    pub fn sketch_trim_between(&mut self, inside: bool) -> bool {
        let (Some(sketch), bounds, construction) = (
            self.session_sketch_name(),
            self.sketching
                .as_ref()
                .map(|s| s.trim_bounds.clone())
                .unwrap_or_default(),
            self.sketching.as_ref().is_some_and(|s| s.cut_construction),
        ) else {
            return false;
        };
        if bounds.len() != 2 {
            self.error("pick the two bounding shapes first", None);
            return false;
        }
        let ok = self
            .submit(CommandKind::SketchTrimBetween {
                sketch,
                bounds,
                inside,
                construction,
            })
            .is_ok();
        if ok {
            if let Some(s) = &mut self.sketching {
                s.trim_bounds.clear();
                s.selection.clear();
            }
        }
        ok
    }

    /// Puts the sketch on another plane and turns the view to face it.
    pub fn sketch_replane(&mut self, plane: PlaneSpec) -> bool {
        let Some(sketch) = self.session_sketch_name() else {
            return false;
        };
        if self
            .submit(CommandKind::SketchSetPlane {
                sketch: sketch.clone(),
                plane,
            })
            .is_err()
        {
            return false;
        }
        // The session was built for the old plane: come back in on the new one.
        self.end_sketch();
        self.edit_sketch(&sketch)
    }

    /// Square-on to the sketch again, after the view was turned or flipped.
    pub fn sketch_view_normal(&mut self) {
        let Some(frame) = self.sketching.as_ref().map(|s| s.frame.clone()) else {
            return;
        };
        self.camera = self.camera_facing(&frame);
        self.auto_fit = false;
        self.view_version += 1;
    }

    /// Turns the view about the sketch's normal by quarter turns (the sketch stays flat to you).
    pub fn sketch_view_rotate(&mut self, quarter_turns: i32) {
        if self.sketching.is_some() {
            self.camera.yaw += f64::from(quarter_turns) * core::f64::consts::FRAC_PI_2;
            self.view_version += 1;
        }
    }

    /// Looks at the sketch from the other side.
    pub fn sketch_view_flip(&mut self) {
        if self.sketching.is_some() {
            self.camera.pitch = -self.camera.pitch;
            self.camera.yaw += core::f64::consts::PI;
            self.view_version += 1;
        }
    }

    pub fn set_hide_bodies(&mut self, hide: bool) {
        self.hide_bodies = hide;
        self.refresh_scene();
        self.view_version += 1;
    }

    /// Ctrl+Z while a shape is half drawn: takes back the last segment of a line chain, or
    /// abandons the shape in progress, instead of undoing something already finished. Returns
    /// whether it did anything (otherwise the caller undoes normally).
    pub fn sketch_undo_in_progress(&mut self) -> bool {
        let Some(s) = &mut self.sketching else {
            return false;
        };
        if s.first.is_none() {
            return false;
        }
        let chain = matches!(s.tool, Tool::Line | Tool::Centerline);
        if chain {
            if let Some(prev) = s.chain.pop() {
                s.first = Some(prev);
                s.second = None;
                // The segment just drawn was the latest command of this sketch.
                let _ = self.undo();
                if let Some(s) = &mut self.sketching {
                    s.cursor = None;
                }
                self.view_version += 1;
                return true;
            }
        }
        s.clear_progress_public();
        self.view_version += 1;
        true
    }

    /// Remembers where a line chain was, for undoing a segment while drawing.
    pub(crate) fn remember_chain_point(&mut self, a: Anchor) {
        if let Some(s) = &mut self.sketching {
            s.chain.push(a);
        }
    }
}
