//! The Modify tools (move / copy, rotate, scale, mirror, patterns, offset, fillet, chamfer) as
//! the app uses them: from the current selection and a few numbers, to one logged command, with
//! a preview of what it would make. Logic only, no egui.

use crate::sketching::{defining_points, is_datum, P2};
use crate::state::AppState;
use dc_doc::CommandKind;
use dc_math::det;
use dc_sketch::{Entity, MirrorAbout, Pattern, Sketch, Transform};
use std::collections::BTreeSet;

/// One modify operation with its numbers.
#[derive(Clone, Debug, PartialEq)]
pub enum ModifyOp {
    Move {
        dx: f64,
        dy: f64,
        copy: bool,
    },
    Rotate {
        cx: f64,
        cy: f64,
        degrees: f64,
        copy: bool,
    },
    Scale {
        cx: f64,
        cy: f64,
        factor: f64,
        copy: bool,
    },
    MirrorAxisX,
    MirrorAxisY,
    /// Mirror the selection about the line selected last.
    MirrorSelectedLine,
    LinearPattern {
        dx: f64,
        dy: f64,
        count: u32,
    },
    CircularPattern {
        cx: f64,
        cy: f64,
        degrees: f64,
        count: u32,
    },
    Offset {
        distance: f64,
    },
    Fillet {
        radius: f64,
    },
    Chamfer {
        d1: f64,
        d2: f64,
    },
}

impl ModifyOp {
    pub fn title(&self) -> &'static str {
        match self {
            ModifyOp::Move { copy: false, .. } => "Move",
            ModifyOp::Move { .. } => "Copy",
            ModifyOp::Rotate { .. } => "Rotate",
            ModifyOp::Scale { .. } => "Scale",
            ModifyOp::MirrorAxisX | ModifyOp::MirrorAxisY | ModifyOp::MirrorSelectedLine => {
                "Mirror"
            }
            ModifyOp::LinearPattern { .. } => "Linear pattern",
            ModifyOp::CircularPattern { .. } => "Circular pattern",
            ModifyOp::Offset { .. } => "Offset",
            ModifyOp::Fillet { .. } => "Fillet",
            ModifyOp::Chamfer { .. } => "Chamfer",
        }
    }
}

/// Runs the same engine operation a command would, on a sketch (for previews).
fn apply_to(sk: &mut Sketch, cmd: &CommandKind) -> Result<(), String> {
    let r = match cmd {
        CommandKind::SketchTransform {
            entities,
            transform,
            copy,
            ..
        } => sk.transform(entities, transform, *copy).map(|_| ()),
        CommandKind::SketchMirror {
            entities, about, ..
        } => sk.mirror(entities, about).map(|_| ()),
        CommandKind::SketchPattern {
            entities, pattern, ..
        } => sk.pattern(entities, pattern).map(|_| ()),
        CommandKind::SketchOffset {
            entities, distance, ..
        } => sk.offset(entities, *distance).map(|_| ()),
        CommandKind::SketchFillet {
            first,
            second,
            radius,
            ..
        } => sk.fillet(first, second, *radius).map(|_| ()),
        CommandKind::SketchChamfer {
            first,
            second,
            distance,
            second_distance,
            ..
        } => sk
            .chamfer(
                first,
                second,
                *distance,
                second_distance.unwrap_or(*distance),
            )
            .map(|_| ()),
        _ => Ok(()),
    };
    r.map_err(|e| e.message)
}

impl AppState {
    /// The shapes (and stand-alone points) selected, without the origin and axes.
    pub fn selected_entities(&self) -> Vec<String> {
        self.sketching
            .as_ref()
            .map(|s| {
                s.selection
                    .iter()
                    .filter(|i| !is_datum(i))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The command that does `op` to the current selection, or why it cannot.
    pub fn modify_command(&self, op: &ModifyOp) -> Result<CommandKind, String> {
        let sketch = self
            .sketching
            .as_ref()
            .map(|s| s.sketch.clone())
            .ok_or("not in a sketch")?;
        let ids = self.selected_entities();
        let sk = self.sketch_data().ok_or("not in a sketch")?;
        let need = |what: &str| -> Result<(), String> {
            if ids.is_empty() {
                Err(format!("select {what} first"))
            } else {
                Ok(())
            }
        };
        let lines: Vec<&String> = ids
            .iter()
            .filter(|i| matches!(sk.entity(i), Some(Entity::Line { .. })))
            .collect();
        Ok(match op {
            ModifyOp::Move { dx, dy, copy } => {
                need("the shapes to move")?;
                CommandKind::SketchTransform {
                    sketch,
                    entities: ids,
                    transform: Transform::Move { dx: *dx, dy: *dy },
                    copy: *copy,
                }
            }
            ModifyOp::Rotate {
                cx,
                cy,
                degrees,
                copy,
            } => {
                need("the shapes to rotate")?;
                CommandKind::SketchTransform {
                    sketch,
                    entities: ids,
                    transform: Transform::Rotate {
                        cx: *cx,
                        cy: *cy,
                        degrees: *degrees,
                    },
                    copy: *copy,
                }
            }
            ModifyOp::Scale {
                cx,
                cy,
                factor,
                copy,
            } => {
                need("the shapes to scale")?;
                CommandKind::SketchTransform {
                    sketch,
                    entities: ids,
                    transform: Transform::Scale {
                        cx: *cx,
                        cy: *cy,
                        factor: *factor,
                    },
                    copy: *copy,
                }
            }
            ModifyOp::MirrorAxisX | ModifyOp::MirrorAxisY => {
                need("the shapes to mirror")?;
                CommandKind::SketchMirror {
                    sketch,
                    entities: ids,
                    about: if matches!(op, ModifyOp::MirrorAxisX) {
                        MirrorAbout::AxisX
                    } else {
                        MirrorAbout::AxisY
                    },
                }
            }
            ModifyOp::MirrorSelectedLine => {
                // The mirror line is the one selected last; everything before it is mirrored.
                let Some((last, rest)) = ids.split_last() else {
                    return Err("select the shapes to mirror, then the mirror line".into());
                };
                if !matches!(sk.entity(last), Some(Entity::Line { .. })) || rest.is_empty() {
                    return Err("select the shapes to mirror, then the mirror line last".into());
                }
                CommandKind::SketchMirror {
                    sketch,
                    entities: rest.to_vec(),
                    about: MirrorAbout::Line {
                        entity: last.clone(),
                    },
                }
            }
            ModifyOp::LinearPattern { dx, dy, count } => {
                need("the shapes to repeat")?;
                CommandKind::SketchPattern {
                    sketch,
                    entities: ids,
                    pattern: Pattern::Linear {
                        dx: *dx,
                        dy: *dy,
                        count: *count,
                    },
                }
            }
            ModifyOp::CircularPattern {
                cx,
                cy,
                degrees,
                count,
            } => {
                need("the shapes to repeat")?;
                CommandKind::SketchPattern {
                    sketch,
                    entities: ids,
                    pattern: Pattern::Circular {
                        cx: *cx,
                        cy: *cy,
                        degrees: *degrees,
                        count: *count,
                    },
                }
            }
            ModifyOp::Offset { distance } => {
                need("the lines (one chain), circles or arcs to offset")?;
                CommandKind::SketchOffset {
                    sketch,
                    entities: ids,
                    distance: *distance,
                }
            }
            ModifyOp::Fillet { radius } => {
                let [a, b] = lines.as_slice() else {
                    return Err("select the two lines that meet at the corner".into());
                };
                CommandKind::SketchFillet {
                    sketch,
                    first: (*a).clone(),
                    second: (*b).clone(),
                    radius: *radius,
                }
            }
            ModifyOp::Chamfer { d1, d2 } => {
                let [a, b] = lines.as_slice() else {
                    return Err("select the two lines that meet at the corner".into());
                };
                CommandKind::SketchChamfer {
                    sketch,
                    first: (*a).clone(),
                    second: (*b).clone(),
                    distance: *d1,
                    second_distance: Some(*d2),
                }
            }
        })
    }

    /// Does the modification as one command; the shapes it made become the selection.
    pub fn apply_modify(&mut self, op: &ModifyOp) -> bool {
        let cmd = match self.modify_command(op) {
            Ok(c) => c,
            Err(why) => {
                self.error(why, None);
                return false;
            }
        };
        let before: BTreeSet<String> = self
            .sketch_data()
            .map(|sk| sk.entities().keys().cloned().collect())
            .unwrap_or_default();
        if self.submit(cmd).is_err() {
            return false;
        }
        let made: Vec<String> = self
            .sketch_data()
            .map(|sk| {
                sk.entities()
                    .iter()
                    .filter(|(id, e)| !before.contains(*id) && !matches!(e, Entity::Point { .. }))
                    .map(|(id, _)| id.clone())
                    .collect()
            })
            .unwrap_or_default();
        if let Some(s) = &mut self.sketching {
            if !made.is_empty() {
                s.selection = made;
            }
        }
        self.view_version += 1;
        true
    }

    /// What `op` would add, as polylines (sketch coordinates), without changing anything.
    pub fn modify_preview(&self, op: &ModifyOp) -> Vec<Vec<P2>> {
        let Ok(cmd) = self.modify_command(op) else {
            return Vec::new();
        };
        let Some(sk) = self.sketch_data() else {
            return Vec::new();
        };
        let mut trial = sk.clone();
        if apply_to(&mut trial, &cmd).is_err() {
            return Vec::new();
        }
        let mut out: Vec<Vec<P2>> = Vec::new();
        let tau = core::f64::consts::TAU;
        for (id, e) in trial.entities() {
            let (new_or_changed, drawn) = (sk.entity(id) != Some(e), e);
            let moved = defining_points(drawn)
                .iter()
                .any(|p| trial.point(p) != sk.point(p));
            if !new_or_changed && !moved {
                continue;
            }
            match e {
                Entity::Line { a, b } => {
                    if let (Some(p), Some(q)) = (trial.point(a), trial.point(b)) {
                        out.push(vec![p, q]);
                    }
                }
                Entity::Circle { center, radius } => {
                    if let Some(c) = trial.point(center) {
                        out.push(
                            (0..=48)
                                .map(|i| {
                                    let t = tau * f64::from(i) / 48.0;
                                    (c.0 + radius * det::cos(t), c.1 + radius * det::sin(t))
                                })
                                .collect(),
                        );
                    }
                }
                Entity::Arc { center, start, end } => {
                    if let (Some(c), Some(s), Some(en)) =
                        (trial.point(center), trial.point(start), trial.point(end))
                    {
                        let r = det::hypot(s.0 - c.0, s.1 - c.1);
                        let a0 = det::atan2(s.1 - c.1, s.0 - c.0);
                        let mut sweep = (det::atan2(en.1 - c.1, en.0 - c.0) - a0) % tau;
                        if sweep < 0.0 {
                            sweep += tau;
                        }
                        out.push(
                            (0..=32)
                                .map(|i| {
                                    let t = a0 + sweep * f64::from(i) / 32.0;
                                    (c.0 + r * det::cos(t), c.1 + r * det::sin(t))
                                })
                                .collect(),
                        );
                    }
                }
                Entity::Point { .. } => {}
            }
        }
        out
    }

    /// The middle of the selected shapes, a sensible default centre for turning and scaling.
    pub fn selection_centre(&self) -> P2 {
        let Some(sk) = self.sketch_data() else {
            return (0.0, 0.0);
        };
        let mut pts: Vec<P2> = Vec::new();
        for id in self.selected_entities() {
            match sk.entity(&id) {
                Some(Entity::Point { x, y }) => pts.push((*x, *y)),
                Some(e) => pts.extend(defining_points(e).iter().filter_map(|p| sk.point(p))),
                None => {}
            }
        }
        if pts.is_empty() {
            return (0.0, 0.0);
        }
        let n = pts.len() as f64;
        (
            pts.iter().map(|p| p.0).sum::<f64>() / n,
            pts.iter().map(|p| p.1).sum::<f64>() / n,
        )
    }
}
