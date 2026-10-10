//! The Trim, Extend and Split tools: what the pointer is over, a preview of what a click would
//! do, the click itself (one logged command), and "power trim" (drag across shapes to trim them
//! all, as one step). Logic only, no egui.

use crate::sketching::{Tool, P2};
use crate::state::AppState;
use dc_doc::CommandKind;
use dc_math::det;
use dc_sketch::Sketch;

/// How near (screen points) the pointer must be to a shape for a tool to act on it.
const REACH_PX: f64 = 9.0;

/// How the Trim tool decides what to remove.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrimMode {
    /// The piece under the pointer, between the nearest cuts (SolidWorks "Trim to closest").
    #[default]
    Closest,
    /// Click two lines / arcs: both are trimmed or extended to the corner where they meet.
    Corner,
    /// Pick two bounding shapes first; then only those cut ("Trim away inside / outside").
    Between,
}

impl TrimMode {
    pub const ALL: [TrimMode; 3] = [TrimMode::Closest, TrimMode::Corner, TrimMode::Between];

    pub fn name(self) -> &'static str {
        match self {
            TrimMode::Closest => "To closest",
            TrimMode::Corner => "Corner",
            TrimMode::Between => "Between bounds",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            TrimMode::Closest => "Click the piece to remove, between the shapes that cut it",
            TrimMode::Corner => "Click a line or arc, then another: both go to the corner where they meet, keeping the sides you clicked",
            TrimMode::Between => "Click two bounding shapes, then click what to trim: only those two cut",
        }
    }
}

/// What the pointer is over for an editing tool, and what a click would do there.
#[derive(Clone, Debug, Default)]
pub struct EditHover {
    pub entity: String,
    /// Where the pointer is, in sketch coordinates (the position the command carries).
    pub at: P2,
    /// The parts that would be removed (trim), as sketch-space polylines.
    pub preview: Vec<Vec<P2>>,
    /// The parts that would be added (extend; a corner trim that lengthens).
    pub added: Vec<Vec<P2>>,
    /// Where a split would cut (two places for a circle).
    pub markers: Vec<P2>,
    /// Why the tool cannot act here, if it cannot.
    pub problem: Option<String>,
}

/// A power-trim stroke in progress.
#[derive(Clone, Debug)]
pub struct TrimStroke {
    /// The pointer's path, in screen points, for drawing the trail.
    pub trail: Vec<P2>,
    /// The sketch with every trim of the stroke applied so far (what the overlay shows).
    pub trial: Sketch,
    /// The trims, as commands, in order.
    pub ops: Vec<CommandKind>,
    /// Shapes already trimmed in this stroke (a stroke trims each shape once).
    done: Vec<String>,
}

impl AppState {
    /// Screen points per millimetre, to turn a pick radius into millimetres.
    fn reach_mm(&self, w: f64, h: f64) -> f64 {
        match (
            self.sketch_to_screen((0.0, 0.0), w, h),
            self.sketch_to_screen((1.0, 0.0), w, h),
        ) {
            (Some(a), Some(b)) => {
                let d = det::hypot(b.0 - a.0, b.1 - a.1).max(1e-9);
                REACH_PX / d
            }
            _ => 1.0,
        }
    }

    /// What cuts a shape for the Trim tool right now, from its settings.
    fn trim_options(&self) -> dc_sketch::edit::TrimOptions {
        self.sketching
            .as_ref()
            .map(|s| dc_sketch::edit::TrimOptions {
                construction: s.cut_construction,
                bounds: if s.trim_mode == TrimMode::Between && s.trim_bounds.len() == 2 {
                    s.trim_bounds.clone()
                } else {
                    Vec::new()
                },
            })
            .unwrap_or_default()
    }

    /// Switches the Trim tool's mode, forgetting half-made picks.
    pub fn set_trim_mode(&mut self, mode: TrimMode) {
        if let Some(s) = &mut self.sketching {
            s.trim_mode = mode;
            s.corner_pick = None;
            s.trim_bounds.clear();
            s.edit = None;
            s.selection.clear();
        }
        self.view_version += 1;
    }

    /// The editing tool's view of the pointer (called from `sketch_hover`).
    pub(crate) fn edit_hover(&mut self, at: Option<(f64, f64, f64, f64)>) {
        let hover = at.and_then(|(x, y, w, h)| self.edit_probe(x, y, w, h));
        if let Some(s) = &mut self.sketching {
            s.edit = hover;
            s.cursor = None;
        }
    }

    fn edit_probe(&self, px: f64, py: f64, w: f64, h: f64) -> Option<EditHover> {
        let s = self.sketching.as_ref()?;
        let tool = s.tool;
        let at = self.screen_to_sketch(px, py, w, h)?;
        let sk = self.view_sketch()?;
        let entity = sk.entity_near(at, self.reach_mm(w, h))?;
        let mut hover = EditHover {
            entity: entity.clone(),
            at,
            ..EditHover::default()
        };
        let construction = s.cut_construction;
        match tool {
            Tool::Trim => match s.trim_mode {
                TrimMode::Closest => {
                    match sk.trim_preview_with(&entity, at, &self.trim_options()) {
                        Ok(p) => hover.preview = vec![p],
                        Err(e) => hover.problem = Some(e.message),
                    }
                }
                TrimMode::Corner => {
                    if let Some((first, first_at)) = &s.corner_pick {
                        match sk.corner_preview(first, *first_at, &entity, at) {
                            Ok((removed, added)) => {
                                hover.preview = removed;
                                hover.added = added;
                            }
                            Err(e) => hover.problem = Some(e.message),
                        }
                    }
                }
                TrimMode::Between => {
                    if s.trim_bounds.len() == 2 {
                        match sk.trim_preview_with(&entity, at, &self.trim_options()) {
                            Ok(p) => hover.preview = vec![p],
                            Err(e) => hover.problem = Some(e.message),
                        }
                    }
                }
            },
            Tool::Extend => match sk.extend_preview_with(&entity, at, construction) {
                Ok(p) => hover.added = vec![p],
                Err(e) => hover.problem = Some(e.message),
            },
            Tool::Split => match sk.split_markers(&entity, at) {
                Ok(p) => hover.markers = p,
                Err(e) => hover.problem = Some(e.message),
            },
            _ => return None,
        }
        Some(hover)
    }

    /// A click with Trim, Extend or Split: does it where the pointer is. Returns whether it did.
    pub(crate) fn edit_click(&mut self, px: f64, py: f64, w: f64, h: f64) -> bool {
        let Some(hover) = self.edit_probe(px, py, w, h) else {
            return false;
        };
        if let Some(why) = &hover.problem {
            self.error(why.clone(), None);
            return false;
        }
        let (Some(tool), Some(sketch), Some(mode), construction) = (
            self.sketching.as_ref().map(|s| s.tool),
            self.sketching.as_ref().map(|s| s.sketch.clone()),
            self.sketching.as_ref().map(|s| s.trim_mode),
            self.sketching.as_ref().is_some_and(|s| s.cut_construction),
        ) else {
            return false;
        };
        let (entity_id, x, y) = (hover.entity, hover.at.0, hover.at.1);
        let kind = match tool {
            Tool::Trim => match mode {
                TrimMode::Closest => CommandKind::SketchTrim {
                    sketch,
                    entity_id,
                    x,
                    y,
                    construction,
                    bounds: Vec::new(),
                },
                TrimMode::Corner => {
                    let first = self.sketching.as_ref().and_then(|s| s.corner_pick.clone());
                    match first {
                        None => {
                            // The first shape: remember it and where it was clicked.
                            if let Some(s) = &mut self.sketching {
                                s.selection = vec![entity_id.clone()];
                                s.corner_pick = Some((entity_id, (x, y)));
                            }
                            self.view_version += 1;
                            return true;
                        }
                        Some((first, (fx, fy))) => CommandKind::SketchCornerTrim {
                            sketch,
                            first,
                            first_x: fx,
                            first_y: fy,
                            second: entity_id,
                            second_x: x,
                            second_y: y,
                        },
                    }
                }
                TrimMode::Between => {
                    let have = self.sketching.as_ref().map_or(0, |s| s.trim_bounds.len());
                    if have < 2 {
                        if let Some(s) = &mut self.sketching {
                            if !s.trim_bounds.contains(&entity_id) {
                                s.trim_bounds.push(entity_id);
                            }
                            s.selection = s.trim_bounds.clone();
                        }
                        self.view_version += 1;
                        return true;
                    }
                    CommandKind::SketchTrim {
                        sketch,
                        entity_id,
                        x,
                        y,
                        construction,
                        bounds: self
                            .sketching
                            .as_ref()
                            .map(|s| s.trim_bounds.clone())
                            .unwrap_or_default(),
                    }
                }
            },
            Tool::Extend => CommandKind::SketchExtend {
                sketch,
                entity_id,
                x,
                y,
                construction,
            },
            _ => CommandKind::SketchSplit {
                sketch,
                entity_id,
                x,
                y,
            },
        };
        let ok = self.submit(kind).is_ok();
        if let Some(s) = &mut self.sketching {
            s.edit = None;
            if ok {
                s.corner_pick = None;
                s.selection.clear();
            }
        }
        ok
    }

    // ---- power trim ---------------------------------------------------------------------------

    /// Starts a power-trim stroke (Trim tool): drag across shapes to trim each where it was crossed.
    pub fn trim_drag_begin(&mut self, px: f64, py: f64) -> bool {
        let Some(sk) = self.sketch_data().cloned() else {
            return false;
        };
        let Some(s) = &mut self.sketching else {
            return false;
        };
        if s.tool != Tool::Trim {
            return false;
        }
        s.edit = None;
        s.stroke = Some(TrimStroke {
            trail: vec![(px, py)],
            trial: sk,
            ops: Vec::new(),
            done: Vec::new(),
        });
        true
    }

    /// The pointer moved along the stroke: trim whatever it crossed since the last position.
    pub fn trim_drag_move(&mut self, px: f64, py: f64, w: f64, h: f64) {
        let reach = self.reach_mm(w, h);
        let Some(last) = self
            .sketching
            .as_ref()
            .and_then(|s| s.stroke.as_ref())
            .and_then(|t| t.trail.last().copied())
        else {
            return;
        };
        // Walk from the last position to this one in small steps, so a fast stroke cannot skip a shape.
        let n = (det::hypot(px - last.0, py - last.1) / 3.0).ceil().max(1.0) as u32;
        let mut samples: Vec<(P2, P2)> = Vec::new();
        for i in 1..=n {
            let t = f64::from(i) / f64::from(n);
            let p = (last.0 + (px - last.0) * t, last.1 + (py - last.1) * t);
            if let Some(at) = self.screen_to_sketch(p.0, p.1, w, h) {
                samples.push((p, at));
            }
        }
        let Some(s) = &mut self.sketching else { return };
        let sketch = s.sketch.clone();
        let Some(stroke) = s.stroke.as_mut() else {
            return;
        };
        for (p, at) in samples {
            stroke.trail.push(p);
            let Some(id) = stroke.trial.entity_near(at, reach) else {
                continue;
            };
            if stroke.done.contains(&id) {
                continue;
            }
            let before = stroke.trial.clone();
            match stroke.trial.trim(&id, at) {
                Ok(rep) => {
                    stroke.done.push(id.clone());
                    // Pieces made by this trim are new shapes: a stroke may cross those again.
                    stroke.done.retain(|d| !rep.created.contains(d));
                    stroke.ops.push(CommandKind::SketchTrim {
                        sketch: sketch.clone(),
                        entity_id: id,
                        x: at.0,
                        y: at.1,
                        construction: false,
                        bounds: Vec::new(),
                    });
                }
                Err(_) => stroke.trial = before,
            }
        }
        self.view_version += 1;
    }

    /// Lifting the button commits every trim of the stroke as one step.
    pub fn trim_drag_end(&mut self) -> bool {
        let Some(stroke) = self.sketching.as_mut().and_then(|s| s.stroke.take()) else {
            return false;
        };
        self.view_version += 1;
        match stroke.ops.len() {
            0 => false,
            1 => self.submit(stroke.ops[0].clone()).is_ok(),
            _ => self
                .submit(CommandKind::Transaction {
                    label: "Power trim".into(),
                    ops: stroke.ops,
                })
                .is_ok(),
        }
    }
}
