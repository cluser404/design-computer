use crate::command::{Change, Command, CommandKind, Expected, Revision};
use crate::diag::{DiagCode, Diagnostic};
use crate::feature::{self, feature_path, FeaturePattern, FeatureSlot, FeatureSpec};
use crate::log::{LogEntry, ReplayError};
use crate::query::{
    clamp_limit, FeatureInfo, PlaneFrame, ProfileInfo, Query, QueryResult, SketchInfo,
};
use crate::state::{page_map, SketchSlot};
use crate::{Actor, Hash128, Role, State};
use dc_sketch::{
    Constraint, Entity, Pattern, Sketch, SketchError, SketchErrorCode, SolveReport, SolveStatus,
    Transform,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Default capability levels by actor kind. Agents and remote users are conservative by default
/// (AGENTS.md §4.2); humans and scripts are trusted. Per-actor overrides via [`Authority::set_role`].
#[derive(Clone, Copy, Debug)]
pub struct Policy {
    pub default_human: Role,
    pub default_script: Role,
    pub default_agent: Role,
    pub default_remote: Role,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            default_human: Role::Full,
            default_script: Role::Full,
            default_agent: Role::ProposeOnly,
            default_remote: Role::ProposeOnly,
        }
    }
}

/// Result of a (dry-run or committed) command.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Outcome {
    /// The revision produced (for a dry run: the revision that *would* be produced).
    pub revision: Revision,
    /// State hash after the command (for a dry run: what it would be).
    pub state_hash: Hash128,
    pub changes: Vec<Change>,
    /// Solver analysis of every sketch the command touched (DOF, redundancy, ...).
    #[serde(default)]
    pub sketch_reports: BTreeMap<String, SolveReport>,
}

/// Validates, orders and applies commands. The only way to mutate a document.
#[derive(Debug, Default)]
pub struct Authority {
    state: State,
    revision: Revision,
    log: Vec<LogEntry>,
    seen: BTreeSet<String>,
    roles: BTreeMap<String, Role>,
    policy: Policy,
    /// What each undoable revision changed, for undo/redo. In memory only: rebuilt on replay.
    records: BTreeMap<u64, Record>,
}

fn normalize(v: f64) -> f64 {
    // Canonicalize -0.0 so serialization and hashing are byte-identical.
    if v == 0.0 {
        0.0
    } else {
        v
    }
}

/// Rewrites a command into canonical form (no negative zero) so the logged bytes are identical
/// for equivalent commands.
fn canonicalize(kind: &mut CommandKind) {
    match kind {
        CommandKind::ParamSet {
            value, expected, ..
        } => {
            *value = normalize(*value);
            if let Expected::Value(v) = expected {
                *v = normalize(*v);
            }
        }
        CommandKind::ParamDelete { expected, .. } => {
            if let Expected::Value(v) = expected {
                *v = normalize(*v);
            }
        }
        CommandKind::SketchAddEntity { entity, .. } => *entity = entity.clone().canonical(),
        CommandKind::SketchAddConstraint { constraint, .. } => {
            *constraint = constraint.clone().canonical()
        }
        CommandKind::SketchSetValue {
            value, expected, ..
        } => {
            *value = normalize(*value);
            if let Expected::Value(v) = expected {
                *v = normalize(*v);
            }
        }
        CommandKind::SketchDragPoint { x, y, .. } => {
            *x = normalize(*x);
            *y = normalize(*y);
        }
        CommandKind::SketchTransform { transform, .. } => match transform {
            Transform::Move { dx, dy } => {
                *dx = normalize(*dx);
                *dy = normalize(*dy);
            }
            Transform::Rotate { cx, cy, degrees } => {
                *cx = normalize(*cx);
                *cy = normalize(*cy);
                *degrees = normalize(*degrees);
            }
            Transform::Scale { cx, cy, factor } => {
                *cx = normalize(*cx);
                *cy = normalize(*cy);
                *factor = normalize(*factor);
            }
        },
        CommandKind::SketchPattern { pattern, .. } => match pattern {
            Pattern::Linear { dx, dy, .. } => {
                *dx = normalize(*dx);
                *dy = normalize(*dy);
            }
            Pattern::Circular {
                cx, cy, degrees, ..
            } => {
                *cx = normalize(*cx);
                *cy = normalize(*cy);
                *degrees = normalize(*degrees);
            }
        },
        CommandKind::SketchOffset { distance, .. } => *distance = normalize(*distance),
        CommandKind::SketchFillet { radius, .. } => *radius = normalize(*radius),
        CommandKind::SketchChamfer {
            distance,
            second_distance,
            ..
        } => {
            *distance = normalize(*distance);
            if let Some(d) = second_distance {
                *d = normalize(*d);
            }
        }
        CommandKind::SketchSetDimensionStyle { style, .. } => {
            style.tol_plus = normalize(style.tol_plus);
            style.tol_minus = normalize(style.tol_minus);
        }
        CommandKind::SketchTrimBetween { .. }
        | CommandKind::SketchMirror { .. }
        | CommandKind::SketchRepair { .. }
        | CommandKind::SketchFullyDefine { .. }
        | CommandKind::SketchSetDriven { .. } => {}
        CommandKind::SketchSetPlane { plane, .. } => *plane = plane.clone().canonical(),
        CommandKind::SketchPlaceDimension { x, y, .. }
        | CommandKind::SketchTrim { x, y, .. }
        | CommandKind::SketchExtend { x, y, .. }
        | CommandKind::SketchSplit { x, y, .. } => {
            *x = normalize(*x);
            *y = normalize(*y);
        }
        CommandKind::SketchCornerTrim {
            first_x,
            first_y,
            second_x,
            second_y,
            ..
        } => {
            for v in [first_x, first_y, second_x, second_y] {
                *v = normalize(*v);
            }
        }
        CommandKind::SketchDrag { points, radius, .. } => {
            for t in points {
                t.x = normalize(t.x);
                t.y = normalize(t.y);
            }
            if let Some(r) = radius {
                r.value = normalize(r.value);
            }
        }
        CommandKind::FeatureExtrude { depth, extent, .. } => {
            *depth = normalize(*depth);
            extent.depth2 = extent.depth2.map(normalize);
        }
        CommandKind::FeatureBoss {
            depth,
            extent,
            pattern,
            ..
        } => {
            *depth = normalize(*depth);
            extent.depth2 = extent.depth2.map(normalize);
            canonical_pattern(pattern);
        }
        CommandKind::FeatureCut {
            depth,
            extent,
            pattern,
            ..
        } => {
            *depth = depth.map(normalize);
            extent.depth2 = extent.depth2.map(normalize);
            canonical_pattern(pattern);
        }
        CommandKind::FeatureSetDepth {
            depth, expected, ..
        } => {
            *depth = normalize(*depth);
            if let Expected::Value(v) = expected {
                *v = normalize(*v);
            }
        }
        CommandKind::FeatureRevolve { .. } => {}
        CommandKind::FeatureFillet { radius, .. } => *radius = normalize(*radius),
        CommandKind::FeatureChamfer { distance, .. } => *distance = normalize(*distance),
        CommandKind::FeatureShell { thickness, .. } => *thickness = normalize(*thickness),
        CommandKind::FeatureLoft { .. } | CommandKind::FeatureSweep { .. } => {}
        CommandKind::FeatureDelete { .. }
        | CommandKind::HistoryUndo { .. }
        | CommandKind::HistoryRedo { .. } => {}
        CommandKind::Transaction { ops, .. } => ops.iter_mut().for_each(canonicalize),
        CommandKind::SketchCreate { plane, .. } => *plane = plane.clone().canonical(),
        CommandKind::FeatureUpdate { spec, .. } => *spec = spec.clone().canonical(),
        CommandKind::FeatureSetLabel { .. }
        | CommandKind::SketchSetLabel { .. }
        | CommandKind::FeatureSuppress { .. }
        | CommandKind::DocumentRollback { .. }
        | CommandKind::FolderSet { .. } => {}
        CommandKind::SketchDelete { .. }
        | CommandKind::SketchMergePoints { .. }
        | CommandKind::SketchRemoveEntity { .. }
        | CommandKind::SketchSetConstruction { .. }
        | CommandKind::SketchSetInfinite { .. }
        | CommandKind::SketchRemoveConstraint { .. } => {}
    }
}

fn valid_name(name: &str) -> Result<(), Diagnostic> {
    let ok = !name.is_empty()
        && name.len() <= 256
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '/'));
    if ok {
        Ok(())
    } else {
        Err(Diagnostic::new(
            DiagCode::InvalidName,
            format!("invalid parameter name {name:?}"),
        )
        .suggest("use 1-256 characters from [A-Za-z0-9_.-/]"))
    }
}

/// Sketch IDs may not contain '/', which separates path components in change entity paths.
fn valid_sketch_id(id: &str) -> Result<(), Diagnostic> {
    let ok = !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
    if ok {
        Ok(())
    } else {
        Err(
            Diagnostic::new(DiagCode::InvalidName, format!("invalid sketch id {id:?}"))
                .suggest("use 1-128 characters from [A-Za-z0-9_.-]"),
        )
    }
}

fn sketch_path(sketch: &str) -> String {
    format!("sketch/{sketch}")
}

fn item_path(sketch: &str, item: &str) -> String {
    format!("sketch/{sketch}/{item}")
}

fn missing_sketch(id: &str) -> Diagnostic {
    Diagnostic::new(DiagCode::NotFound, format!("sketch '{id}' does not exist"))
        .entity(sketch_path(id))
        .suggest("create it first with sketch.create")
}

fn sketch_diag(sketch: &str, e: SketchError) -> Diagnostic {
    let code = match e.code {
        SketchErrorCode::InvalidId => DiagCode::InvalidName,
        SketchErrorCode::InvalidValue => DiagCode::InvalidValue,
        SketchErrorCode::AlreadyExists => DiagCode::AlreadyExists,
        SketchErrorCode::NotFound => DiagCode::NotFound,
        SketchErrorCode::WrongKind => DiagCode::WrongKind,
        SketchErrorCode::InUse => DiagCode::InUse,
    };
    let mut d = Diagnostic::new(code, format!("sketch '{sketch}': {}", e.message));
    for ent in e.entities {
        d = d.entity(item_path(sketch, &ent));
    }
    d
}

fn unsolvable(sketch: &str, r: &SolveReport) -> Diagnostic {
    let (message, suggestion) = match r.status {
        SolveStatus::Inconsistent => (
            format!(
                "sketch '{sketch}' would have contradictory constraints (max residual {:.3e})",
                r.max_residual
            ),
            "remove or relax one of the listed constraints; dry-run commands first to check",
        ),
        _ => (
            format!(
                "sketch '{sketch}' could not be solved: the solver did not converge (max residual {:.3e})",
                r.max_residual
            ),
            "make a smaller change, or add constraints one at a time",
        ),
    };
    let mut d = Diagnostic::new(DiagCode::Unsolvable, message).suggest(suggestion);
    for c in &r.conflicting {
        d = d.entity(item_path(sketch, c));
    }
    d
}

fn to_json<T: Serialize>(v: &T) -> Value {
    serde_json::to_value(v).expect("document items always serialize")
}

/// Structural diff of two versions of a sketch, as entity/constraint changes.
fn diff_sketch(id: &str, old: &Sketch, new: &Sketch) -> Vec<Change> {
    fn diff_maps<T: Serialize + PartialEq>(
        sketch: &str,
        old: &BTreeMap<String, T>,
        new: &BTreeMap<String, T>,
        out: &mut Vec<Change>,
    ) {
        let keys: BTreeSet<&String> = old.keys().chain(new.keys()).collect();
        for k in keys {
            let (o, n) = (old.get(k), new.get(k));
            if o != n {
                out.push(Change {
                    entity: item_path(sketch, k),
                    before: o.map(to_json),
                    after: n.map(to_json),
                });
            }
        }
    }
    let mut out = Vec::new();
    diff_maps(id, old.entities(), new.entities(), &mut out);
    diff_maps(id, old.constraints(), new.constraints(), &mut out);
    let places: BTreeSet<&String> = old
        .placements()
        .keys()
        .chain(new.placements().keys())
        .collect();
    for k in places {
        let (o, n) = (old.placement(k), new.placement(k));
        if o != n {
            out.push(Change {
                entity: format!("{}#placement", item_path(id, k)),
                before: o.map(|p| Value::from(vec![p.0, p.1])),
                after: n.map(|p| Value::from(vec![p.0, p.1])),
            });
        }
    }
    for k in old.driven().symmetric_difference(new.driven()) {
        out.push(Change {
            entity: format!("{}#driven", item_path(id, k)),
            before: Some(Value::from(old.is_driven(k))),
            after: Some(Value::from(new.is_driven(k))),
        });
    }
    let styled: BTreeSet<&String> = old.styles().keys().chain(new.styles().keys()).collect();
    for k in styled {
        let (o, n) = (old.style(k), new.style(k));
        if o != n {
            out.push(Change {
                entity: format!("{}#style", item_path(id, k)),
                before: o.map(to_json),
                after: n.map(to_json),
            });
        }
    }
    for k in old.construction().symmetric_difference(new.construction()) {
        out.push(Change {
            entity: format!("{}#construction", item_path(id, k)),
            before: Some(Value::from(old.is_construction(k))),
            after: Some(Value::from(new.is_construction(k))),
        });
    }
    for k in old.infinite().symmetric_difference(new.infinite()) {
        out.push(Change {
            entity: format!("{}#infinite", item_path(id, k)),
            before: Some(Value::from(old.is_infinite(k))),
            after: Some(Value::from(new.is_infinite(k))),
        });
    }
    out
}

#[derive(Clone, Debug)]
enum Undo {
    Param {
        name: String,
        before: Option<f64>,
    },
    Sketch {
        id: String,
        before: Option<SketchSlot>,
    },
    Feature {
        id: String,
        before: Option<FeatureSlot>,
    },
    /// An organisation entry (label, folder, suppression, rollback).
    Meta {
        key: String,
        before: Option<String>,
    },
}

#[derive(Default)]
struct Journal {
    changes: Vec<Change>,
    undo: Vec<Undo>,
    reports: BTreeMap<String, SolveReport>,
}

/// Solves `sk` and, if it is solvable, stores it, recording the diff and the undo step.
fn commit_sketch(
    state: &mut State,
    j: &mut Journal,
    id: &str,
    before: Option<SketchSlot>,
    blame: Option<&str>,
    mut sk: Sketch,
) -> Result<(), Diagnostic> {
    // A dimension that no longer exists has no place to be drawn.
    sk.prune_placements();
    // If something has to be blamed for a contradiction, it should be the constraint the caller
    // just added or edited, not whichever happens to sort last by ID.
    let report = match blame {
        Some(c) => sk.solve_blaming(c),
        None => sk.solve(),
    };
    if !report.applied {
        return Err(unsolvable(id, &report));
    }
    if let Some(b) = &before {
        // Dimensions stay where they were put relative to what they measure.
        sk.follow_placements(&b.sketch);
        j.changes.extend(diff_sketch(id, &b.sketch, &sk));
    }
    state.put_sketch(
        id,
        SketchSlot {
            sketch: sk,
            report: report.clone(),
        },
    );
    j.undo.push(Undo::Sketch {
        id: id.to_owned(),
        before,
    });
    j.reports.insert(id.to_owned(), report);
    rebuild_dependents(state, j, id)
}

/// Rebuilds every feature that reads `sketch`, and everything built on those. If one can no
/// longer be built, the whole command is rejected (and rolled back) with a diagnostic naming it:
/// committed features are always valid.
fn rebuild_dependents(state: &mut State, j: &mut Journal, sketch: &str) -> Result<(), Diagnostic> {
    let roots = state.features_of_sketch(sketch);
    rebuild_closure(state, j, roots, &format!("sketch '{sketch}'"))
}

/// Rebuilds `roots` and every feature transitively built on them, in dependency order (a cut is
/// always rebuilt after the feature it cuts).
fn rebuild_closure(
    state: &mut State,
    j: &mut Journal,
    roots: Vec<String>,
    because: &str,
) -> Result<(), Diagnostic> {
    let mut set: BTreeSet<String> = roots.into_iter().collect();
    loop {
        let more: Vec<String> = set
            .iter()
            .flat_map(|f| state.features_depending_on(f))
            .filter(|f| !set.contains(f))
            .collect();
        if more.is_empty() {
            break;
        }
        set.extend(more);
    }
    let mut order: Vec<(usize, String)> = set
        .into_iter()
        .map(|f| (state.feature_depth(&f), f))
        .collect();
    order.sort();
    for (_, fid) in order {
        let old = state.feature(&fid).expect("listed above").clone();
        let slot = build_feature(state, &fid, &old.spec).map_err(|inner| {
            let mut d = Diagnostic::new(
                DiagCode::FeatureFailed,
                format!(
                    "this edit would break feature '{fid}' ({}{}, after a change to {because}): {}",
                    old.spec.kind(),
                    old.spec
                        .sketch()
                        .map(|s| format!(" of sketch '{s}'"))
                        .unwrap_or_default(),
                    inner.message
                ),
            )
            .entity(feature_path(&fid))
            .suggest("delete or change the feature first (e.g. in the same transaction), or choose a different edit");
            for e in inner.entities {
                d = d.entity(e);
            }
            d
        })?;
        if slot.summary != old.summary {
            j.changes.push(Change {
                entity: feature_path(&fid),
                before: Some(to_json(&old.view())),
                after: Some(to_json(&slot.view())),
            });
        }
        state.put_feature(&fid, slot);
        j.undo.push(Undo::Feature {
            id: fid.clone(),
            before: Some(old),
        });
        check_attached(state, &fid)?;
    }
    Ok(())
}

/// Every sketch placed on a face of `feature` must still have that face. An edit that removes or
/// bends it is rejected, naming the sketch, even if nothing has been built from the sketch yet.
fn check_attached(state: &State, feature: &str) -> Result<(), Diagnostic> {
    for s in state.sketches_on_feature(feature) {
        state.sketch_plane(&s).map_err(|inner| {
            Diagnostic::new(
                DiagCode::FeatureFailed,
                format!(
                    "this edit would detach sketch '{s}' from its face on feature '{feature}': {}",
                    inner.message
                ),
            )
            .entity(sketch_path(&s))
            .entity(feature_path(feature))
            .suggest("choose a different edit, or move the sketch to a face that still exists")
        })?;
    }
    Ok(())
}

/// Builds a feature from its spec against the current sketches and features.
fn build_feature(
    state: &State,
    feature: &str,
    spec: &FeatureSpec,
) -> Result<FeatureSlot, Diagnostic> {
    // Extrudes, cuts and bosses read a sketch; a fillet works on the part itself.
    let sketch = match spec.sketch() {
        Some(id) => Some((state.sketch(id).ok_or_else(|| missing_sketch(id))?, id)),
        None => None,
    };
    let target = match spec.target() {
        Some(t) => Some(state.feature(t).ok_or_else(|| missing_feature(t))?),
        None => None,
    };
    let plane = match &sketch {
        Some((_, id)) => Some(state.sketch_plane(id)?),
        None => None,
    };
    // A loft reads a second sketch and a sweep its path.
    let other = match spec {
        FeatureSpec::Loft { to_sketch: id, .. }
        | FeatureSpec::Sweep {
            path_sketch: id, ..
        } => {
            let sk = state.sketch(id).ok_or_else(|| missing_sketch(id))?;
            Some((state.sketch_plane(id)?, &sk.sketch))
        }
        _ => None,
    };
    let built = feature::build(
        feature,
        plane.as_ref(),
        sketch.as_ref().map(|(s, _)| &s.sketch),
        other.as_ref().map(|(p, s)| (p, *s)),
        target,
        spec,
    )
    .map_err(|mut d| {
        d.message = format!("feature '{feature}': {}", d.message);
        // Say what it is about, so a client can show it: the sketch it is made from, the part
        // it works on.
        if d.entities.is_empty() {
            if let Some(sk) = spec.sketch() {
                d.entities.push(sketch_path(sk));
            }
            if let Some(t) = spec.target() {
                d.entities.push(feature_path(t));
            }
        }
        d
    })?;
    Ok(FeatureSlot {
        spec: spec.clone(),
        solid: built.solid,
        summary: built.summary,
        stepped: built.stepped,
    })
}

/// Sets (or removes) one organisation entry, recording the diff and the undo step.
fn put_meta(state: &mut State, j: &mut Journal, key: &str, value: Option<String>) {
    let before = state.meta(key).map(str::to_owned);
    if before == value {
        return;
    }
    state.set_meta(key, value.clone());
    j.changes.push(Change {
        entity: format!("meta/{key}"),
        before: before.clone().map(Value::from),
        after: value.map(Value::from),
    });
    j.undo.push(Undo::Meta {
        key: key.to_owned(),
        before,
    });
}

/// A label as stored: trimmed, at most 80 characters, no control characters; empty means none.
fn check_label(label: &str) -> Result<Option<String>, Diagnostic> {
    let l = label.trim();
    if l.chars().count() > 80 || l.chars().any(char::is_control) {
        return Err(Diagnostic::new(
            DiagCode::InvalidValue,
            "a label has at most 80 characters and no control characters",
        ));
    }
    Ok((!l.is_empty()).then(|| l.to_owned()))
}

/// Nothing can be added while the rollback bar is not at the end.
fn not_rolled_back(state: &State) -> Result<(), Diagnostic> {
    match state.rollback() {
        None => Ok(()),
        Some(bar) => Err(Diagnostic::new(
            DiagCode::RolledBack,
            format!("the rollback bar is after '{bar}': features after it are off, so nothing can be added"),
        )
        .suggest("roll forward to the end first (document.rollback with no `to`)")),
    }
}

/// Removes what the organisation says about a feature that is being deleted.
fn forget_feature(state: &mut State, j: &mut Journal, id: &str) {
    put_meta(state, j, &format!("label/feature/{id}"), None);
    put_meta(state, j, &format!("suppress/{id}"), None);
    if state.rollback() == Some(id) {
        put_meta(state, j, "rollback", None);
    }
    for (folder, members) in state.folders() {
        if members.iter().any(|m| m == id) {
            let rest: Vec<String> = members.into_iter().filter(|m| m != id).collect();
            put_meta(state, j, &format!("folder/{folder}"), folder_value(&rest));
        }
    }
}

fn folder_value(members: &[String]) -> Option<String> {
    (!members.is_empty())
        .then(|| serde_json::to_string(members).expect("a list of strings serializes"))
}

/// Replaces a feature's recipe and rebuilds what is built on it.
fn replace_feature(
    state: &mut State,
    j: &mut Journal,
    feature: &str,
    spec: FeatureSpec,
) -> Result<(), Diagnostic> {
    let old = state
        .feature(feature)
        .ok_or_else(|| missing_feature(feature))?
        .clone();
    let slot = build_feature(state, feature, &spec)?;
    j.changes.push(Change {
        entity: feature_path(feature),
        before: Some(to_json(&old.view())),
        after: Some(to_json(&slot.view())),
    });
    state.put_feature(feature, slot);
    j.undo.push(Undo::Feature {
        id: feature.to_owned(),
        before: Some(old),
    });
    check_attached(state, feature)?;
    let dependents = state.features_depending_on(feature);
    rebuild_closure(state, j, dependents, &format!("feature '{feature}'"))
}

/// Creates a feature from `spec` under a new name, recording the change and its undo.
fn create_feature(
    state: &mut State,
    j: &mut Journal,
    feature: &str,
    spec: FeatureSpec,
) -> Result<(), Diagnostic> {
    valid_sketch_id(feature)?;
    not_rolled_back(state)?;
    if state.feature(feature).is_some() {
        return Err(Diagnostic::new(
            DiagCode::AlreadyExists,
            format!("feature '{feature}' already exists"),
        )
        .entity(feature_path(feature)));
    }
    let slot = build_feature(state, feature, &spec)?;
    j.changes.push(Change {
        entity: feature_path(feature),
        before: None,
        after: Some(to_json(&slot.view())),
    });
    state.put_feature(feature, slot);
    j.undo.push(Undo::Feature {
        id: feature.to_owned(),
        before: None,
    });
    Ok(())
}

/// Normalises the numbers of a pattern the same way a feature spec is (no negative zero).
fn canonical_pattern(p: &mut Option<FeaturePattern>) {
    if let Some(pt) = p.take() {
        *p = Some(pt.canonical());
    }
}

fn missing_feature(id: &str) -> Diagnostic {
    Diagnostic::new(DiagCode::NotFound, format!("feature '{id}' does not exist"))
        .entity(feature_path(id))
}

/// Clones the sketch, applies `f`, re-solves and commits.
fn edit_sketch(
    state: &mut State,
    j: &mut Journal,
    id: &str,
    blame: Option<&str>,
    f: impl FnOnce(&mut Sketch) -> Result<(), SketchError>,
) -> Result<(), Diagnostic> {
    let slot = state.sketch(id).ok_or_else(|| missing_sketch(id))?.clone();
    let mut sk = slot.sketch.clone();
    f(&mut sk).map_err(|e| sketch_diag(id, e))?;
    commit_sketch(state, j, id, Some(slot), blame, sk)
}

fn apply_op(
    state: &mut State,
    op: &CommandKind,
    j: &mut Journal,
    in_tx: bool,
) -> Result<(), Diagnostic> {
    match op {
        CommandKind::ParamSet {
            name,
            value,
            expected,
        } => {
            valid_name(name)?;
            if !value.is_finite() {
                return Err(Diagnostic::new(
                    DiagCode::InvalidValue,
                    format!("value for '{name}' must be finite, got {value}"),
                )
                .entity(name.as_str()));
            }
            expected.check(name, state.get(name))?;
            let v = normalize(*value);
            let before = state.set(name, v);
            j.undo.push(Undo::Param {
                name: name.clone(),
                before,
            });
            j.changes.push(Change {
                entity: name.clone(),
                before: before.map(Value::from),
                after: Some(Value::from(v)),
            });
            Ok(())
        }
        CommandKind::ParamDelete { name, expected } => {
            valid_name(name)?;
            let current = state.get(name);
            if current.is_none() {
                return Err(Diagnostic::new(
                    DiagCode::NotFound,
                    format!("'{name}' does not exist"),
                )
                .entity(name.as_str()));
            }
            expected.check(name, current)?;
            let before = state.remove(name);
            j.undo.push(Undo::Param {
                name: name.clone(),
                before,
            });
            j.changes.push(Change {
                entity: name.clone(),
                before: before.map(Value::from),
                after: None,
            });
            Ok(())
        }
        CommandKind::SketchCreate { sketch, plane } => {
            valid_sketch_id(sketch)?;
            if state.sketch(sketch).is_some() {
                return Err(Diagnostic::new(
                    DiagCode::AlreadyExists,
                    format!("sketch '{sketch}' already exists"),
                )
                .entity(sketch_path(sketch)));
            }
            // A face-attached plane must resolve right now: the face has to exist and be flat.
            state.resolve_plane(plane)?;
            let sk = Sketch::on_plane(plane.clone()).map_err(|e| sketch_diag(sketch, e))?;
            j.changes.push(Change {
                entity: sketch_path(sketch),
                before: None,
                after: Some(to_json(&sk)),
            });
            commit_sketch(state, j, sketch, None, None, sk)
        }
        CommandKind::SketchDelete { sketch } => {
            let users = state.features_of_sketch(sketch);
            if !users.is_empty() {
                let mut d = Diagnostic::new(
                    DiagCode::InUse,
                    format!(
                        "sketch '{sketch}' is used by feature(s) {}",
                        users.join(", ")
                    ),
                )
                .entity(sketch_path(sketch))
                .suggest("delete those features first (in the same transaction if you like)");
                for f in &users {
                    d = d.entity(feature_path(f));
                }
                return Err(d);
            }
            let slot = state
                .remove_sketch(sketch)
                .ok_or_else(|| missing_sketch(sketch))?;
            j.changes.push(Change {
                entity: sketch_path(sketch),
                before: Some(to_json(&slot.sketch)),
                after: None,
            });
            j.undo.push(Undo::Sketch {
                id: sketch.clone(),
                before: Some(slot),
            });
            put_meta(state, j, &format!("label/sketch/{sketch}"), None);
            Ok(())
        }
        CommandKind::SketchAddEntity {
            sketch,
            entity_id,
            entity,
        } => edit_sketch(state, j, sketch, None, |sk| {
            sk.add_entity(entity_id, entity.clone())
        }),
        CommandKind::SketchRemoveEntity { sketch, entity_id } => {
            edit_sketch(state, j, sketch, None, |sk| {
                sk.remove_entity(entity_id).map(|_| ())
            })
        }
        CommandKind::SketchSetConstruction {
            sketch,
            entity_id,
            construction,
        } => edit_sketch(state, j, sketch, None, |sk| {
            sk.set_construction(entity_id, *construction)
        }),
        CommandKind::SketchSetInfinite {
            sketch,
            entity_id,
            infinite,
        } => edit_sketch(state, j, sketch, None, |sk| {
            sk.set_infinite(entity_id, *infinite)
        }),
        CommandKind::SketchPlaceDimension {
            sketch,
            constraint_id,
            x,
            y,
        } => edit_sketch(state, j, sketch, None, |sk| {
            sk.set_placement(constraint_id, *x, *y)
        }),
        CommandKind::SketchTrimBetween {
            sketch,
            bounds,
            inside,
            construction,
        } => edit_sketch(state, j, sketch, None, |sk| {
            sk.trim_between(bounds, *inside, *construction).map(|_| ())
        }),
        CommandKind::SketchTransform {
            sketch,
            entities,
            transform,
            copy,
        } => edit_sketch(state, j, sketch, None, |sk| {
            sk.transform(entities, transform, *copy).map(|_| ())
        }),
        CommandKind::SketchMirror {
            sketch,
            entities,
            about,
        } => edit_sketch(state, j, sketch, None, |sk| {
            sk.mirror(entities, about).map(|_| ())
        }),
        CommandKind::SketchPattern {
            sketch,
            entities,
            pattern,
        } => edit_sketch(state, j, sketch, None, |sk| {
            sk.pattern(entities, pattern).map(|_| ())
        }),
        CommandKind::SketchOffset {
            sketch,
            entities,
            distance,
        } => edit_sketch(state, j, sketch, None, |sk| {
            sk.offset(entities, *distance).map(|_| ())
        }),
        CommandKind::SketchFillet {
            sketch,
            first,
            second,
            radius,
        } => edit_sketch(state, j, sketch, None, |sk| {
            sk.fillet(first, second, *radius).map(|_| ())
        }),
        CommandKind::SketchChamfer {
            sketch,
            first,
            second,
            distance,
            second_distance,
        } => edit_sketch(state, j, sketch, None, |sk| {
            sk.chamfer(
                first,
                second,
                *distance,
                second_distance.unwrap_or(*distance),
            )
            .map(|_| ())
        }),
        CommandKind::SketchRepair { sketch } => edit_sketch(state, j, sketch, None, |sk| {
            sk.repair();
            Ok(())
        }),
        CommandKind::SketchFullyDefine { sketch } => {
            edit_sketch(state, j, sketch, None, |sk| sk.fully_define().map(|_| ()))
        }
        CommandKind::SketchSetDriven {
            sketch,
            constraint_id,
            driven,
        } => edit_sketch(state, j, sketch, None, |sk| {
            sk.set_driven(constraint_id, *driven)
        }),
        CommandKind::SketchSetDimensionStyle {
            sketch,
            constraint_id,
            style,
        } => edit_sketch(state, j, sketch, None, |sk| {
            sk.set_style(constraint_id, style.clone())
        }),
        CommandKind::SketchSetPlane { sketch, plane } => {
            edit_sketch(state, j, sketch, None, |sk| {
                sk.set_plane(plane.clone());
                Ok(())
            })
        }
        CommandKind::SketchAddConstraint {
            sketch,
            constraint_id,
            constraint,
        } => edit_sketch(state, j, sketch, Some(constraint_id), |sk| {
            sk.add_constraint(constraint_id, constraint.clone())
        }),
        CommandKind::SketchRemoveConstraint {
            sketch,
            constraint_id,
        } => edit_sketch(state, j, sketch, None, |sk| {
            sk.remove_constraint(constraint_id).map(|_| ())
        }),
        CommandKind::SketchSetValue {
            sketch,
            constraint_id: id,
            value,
            expected,
        } => {
            let slot = state.sketch(sketch).ok_or_else(|| missing_sketch(sketch))?;
            let current = match slot.sketch.constraints().get(id) {
                None => {
                    return Err(Diagnostic::new(
                        DiagCode::NotFound,
                        format!("sketch '{sketch}': constraint '{id}' does not exist"),
                    )
                    .entity(item_path(sketch, id)))
                }
                Some(c) => c.value(),
            };
            expected.check(&item_path(sketch, id), current)?;
            edit_sketch(state, j, sketch, Some(id), |sk| {
                sk.set_constraint_value(id, *value)
            })
        }
        CommandKind::SketchDragPoint {
            sketch,
            point,
            x,
            y,
        } => {
            let slot = state
                .sketch(sketch)
                .ok_or_else(|| missing_sketch(sketch))?
                .clone();
            let mut sk = slot.sketch.clone();
            let report = sk
                .drag_point(point, *x, *y)
                .map_err(|e| sketch_diag(sketch, e))?;
            if !report.applied {
                return Err(unsolvable(sketch, &report));
            }
            commit_sketch(state, j, sketch, Some(slot), None, sk)
        }
        CommandKind::SketchCornerTrim {
            sketch,
            first,
            first_x,
            first_y,
            second,
            second_x,
            second_y,
        } => {
            let slot = state
                .sketch(sketch)
                .ok_or_else(|| missing_sketch(sketch))?
                .clone();
            if ![first_x, first_y, second_x, second_y]
                .iter()
                .all(|v| v.is_finite())
            {
                return Err(Diagnostic::new(
                    DiagCode::InvalidValue,
                    "the positions must be finite numbers",
                ));
            }
            let mut sk = slot.sketch.clone();
            sk.corner_trim(first, (*first_x, *first_y), second, (*second_x, *second_y))
                .map_err(|e| sketch_diag(sketch, e))?;
            commit_sketch(state, j, sketch, Some(slot), None, sk)
        }
        CommandKind::SketchMergePoints { sketch, a, b } => {
            let slot = state
                .sketch(sketch)
                .ok_or_else(|| missing_sketch(sketch))?
                .clone();
            let mut sk = slot.sketch.clone();
            sk.merge_points(a, b).map_err(|e| sketch_diag(sketch, e))?;
            commit_sketch(state, j, sketch, Some(slot), None, sk)
        }
        CommandKind::SketchTrim {
            sketch,
            entity_id,
            x,
            y,
            ..
        }
        | CommandKind::SketchExtend {
            sketch,
            entity_id,
            x,
            y,
            ..
        }
        | CommandKind::SketchSplit {
            sketch,
            entity_id,
            x,
            y,
        } => {
            let slot = state
                .sketch(sketch)
                .ok_or_else(|| missing_sketch(sketch))?
                .clone();
            let mut sk = slot.sketch.clone();
            let at = (*x, *y);
            if !x.is_finite() || !y.is_finite() {
                return Err(Diagnostic::new(
                    DiagCode::InvalidValue,
                    "the position must be finite numbers",
                ));
            }
            match op {
                CommandKind::SketchTrim {
                    construction,
                    bounds,
                    ..
                } => sk
                    .trim_with(
                        entity_id,
                        at,
                        &dc_sketch::edit::TrimOptions {
                            construction: *construction,
                            bounds: bounds.clone(),
                        },
                    )
                    .map(|_| ()),
                CommandKind::SketchExtend { construction, .. } => {
                    sk.extend_with(entity_id, at, *construction).map(|_| ())
                }
                _ => sk.split(entity_id, at).map(|_| ()),
            }
            .map_err(|e| sketch_diag(sketch, e))?;
            commit_sketch(state, j, sketch, Some(slot), None, sk)
        }
        CommandKind::SketchDrag {
            sketch,
            points,
            radius,
        } => {
            let slot = state
                .sketch(sketch)
                .ok_or_else(|| missing_sketch(sketch))?
                .clone();
            let mut sk = slot.sketch.clone();
            let report = sk
                .drag(points, radius.as_ref())
                .map_err(|e| sketch_diag(sketch, e))?;
            if !report.applied {
                return Err(unsolvable(sketch, &report));
            }
            commit_sketch(state, j, sketch, Some(slot), None, sk)
        }
        CommandKind::FeatureExtrude {
            feature,
            sketch,
            profiles,
            depth,
            reverse,
            extent,
        } => {
            valid_sketch_id(feature)?;
            not_rolled_back(state)?;
            if state.feature(feature).is_some() {
                return Err(Diagnostic::new(
                    DiagCode::AlreadyExists,
                    format!("feature '{feature}' already exists"),
                )
                .entity(feature_path(feature)));
            }
            let spec = FeatureSpec::Extrude {
                sketch: sketch.clone(),
                profiles: profiles.clone(),
                depth: *depth,
                reverse: *reverse,
                extent: extent.clone(),
            };
            let slot = build_feature(state, feature, &spec)?;
            j.changes.push(Change {
                entity: feature_path(feature),
                before: None,
                after: Some(to_json(&slot.view())),
            });
            state.put_feature(feature, slot);
            j.undo.push(Undo::Feature {
                id: feature.clone(),
                before: None,
            });
            Ok(())
        }
        CommandKind::FeatureCut {
            feature,
            target,
            sketch,
            profiles,
            depth,
            reverse,
            extent,
            pattern,
        } => {
            valid_sketch_id(feature)?;
            not_rolled_back(state)?;
            if state.feature(feature).is_some() {
                return Err(Diagnostic::new(
                    DiagCode::AlreadyExists,
                    format!("feature '{feature}' already exists"),
                )
                .entity(feature_path(feature)));
            }
            let spec = FeatureSpec::Cut {
                target: target.clone(),
                sketch: sketch.clone(),
                profiles: profiles.clone(),
                depth: *depth,
                reverse: *reverse,
                extent: extent.clone(),
                pattern: pattern.clone(),
            };
            let slot = build_feature(state, feature, &spec)?;
            j.changes.push(Change {
                entity: feature_path(feature),
                before: None,
                after: Some(to_json(&slot.view())),
            });
            state.put_feature(feature, slot);
            j.undo.push(Undo::Feature {
                id: feature.clone(),
                before: None,
            });
            Ok(())
        }
        CommandKind::FeatureBoss {
            feature,
            target,
            sketch,
            profiles,
            depth,
            reverse,
            extent,
            pattern,
        } => {
            valid_sketch_id(feature)?;
            not_rolled_back(state)?;
            if state.feature(feature).is_some() {
                return Err(Diagnostic::new(
                    DiagCode::AlreadyExists,
                    format!("feature '{feature}' already exists"),
                )
                .entity(feature_path(feature)));
            }
            let spec = FeatureSpec::Boss {
                target: target.clone(),
                sketch: sketch.clone(),
                profiles: profiles.clone(),
                depth: *depth,
                reverse: *reverse,
                extent: extent.clone(),
                pattern: pattern.clone(),
            };
            let slot = build_feature(state, feature, &spec)?;
            j.changes.push(Change {
                entity: feature_path(feature),
                before: None,
                after: Some(to_json(&slot.view())),
            });
            state.put_feature(feature, slot);
            j.undo.push(Undo::Feature {
                id: feature.clone(),
                before: None,
            });
            Ok(())
        }
        CommandKind::FeatureUpdate { feature, spec } => {
            let old = state
                .feature(feature)
                .ok_or_else(|| missing_feature(feature))?;
            if std::mem::discriminant(&old.spec) != std::mem::discriminant(spec) {
                return Err(Diagnostic::new(
                    DiagCode::WrongKind,
                    format!(
                        "feature '{feature}' is a {}: its recipe can change but not its kind",
                        old.spec.kind()
                    ),
                )
                .entity(feature_path(feature)));
            }
            replace_feature(state, j, feature, spec.clone())
        }
        CommandKind::FeatureSetLabel { feature, label } => {
            if state.feature(feature).is_none() {
                return Err(missing_feature(feature));
            }
            let l = check_label(label)?;
            put_meta(state, j, &format!("label/feature/{feature}"), l);
            Ok(())
        }
        CommandKind::SketchSetLabel { sketch, label } => {
            if state.sketch(sketch).is_none() {
                return Err(missing_sketch(sketch));
            }
            let l = check_label(label)?;
            put_meta(state, j, &format!("label/sketch/{sketch}"), l);
            Ok(())
        }
        CommandKind::FeatureSuppress {
            feature,
            suppressed,
        } => {
            if state.feature(feature).is_none() {
                return Err(missing_feature(feature));
            }
            put_meta(
                state,
                j,
                &format!("suppress/{feature}"),
                suppressed.then(|| "1".to_owned()),
            );
            Ok(())
        }
        CommandKind::DocumentRollback { to } => {
            if let Some(f) = to {
                if state.feature(f).is_none() {
                    return Err(missing_feature(f));
                }
            }
            put_meta(state, j, "rollback", to.clone());
            Ok(())
        }
        CommandKind::FolderSet { folder, features } => {
            let ok = !folder.is_empty()
                && folder.chars().count() <= 40
                && !folder.contains('/')
                && !folder.chars().any(char::is_control);
            if !ok {
                return Err(Diagnostic::new(
                    DiagCode::InvalidName,
                    format!("invalid folder name {folder:?}"),
                )
                .suggest("use 1-40 characters, no '/' and no control characters"));
            }
            let mut seen = BTreeSet::new();
            for f in features {
                if state.feature(f).is_none() {
                    return Err(missing_feature(f));
                }
                if !seen.insert(f.clone()) {
                    return Err(Diagnostic::new(
                        DiagCode::InvalidValue,
                        format!("feature '{f}' is listed twice"),
                    ));
                }
            }
            // A feature is in one folder only: take them out of any other.
            for (other, members) in state.folders() {
                if &other == folder {
                    continue;
                }
                let rest: Vec<String> = members
                    .iter()
                    .filter(|m| !seen.contains(*m))
                    .cloned()
                    .collect();
                if rest.len() != members.len() {
                    put_meta(state, j, &format!("folder/{other}"), folder_value(&rest));
                }
            }
            put_meta(
                state,
                j,
                &format!("folder/{folder}"),
                folder_value(features),
            );
            Ok(())
        }
        CommandKind::FeatureRevolve {
            feature,
            sketch,
            profiles,
            axis,
            angle,
        } => {
            let spec = FeatureSpec::Revolve {
                sketch: sketch.clone(),
                profiles: profiles.clone(),
                axis: axis.clone(),
                angle: *angle,
            };
            create_feature(state, j, feature, spec)
        }
        CommandKind::FeatureFillet {
            feature,
            target,
            edges,
            radius,
        } => {
            let spec = FeatureSpec::Fillet {
                target: target.clone(),
                edges: edges.clone(),
                radius: *radius,
            };
            create_feature(state, j, feature, spec)
        }
        CommandKind::FeatureChamfer {
            feature,
            target,
            edges,
            distance,
        } => {
            let spec = FeatureSpec::Chamfer {
                target: target.clone(),
                edges: edges.clone(),
                distance: *distance,
            };
            create_feature(state, j, feature, spec)
        }
        CommandKind::FeatureLoft {
            feature,
            sketch,
            profiles,
            to_sketch,
            to_profiles,
        } => {
            let spec = FeatureSpec::Loft {
                sketch: sketch.clone(),
                profiles: profiles.clone(),
                to_sketch: to_sketch.clone(),
                to_profiles: to_profiles.clone(),
            };
            create_feature(state, j, feature, spec)
        }
        CommandKind::FeatureSweep {
            feature,
            sketch,
            profiles,
            path_sketch,
            path,
        } => {
            let spec = FeatureSpec::Sweep {
                sketch: sketch.clone(),
                profiles: profiles.clone(),
                path_sketch: path_sketch.clone(),
                path: path.clone(),
            };
            create_feature(state, j, feature, spec)
        }
        CommandKind::FeatureShell {
            feature,
            target,
            faces,
            thickness,
        } => {
            let spec = FeatureSpec::Shell {
                target: target.clone(),
                faces: faces.clone(),
                thickness: *thickness,
            };
            create_feature(state, j, feature, spec)
        }
        CommandKind::FeatureSetDepth {
            feature,
            depth,
            expected,
        } => {
            let old = state
                .feature(feature)
                .ok_or_else(|| missing_feature(feature))?
                .clone();
            let spec = match &old.spec {
                FeatureSpec::Extrude {
                    sketch,
                    profiles,
                    depth: current,
                    reverse,
                    extent,
                } => {
                    expected.check(&feature_path(feature), Some(*current))?;
                    FeatureSpec::Extrude {
                        sketch: sketch.clone(),
                        profiles: profiles.clone(),
                        depth: *depth,
                        reverse: *reverse,
                        extent: extent.clone(),
                    }
                }
                FeatureSpec::Cut {
                    target,
                    sketch,
                    profiles,
                    depth: current,
                    reverse,
                    extent,
                    pattern,
                } => {
                    expected.check(&feature_path(feature), *current)?;
                    FeatureSpec::Cut {
                        target: target.clone(),
                        sketch: sketch.clone(),
                        profiles: profiles.clone(),
                        depth: Some(*depth),
                        reverse: *reverse,
                        extent: extent.clone(),
                        pattern: pattern.clone(),
                    }
                }
                FeatureSpec::Boss {
                    target,
                    sketch,
                    profiles,
                    depth: current,
                    reverse,
                    extent,
                    pattern,
                } => {
                    expected.check(&feature_path(feature), Some(*current))?;
                    FeatureSpec::Boss {
                        target: target.clone(),
                        sketch: sketch.clone(),
                        profiles: profiles.clone(),
                        depth: *depth,
                        reverse: *reverse,
                        extent: extent.clone(),
                        pattern: pattern.clone(),
                    }
                }
                FeatureSpec::Revolve { .. } => {
                    return Err(Diagnostic::new(
                        DiagCode::WrongKind,
                        format!("feature '{feature}' is a revolve: it has no depth to set"),
                    )
                    .entity(feature_path(feature)));
                }
                FeatureSpec::Fillet {
                    target,
                    edges,
                    radius: current,
                } => {
                    expected.check(&feature_path(feature), Some(*current))?;
                    FeatureSpec::Fillet {
                        target: target.clone(),
                        edges: edges.clone(),
                        radius: *depth,
                    }
                }
                FeatureSpec::Chamfer {
                    target,
                    edges,
                    distance: current,
                } => {
                    expected.check(&feature_path(feature), Some(*current))?;
                    FeatureSpec::Chamfer {
                        target: target.clone(),
                        edges: edges.clone(),
                        distance: *depth,
                    }
                }
                FeatureSpec::Loft { .. } | FeatureSpec::Sweep { .. } => {
                    return Err(Diagnostic::new(
                        DiagCode::WrongKind,
                        format!("feature '{feature}' is a loft or a sweep: it has no depth to set"),
                    )
                    .entity(feature_path(feature)));
                }
                FeatureSpec::Shell {
                    target,
                    faces,
                    thickness: current,
                } => {
                    expected.check(&feature_path(feature), Some(*current))?;
                    FeatureSpec::Shell {
                        target: target.clone(),
                        faces: faces.clone(),
                        thickness: *depth,
                    }
                }
            };
            let slot = build_feature(state, feature, &spec)?;
            j.changes.push(Change {
                entity: feature_path(feature),
                before: Some(to_json(&old.view())),
                after: Some(to_json(&slot.view())),
            });
            state.put_feature(feature, slot);
            j.undo.push(Undo::Feature {
                id: feature.clone(),
                before: Some(old),
            });
            check_attached(state, feature)?;
            // Everything built on this feature, or sketched on its faces, depends on it.
            let dependents = state.features_depending_on(feature);
            rebuild_closure(state, j, dependents, &format!("feature '{feature}'"))
        }
        CommandKind::FeatureDelete { feature } => {
            let users = state.features_depending_on(feature);
            let attached = state.sketches_on_feature(feature);
            if !users.is_empty() || !attached.is_empty() {
                let mut what: Vec<String> =
                    users.iter().map(|u| format!("feature '{u}'")).collect();
                what.extend(
                    attached
                        .iter()
                        .map(|a| format!("sketch '{a}' (placed on its face)")),
                );
                let mut d = Diagnostic::new(
                    DiagCode::InUse,
                    format!("feature '{feature}' is used by {}", what.join(", ")),
                )
                .entity(feature_path(feature))
                .suggest("delete those first (in the same transaction if you like)");
                for u in &users {
                    d = d.entity(feature_path(u));
                }
                for a in &attached {
                    d = d.entity(sketch_path(a));
                }
                return Err(d);
            }
            let old = state
                .remove_feature(feature)
                .ok_or_else(|| missing_feature(feature))?;
            j.changes.push(Change {
                entity: feature_path(feature),
                before: Some(to_json(&old.view())),
                after: None,
            });
            j.undo.push(Undo::Feature {
                id: feature.clone(),
                before: Some(old),
            });
            forget_feature(state, j, feature);
            Ok(())
        }
        CommandKind::HistoryUndo { .. } | CommandKind::HistoryRedo { .. } => Err(Diagnostic::new(
            DiagCode::InvalidValue,
            "history.undo / history.redo must be submitted on their own, not inside a transaction",
        )),
        CommandKind::Transaction { label, ops } => {
            if in_tx {
                return Err(Diagnostic::new(
                    DiagCode::NestedTransaction,
                    format!("transaction '{label}' is nested inside another transaction"),
                ));
            }
            if ops.is_empty() {
                return Err(Diagnostic::new(
                    DiagCode::EmptyTransaction,
                    format!("transaction '{label}' has no operations"),
                ));
            }
            for o in ops {
                apply_op(state, o, j, true)?;
            }
            Ok(())
        }
    }
}

fn revert(state: &mut State, undo: &[Undo]) {
    for u in undo.iter().rev() {
        match u {
            Undo::Param {
                name,
                before: Some(v),
            } => {
                state.set(name, *v);
            }
            Undo::Param { name, before: None } => {
                state.remove(name);
            }
            Undo::Sketch {
                id,
                before: Some(slot),
            } => {
                state.put_sketch(id, slot.clone());
            }
            Undo::Sketch { id, before: None } => {
                state.remove_sketch(id);
            }
            Undo::Feature {
                id,
                before: Some(slot),
            } => {
                state.put_feature(id, slot.clone());
            }
            Undo::Feature { id, before: None } => {
                state.remove_feature(id);
            }
            Undo::Meta { key, before } => {
                state.set_meta(key, before.clone());
            }
        }
    }
}

/// Applies atomically; on failure the state is exactly as before. Cost is O(changes).
fn apply(state: &mut State, kind: &CommandKind) -> Result<Journal, Diagnostic> {
    let mut j = Journal::default();
    match apply_op(state, kind, &mut j, false) {
        Ok(()) => Ok(j),
        Err(d) => {
            revert(state, &j.undo);
            Err(d)
        }
    }
}

impl Authority {
    pub fn new(policy: Policy) -> Self {
        Self {
            policy,
            ..Self::default()
        }
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn state_hash(&self) -> Hash128 {
        self.state.hash()
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn log(&self) -> &[LogEntry] {
        &self.log
    }

    pub fn set_role(&mut self, actor: &Actor, role: Role) {
        self.roles.insert(actor.key(), role);
    }

    pub fn role_of(&self, actor: &Actor) -> Role {
        if let Some(r) = self.roles.get(&actor.key()) {
            return *r;
        }
        match actor {
            Actor::Human(_) => self.policy.default_human,
            Actor::Script(_) => self.policy.default_script,
            Actor::Agent(_) => self.policy.default_agent,
            Actor::Remote(_) => self.policy.default_remote,
        }
    }

    fn precheck(&self, cmd: &Command) -> Result<(), Diagnostic> {
        match self.role_of(&cmd.actor) {
            Role::Full => {}
            Role::ReadOnly => {
                return Err(Diagnostic::new(
                    DiagCode::PermissionDenied,
                    format!("{} is read-only", cmd.actor),
                ))
            }
            Role::ProposeOnly => {
                return Err(Diagnostic::new(
                    DiagCode::ProposalRequired,
                    format!(
                        "{} may only submit proposals, not commit directly",
                        cmd.actor
                    ),
                )
                .suggest("submit as a proposal, or ask for the Full role"))
            }
        }
        if self.seen.contains(&cmd.id) {
            return Err(Diagnostic::new(
                DiagCode::DuplicateCommand,
                format!("command id '{}' was already applied", cmd.id),
            ));
        }
        if cmd.base_revision > self.revision {
            return Err(Diagnostic::new(
                DiagCode::FutureRevision,
                format!(
                    "command is based on revision {}, but the document is at {}",
                    cmd.base_revision.0, self.revision.0
                ),
            ));
        }
        Ok(())
    }

    /// Validates and applies a command, then records it. Atomic: on error nothing changes.
    pub fn submit(&mut self, mut cmd: Command) -> Result<Outcome, Diagnostic> {
        canonicalize(&mut cmd.kind);
        self.resolve_history(&mut cmd)?;
        self.precheck(&cmd)?;
        let rev = self.revision.0 + 1;
        let (eff, _) = self.run_command(&cmd.actor, &cmd.kind, rev)?;
        self.revision = Revision(rev);
        let state_hash = self.state.hash();
        self.seen.insert(cmd.id.clone());
        self.log.push(LogEntry {
            revision: self.revision,
            command: cmd,
            state_hash,
        });
        Ok(Outcome {
            revision: self.revision,
            state_hash,
            changes: eff.changes,
            sketch_reports: eff.reports,
        })
    }

    /// Reports exactly what [`submit`](Self::submit) would do, without committing anything.
    pub fn dry_run(&mut self, cmd: &Command) -> Result<Outcome, Diagnostic> {
        let mut cmd = cmd.clone();
        canonicalize(&mut cmd.kind);
        self.resolve_history(&mut cmd)?;
        self.precheck(&cmd)?;
        let rev = self.revision.0 + 1;
        let (eff, rollback) = self.run_command(&cmd.actor, &cmd.kind, rev)?;
        let state_hash = self.state.hash();
        self.rollback(rev, rollback);
        Ok(Outcome {
            revision: Revision(rev),
            state_hash,
            changes: eff.changes,
            sketch_reports: eff.reports,
        })
    }

    /// Like [`dry_run`](Self::dry_run), and also returns the solids the document would show
    /// afterwards (one per result feature), so a client can preview a feature before it is
    /// committed. Nothing is changed.
    pub fn preview_solids(
        &mut self,
        cmd: &Command,
    ) -> Result<Vec<(String, std::sync::Arc<dc_kernel::Solid>)>, Diagnostic> {
        let mut cmd = cmd.clone();
        canonicalize(&mut cmd.kind);
        self.resolve_history(&mut cmd)?;
        self.precheck(&cmd)?;
        let rev = self.revision.0 + 1;
        let (_, rollback) = self.run_command(&cmd.actor, &cmd.kind, rev)?;
        let solids = self
            .state
            .result_features()
            .into_iter()
            .filter_map(|f| self.state.feature(&f).map(|s| (f, s.solid.clone())))
            .collect();
        self.rollback(rev, rollback);
        Ok(solids)
    }

    fn feature_info(id: &str, slot: &FeatureSlot) -> FeatureInfo {
        FeatureInfo {
            id: id.to_owned(),
            kind: slot.spec.kind().to_owned(),
            spec: slot.spec.clone(),
            summary: slot.summary.clone(),
        }
    }

    /// The built solid of a feature (for export and rendering).
    pub fn solid(&self, feature: &str) -> Option<std::sync::Arc<dc_kernel::Solid>> {
        self.state.feature(feature).map(|f| f.solid.clone())
    }

    fn sketch_info(&self, id: &str, slot: &SketchSlot) -> SketchInfo {
        SketchInfo {
            id: id.to_owned(),
            entities: slot.sketch.entities().len(),
            constraints: slot.sketch.constraints().len(),
            status: slot.report.status,
            dof: slot.report.dof,
            fully_constrained: slot.report.is_fully_constrained(),
            plane: slot.sketch.plane_spec(),
            frame: self.state.sketch_plane(id).ok().map(|p| {
                let (o, x, y, n) = (p.origin, p.x_axis, p.y_axis, p.normal());
                PlaneFrame {
                    origin: [o.x, o.y, o.z],
                    x_axis: [x.x, x.y, x.z],
                    y_axis: [y.x, y.y, y.z],
                    normal: [n.x, n.y, n.z],
                }
            }),
        }
    }

    /// Answers a query. All results are bounded (see [`crate::query::MAX_PAGE`]).
    pub fn query(&self, q: &Query) -> Result<QueryResult, Diagnostic> {
        Ok(match q {
            Query::Summary => QueryResult::Summary {
                revision: self.revision,
                param_count: self.state.len(),
                sketch_count: self.state.sketch_count(),
                feature_count: self.state.feature_count(),
                state_hash: self.state.hash(),
            },
            Query::Organization => {
                let cap = crate::query::MAX_PAGE;
                QueryResult::Organization {
                    labels: self
                        .state
                        .meta_entries("label/")
                        .take(cap)
                        .map(|(k, v)| (k["label/".len()..].to_owned(), v.to_owned()))
                        .collect(),
                    folders: self.state.folders().into_iter().take(cap).collect(),
                    suppressed: self.state.suppressed().into_iter().take(cap).collect(),
                    rollback: self.state.rollback().map(str::to_owned),
                    hidden: self.state.hidden_features().into_iter().take(cap).collect(),
                    order: self
                        .state
                        .ordered_features()
                        .into_iter()
                        .take(cap)
                        .collect(),
                }
            }
            Query::Param { name } => QueryResult::Param {
                name: name.clone(),
                value: self.state.get(name),
            },
            Query::Params {
                prefix,
                after,
                limit,
            } => {
                let (items, next) =
                    self.state
                        .param_page(prefix, after.as_deref(), clamp_limit(*limit));
                QueryResult::Params { items, next }
            }
            Query::Sketches {
                prefix,
                after,
                limit,
            } => {
                let (items, next) = page_map(
                    self.state.sketch_map(),
                    prefix,
                    after.as_deref(),
                    clamp_limit(*limit),
                    |id, slot| self.sketch_info(id, slot),
                );
                QueryResult::Sketches { items, next }
            }
            Query::SketchSummary { sketch } => {
                let slot = self
                    .state
                    .sketch(sketch)
                    .ok_or_else(|| missing_sketch(sketch))?;
                QueryResult::SketchSummary {
                    info: self.sketch_info(sketch, slot),
                    report: slot.report.clone(),
                }
            }
            Query::SketchEntities {
                sketch,
                after,
                limit,
            } => {
                let slot = self
                    .state
                    .sketch(sketch)
                    .ok_or_else(|| missing_sketch(sketch))?;
                let (items, next) = page_map(
                    slot.sketch.entities(),
                    "",
                    after.as_deref(),
                    clamp_limit(*limit),
                    |k, v: &Entity| (k.to_owned(), v.clone()),
                );
                let construction = items
                    .iter()
                    .filter(|(k, _)| slot.sketch.is_construction(k))
                    .map(|(k, _)| k.clone())
                    .collect();
                let infinite = items
                    .iter()
                    .filter(|(k, _)| slot.sketch.is_infinite(k))
                    .map(|(k, _)| k.clone())
                    .collect();
                QueryResult::SketchEntities {
                    items,
                    next,
                    construction,
                    infinite,
                }
            }
            Query::SketchConstraints {
                sketch,
                after,
                limit,
            } => {
                let slot = self
                    .state
                    .sketch(sketch)
                    .ok_or_else(|| missing_sketch(sketch))?;
                let (items, next) = page_map(
                    slot.sketch.constraints(),
                    "",
                    after.as_deref(),
                    clamp_limit(*limit),
                    |k, v: &Constraint| (k.to_owned(), v.clone()),
                );
                let placements = items
                    .iter()
                    .filter_map(|(k, _)| Some((k.clone(), slot.sketch.placement(k)?)))
                    .map(|(k, p)| (k, [p.0, p.1]))
                    .collect();
                let driven = items
                    .iter()
                    .filter(|(k, _)| slot.sketch.is_driven(k))
                    .map(|(k, _)| k.clone())
                    .collect();
                let styles = items
                    .iter()
                    .filter_map(|(k, _)| Some((k.clone(), slot.sketch.style(k)?.clone())))
                    .collect();
                QueryResult::SketchConstraints {
                    items,
                    next,
                    placements,
                    driven,
                    styles,
                }
            }
            Query::SketchProfiles {
                sketch,
                after,
                limit,
            } => {
                let slot = self
                    .state
                    .sketch(sketch)
                    .ok_or_else(|| missing_sketch(sketch))?;
                let report = slot.sketch.profiles();
                let limit = clamp_limit(*limit);
                let eligible: Vec<_> = report
                    .profiles
                    .iter()
                    .filter(|p| after.as_deref().is_none_or(|a| p.id.as_str() > a))
                    .collect();
                let profiles: Vec<ProfileInfo> = eligible
                    .iter()
                    .take(limit)
                    .map(|p| ProfileInfo {
                        id: p.id.clone(),
                        area: p.area,
                        outer: p.outer.edges.iter().map(|e| e.id.clone()).collect(),
                        holes: p
                            .holes
                            .iter()
                            .map(|h| h.edges.iter().map(|e| e.id.clone()).collect())
                            .collect(),
                    })
                    .collect();
                let next = (eligible.len() > limit)
                    .then(|| profiles.last().map(|p| p.id.clone()))
                    .flatten();
                QueryResult::SketchProfiles {
                    profiles,
                    next,
                    problem_count: report.problems.len(),
                    problems: report.problems.into_iter().take(100).collect(),
                }
            }
            Query::Features {
                prefix,
                after,
                limit,
            } => {
                let (items, next) = page_map(
                    self.state.feature_map(),
                    prefix,
                    after.as_deref(),
                    clamp_limit(*limit),
                    Self::feature_info,
                );
                QueryResult::Features { items, next }
            }
            Query::FeatureSummary { feature } => {
                let slot = self
                    .state
                    .feature(feature)
                    .ok_or_else(|| missing_feature(feature))?;
                QueryResult::FeatureSummary {
                    info: Self::feature_info(feature, slot),
                }
            }
            Query::FeatureFaces {
                feature,
                after,
                limit,
            } => {
                let slot = self
                    .state
                    .feature(feature)
                    .ok_or_else(|| missing_feature(feature))?;
                let limit = clamp_limit(*limit);
                let eligible: Vec<_> = feature::face_infos(&slot.solid)
                    .into_iter()
                    .filter(|f| after.as_deref().is_none_or(|a| f.name.as_str() > a))
                    .collect();
                let next = (eligible.len() > limit).then(|| eligible[limit - 1].name.clone());
                QueryResult::FeatureFaces {
                    items: eligible.into_iter().take(limit).collect(),
                    next,
                }
            }
            Query::Log { from, limit } => {
                let start = from.saturating_sub(1) as usize;
                let limit = clamp_limit(*limit);
                let slice = self.log.get(start..).unwrap_or(&[]);
                let entries: Vec<LogEntry> = slice.iter().take(limit).cloned().collect();
                let next = (slice.len() > limit).then(|| (start + limit + 1) as u64);
                QueryResult::Log { entries, next }
            }
        })
    }

    /// Serializes the log as deterministic JSON Lines (`history.jsonl`).
    pub fn log_jsonl(&self) -> String {
        let mut out = String::new();
        for e in &self.log {
            out.push_str(&serde_json::to_string(e).expect("log entries always serialize"));
            out.push('\n');
        }
        out
    }

    /// Rebuilds a document from `history.jsonl`, verifying revision order and the state hash after
    /// every entry. A divergence (e.g. non-deterministic code) is detected at the exact revision.
    pub fn replay(jsonl: &str, policy: Policy) -> Result<Self, ReplayError> {
        let mut a = Authority::new(policy);
        for (i, line) in jsonl.lines().enumerate() {
            let line_no = i + 1;
            if line.trim().is_empty() {
                continue;
            }
            let entry: LogEntry = serde_json::from_str(line).map_err(|e| ReplayError::Parse {
                line: line_no,
                message: e.to_string(),
            })?;
            let expected = Revision(a.revision.0 + 1);
            if entry.revision != expected {
                return Err(ReplayError::Revision {
                    line: line_no,
                    expected,
                    found: entry.revision,
                });
            }
            // History is already authorized; replay does not re-check roles.
            a.run_command(&entry.command.actor, &entry.command.kind, entry.revision.0)
                .map_err(|diagnostic| ReplayError::Apply {
                    line: line_no,
                    diagnostic,
                })?;
            let actual = a.state.hash();
            if actual != entry.state_hash {
                return Err(ReplayError::HashMismatch {
                    revision: entry.revision,
                    expected: entry.state_hash,
                    actual,
                });
            }
            a.revision = entry.revision;
            a.seen.insert(entry.command.id.clone());
            a.log.push(entry);
        }
        Ok(a)
    }
}

// ---------------------------------------------------------------------------------------------
// Undo / redo
// ---------------------------------------------------------------------------------------------

/// Something a command can change: a parameter, a sketch or a feature.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ItemKey {
    Param(String),
    Sketch(String),
    Feature(String),
    Meta(String),
}

impl ItemKey {
    fn path(&self) -> String {
        match self {
            ItemKey::Param(n) => n.clone(),
            ItemKey::Sketch(i) => sketch_path(i),
            ItemKey::Feature(i) => feature_path(i),
            ItemKey::Meta(k) => format!("meta/{k}"),
        }
    }
}

fn keys_of(undo: &[Undo]) -> BTreeSet<ItemKey> {
    undo.iter()
        .map(|u| match u {
            Undo::Param { name, .. } => ItemKey::Param(name.clone()),
            Undo::Sketch { id, .. } => ItemKey::Sketch(id.clone()),
            Undo::Feature { id, .. } => ItemKey::Feature(id.clone()),
            Undo::Meta { key, .. } => ItemKey::Meta(key.clone()),
        })
        .collect()
}

/// The value of one item at one moment.
#[derive(Clone, Debug)]
enum ItemSnap {
    Param(String, Option<f64>),
    Sketch(String, Option<SketchSlot>),
    Feature(String, Option<FeatureSlot>),
    Meta(String, Option<String>),
}

fn capture(state: &State, k: &ItemKey) -> ItemSnap {
    match k {
        ItemKey::Param(n) => ItemSnap::Param(n.clone(), state.get(n)),
        ItemKey::Sketch(i) => ItemSnap::Sketch(i.clone(), state.sketch(i).cloned()),
        ItemKey::Feature(i) => ItemSnap::Feature(i.clone(), state.feature(i).cloned()),
        ItemKey::Meta(k) => ItemSnap::Meta(k.clone(), state.meta(k).map(str::to_owned)),
    }
}

fn restore(state: &mut State, s: &ItemSnap) {
    match s {
        ItemSnap::Param(n, Some(v)) => {
            state.set(n, *v);
        }
        ItemSnap::Param(n, None) => {
            state.remove(n);
        }
        ItemSnap::Sketch(i, Some(slot)) => {
            state.put_sketch(i, slot.clone());
        }
        ItemSnap::Sketch(i, None) => {
            state.remove_sketch(i);
        }
        ItemSnap::Feature(i, Some(slot)) => {
            state.put_feature(i, slot.clone());
        }
        ItemSnap::Feature(i, None) => {
            state.remove_feature(i);
        }
        ItemSnap::Meta(k, v) => {
            state.set_meta(k, v.clone());
        }
    }
}

/// What changed between two snapshots of the same item, as diff records.
fn diff_snaps(from: &ItemSnap, to: &ItemSnap) -> Vec<Change> {
    match (from, to) {
        (ItemSnap::Param(n, a), ItemSnap::Param(_, b)) if a != b => vec![Change {
            entity: n.clone(),
            before: a.map(Value::from),
            after: b.map(Value::from),
        }],
        (ItemSnap::Sketch(i, Some(a)), ItemSnap::Sketch(_, Some(b))) => {
            diff_sketch(i, &a.sketch, &b.sketch)
        }
        (ItemSnap::Sketch(i, a), ItemSnap::Sketch(_, b)) if a.is_some() != b.is_some() => {
            vec![Change {
                entity: sketch_path(i),
                before: a.as_ref().map(|s| to_json(&s.sketch)),
                after: b.as_ref().map(|s| to_json(&s.sketch)),
            }]
        }
        (ItemSnap::Feature(i, a), ItemSnap::Feature(_, b)) => {
            let (va, vb) = (a.as_ref().map(|f| f.view()), b.as_ref().map(|f| f.view()));
            if va != vb {
                vec![Change {
                    entity: feature_path(i),
                    before: va.as_ref().map(to_json),
                    after: vb.as_ref().map(to_json),
                }]
            } else {
                vec![]
            }
        }
        (ItemSnap::Meta(k, a), ItemSnap::Meta(_, b)) if a != b => vec![Change {
            entity: format!("meta/{k}"),
            before: a.clone().map(Value::from),
            after: b.clone().map(Value::from),
        }],
        _ => vec![],
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Active,
    Undone,
}

/// What one committed command changed, kept so it can be undone and redone.
#[derive(Clone, Debug)]
struct Record {
    actor: Actor,
    keys: BTreeSet<ItemKey>,
    /// Inverse of the command (the journal it produced).
    undo: Vec<Undo>,
    status: Status,
    /// Revision of the undo command, so "most recently undone" is well defined.
    undone_at: u64,
    /// State of the touched items just before they were undone: what redo restores.
    redo: Vec<ItemSnap>,
}

struct Effects {
    changes: Vec<Change>,
    reports: BTreeMap<String, SolveReport>,
}

/// Enough to put things back exactly as they were after a dry run of undo/redo.
struct Rollback {
    target: u64,
    snaps: Vec<ItemSnap>,
    status: Status,
    undone_at: u64,
    redo: Vec<ItemSnap>,
}

fn nothing_to_undo(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new(DiagCode::NothingToUndo, msg).suggest(
        "query the log to see your revisions; only your own un-undone commands can be undone",
    )
}

impl Authority {
    /// Fills in "my latest un-undone command" / "my latest undone command" when the client left
    /// the revision out, so the logged command always names a concrete revision.
    fn resolve_history(&self, cmd: &mut Command) -> Result<(), Diagnostic> {
        let actor = cmd.actor.clone();
        match &mut cmd.kind {
            CommandKind::HistoryUndo { revision } if revision.is_none() => {
                let t = self
                    .records
                    .iter()
                    .rev()
                    .find(|(_, r)| r.actor == actor && r.status == Status::Active)
                    .map(|(k, _)| *k)
                    .ok_or_else(|| nothing_to_undo(format!("{actor} has no command to undo")))?;
                *revision = Some(Revision(t));
            }
            CommandKind::HistoryRedo { revision } if revision.is_none() => {
                let t = self
                    .records
                    .iter()
                    .filter(|(_, r)| r.actor == actor && r.status == Status::Undone)
                    .max_by_key(|(_, r)| r.undone_at)
                    .map(|(k, _)| *k)
                    .ok_or_else(|| {
                        nothing_to_undo(format!("{actor} has no undone command to redo"))
                    })?;
                *revision = Some(Revision(t));
            }
            _ => {}
        }
        Ok(())
    }

    /// Applies a command's effect to the state and keeps undo bookkeeping. The same code path
    /// serves `submit`, `dry_run` and `replay`.
    fn run_command(
        &mut self,
        actor: &Actor,
        kind: &CommandKind,
        rev: u64,
    ) -> Result<(Effects, Option<Rollback>), Diagnostic> {
        match kind {
            CommandKind::HistoryUndo { revision } => {
                let t = revision.ok_or_else(|| nothing_to_undo("no revision to undo"))?;
                self.undo(actor, t.0, rev).map(|(e, r)| (e, Some(r)))
            }
            CommandKind::HistoryRedo { revision } => {
                let t = revision.ok_or_else(|| nothing_to_undo("no revision to redo"))?;
                self.redo(actor, t.0).map(|(e, r)| (e, Some(r)))
            }
            _ => {
                let j = apply(&mut self.state, kind)?;
                let keys = keys_of(&j.undo);
                // Undo of dry runs is handled by the caller via the journal, so remember how.
                self.records.insert(
                    rev,
                    Record {
                        actor: actor.clone(),
                        keys,
                        undo: j.undo,
                        status: Status::Active,
                        undone_at: 0,
                        redo: Vec::new(),
                    },
                );
                Ok((
                    Effects {
                        changes: j.changes,
                        reports: j.reports,
                    },
                    None,
                ))
            }
        }
    }

    /// Reverses a dry run: for ordinary commands drop the record and restore the state through
    /// the stored inverse; for undo/redo restore the snapshots taken beforehand.
    fn rollback(&mut self, rev: u64, rb: Option<Rollback>) {
        match rb {
            None => {
                if let Some(rec) = self.records.remove(&rev) {
                    revert(&mut self.state, &rec.undo);
                }
            }
            Some(r) => {
                for s in &r.snaps {
                    restore(&mut self.state, s);
                }
                if let Some(rec) = self.records.get_mut(&r.target) {
                    rec.status = r.status;
                    rec.undone_at = r.undone_at;
                    rec.redo = r.redo;
                }
            }
        }
    }

    /// Later, still-active revisions that touched any of `keys`.
    fn conflicts(&self, target: u64, keys: &BTreeSet<ItemKey>) -> (Vec<u64>, BTreeSet<ItemKey>) {
        let mut revs = Vec::new();
        let mut items = BTreeSet::new();
        for (r, rec) in self.records.range(target + 1..) {
            if rec.status == Status::Active {
                let hit: Vec<_> = rec.keys.intersection(keys).cloned().collect();
                if !hit.is_empty() {
                    revs.push(*r);
                    items.extend(hit);
                }
            }
        }
        (revs, items)
    }

    fn conflict_error(
        verb: &str,
        target: u64,
        revs: &[u64],
        items: &BTreeSet<ItemKey>,
    ) -> Diagnostic {
        let mut d = Diagnostic::new(
            DiagCode::UndoConflict,
            format!(
                "cannot {verb} revision {target}: later revision(s) {} changed the same items",
                revs.iter()
                    .map(u64::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
        .suggest(format!(
            "{verb} those later revisions first, or make the change you want as a new command"
        ));
        for i in items {
            d = d.entity(i.path());
        }
        d
    }

    fn check_owner(&self, actor: &Actor, target: u64, want: Status) -> Result<&Record, Diagnostic> {
        let rec = self.records.get(&target).ok_or_else(|| {
            nothing_to_undo(format!("revision {target} is not an undoable command"))
        })?;
        if &rec.actor != actor {
            return Err(Diagnostic::new(
                DiagCode::PermissionDenied,
                format!(
                    "revision {target} was made by {}; only they can undo or redo it",
                    rec.actor
                ),
            ));
        }
        if rec.status != want {
            let msg = match want {
                Status::Active => format!("revision {target} was already undone"),
                Status::Undone => {
                    format!("revision {target} is not undone, so there is nothing to redo")
                }
            };
            return Err(nothing_to_undo(msg));
        }
        Ok(rec)
    }

    fn effects_between(&self, from: &[ItemSnap], keys: &BTreeSet<ItemKey>) -> Effects {
        let mut changes = Vec::new();
        let mut reports = BTreeMap::new();
        for (a, k) in from.iter().zip(keys) {
            let b = capture(&self.state, k);
            changes.extend(diff_snaps(a, &b));
            if let ItemSnap::Sketch(id, Some(slot)) = &b {
                reports.insert(id.clone(), slot.report.clone());
            }
        }
        Effects { changes, reports }
    }

    fn undo(
        &mut self,
        actor: &Actor,
        target: u64,
        rev: u64,
    ) -> Result<(Effects, Rollback), Diagnostic> {
        let rec = self.check_owner(actor, target, Status::Active)?;
        let (keys, inverse) = (rec.keys.clone(), rec.undo.clone());
        let (revs, items) = self.conflicts(target, &keys);
        if !revs.is_empty() {
            return Err(Self::conflict_error("undo", target, &revs, &items));
        }
        let current: Vec<ItemSnap> = keys.iter().map(|k| capture(&self.state, k)).collect();
        revert(&mut self.state, &inverse);
        let eff = self.effects_between(&current, &keys);
        let rec = self.records.get_mut(&target).expect("checked above");
        rec.status = Status::Undone;
        rec.undone_at = rev;
        rec.redo = current.clone();
        Ok((
            eff,
            Rollback {
                target,
                snaps: current,
                status: Status::Active,
                undone_at: 0,
                redo: Vec::new(),
            },
        ))
    }

    fn redo(&mut self, actor: &Actor, target: u64) -> Result<(Effects, Rollback), Diagnostic> {
        let rec = self.check_owner(actor, target, Status::Undone)?;
        let (keys, after, undone_at) = (rec.keys.clone(), rec.redo.clone(), rec.undone_at);
        let (revs, items) = self.conflicts(target, &keys);
        if !revs.is_empty() {
            return Err(Self::conflict_error("redo", target, &revs, &items));
        }
        let current: Vec<ItemSnap> = keys.iter().map(|k| capture(&self.state, k)).collect();
        for s in &after {
            restore(&mut self.state, s);
        }
        let eff = self.effects_between(&current, &keys);
        let rec = self.records.get_mut(&target).expect("checked above");
        rec.status = Status::Active;
        rec.redo = Vec::new();
        Ok((
            eff,
            Rollback {
                target,
                snaps: current,
                status: Status::Undone,
                undone_at,
                redo: after,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn human() -> Actor {
        Actor::Human("ann".into())
    }

    fn set(
        id: &str,
        actor: Actor,
        base: u64,
        name: &str,
        value: f64,
        expected: Expected,
    ) -> Command {
        Command {
            id: id.into(),
            actor,
            base_revision: Revision(base),
            kind: CommandKind::ParamSet {
                name: name.into(),
                value,
                expected,
            },
        }
    }

    #[test]
    fn set_and_query() {
        let mut a = Authority::default();
        let out = a
            .submit(set("c1", human(), 0, "depth", 10.0, Expected::Absent))
            .unwrap();
        assert_eq!(out.revision, Revision(1));
        assert_eq!(out.changes[0].before, None);
        assert_eq!(
            a.query(&Query::Param {
                name: "depth".into()
            })
            .unwrap(),
            QueryResult::Param {
                name: "depth".into(),
                value: Some(10.0)
            }
        );
    }

    #[test]
    fn stale_expectation_is_a_structured_error_and_changes_nothing() {
        let mut a = Authority::default();
        a.submit(set("c1", human(), 0, "depth", 10.0, Expected::Any))
            .unwrap();
        a.submit(set("c2", human(), 1, "depth", 12.0, Expected::Value(10.0)))
            .unwrap();
        let h = a.state_hash();
        // Another actor still thinks depth is 10.
        let err = a
            .submit(set(
                "c3",
                Actor::Script("s".into()),
                1,
                "depth",
                15.0,
                Expected::Value(10.0),
            ))
            .unwrap_err();
        assert_eq!(err.code, DiagCode::StaleExpectation);
        assert_eq!(err.entities, vec!["depth".to_string()]);
        assert_eq!(a.state_hash(), h);
        assert_eq!(a.revision(), Revision(2));
    }

    #[test]
    fn transaction_is_atomic() {
        let mut a = Authority::default();
        a.submit(set("c1", human(), 0, "a", 1.0, Expected::Any))
            .unwrap();
        let h = a.state_hash();
        let tx = Command {
            id: "t1".into(),
            actor: human(),
            base_revision: Revision(1),
            kind: CommandKind::Transaction {
                label: "two edits, second fails".into(),
                ops: vec![
                    CommandKind::ParamSet {
                        name: "a".into(),
                        value: 2.0,
                        expected: Expected::Any,
                    },
                    CommandKind::ParamSet {
                        name: "b".into(),
                        value: f64::NAN,
                        expected: Expected::Any,
                    },
                ],
            },
        };
        let err = a.submit(tx).unwrap_err();
        assert_eq!(err.code, DiagCode::InvalidValue);
        assert_eq!(
            a.state().get("a"),
            Some(1.0),
            "first op must be rolled back"
        );
        assert_eq!(a.state_hash(), h);
        assert_eq!(a.revision(), Revision(1));
    }

    #[test]
    fn nested_and_empty_transactions_rejected() {
        let mut a = Authority::default();
        let nested = CommandKind::Transaction {
            label: "outer".into(),
            ops: vec![CommandKind::Transaction {
                label: "inner".into(),
                ops: vec![],
            }],
        };
        let c = Command {
            id: "n".into(),
            actor: human(),
            base_revision: Revision(0),
            kind: nested,
        };
        assert_eq!(a.submit(c).unwrap_err().code, DiagCode::NestedTransaction);
        let empty = Command {
            id: "e".into(),
            actor: human(),
            base_revision: Revision(0),
            kind: CommandKind::Transaction {
                label: "x".into(),
                ops: vec![],
            },
        };
        assert_eq!(
            a.submit(empty).unwrap_err().code,
            DiagCode::EmptyTransaction
        );
    }

    #[test]
    fn dry_run_commits_nothing_but_predicts_exactly() {
        let mut a = Authority::default();
        a.submit(set("c1", human(), 0, "x", 1.0, Expected::Any))
            .unwrap();
        let before = a.state_hash();
        let cmd = set("c2", human(), 1, "x", 2.0, Expected::Value(1.0));
        let predicted = a.dry_run(&cmd).unwrap();
        assert_eq!(a.state_hash(), before);
        assert_eq!(a.revision(), Revision(1));
        let real = a.submit(cmd).unwrap();
        assert_eq!(predicted, real);
    }

    #[test]
    fn duplicate_ids_and_future_revisions_rejected() {
        let mut a = Authority::default();
        a.submit(set("c1", human(), 0, "x", 1.0, Expected::Any))
            .unwrap();
        assert_eq!(
            a.submit(set("c1", human(), 1, "x", 2.0, Expected::Any))
                .unwrap_err()
                .code,
            DiagCode::DuplicateCommand
        );
        assert_eq!(
            a.submit(set("c9", human(), 7, "x", 2.0, Expected::Any))
                .unwrap_err()
                .code,
            DiagCode::FutureRevision
        );
    }

    #[test]
    fn roles_gate_actors() {
        let mut a = Authority::default();
        let agent = Actor::Agent("claude".into());
        let e = a
            .submit(set("c1", agent.clone(), 0, "x", 1.0, Expected::Any))
            .unwrap_err();
        assert_eq!(e.code, DiagCode::ProposalRequired);
        a.set_role(&agent, Role::ReadOnly);
        let e = a
            .submit(set("c2", agent.clone(), 0, "x", 1.0, Expected::Any))
            .unwrap_err();
        assert_eq!(e.code, DiagCode::PermissionDenied);
        a.set_role(&agent, Role::Full);
        a.submit(set("c3", agent.clone(), 0, "x", 1.0, Expected::Any))
            .unwrap();
        // Attribution is recorded.
        assert_eq!(a.log()[0].command.actor, agent);
    }

    #[test]
    fn negative_zero_is_canonicalized() {
        let mut a = Authority::default();
        let mut b = Authority::default();
        a.submit(set("c1", human(), 0, "x", -0.0, Expected::Any))
            .unwrap();
        b.submit(set("c1", human(), 0, "x", 0.0, Expected::Any))
            .unwrap();
        assert_eq!(a.state_hash(), b.state_hash());
        assert_eq!(a.log_jsonl(), b.log_jsonl());
    }

    #[test]
    fn incremental_hash_matches_recompute() {
        let mut a = Authority::default();
        for i in 0..200u64 {
            let name = format!("p{}", i % 37);
            let cmd = if i % 11 == 10 && a.state().get(&name).is_some() {
                Command {
                    id: format!("c{i}"),
                    actor: human(),
                    base_revision: a.revision(),
                    kind: CommandKind::ParamDelete {
                        name,
                        expected: Expected::Any,
                    },
                }
            } else {
                set(
                    &format!("c{i}"),
                    human(),
                    a.revision().0,
                    &name,
                    i as f64 * 0.5,
                    Expected::Any,
                )
            };
            a.submit(cmd).unwrap();
            assert_eq!(a.state().hash(), a.state().recompute_hash());
        }
    }

    #[test]
    fn replay_reproduces_state_and_detects_tampering() {
        let mut a = Authority::default();
        for i in 0..20u64 {
            a.submit(set(
                &format!("c{i}"),
                human(),
                i,
                &format!("p{}", i % 5),
                i as f64,
                Expected::Any,
            ))
            .unwrap();
        }
        let jsonl = a.log_jsonl();
        let b = Authority::replay(&jsonl, Policy::default()).unwrap();
        assert_eq!(b.state_hash(), a.state_hash());
        assert_eq!(b.revision(), a.revision());
        assert_eq!(b.log_jsonl(), jsonl, "serialization must be byte-identical");

        let tampered = jsonl.replacen("\"value\":3.0", "\"value\":3.5", 1);
        assert_ne!(tampered, jsonl);
        match Authority::replay(&tampered, Policy::default()) {
            Err(ReplayError::HashMismatch { .. }) => {}
            other => panic!("expected hash mismatch, got {other:?}"),
        }
    }

    #[test]
    fn queries_are_bounded_and_paginated() {
        let mut a = Authority::default();
        for i in 0..2500u64 {
            a.submit(set(
                &format!("c{i}"),
                human(),
                i,
                &format!("p{i:05}"),
                i as f64,
                Expected::Any,
            ))
            .unwrap();
        }
        // Absurd limit is clamped.
        match a
            .query(&Query::Params {
                prefix: String::new(),
                after: None,
                limit: Some(1_000_000),
            })
            .unwrap()
        {
            QueryResult::Params { items, next } => {
                assert_eq!(items.len(), crate::query::MAX_PAGE);
                assert!(next.is_some());
            }
            _ => unreachable!(),
        }
        // Walk all pages: complete, ordered, no duplicates.
        let mut all = Vec::new();
        let mut after = None;
        loop {
            match a
                .query(&Query::Params {
                    prefix: "p".into(),
                    after: after.clone(),
                    limit: Some(700),
                })
                .unwrap()
            {
                QueryResult::Params { items, next } => {
                    all.extend(items);
                    if next.is_none() {
                        break;
                    }
                    after = next;
                }
                _ => unreachable!(),
            }
        }
        assert_eq!(all.len(), 2500);
        assert!(all.windows(2).all(|w| w[0].0 < w[1].0));
        // Log pages too.
        match a
            .query(&Query::Log {
                from: 2400,
                limit: Some(50),
            })
            .unwrap()
        {
            QueryResult::Log { entries, next } => {
                assert_eq!(entries.len(), 50);
                assert_eq!(entries[0].revision, Revision(2400));
                assert_eq!(next, Some(2450));
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn command_json_shape_is_agent_friendly() {
        let c = set(
            "c1",
            Actor::Agent("claude".into()),
            3,
            "depth",
            12.5,
            Expected::Value(10.0),
        );
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(
            json,
            r#"{"id":"c1","actor":{"kind":"agent","name":"claude"},"base_revision":3,"type":"param.set","name":"depth","value":12.5,"expected":{"value":10.0}}"#
        );
        let back: Command = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
        // `expected` is optional on input.
        let minimal = r#"{"id":"c2","actor":{"kind":"human","name":"ann"},"base_revision":0,"type":"param.set","name":"w","value":1}"#;
        let m: Command = serde_json::from_str(minimal).unwrap();
        assert!(matches!(
            m.kind,
            CommandKind::ParamSet {
                expected: Expected::Any,
                ..
            }
        ));
    }
}
