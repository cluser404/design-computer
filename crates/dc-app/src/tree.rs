//! The model tree (SolidWorks' FeatureManager design tree) as data: the document, its three
//! planes and origin, then the features in build order with each sketch nested under the first
//! feature that uses it, then the sketches nothing uses yet. Logic only, no egui.

use crate::state::{AppState, Selected};
use crate::ui::{CadIcon, Icon};
use dc_sketch::{PlaneSpec, SolveStatus};

#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    Root,
    Plane(PlaneSpec),
    Origin,
    Feature(String),
    Sketch(String),
    Face {
        feature: String,
        face: String,
    },
    /// A named group of features (`folder.set`).
    Folder(String),
}

#[derive(Clone, Debug)]
pub struct Node {
    pub kind: NodeKind,
    pub label: String,
    /// A short, quieter text after the label ("cut of plate", "2 DOF").
    pub note: Option<String>,
    /// Something is wrong or open (shown in a warning colour).
    pub warn: bool,
    pub icon: Icon,
    pub children: Vec<Node>,
    /// Why a feature is off, if it is: suppressed, after the rollback bar, or built on such a one.
    pub off: Option<&'static str>,
    /// The rollback bar sits right after this node.
    pub bar_after: bool,
}

impl Node {
    fn leaf(kind: NodeKind, label: impl Into<String>, icon: Icon) -> Node {
        Node {
            kind,
            label: label.into(),
            note: None,
            warn: false,
            icon,
            children: Vec::new(),
            off: None,
            bar_after: false,
        }
    }

    /// A stable key for remembering whether the node is open.
    pub fn key(&self) -> String {
        match &self.kind {
            NodeKind::Root => "root".into(),
            NodeKind::Plane(p) => format!("plane:{p:?}"),
            NodeKind::Origin => "origin".into(),
            NodeKind::Feature(f) => format!("feature:{f}"),
            NodeKind::Sketch(s) => format!("sketch:{s}"),
            NodeKind::Face { feature, face } => format!("face:{feature}:{face}"),
            NodeKind::Folder(f) => format!("folder:{f}"),
        }
    }

    /// Is this node the current selection?
    pub fn is_selected(&self, sel: &Selected) -> bool {
        match (&self.kind, sel) {
            (NodeKind::Feature(a), Selected::Feature(b)) => a == b,
            (NodeKind::Sketch(a), Selected::Sketch(b)) => a == b,
            (
                NodeKind::Face { feature, face },
                Selected::Face {
                    feature: f,
                    face: n,
                },
            ) => feature == f && face == n,
            _ => false,
        }
    }

    /// Dropping a feature on this row moves it to: `Some(Some(folder))` into that folder,
    /// `Some(None)` out of any folder (the document row), `None` this row takes no drops.
    pub fn drop_folder(&self) -> Option<Option<String>> {
        match &self.kind {
            NodeKind::Folder(f) => Some(Some(f.clone())),
            NodeKind::Root => Some(None),
            _ => None,
        }
    }

    /// Every node below this one, depth first, this one included.
    pub fn walk(&self) -> Vec<&Node> {
        let mut v = vec![self];
        for c in &self.children {
            v.extend(c.walk());
        }
        v
    }
}

fn feature_icon(kind: &str) -> Icon {
    Icon::Cad(match kind {
        "cut" => CadIcon::ExtrudedCut,
        "boss" => CadIcon::Boss,
        "revolve" => CadIcon::ExtrudedBoss,
        "fillet" => CadIcon::Fillet,
        "chamfer" => CadIcon::Chamfer,
        _ => CadIcon::ExtrudedBoss,
    })
}

fn sketch_note(info: &dc_doc::SketchInfo) -> (String, bool) {
    if info.entities == 0 && info.status == SolveStatus::Solved {
        return ("empty".to_owned(), false);
    }
    match (info.status, info.fully_constrained) {
        (SolveStatus::Solved, true) => ("fully defined".to_owned(), false),
        (SolveStatus::Solved, false) => (format!("{} DOF", info.dof), true),
        _ => ("unsolved".to_owned(), true),
    }
}

/// The tree for `app`. A non-empty `filter` keeps only the nodes whose label or note contains
/// it (ignoring case), together with the nodes above them. The faces of the selected feature are
/// listed below it so one can be picked by name.
pub fn build(app: &AppState, filter: &str) -> Node {
    let name = app.path.as_ref().and_then(|p| p.file_stem()).map_or_else(
        || "untitled".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    let mut root = Node::leaf(NodeKind::Root, name, Icon::Cad(CadIcon::ExtrudedBoss));
    for (label, spec, note) in [
        ("Front Plane", PlaneSpec::Xz { offset: 0.0 }, "XZ"),
        ("Top Plane", PlaneSpec::Xy { offset: 0.0 }, "XY"),
        ("Right Plane", PlaneSpec::Yz { offset: 0.0 }, "YZ"),
    ] {
        let mut n = Node::leaf(NodeKind::Plane(spec), label, Icon::Cad(CadIcon::NormalTo));
        n.note = Some(note.to_owned());
        root.children.push(n);
    }
    root.children.push(Node::leaf(
        NodeKind::Origin,
        "Origin",
        Icon::Cad(CadIcon::Origin),
    ));

    let sketches = app.sketch_rows();
    let state = app.authority.state();
    // Build order, as a design tree shows it: what a feature is built on comes before it.
    let rows = app.feature_rows();
    let order = state.ordered_features();
    let hidden = state.hidden_features();
    let suppressed = state.suppressed();
    let rollback = state.rollback().map(str::to_owned);
    let folders = state.folders();
    let folder_of = |id: &str| {
        folders
            .iter()
            .find(|(_, m)| m.iter().any(|x| x == id))
            .map(|(n, _)| n.clone())
    };
    let mut placed = std::collections::BTreeSet::new();
    let mut folder_nodes: std::collections::BTreeMap<String, usize> = Default::default();
    for id in &order {
        let Some(f) = rows.iter().find(|r| &r.id == id) else {
            continue;
        };
        let mut node = Node::leaf(
            NodeKind::Feature(f.id.clone()),
            state.label("feature", &f.id),
            feature_icon(&f.kind),
        );
        node.note = Some(match &f.target {
            Some(t) => format!("{} of {}", f.kind, state.label("feature", t)),
            None => f.kind.clone(),
        });
        if hidden.contains(&f.id) {
            node.off = Some(if suppressed.contains(&f.id) {
                "suppressed"
            } else if rollback.is_some() && state.feature_is_after_rollback(&f.id) {
                "after the rollback bar"
            } else {
                "built on something that is off"
            });
        }
        node.bar_after = rollback.as_deref() == Some(f.id.as_str());
        if !f.sketch.is_empty() && !placed.contains(&f.sketch) {
            if let Some(s) = sketches.iter().find(|s| s.id == f.sketch) {
                placed.insert(s.id.clone());
                let (note, warn) = sketch_note(&s.info);
                let mut sn = Node::leaf(
                    NodeKind::Sketch(s.id.clone()),
                    state.label("sketch", &s.id),
                    Icon::Cad(CadIcon::Sketch),
                );
                sn.note = Some(note);
                sn.warn = warn;
                node.children.push(sn);
            }
        }
        if let Selected::Feature(sel) | Selected::Face { feature: sel, .. } = &app.selected {
            if sel == &f.id {
                for face in app.faces_of(&f.id) {
                    node.children.push(Node::leaf(
                        NodeKind::Face {
                            feature: f.id.clone(),
                            face: face.name.clone(),
                        },
                        face.name,
                        Icon::Glyph(crate::ui::icon_font::SQUARE),
                    ));
                }
            }
        }
        match folder_of(&f.id) {
            Some(folder) => {
                let at = *folder_nodes.entry(folder.clone()).or_insert_with(|| {
                    let mut n = Node::leaf(
                        NodeKind::Folder(folder.clone()),
                        folder.clone(),
                        Icon::Glyph(crate::ui::icon_font::FOLDER_SIMPLE),
                    );
                    n.note = None;
                    root.children.push(n);
                    root.children.len() - 1
                });
                let fnode = &mut root.children[at];
                fnode.bar_after = node.bar_after;
                fnode.children.push(node);
            }
            None => root.children.push(node),
        }
    }
    for n in &mut root.children {
        if matches!(n.kind, NodeKind::Folder(_)) {
            n.note = Some(format!("{} features", n.children.len()));
        }
    }
    for s in &sketches {
        if placed.contains(&s.id) {
            continue;
        }
        let (note, warn) = sketch_note(&s.info);
        let mut sn = Node::leaf(
            NodeKind::Sketch(s.id.clone()),
            state.label("sketch", &s.id),
            Icon::Cad(CadIcon::Sketch),
        );
        sn.note = Some(note);
        sn.warn = warn;
        root.children.push(sn);
    }
    if filter.trim().is_empty() {
        root
    } else {
        prune(root, &filter.trim().to_lowercase()).unwrap_or_else(|| {
            let mut r = Node::leaf(NodeKind::Root, "no match", Icon::Cad(CadIcon::Select));
            r.note = Some(format!("nothing contains \"{}\"", filter.trim()));
            r
        })
    }
}

/// Keeps the nodes that match `needle`, and those that have a match below them.
fn prune(mut n: Node, needle: &str) -> Option<Node> {
    let own = n.label.to_lowercase().contains(needle)
        || n.note
            .as_ref()
            .is_some_and(|x| x.to_lowercase().contains(needle));
    let kids: Vec<Node> = n
        .children
        .drain(..)
        .filter_map(|c| prune(c, needle))
        .collect();
    if own || !kids.is_empty() {
        n.children = kids;
        Some(n)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(n: &Node) -> Vec<String> {
        n.children.iter().map(|c| c.label.clone()).collect()
    }

    #[test]
    fn a_new_document_has_the_planes_and_the_origin_and_nothing_else() {
        let t = build(&AppState::new(), "");
        assert_eq!(t.label, "untitled");
        assert_eq!(
            labels(&t),
            ["Front Plane", "Top Plane", "Right Plane", "Origin"]
        );
        assert!(t.children.iter().all(|c| c.children.is_empty()));
    }

    #[test]
    fn each_sketch_sits_under_the_first_feature_that_uses_it_and_unused_ones_come_last() {
        let mut app = AppState::new();
        app.load_demo();
        let t = build(&app, "");
        let features: Vec<&Node> = t
            .children
            .iter()
            .filter(|c| matches!(c.kind, NodeKind::Feature(_)))
            .collect();
        assert_eq!(features.len(), 4, "{:?}", labels(&t));
        // Every feature carries its own sketch, and no sketch is listed twice.
        let mut seen = std::collections::BTreeSet::new();
        for f in &features {
            for s in f
                .children
                .iter()
                .filter(|c| matches!(c.kind, NodeKind::Sketch(_)))
            {
                assert!(seen.insert(s.label.clone()), "{} appears twice", s.label);
            }
        }
        assert!(!seen.is_empty());
        let all_sketches: usize = t
            .walk()
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Sketch(_)))
            .count();
        assert_eq!(
            all_sketches,
            app.sketch_rows().len(),
            "every sketch is in the tree once"
        );
        // A sketch no feature uses is a child of the root, after the features.
        let mut a2 = AppState::new();
        a2.begin_sketch(PlaneSpec::Xy { offset: 0.0 }).unwrap();
        a2.end_sketch();
        let t2 = build(&a2, "");
        assert!(matches!(
            t2.children.last().unwrap().kind,
            NodeKind::Sketch(_)
        ));
    }

    #[test]
    fn features_are_listed_in_build_order_not_alphabetically() {
        let mut app = AppState::new();
        app.load_showcase();
        let t = build(&app, "");
        let order: Vec<String> = t
            .children
            .iter()
            .filter(|c| matches!(c.kind, NodeKind::Feature(_)))
            .map(|c| c.label.clone())
            .collect();
        assert_eq!(
            order.first().map(String::as_str),
            Some("plate"),
            "{order:?}"
        );
        // Every feature comes after the one it is built on.
        for f in app.feature_rows() {
            if let Some(target) = &f.target {
                let at = |n: &str| order.iter().position(|x| x == n).unwrap();
                assert!(
                    at(target) < at(&f.id),
                    "{} before {target}: {order:?}",
                    f.id
                );
            }
        }
        assert_ne!(order, {
            let mut a = order.clone();
            a.sort();
            a
        });
    }

    #[test]
    fn the_faces_of_the_selected_feature_are_listed_below_it() {
        let mut app = AppState::new();
        app.load_demo();
        let first = app.feature_rows()[0].id.clone();
        assert!(build(&app, "")
            .walk()
            .iter()
            .all(|n| !matches!(n.kind, NodeKind::Face { .. })));
        app.select(Selected::Feature(first.clone()));
        let t = build(&app, "");
        let faces = t
            .walk()
            .into_iter()
            .filter(|n| matches!(n.kind, NodeKind::Face { .. }))
            .count();
        assert_eq!(faces, app.faces_of(&first).len());
        // And the node knows it is selected.
        assert!(t.walk().iter().any(|n| n.is_selected(&app.selected)));
    }

    #[test]
    fn a_filter_keeps_matches_and_the_nodes_above_them_and_says_when_nothing_matches() {
        let mut app = AppState::new();
        app.load_demo();
        let id = app.feature_rows()[0].id.clone();
        let t = build(&app, &id.to_uppercase());
        let kept: Vec<String> = t.walk().iter().map(|n| n.label.clone()).collect();
        assert!(kept.contains(&id), "{kept:?}");
        assert!(!kept.contains(&"Origin".to_owned()));
        assert!(t.children.len() < build(&app, "").children.len());
        let none = build(&app, "zzzqq");
        assert_eq!(none.label, "no match");
        assert!(none.children.is_empty());
        // The plane notes are searched too.
        assert!(build(&app, "xz")
            .walk()
            .iter()
            .any(|n| n.label == "Front Plane"));
    }

    #[test]
    fn only_folders_and_the_document_row_take_dropped_features() {
        let mut app = AppState::new();
        app.load_demo();
        let id = app.authority.state().ordered_features()[0].clone();
        assert!(app.move_to_folder(&id, Some("Base")));
        let t = build(&app, "");
        assert_eq!(t.drop_folder(), Some(None));
        let f = t
            .children
            .iter()
            .find(|n| matches!(n.kind, NodeKind::Folder(_)))
            .unwrap();
        assert_eq!(f.drop_folder(), Some(Some("Base".to_owned())));
        assert!(f.children[0].drop_folder().is_none());
        assert!(t.children[0].drop_folder().is_none());
    }

    #[test]
    fn keys_are_unique_across_the_tree() {
        let mut app = AppState::new();
        app.load_demo();
        let first = app.feature_rows()[0].id.clone();
        app.select(Selected::Feature(first));
        let t = build(&app, "");
        let mut keys = std::collections::BTreeSet::new();
        for n in t.walk() {
            assert!(keys.insert(n.key()), "{} repeats", n.key());
        }
    }
}
