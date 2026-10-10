//! The small toolbar that appears next to the selection in the 3D view (SolidWorks' context
//! toolbar): the few things one most likely wants to do with what is selected. Logic only.

use crate::state::{AppState, Selected};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextAction {
    SketchOnFace,
    EditFeature(String),
    Suppress(String),
    Unsuppress(String),
    EditSketch(String),
    Rename(bool, String),
    Delete,
}

impl ContextAction {
    pub fn label(&self) -> &'static str {
        match self {
            ContextAction::SketchOnFace => "Sketch on face",
            ContextAction::EditFeature(_) => "Edit feature",
            ContextAction::Suppress(_) => "Suppress",
            ContextAction::Unsuppress(_) => "Unsuppress",
            ContextAction::EditSketch(_) => "Edit sketch",
            ContextAction::Rename(..) => "Rename",
            ContextAction::Delete => "Delete",
        }
    }
}

/// What the toolbar offers for the current selection (nothing while sketching).
pub fn actions(app: &AppState) -> Vec<ContextAction> {
    if app.sketching.is_some() || app.feature_draft.is_some() {
        return Vec::new();
    }
    let state = app.authority.state();
    match &app.selected {
        Selected::None => Vec::new(),
        Selected::Face { feature, .. } => {
            let mut v = vec![ContextAction::SketchOnFace];
            v.extend(feature_actions(app, feature));
            v
        }
        Selected::Feature(f) => feature_actions(app, f),
        Selected::Edge { feature, .. } => feature_actions(app, feature),
        Selected::SketchEntity { sketch, .. } => vec![
            ContextAction::EditSketch(sketch.clone()),
            ContextAction::Rename(false, sketch.clone()),
        ],
        Selected::Sketch(s) => {
            let _ = state;
            vec![
                ContextAction::EditSketch(s.clone()),
                ContextAction::Rename(false, s.clone()),
                ContextAction::Delete,
            ]
        }
    }
}

fn feature_actions(app: &AppState, f: &str) -> Vec<ContextAction> {
    let state = app.authority.state();
    let mut v = Vec::new();
    if !state.hidden_features().contains(f) {
        v.push(ContextAction::EditFeature(f.to_owned()));
    }
    v.push(if state.suppressed().contains(f) {
        ContextAction::Unsuppress(f.to_owned())
    } else {
        ContextAction::Suppress(f.to_owned())
    });
    v.push(ContextAction::Rename(true, f.to_owned()));
    v.push(ContextAction::Delete);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_toolbar_follows_the_selection_and_stays_away_while_working() {
        let mut a = AppState::new();
        a.load_demo();
        assert!(actions(&a).is_empty(), "nothing selected");
        let f = a.authority.state().ordered_features()[0].clone();
        a.select(Selected::Feature(f.clone()));
        let v = actions(&a);
        assert_eq!(v[0], ContextAction::EditFeature(f.clone()));
        assert!(v.contains(&ContextAction::Suppress(f.clone())));
        assert_eq!(v.last(), Some(&ContextAction::Delete));
        a.set_suppressed(&f, true);
        let v = actions(&a);
        assert!(v.contains(&ContextAction::Unsuppress(f.clone())));
        assert!(
            !v.contains(&ContextAction::EditFeature(f.clone())),
            "off features cannot be edited"
        );
        a.set_suppressed(&f, false);
        let face = a.faces_of(&f)[0].name.clone();
        a.select(Selected::Face {
            feature: f.clone(),
            face,
        });
        assert_eq!(actions(&a)[0], ContextAction::SketchOnFace);
        let s = a.sketch_rows()[0].id.clone();
        a.select(Selected::Sketch(s.clone()));
        assert_eq!(actions(&a)[0], ContextAction::EditSketch(s));
        // A feature page open: no toolbar.
        a.select(Selected::Feature(f));
        a.begin_feature(crate::featuring::FeatureKind::Extrude);
        assert!(actions(&a).is_empty());
    }
}
