//! What the 3D view draws on top of the model to show what is hovered, selected or chosen for a
//! command: edges of parts and lines of sketches. Logic only (3D segments); the UI projects and
//! paints them.

use crate::state::{AppState, Hover, Selected};
use dc_math::Vec3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HighlightKind {
    /// Under the pointer.
    Hover,
    /// Selected.
    Selected,
    /// Chosen for the command being set up (an edge to round).
    Chosen,
    /// The axis of a revolve.
    Axis,
}

#[derive(Clone, Debug)]
pub struct Highlight {
    pub kind: HighlightKind,
    pub segments: Vec<[Vec3; 2]>,
}

fn edge_segments(app: &AppState, feature: &str, edge: &str) -> Vec<[Vec3; 2]> {
    app.pick_scene()
        .bodies
        .iter()
        .find(|b| b.id == feature)
        .map(|b| b.edge_segments(edge))
        .unwrap_or_default()
}

/// The edges round a face, for outlining it.
fn face_outline(app: &AppState, feature: &str, face: &str) -> Vec<[Vec3; 2]> {
    app.pick_scene()
        .bodies
        .iter()
        .find(|b| b.id == feature)
        .map(|b| {
            let Some(fi) = b.face_names.iter().position(|n| n == face) else {
                return Vec::new();
            };
            b.edges
                .iter()
                .zip(&b.edge_faces)
                .filter(|(_, f)| f[0] as usize == fi || f[1] as usize == fi)
                .map(|(e, _)| *e)
                .collect()
        })
        .unwrap_or_default()
}

fn line_segments(app: &AppState, sketch: &str, entity: &str) -> Vec<[Vec3; 2]> {
    let id = format!("sketch/{sketch}/{entity}");
    app.pick_scene()
        .lines
        .iter()
        .filter(|l| l.id == id)
        .flat_map(|l| l.points.windows(2).map(|w| [w[0], w[1]]))
        .collect()
}

/// Everything to draw, bottom layer first.
pub fn highlights(app: &AppState) -> Vec<Highlight> {
    let mut v = Vec::new();
    if let Some(d) = &app.feature_draft {
        if let Some(t) = &d.target {
            for [a, b] in &d.edges {
                let mut name = [a.clone(), b.clone()];
                name.sort();
                let segs = edge_segments(app, t, &name.join("|"));
                if !segs.is_empty() {
                    v.push(Highlight {
                        kind: HighlightKind::Chosen,
                        segments: segs,
                    });
                }
            }
        }
        if d.kind == crate::featuring::FeatureKind::Revolve {
            let segs = line_segments(app, &d.sketch, &d.axis);
            if !segs.is_empty() {
                v.push(Highlight {
                    kind: HighlightKind::Axis,
                    segments: segs,
                });
            }
        }
    }
    let primary = app.selected.clone();
    let selected: Vec<[Vec3; 2]> = app
        .selection()
        .iter()
        .flat_map(|s| match s {
            Selected::Edge { feature, edge } => edge_segments(app, feature, edge),
            Selected::SketchEntity { sketch, entity } => line_segments(app, sketch, entity),
            // The primary face is tinted; the others in a selection are outlined.
            Selected::Face { feature, face } if *s != primary => face_outline(app, feature, face),
            _ => Vec::new(),
        })
        .collect();
    if !selected.is_empty() {
        v.push(Highlight {
            kind: HighlightKind::Selected,
            segments: selected,
        });
    }
    let hover = match &app.hover_other {
        Some(Hover::Edge { feature, edge }) => edge_segments(app, feature, edge),
        Some(Hover::SketchEntity { sketch, entity }) => line_segments(app, sketch, entity),
        None => Vec::new(),
    };
    if !hover.is_empty() {
        v.push(Highlight {
            kind: HighlightKind::Hover,
            segments: hover,
        });
    }
    v
}
