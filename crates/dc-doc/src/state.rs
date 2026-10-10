use crate::diag::{DiagCode, Diagnostic};
use crate::feature::FeatureSlot;
use crate::Hash128;
use dc_kernel::FaceFrameError;
use dc_math::Plane;
use dc_sketch::PlaneSpec;
use dc_sketch::{Sketch, SolveReport};
use std::collections::BTreeMap;
use std::ops::Bound;

/// A sketch together with the result of its last solve. Committed sketches are always solved;
/// the report is derived data (not hashed) cached for queries.
#[derive(Clone, Debug)]
pub struct SketchSlot {
    pub sketch: Sketch,
    pub report: SolveReport,
}

/// Document content: named numeric parameters and sketches.
///
/// The state hash is maintained **incrementally**: each item contributes a leaf hash that is
/// added to / removed from an accumulator, so a change costs O(changed item), not O(model size).
/// The accumulator is an order-independent multiset hash (wrapping sum of 128-bit leaf hashes).
/// `[OPEN]`: replace with a proper hierarchical Merkle tree once the document has more structure
/// (AGENTS.md §4.5, §10a).
#[derive(Clone, Debug, Default)]
pub struct State {
    params: BTreeMap<String, f64>,
    sketches: BTreeMap<String, SketchSlot>,
    features: BTreeMap<String, FeatureSlot>,
    /// How the document is organised and what is hidden: names shown to people (labels), folders,
    /// suppressed features and the rollback position. Keys are paths such as `label/feature/<id>`,
    /// `folder/<name>`, `suppress/<id>` and `rollback`; it is part of the design, so it is hashed
    /// and undoable like the rest. Empty for documents that use none of it (their hash is unchanged).
    meta: BTreeMap<String, String>,
    acc: u128,
}

fn param_leaf(name: &str, value: f64) -> u128 {
    Hash128::of(&[b"param", name.as_bytes(), &value.to_bits().to_le_bytes()]).0
}

fn feature_leaf(id: &str, f: &FeatureSlot) -> u128 {
    // The recipe plus the digest of what it built: replaying on a machine whose kernel computes
    // anything differently changes this hash and is caught immediately.
    let spec = serde_json::to_vec(&f.spec).expect("specs always serialize");
    Hash128::of(&[
        b"feature",
        id.as_bytes(),
        &spec,
        &f.summary.digest.0.to_le_bytes(),
    ])
    .0
}

fn meta_leaf(key: &str, value: &str) -> u128 {
    Hash128::of(&[b"meta", key.as_bytes(), value.as_bytes()]).0
}

fn sketch_leaf(id: &str, sketch: &Sketch) -> u128 {
    // BTreeMap-ordered, shortest-round-trip floats: byte-identical for equal sketches.
    let json = serde_json::to_vec(sketch).expect("sketches always serialize");
    Hash128::of(&[b"sketch", id.as_bytes(), &json]).0
}

/// Bounded, ordered page of a map keyed by name: entries starting with `prefix`, after the
/// cursor. Returns the mapped items and, if more remain, the cursor for the next page.
pub(crate) fn page_map<V, T>(
    map: &BTreeMap<String, V>,
    prefix: &str,
    after: Option<&str>,
    limit: usize,
    f: impl Fn(&str, &V) -> T,
) -> (Vec<T>, Option<String>) {
    let start = match after {
        Some(a) if a >= prefix => Bound::Excluded(a),
        _ => Bound::Included(prefix),
    };
    let mut iter = map
        .range::<str, _>((start, Bound::Unbounded))
        .take_while(|(k, _)| k.starts_with(prefix));
    let mut items = Vec::new();
    let mut last = None;
    for (k, v) in iter.by_ref().take(limit) {
        items.push(f(k, v));
        last = Some(k.clone());
    }
    let next = if iter.next().is_some() { last } else { None };
    (items, next)
}

impl State {
    pub fn get(&self, name: &str) -> Option<f64> {
        self.params.get(name).copied()
    }

    pub fn len(&self) -> usize {
        self.params.len()
    }

    pub fn is_empty(&self) -> bool {
        self.params.is_empty() && self.sketches.is_empty() && self.features.is_empty()
    }

    pub fn sketch_count(&self) -> usize {
        self.sketches.len()
    }

    /// All feature IDs, in order.
    pub fn feature_ids(&self) -> impl Iterator<Item = &str> {
        self.features.keys().map(String::as_str)
    }

    /// All sketch IDs, in order.
    pub fn sketch_ids(&self) -> impl Iterator<Item = &str> {
        self.sketches.keys().map(String::as_str)
    }

    /// The features that are final bodies: nothing cuts or adds to them. A cut's target is a
    /// stage on the way to the cut's result, so only the end of each chain is shown or exported.
    ///
    /// Features that are hidden (suppressed, after the rollback bar, or built on such) are not
    /// part of the model, so the feature they were built on becomes the end of its chain again.
    pub fn result_features(&self) -> Vec<String> {
        let hidden = self.hidden_features();
        let targeted: std::collections::BTreeSet<&str> = self
            .features
            .iter()
            .filter(|(id, _)| !hidden.contains(*id))
            .filter_map(|(_, f)| f.spec.target())
            .collect();
        self.features
            .keys()
            .filter(|k| !targeted.contains(k.as_str()) && !hidden.contains(*k))
            .cloned()
            .collect()
    }

    pub fn feature_count(&self) -> usize {
        self.features.len()
    }

    pub fn feature(&self, id: &str) -> Option<&FeatureSlot> {
        self.features.get(id)
    }

    /// IDs of the features built from `sketch`, in order.
    pub fn features_of_sketch(&self, sketch: &str) -> Vec<String> {
        self.features
            .iter()
            .filter(|(_, f)| f.spec.sketches().contains(&sketch))
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// The features `slot` is built from: the one it cuts or adds to, and the one whose face its
    /// sketch is placed on.
    fn feature_deps(&self, slot: &FeatureSlot) -> Vec<String> {
        let mut deps: Vec<String> = slot.spec.target().map(str::to_owned).into_iter().collect();
        for id in slot.spec.sketches() {
            if let Some(sk) = self.sketches.get(id) {
                if let Some(f) = sk.sketch.plane_spec().attached_feature() {
                    if !deps.iter().any(|d| d == f) {
                        deps.push(f.to_owned());
                    }
                }
            }
        }
        deps
    }

    /// IDs of the features that must be rebuilt when `feature` changes: those built on it and
    /// those whose sketch sits on one of its faces. In order.
    pub fn features_depending_on(&self, feature: &str) -> Vec<String> {
        self.features
            .iter()
            .filter(|(_, f)| self.feature_deps(f).iter().any(|d| d == feature))
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// IDs of the sketches placed on a face of `feature`, in order.
    pub fn sketches_on_feature(&self, feature: &str) -> Vec<String> {
        self.sketches
            .iter()
            .filter(|(_, s)| s.sketch.plane_spec().attached_feature() == Some(feature))
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// How many features deep this one sits: 0 if it depends on no other feature, otherwise one
    /// more than the deepest feature it depends on. Rebuilding in this order is always safe.
    pub fn feature_depth(&self, id: &str) -> usize {
        match self.features.get(id) {
            None => 0,
            Some(f) => self
                .feature_deps(f)
                .iter()
                .map(|d| 1 + self.feature_depth(d))
                .max()
                .unwrap_or(0),
        }
    }

    /// The 3D plane a sketch spec refers to, with an explanation if it cannot be resolved.
    pub fn resolve_plane(&self, spec: &PlaneSpec) -> Result<Plane, Diagnostic> {
        match spec {
            PlaneSpec::Face {
                feature,
                face,
                offset,
            } => {
                let slot = self.features.get(feature).ok_or_else(|| {
                    Diagnostic::new(
                        DiagCode::NotFound,
                        format!(
                            "a sketch plane refers to feature '{feature}', which does not exist"
                        ),
                    )
                    .entity(format!("feature/{feature}"))
                    .suggest("create the feature first, or place the sketch on a world plane")
                })?;
                let (origin, normal, x) = slot.solid.planar_face_frame(face).map_err(|e| match e {
                    FaceFrameError::NotFound { available } => {
                        let shown: Vec<&str> = available.iter().take(20).map(String::as_str).collect();
                        // A face that an edit split into several is named `<face>#<suffix>` now.
                        let prefix = format!("{face}#");
                        let pieces: Vec<&str> = available
                            .iter()
                            .filter(|n| n.starts_with(&prefix))
                            .map(String::as_str)
                            .collect();
                        let split = if pieces.is_empty() {
                            String::new()
                        } else {
                            format!("; it was split into {}", pieces.join(", "))
                        };
                        Diagnostic::new(
                            DiagCode::NotFound,
                            format!(
                                "feature '{feature}' has no face '{face}' (faces: {}{}){split}",
                                shown.join(", "),
                                if available.len() > shown.len() { ", ..." } else { "" }
                            ),
                        )
                        .entity(format!("feature/{feature}"))
                        .suggest("query feature.faces for the current face names; a face disappears when an edit removes it")
                    }
                    FaceFrameError::NotPlanar => Diagnostic::new(
                        DiagCode::WrongKind,
                        format!("face '{face}' of '{feature}' is curved: sketches can only be placed on flat faces"),
                    )
                    .entity(format!("feature/{feature}")),
                })?;
                Ok(Plane {
                    origin,
                    x_axis: x,
                    y_axis: normal.cross(x),
                }
                .offset(*offset))
            }
            named => Ok(named.named_plane().expect("world planes always resolve")),
        }
    }

    /// The 3D plane an existing sketch lies in.
    pub fn sketch_plane(&self, id: &str) -> Result<Plane, Diagnostic> {
        let sk = self.sketches.get(id).ok_or_else(|| {
            Diagnostic::new(DiagCode::NotFound, format!("sketch '{id}' does not exist"))
                .entity(format!("sketch/{id}"))
        })?;
        self.resolve_plane(&sk.sketch.plane_spec())
    }

    pub fn put_feature(&mut self, id: &str, slot: FeatureSlot) -> Option<FeatureSlot> {
        let leaf = feature_leaf(id, &slot);
        let old = self.features.insert(id.to_owned(), slot);
        if let Some(o) = &old {
            self.acc = self.acc.wrapping_sub(feature_leaf(id, o));
        }
        self.acc = self.acc.wrapping_add(leaf);
        old
    }

    pub fn remove_feature(&mut self, id: &str) -> Option<FeatureSlot> {
        let old = self.features.remove(id);
        if let Some(o) = &old {
            self.acc = self.acc.wrapping_sub(feature_leaf(id, o));
        }
        old
    }

    /// One organisation entry (see the `meta` field).
    pub fn meta(&self, key: &str) -> Option<&str> {
        self.meta.get(key).map(String::as_str)
    }

    /// Sets (or with `None` removes) an organisation entry, returning what it was.
    pub fn set_meta(&mut self, key: &str, value: Option<String>) -> Option<String> {
        let old = match &value {
            Some(v) => self.meta.insert(key.to_owned(), v.clone()),
            None => self.meta.remove(key),
        };
        if let Some(o) = &old {
            self.acc = self.acc.wrapping_sub(meta_leaf(key, o));
        }
        if let Some(v) = &value {
            self.acc = self.acc.wrapping_add(meta_leaf(key, v));
        }
        old
    }

    /// The entries whose key starts with `prefix`, in key order.
    pub fn meta_entries<'a>(
        &'a self,
        prefix: &'a str,
    ) -> impl Iterator<Item = (&'a str, &'a str)> + 'a {
        self.meta
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .take_while(move |(k, _)| k.starts_with(prefix))
            .map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// The name people see for a feature or sketch (`kind` is `feature` or `sketch`); the ID
    /// itself if it has no label.
    pub fn label<'a>(&'a self, kind: &str, id: &'a str) -> &'a str {
        self.meta(&format!("label/{kind}/{id}")).unwrap_or(id)
    }

    /// The folders and the features in each, by folder name.
    pub fn folders(&self) -> Vec<(String, Vec<String>)> {
        self.meta_entries("folder/")
            .map(|(k, v)| {
                (
                    k["folder/".len()..].to_owned(),
                    serde_json::from_str(v).unwrap_or_default(),
                )
            })
            .collect()
    }

    /// The features switched off by hand.
    pub fn suppressed(&self) -> std::collections::BTreeSet<String> {
        self.meta_entries("suppress/")
            .map(|(k, _)| k["suppress/".len()..].to_owned())
            .collect()
    }

    /// The feature the rollback bar sits after: everything after it in the tree is switched off.
    pub fn rollback(&self) -> Option<&str> {
        self.meta("rollback")
    }

    /// The features in build order, as the design tree lists them: what a feature is built on
    /// comes first, then by name.
    pub fn ordered_features(&self) -> Vec<String> {
        let mut v: Vec<String> = self.features.keys().cloned().collect();
        v.sort_by_key(|f| (self.feature_depth(f), f.clone()));
        v
    }

    /// Is `id` after the rollback bar in build order?
    pub fn feature_is_after_rollback(&self, id: &str) -> bool {
        let Some(bar) = self.rollback() else {
            return false;
        };
        let order = self.ordered_features();
        match (
            order.iter().position(|f| f == bar),
            order.iter().position(|f| f == id),
        ) {
            (Some(b), Some(i)) => i > b,
            _ => false,
        }
    }

    /// The features that are not part of the model right now: the ones suppressed, the ones after
    /// the rollback bar, and everything built on any of those.
    pub fn hidden_features(&self) -> std::collections::BTreeSet<String> {
        let mut hidden = self.suppressed();
        if let Some(bar) = self.rollback() {
            let order = self.ordered_features();
            if let Some(at) = order.iter().position(|f| f == bar) {
                hidden.extend(order[at + 1..].iter().cloned());
            }
        }
        hidden.retain(|f| self.features.contains_key(f));
        loop {
            let more: Vec<String> = self
                .features
                .iter()
                .filter(|(id, f)| {
                    !hidden.contains(*id) && self.feature_deps(f).iter().any(|d| hidden.contains(d))
                })
                .map(|(id, _)| id.clone())
                .collect();
            if more.is_empty() {
                return hidden;
            }
            hidden.extend(more);
        }
    }

    pub(crate) fn feature_map(&self) -> &BTreeMap<String, FeatureSlot> {
        &self.features
    }

    pub fn sketch(&self, id: &str) -> Option<&SketchSlot> {
        self.sketches.get(id)
    }

    /// Sets a value, returning the previous one. `value` must already be normalized (finite,
    /// no negative zero); the authority guarantees this.
    pub fn set(&mut self, name: &str, value: f64) -> Option<f64> {
        let old = self.params.insert(name.to_owned(), value);
        if let Some(o) = old {
            self.acc = self.acc.wrapping_sub(param_leaf(name, o));
        }
        self.acc = self.acc.wrapping_add(param_leaf(name, value));
        old
    }

    pub fn remove(&mut self, name: &str) -> Option<f64> {
        let old = self.params.remove(name);
        if let Some(o) = old {
            self.acc = self.acc.wrapping_sub(param_leaf(name, o));
        }
        old
    }

    /// Inserts or replaces a sketch, returning the previous slot.
    pub fn put_sketch(&mut self, id: &str, slot: SketchSlot) -> Option<SketchSlot> {
        let leaf = sketch_leaf(id, &slot.sketch);
        let old = self.sketches.insert(id.to_owned(), slot);
        if let Some(o) = &old {
            self.acc = self.acc.wrapping_sub(sketch_leaf(id, &o.sketch));
        }
        self.acc = self.acc.wrapping_add(leaf);
        old
    }

    pub fn remove_sketch(&mut self, id: &str) -> Option<SketchSlot> {
        let old = self.sketches.remove(id);
        if let Some(o) = &old {
            self.acc = self.acc.wrapping_sub(sketch_leaf(id, &o.sketch));
        }
        old
    }

    fn finish(&self, acc: u128) -> Hash128 {
        let (p, s, f) = (
            (self.params.len() as u64).to_le_bytes(),
            (self.sketches.len() as u64).to_le_bytes(),
            (self.features.len() as u64).to_le_bytes(),
        );
        let acc = acc.to_le_bytes();
        if self.meta.is_empty() {
            // As before organisation existed, so older documents keep their hashes.
            Hash128::of(&[&acc, &p, &s, &f])
        } else {
            let m = (self.meta.len() as u64).to_le_bytes();
            Hash128::of(&[&acc, &p, &s, &f, &m])
        }
    }

    /// O(1) hash of the whole state.
    pub fn hash(&self) -> Hash128 {
        self.finish(self.acc)
    }

    /// O(n) from-scratch hash; used by tests to verify the incremental one.
    pub fn recompute_hash(&self) -> Hash128 {
        let acc = self
            .params
            .iter()
            .fold(0u128, |a, (k, v)| a.wrapping_add(param_leaf(k, *v)));
        let acc = self
            .sketches
            .iter()
            .fold(acc, |a, (k, v)| a.wrapping_add(sketch_leaf(k, &v.sketch)));
        let acc = self
            .features
            .iter()
            .fold(acc, |a, (k, v)| a.wrapping_add(feature_leaf(k, v)));
        let acc = self
            .meta
            .iter()
            .fold(acc, |a, (k, v)| a.wrapping_add(meta_leaf(k, v)));
        self.finish(acc)
    }

    pub fn param_page(
        &self,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> (Vec<(String, f64)>, Option<String>) {
        page_map(&self.params, prefix, after, limit, |k, v| {
            (k.to_owned(), *v)
        })
    }

    pub(crate) fn sketch_map(&self) -> &BTreeMap<String, SketchSlot> {
        &self.sketches
    }
}
