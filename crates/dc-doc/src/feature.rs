//! Parametric features: the recipe (`FeatureSpec`) is the intent and what gets stored and hashed;
//! the solid is a deterministic consequence, rebuilt whenever its inputs change.

use crate::diag::{DiagCode, Diagnostic};
use crate::Hash128;
use dc_features::solid_ops::{boss_with_tool, cut_with_tool, reach, tool_solid, ToolError};
use dc_features::{build_solid, RimSpec, Round, StepError, Stepped};
use dc_kernel::{RimError, RimRound, Solid, Surface};
use dc_math::{det, Plane, Tolerance, Vec2};
use dc_sketch::{Curve2, Loop, LoopEdge, Profile, ProfileProblem, Sketch};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// How a feature ends along the sketch plane's normal, beyond its plain depth.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EndMode {
    /// From the sketch plane out to `depth` (or through all for a cut with no depth).
    #[default]
    Blind,
    /// Symmetric about the sketch plane: `depth` is the total, half each way.
    MidPlane,
    /// A cut through the whole part in both directions.
    ThroughBoth,
}

/// Copies of a cut or boss, laid out in the plane of the sketch it comes from (millimetres and
/// degrees, in that sketch's own coordinates). One feature, one intent: "these holes", not one
/// feature per hole. The first copy is the profile itself.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FeaturePattern {
    /// `count` copies in a row, each `step` further along than the one before. With `step2` and
    /// `count2` it is a grid: `count2` rows, each `step2` from the last.
    Linear {
        step: [f64; 2],
        count: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        step2: Option<[f64; 2]>,
        #[serde(default = "one", skip_serializing_if = "is_one")]
        count2: u32,
    },
    /// `count` copies, each turned `step_degrees` further about `center` (counter-clockwise):
    /// 360 / count for an even ring.
    Circular {
        center: [f64; 2],
        count: u32,
        step_degrees: f64,
    },
    /// The profile and its mirror image across the line through `point` at `direction_degrees`.
    Mirror {
        point: [f64; 2],
        direction_degrees: f64,
    },
}

fn one() -> u32 {
    1
}

fn is_one(n: &u32) -> bool {
    *n == 1
}

/// Most copies one pattern may make.
const MAX_PATTERN: u64 = 1024;

impl FeaturePattern {
    pub(crate) fn canonical(self) -> FeaturePattern {
        let z = |v: f64| if v == 0.0 { 0.0 } else { v };
        let z2 = |a: [f64; 2]| [z(a[0]), z(a[1])];
        match self {
            FeaturePattern::Linear {
                step,
                count,
                step2,
                count2,
            } => FeaturePattern::Linear {
                step: z2(step),
                count,
                step2: step2.map(z2),
                count2,
            },
            FeaturePattern::Circular {
                center,
                count,
                step_degrees,
            } => FeaturePattern::Circular {
                center: z2(center),
                count,
                step_degrees: z(step_degrees),
            },
            FeaturePattern::Mirror {
                point,
                direction_degrees,
            } => FeaturePattern::Mirror {
                point: z2(point),
                direction_degrees: z(direction_degrees),
            },
        }
    }

    /// How many copies, the original included.
    pub fn copies(&self) -> u64 {
        match self {
            FeaturePattern::Linear { count, count2, .. } => u64::from(*count) * u64::from(*count2),
            FeaturePattern::Circular { count, .. } => u64::from(*count),
            FeaturePattern::Mirror { .. } => 2,
        }
    }

    /// Checks the numbers; the reason is for the person (or agent) who wrote them.
    fn check(&self) -> Result<(), Diagnostic> {
        let bad = |m: String| Diagnostic::new(DiagCode::InvalidValue, m);
        let finite = |a: &[f64]| a.iter().all(|v| v.is_finite());
        match self {
            FeaturePattern::Linear {
                step,
                count,
                step2,
                count2,
            } => {
                if !finite(step) || !step2.as_ref().is_none_or(|s| finite(s)) {
                    return Err(bad("pattern steps must be finite numbers".into()));
                }
                if *count == 0 || *count2 == 0 {
                    return Err(bad("a pattern needs at least one copy".into()));
                }
                if *count > 1 && step[0] == 0.0 && step[1] == 0.0 {
                    return Err(bad(
                        "the pattern's step is zero, so its copies would lie on top of each other"
                            .into(),
                    ));
                }
                if *count2 > 1 && step2.is_none_or(|s| s == [0.0, 0.0]) {
                    return Err(bad(
                        "a second direction with several rows needs a non-zero step2".into(),
                    ));
                }
            }
            FeaturePattern::Circular {
                center,
                count,
                step_degrees,
            } => {
                if !finite(center) || !step_degrees.is_finite() {
                    return Err(bad("pattern numbers must be finite".into()));
                }
                if *count == 0 {
                    return Err(bad("a pattern needs at least one copy".into()));
                }
                if *count > 1 && step_degrees.rem_euclid(360.0) == 0.0 {
                    return Err(bad(
                        "a step of a whole number of turns would put every copy on the first"
                            .into(),
                    ));
                }
            }
            FeaturePattern::Mirror {
                point,
                direction_degrees,
            } => {
                if !finite(point) || !direction_degrees.is_finite() {
                    return Err(bad("pattern numbers must be finite".into()));
                }
            }
        }
        if self.copies() > MAX_PATTERN {
            return Err(bad(format!(
                "a pattern may make at most {MAX_PATTERN} copies, this one asks for {}",
                self.copies()
            )));
        }
        Ok(())
    }

    /// The profile and its copies, in the sketch's frame, in a fixed order (the original first).
    fn instances(&self, p: &Profile) -> Vec<Profile> {
        let mut out = Vec::new();
        match self {
            FeaturePattern::Linear {
                step,
                count,
                step2,
                count2,
            } => {
                let s2 = step2.unwrap_or([0.0, 0.0]);
                for j in 0..*count2 {
                    for i in 0..*count {
                        let t = Vec2::new(
                            step[0] * f64::from(i) + s2[0] * f64::from(j),
                            step[1] * f64::from(i) + s2[1] * f64::from(j),
                        );
                        out.push(if i == 0 && j == 0 {
                            p.clone()
                        } else {
                            map_profile(p, &|q| q + t)
                        });
                    }
                }
            }
            FeaturePattern::Circular {
                center,
                count,
                step_degrees,
            } => {
                let c = Vec2::new(center[0], center[1]);
                for i in 0..*count {
                    if i == 0 {
                        out.push(p.clone());
                        continue;
                    }
                    let a = (step_degrees * f64::from(i)).to_radians();
                    let (sn, cs) = (det::sin(a), det::cos(a));
                    out.push(map_profile(p, &|q| {
                        let d = q - c;
                        c + Vec2::new(d.x * cs - d.y * sn, d.x * sn + d.y * cs)
                    }));
                }
            }
            FeaturePattern::Mirror {
                point,
                direction_degrees,
            } => {
                out.push(p.clone());
                let o = Vec2::new(point[0], point[1]);
                let a = (2.0 * direction_degrees).to_radians();
                let (sn, cs) = (det::sin(a), det::cos(a));
                out.push(map_profile(p, &|q| {
                    let d = q - o;
                    o + Vec2::new(d.x * cs + d.y * sn, d.x * sn - d.y * cs)
                }));
            }
        }
        out
    }
}

/// The options of a feature's extent that go beyond "this deep, this way".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Extent {
    #[serde(default, skip_serializing_if = "is_blind")]
    pub mode: EndMode,
    /// A second direction (against the first), this deep; for blind extents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth2: Option<f64>,
    /// A cut that removes everything outside the profile instead of inside it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub outside: bool,
}

fn is_blind(m: &EndMode) -> bool {
    *m == EndMode::Blind
}

impl Extent {
    pub fn is_default(&self) -> bool {
        *self == Extent::default()
    }

    fn canonical(mut self) -> Extent {
        self.depth2 = self.depth2.map(|d| if d == 0.0 { 0.0 } else { d });
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FeatureSpec {
    /// Extrudes one closed profile of a sketch along the sketch plane's normal.
    Extrude {
        sketch: String,
        /// Profile IDs from `sketch.profiles` (e.g. `profile.l1`). Empty means every closed
        /// region of the sketch.
        #[serde(default)]
        profiles: Vec<String>,
        depth: f64,
        /// Extrude against the plane normal instead of along it.
        #[serde(default)]
        reverse: bool,
        #[serde(default, skip_serializing_if = "Extent::is_default")]
        extent: Extent,
    },
    /// Removes the region of a sketch profile from another feature, from the sketch plane to a
    /// depth (or all the way through).
    Cut {
        /// The feature being cut (an extrude, a cut or a boss).
        target: String,
        sketch: String,
        #[serde(default)]
        profiles: Vec<String>,
        /// `None` means through all, as far as the part goes.
        #[serde(default)]
        depth: Option<f64>,
        /// Cut against the sketch plane's normal instead of along it.
        #[serde(default)]
        reverse: bool,
        #[serde(default, skip_serializing_if = "Extent::is_default")]
        extent: Extent,
        /// Copies of the cut (see [`FeaturePattern`]); omitted for a single cut.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pattern: Option<FeaturePattern>,
    },
    /// Adds the region of a sketch profile to another feature, from the sketch plane out to a
    /// depth. The boss must touch the part.
    Boss {
        target: String,
        sketch: String,
        #[serde(default)]
        profiles: Vec<String>,
        depth: f64,
        #[serde(default)]
        reverse: bool,
        #[serde(default, skip_serializing_if = "Extent::is_default")]
        extent: Extent,
        /// Copies of the boss (see [`FeaturePattern`]); omitted for a single boss.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pattern: Option<FeaturePattern>,
    },
    /// Revolves a closed profile once round an axis in its sketch: `"x"` or `"y"` (the sketch's
    /// own axes) or the ID of a line (from its first point to its second). The profile must lie
    /// on one side of the axis and be made of lines parallel or square to it (a shaft, a bushing,
    /// a washer): slanted lines and arcs would need cones, spheres and tori.
    Revolve {
        sketch: String,
        #[serde(default)]
        profiles: Vec<String>,
        axis: String,
        /// Degrees to turn (more than 0, less than 360); omitted for a full turn.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        angle: Option<f64>,
    },
    /// Rounds edges that run parallel to the part's axis with a circular arc of `radius`,
    /// tangent to the two faces. Each edge is named by the two faces that meet along it (from
    /// `feature.faces`), so the choice survives rebuilds.
    Fillet {
        target: String,
        edges: Vec<[String; 2]>,
        radius: f64,
    },
    /// Bevels edges that run parallel to the part's axis: a flat face `distance` from the
    /// edge along each of its two faces.
    Chamfer {
        target: String,
        edges: Vec<[String; 2]>,
        distance: f64,
    },
    /// Joins the outline of one sketch to the outline of another with flat faces (a frustum, a
    /// tapered block, a transition between two shapes). Both outlines are closed polygons of
    /// straight lines with the same number of corners; corresponding edges must be parallel or
    /// meet so every side is flat.
    Loft {
        sketch: String,
        #[serde(default)]
        profiles: Vec<String>,
        to_sketch: String,
        #[serde(default)]
        to_profiles: Vec<String>,
    },
    /// Pushes a profile along a straight line of another sketch (from its first point to its
    /// second): an oblique prism. A profile with arcs or circles can only be pushed straight
    /// along its own normal.
    Sweep {
        sketch: String,
        #[serde(default)]
        profiles: Vec<String>,
        path_sketch: String,
        path: String,
    },
    /// Hollows a prism-shaped part with walls `thickness` thick, leaving the named end faces open.
    Shell {
        target: String,
        #[serde(default)]
        faces: Vec<String>,
        thickness: f64,
    },
}

impl FeatureSpec {
    /// The sketch this feature reads, if it reads one (a fillet works on the part itself).
    pub fn sketch(&self) -> Option<&str> {
        match self {
            FeatureSpec::Extrude { sketch, .. }
            | FeatureSpec::Cut { sketch, .. }
            | FeatureSpec::Boss { sketch, .. }
            | FeatureSpec::Revolve { sketch, .. }
            | FeatureSpec::Loft { sketch, .. }
            | FeatureSpec::Sweep { sketch, .. } => Some(sketch),
            FeatureSpec::Fillet { .. }
            | FeatureSpec::Chamfer { .. }
            | FeatureSpec::Shell { .. } => None,
        }
    }

    /// Every sketch this feature reads (a loft reads two, a sweep a profile and a path).
    pub fn sketches(&self) -> Vec<&str> {
        match self {
            FeatureSpec::Loft {
                sketch, to_sketch, ..
            } => vec![sketch, to_sketch],
            FeatureSpec::Sweep {
                sketch,
                path_sketch,
                ..
            } => vec![sketch, path_sketch],
            other => other.sketch().into_iter().collect(),
        }
    }

    /// The feature this one is built on, if any.
    pub fn target(&self) -> Option<&str> {
        match self {
            FeatureSpec::Extrude { .. }
            | FeatureSpec::Revolve { .. }
            | FeatureSpec::Loft { .. }
            | FeatureSpec::Sweep { .. } => None,
            FeatureSpec::Cut { target, .. }
            | FeatureSpec::Boss { target, .. }
            | FeatureSpec::Fillet { target, .. }
            | FeatureSpec::Chamfer { target, .. }
            | FeatureSpec::Shell { target, .. } => Some(target),
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            FeatureSpec::Extrude { .. } => "extrude",
            FeatureSpec::Cut { .. } => "cut",
            FeatureSpec::Boss { .. } => "boss",
            FeatureSpec::Revolve { .. } => "revolve",
            FeatureSpec::Fillet { .. } => "fillet",
            FeatureSpec::Chamfer { .. } => "chamfer",
            FeatureSpec::Shell { .. } => "shell",
            FeatureSpec::Loft { .. } => "loft",
            FeatureSpec::Sweep { .. } => "sweep",
        }
    }

    pub fn canonical(self) -> FeatureSpec {
        let sorted = |mut v: Vec<String>| {
            v.sort();
            v.dedup();
            v
        };
        let z = |v: f64| if v == 0.0 { 0.0 } else { v };
        match self {
            FeatureSpec::Extrude {
                sketch,
                profiles,
                depth,
                reverse,
                extent,
            } => FeatureSpec::Extrude {
                sketch,
                profiles: sorted(profiles),
                depth: z(depth),
                reverse,
                extent: extent.canonical(),
            },
            FeatureSpec::Cut {
                target,
                sketch,
                profiles,
                depth,
                reverse,
                extent,
                pattern,
            } => FeatureSpec::Cut {
                target,
                sketch,
                profiles: sorted(profiles),
                depth: depth.map(z),
                reverse,
                extent: extent.canonical(),
                pattern: pattern.map(FeaturePattern::canonical),
            },
            FeatureSpec::Boss {
                target,
                sketch,
                profiles,
                depth,
                reverse,
                extent,
                pattern,
            } => FeatureSpec::Boss {
                target,
                sketch,
                profiles: sorted(profiles),
                depth: z(depth),
                reverse,
                extent: extent.canonical(),
                pattern: pattern.map(FeaturePattern::canonical),
            },
            FeatureSpec::Revolve {
                sketch,
                profiles,
                axis,
                angle,
            } => FeatureSpec::Revolve {
                sketch,
                profiles: sorted(profiles),
                axis,
                angle: angle.filter(|a| (*a - 360.0).abs() > 1e-12),
            },
            FeatureSpec::Fillet {
                target,
                edges,
                radius,
            } => FeatureSpec::Fillet {
                target,
                edges,
                radius: z(radius),
            },
            FeatureSpec::Chamfer {
                target,
                edges,
                distance,
            } => FeatureSpec::Chamfer {
                target,
                edges,
                distance: z(distance),
            },
            FeatureSpec::Shell {
                target,
                faces,
                thickness,
            } => FeatureSpec::Shell {
                target,
                faces: sorted(faces),
                thickness: z(thickness),
            },
            FeatureSpec::Loft {
                sketch,
                profiles,
                to_sketch,
                to_profiles,
            } => FeatureSpec::Loft {
                sketch,
                profiles: sorted(profiles),
                to_sketch,
                to_profiles: sorted(to_profiles),
            },
            FeatureSpec::Sweep {
                sketch,
                profiles,
                path_sketch,
                path,
            } => FeatureSpec::Sweep {
                sketch,
                profiles: sorted(profiles),
                path_sketch,
                path,
            },
        }
    }
}

/// Exact measurements of a built solid, as data (AGENTS.md §4.4). All values are computed from the
/// B-rep, not from a mesh.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SolidSummary {
    pub volume: f64,
    pub area: f64,
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
    pub vertices: usize,
    pub edges: usize,
    pub faces: usize,
    /// Hash of the solid's canonical bytes. Equal on every platform if the kernel is deterministic;
    /// it is part of the document hash, so a divergence is detected on replay.
    pub digest: Hash128,
}

/// A feature and what it built.
#[derive(Clone, Debug)]
pub struct FeatureSlot {
    pub spec: FeatureSpec,
    pub solid: Arc<Solid>,
    pub summary: SolidSummary,
    /// The layered model the solid was built from; later cuts and bosses work on this. `None`
    /// once an edge was rounded across the axis (a shaft's rim): the solid is then no longer a
    /// stack of prisms.
    pub stepped: Option<Arc<Stepped>>,
}

impl FeatureSlot {
    /// The layered model, or why this feature has none.
    pub fn model(&self, id: &str, what: &str) -> Result<&Stepped, Diagnostic> {
        self.stepped.as_deref().ok_or_else(|| {
            unsupported(format!(
                "'{id}' has no layered model (an edge was rounded across its axis, or a cut or boss came from another direction), so {what} cannot work on it yet"
            ))
            .suggest("round edges and rims last, and keep cuts and bosses along the base sketch's axis if they must be rounded")
            .entity(feature_path(id))
        })
    }
}

/// Stored/serialized view of a feature, used in change records and queries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FeatureView {
    pub spec: FeatureSpec,
    pub summary: SolidSummary,
}

impl FeatureSlot {
    pub fn view(&self) -> FeatureView {
        FeatureView {
            spec: self.spec.clone(),
            summary: self.summary.clone(),
        }
    }
}

/// One face of a solid, as a query result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FaceInfo {
    /// Deterministic name (`cap_start`, `cap_end`, `side:<sketch entity>`, `cap_end:<feature>`):
    /// stable across rebuilds, and what a sketch placed on this face refers to.
    pub name: String,
    /// `plane` or `cylinder`.
    pub surface: String,
    pub area: f64,
    /// Outward unit normal, for planar faces.
    pub normal: Option<[f64; 3]>,
    pub radius: Option<f64>,
    /// For cylinders: true if material is inside (a boss), false if outside (a hole or notch).
    pub convex: Option<bool>,
}

pub fn face_infos(solid: &Solid) -> Vec<FaceInfo> {
    let mut v: Vec<FaceInfo> = solid
        .faces
        .iter()
        .map(|f| {
            let (surface, normal, radius, convex) = match &f.surface {
                Surface::Plane { normal, .. } => {
                    let n = if f.same_sense { *normal } else { -*normal };
                    ("plane", Some([n.x, n.y, n.z]), None, None)
                }
                Surface::Cylinder { radius, .. } => {
                    ("cylinder", None, Some(*radius), Some(f.same_sense))
                }
                Surface::Cone { .. } => ("cone", None, None, Some(f.same_sense)),
                Surface::Torus { minor, .. } => ("torus", None, Some(*minor), Some(f.same_sense)),
            };
            FaceInfo {
                name: f.name.clone(),
                surface: surface.into(),
                area: solid.face_area(f),
                normal,
                radius,
                convex,
            }
        })
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

pub fn feature_path(id: &str) -> String {
    format!("feature/{id}")
}

fn summarize(solid: &Solid) -> SolidSummary {
    let b = solid.bounds().expect("a valid solid has vertices");
    SolidSummary {
        volume: solid.volume(),
        area: solid.area(),
        bounds_min: [b.min.x, b.min.y, b.min.z],
        bounds_max: [b.max.x, b.max.y, b.max.z],
        vertices: solid.vertices.len(),
        edges: solid.edges.len(),
        faces: solid.faces.len(),
        digest: Hash128::of(&[&solid.canonical_bytes()]),
    }
}

fn problem_text(problems: &[ProfileProblem]) -> String {
    problems
        .iter()
        .take(3)
        .map(|p| p.message.clone())
        .collect::<Vec<_>>()
        .join("; ")
}

/// What a successful build produces.
pub struct Built {
    pub solid: Arc<Solid>,
    pub summary: SolidSummary,
    pub stepped: Option<Arc<Stepped>>,
}

fn unsupported(m: String) -> Diagnostic {
    Diagnostic::new(DiagCode::Unsupported, m)
}

fn bad_depth(d: f64) -> Diagnostic {
    Diagnostic::new(
        DiagCode::InvalidValue,
        format!("depth must be finite and greater than zero, got {d}"),
    )
}

/// Finds a closed profile by ID, or explains what is available and what blocks the rest.
/// The profiles a feature uses: the ones named, or every closed region of the sketch when none
/// is named. Each is returned with its ID; an unknown name is refused with what is available.
fn find_profiles(
    sketch: &Sketch,
    sketch_id: &str,
    wanted: &[String],
) -> Result<Vec<(String, Profile)>, Diagnostic> {
    let report = sketch.profiles();
    if wanted.is_empty() {
        if report.profiles.is_empty() {
            return Err(no_profile(sketch_id, "", &report));
        }
        return Ok(report
            .profiles
            .iter()
            .map(|p| (p.id.clone(), p.clone()))
            .collect());
    }
    wanted
        .iter()
        .map(|w| {
            report
                .profiles
                .iter()
                .find(|p| &p.id == w)
                .map(|p| (p.id.clone(), p.clone()))
                .ok_or_else(|| no_profile(sketch_id, w, &report))
        })
        .collect()
}

fn no_profile(sketch_id: &str, profile: &str, report: &dc_sketch::ProfileReport) -> Diagnostic {
    let available: Vec<&str> = report
        .profiles
        .iter()
        .take(20)
        .map(|p| p.id.as_str())
        .collect();
    let detail = if available.is_empty() {
        if report.problems.is_empty() {
            " (it has no closed profiles at all)".to_owned()
        } else {
            format!(
                " (it has no closed profiles at all: {})",
                problem_text(&report.problems)
            )
        }
    } else {
        format!(" (available: {})", available.join(", "))
    };
    let mut d = Diagnostic::new(
        DiagCode::NotFound,
        format!(
            "sketch '{sketch_id}' has no closed profile{}{detail}",
            if profile.is_empty() {
                String::new()
            } else {
                format!(" '{profile}'")
            }
        ),
    )
    .entity(format!("sketch/{sketch_id}"));
    for pr in report.problems.iter().take(5) {
        for e in &pr.entities {
            d = d.entity(format!("sketch/{sketch_id}/{e}"));
        }
    }
    d.suggest("query sketch.profiles for the available profiles and the problems blocking others")
}

fn finish(stepped: Stepped) -> Result<Built, Diagnostic> {
    let solid = build_solid(&stepped).map_err(|e| step_diag(e, "this feature"))?;
    let summary = summarize(&solid);
    Ok(Built {
        solid: Arc::new(solid),
        summary,
        stepped: Some(Arc::new(stepped)),
    })
}

fn step_diag(e: StepError, what: &str) -> Diagnostic {
    match e {
        StepError::Region(r) => Diagnostic::new(DiagCode::Unsupported, format!("{what}: {r}"))
            .suggest("move or resize the sketch so its outline clearly overlaps or clearly misses the part's edges"),
        StepError::Nothing(m) => Diagnostic::new(DiagCode::FeatureFailed, format!("{what}: {m}")),
        StepError::Unsupported(m) => Diagnostic::new(DiagCode::Unsupported, format!("{what}: {m}")),
        StepError::NoContact(m) => Diagnostic::new(DiagCode::FeatureFailed, format!("{what}: {m}"))
            .suggest("place the boss sketch on a face of the part (or overlapping it) so it is attached"),
        StepError::Invalid(v) => Diagnostic::new(
            DiagCode::FeatureFailed,
            format!("{what} produced an invalid solid: {}", v.join("; ")),
        )
        .suggest("this is a kernel bug; please report the sketch and feature that triggered it"),
    }
}

/// Where the tool starts and ends along the model's axis, and its profile in the model's frame.
struct Tool {
    profile: Profile,
    start: f64,
    end: f64,
}

/// Checks the options that depend on what kind of feature this is.
fn check_extent(
    what: &str,
    extent: &Extent,
    depth: Option<f64>,
    cut: bool,
) -> Result<(), Diagnostic> {
    let unsupported = |m: String| Diagnostic::new(DiagCode::Unsupported, format!("{what}: {m}"));
    if let Some(d2) = extent.depth2 {
        if !d2.is_finite() || d2 <= 0.0 {
            return Err(bad_depth(d2));
        }
        if extent.mode != EndMode::Blind {
            return Err(unsupported(
                "a second direction only goes with a blind extent (mid plane and through-all-both already go both ways)".into(),
            ));
        }
    }
    if extent.mode == EndMode::ThroughBoth && !cut {
        return Err(unsupported(
            "through all in both directions is for cuts".into(),
        ));
    }
    if extent.mode == EndMode::MidPlane && depth.is_none() {
        return Err(unsupported("a mid plane extent needs a depth".into()));
    }
    if extent.outside && !cut {
        return Err(unsupported(
            "cutting outside the profile is for cuts".into(),
        ));
    }
    Ok(())
}

/// The span of an extent as offsets from the sketch plane: `(from, to)` in the order the feature
/// runs. Blind keeps its direction; the other modes are `lo..hi`.
fn extent_span(dir: f64, depth: Option<f64>, extent: &Extent) -> (f64, f64) {
    match extent.mode {
        EndMode::Blind => {
            let far = match depth {
                Some(d) => dir * d,
                None => dir * f64::INFINITY,
            };
            match extent.depth2 {
                Some(d2) => {
                    // The second direction starts where the plane is and goes the other way.
                    let back = -dir * if depth.is_none() { f64::INFINITY } else { d2 };
                    (back, far)
                }
                None => (0.0, far),
            }
        }
        EndMode::MidPlane => {
            let half = depth.unwrap_or(0.0) / 2.0;
            (-half, half)
        }
        EndMode::ThroughBoth => (f64::NEG_INFINITY, f64::INFINITY),
    }
}

fn place_tool(
    model: &Stepped,
    tool_plane: &Plane,
    profile: Profile,
    reverse: bool,
    depth: Option<f64>,
    extent: &Extent,
    what: &str,
) -> Result<Tool, Diagnostic> {
    let mp = &model.plane;
    if !Tolerance::DEFAULT.parallel(tool_plane.normal(), mp.normal()) {
        return Err(Diagnostic::new(
            DiagCode::Unsupported,
            format!("{what}: the sketch is not parallel to the part's sketch plane, so this cut/boss would not run along the part's axis; other directions are not supported yet"),
        )
        .suggest("sketch on a plane parallel to the part's base sketch (or on one of its flat end faces)"));
    }
    let n = mp.normal();
    let start = (tool_plane.origin - mp.origin).dot(n);
    // +1 if the tool sketch's normal points along the model axis, -1 if against it.
    let along = tool_plane.normal().dot(n).signum();
    let dir = if reverse { -along } else { along };
    let (from, to) = extent_span(dir, depth, extent);
    let plain = extent.mode == EndMode::Blind && extent.depth2.is_none();
    // The plain case keeps its old shape (start at the plane, end at the far end) so existing
    // parts keep their face names; the others are a span low to high.
    let (start, end) = if plain {
        (start, if to.is_infinite() { to } else { start + to })
    } else {
        let (a, b) = (start + from, start + to);
        (a.min(b), a.max(b))
    };
    Ok(Tool {
        profile: transform_profile(&profile, tool_plane, mp),
        start,
        end,
    })
}

/// Every copy of every profile a cut or boss uses, in order: each profile, then its pattern
/// copies. Never empty.
fn tool_copies(
    sketch: &Sketch,
    sketch_id: &str,
    wanted: &[String],
    pattern: Option<&FeaturePattern>,
) -> Result<Vec<Profile>, Diagnostic> {
    let mut out = Vec::new();
    for (_, p) in find_profiles(sketch, sketch_id, wanted)? {
        match pattern {
            Some(pt) => out.extend(pt.instances(&p)),
            None => out.push(p),
        }
    }
    Ok(out)
}

/// Builds `spec`. `plane` is the sketch's resolved 3D plane, `sketch` the sketch the spec reads
/// and `target` the feature it is built on (for cuts and bosses). `feature` is this feature's own
/// ID, used to name the edges and faces a cut or boss introduces.
pub fn build(
    feature: &str,
    plane: Option<&Plane>,
    sketch: Option<&Sketch>,
    second: Option<(&Plane, &Sketch)>,
    target: Option<&FeatureSlot>,
    spec: &FeatureSpec,
) -> Result<Built, Diagnostic> {
    let sketch_inputs = || -> Result<(&Plane, &Sketch), Diagnostic> {
        plane.zip(sketch).ok_or_else(|| {
            Diagnostic::new(
                DiagCode::NotFound,
                format!("feature '{feature}' reads a sketch, but none was given"),
            )
        })
    };
    let missing_target = |t: &str| {
        Diagnostic::new(
            DiagCode::NotFound,
            format!("target feature '{t}' does not exist"),
        )
        .entity(feature_path(t))
    };
    match spec {
        FeatureSpec::Extrude {
            sketch: sk_id,
            profiles,
            depth,
            reverse,
            extent,
        } => {
            let (plane, sketch) = sketch_inputs()?;
            if !depth.is_finite() || *depth <= 0.0 {
                return Err(bad_depth(*depth));
            }
            check_extent("extrude", extent, Some(*depth), false)?;
            let ps: Vec<Profile> = find_profiles(sketch, sk_id, profiles)?
                .into_iter()
                .map(|(_, p)| p)
                .collect();
            if extent.mode == EndMode::Blind && extent.depth2.is_none() {
                return finish(Stepped::extrusion_all(*plane, ps, *depth, *reverse));
            }
            let dir = if *reverse { -1.0 } else { 1.0 };
            let (a, b) = extent_span(dir, Some(*depth), extent);
            finish(Stepped::extrusion_span_all(*plane, ps, a.min(b), a.max(b)))
        }
        FeatureSpec::Cut {
            target: tgt,
            sketch: sk_id,
            profiles,
            depth,
            reverse,
            extent,
            pattern,
        } => {
            let (plane, sketch) = sketch_inputs()?;
            if let Some(d) = depth {
                if !d.is_finite() || *d <= 0.0 {
                    return Err(bad_depth(*d));
                }
            }
            check_extent("cut", extent, *depth, true)?;
            if let Some(pt) = pattern {
                pt.check()?;
                if extent.outside {
                    return Err(unsupported(
                        "a pattern of cuts that keep only what is inside the profile makes no sense".into(),
                    ));
                }
            }
            let target = target.ok_or_else(|| missing_target(tgt))?;
            // Along the part's own axis the layered model builds it (and keeps face names stable);
            // any other direction, or a part without a layered model, goes through the boolean.
            let Some(model) = along_axis(target, plane) else {
                return boolean_tool(
                    feature,
                    target,
                    tgt,
                    plane,
                    sketch,
                    sk_id,
                    profiles,
                    pattern.as_ref(),
                    BoolTool::Cut {
                        depth: *depth,
                        reverse: *reverse,
                        extent,
                    },
                );
            };
            let copies = tool_copies(sketch, sk_id, profiles, pattern.as_ref())?;
            let tool = place_tool(
                model,
                plane,
                copies[0].clone(),
                *reverse,
                *depth,
                extent,
                "cut",
            )?;
            let plain = extent.mode == EndMode::Blind && extent.depth2.is_none();
            let (lo, hi) = (tool.start.min(tool.end), tool.start.max(tool.end));
            let lin = Tolerance::DEFAULT.linear;
            let reaches = |lo: f64, hi: f64| hi.min(model.max_w()) - lo.max(model.min_w()) > lin;
            if !reaches(lo, hi) {
                // The same cut the other way round ends at the mirror image of its end.
                let flipped_end = if tool.end.is_infinite() {
                    -tool.end
                } else {
                    2.0 * tool.start - tool.end
                };
                let flipped_reaches =
                    reaches(tool.start.min(flipped_end), tool.start.max(flipped_end));
                let mut d = Diagnostic::new(
                    DiagCode::FeatureFailed,
                    format!(
                        "the cut starts at {:.4} and goes {} but the part spans {:.4} to {:.4} along that axis, so it never reaches the part",
                        tool.start,
                        if tool.end > tool.start { "up" } else { "down" },
                        model.min_w(),
                        model.max_w()
                    ),
                );
                if flipped_reaches && plain {
                    d = d.suggest(format!(
                        "the other direction would reach it: set reverse to {}",
                        !reverse
                    ));
                }
                return Err(d);
            }
            let mut stepped = model.clone();
            for (i, copy) in copies.into_iter().enumerate() {
                // The first copy keeps the feature's own name (so a plain cut is unchanged);
                // the others get a suffix, so the faces they make have distinct, stable names.
                let (owner, what) = if i == 0 {
                    (feature.to_owned(), format!("cut of '{tgt}' by '{sk_id}'"))
                } else {
                    (
                        format!("{feature}~{i}"),
                        format!("copy {} of the cut pattern of '{tgt}' by '{sk_id}'", i + 1),
                    )
                };
                let t = if i == 0 {
                    Tool {
                        profile: tool.profile.clone(),
                        start: tool.start,
                        end: tool.end,
                    }
                } else {
                    place_tool(model, plane, copy, *reverse, *depth, extent, "cut")?
                };
                stepped = stepped
                    .cut_with(&t.profile, t.start, t.end, &owner, extent.outside)
                    .map_err(|e| step_diag(e, &what))?;
            }
            finish(stepped)
        }
        FeatureSpec::Boss {
            target: tgt,
            sketch: sk_id,
            profiles,
            depth,
            reverse,
            extent,
            pattern,
        } => {
            let (plane, sketch) = sketch_inputs()?;
            if !depth.is_finite() || *depth <= 0.0 {
                return Err(bad_depth(*depth));
            }
            check_extent("boss", extent, Some(*depth), false)?;
            if let Some(pt) = pattern {
                pt.check()?;
            }
            let target = target.ok_or_else(|| missing_target(tgt))?;
            let Some(model) = along_axis(target, plane) else {
                return boolean_tool(
                    feature,
                    target,
                    tgt,
                    plane,
                    sketch,
                    sk_id,
                    profiles,
                    pattern.as_ref(),
                    BoolTool::Boss {
                        depth: *depth,
                        reverse: *reverse,
                        extent,
                    },
                );
            };
            let copies = tool_copies(sketch, sk_id, profiles, pattern.as_ref())?;
            let mut stepped = model.clone();
            for (i, copy) in copies.into_iter().enumerate() {
                let (owner, what) = if i == 0 {
                    (
                        feature.to_owned(),
                        format!("boss on '{tgt}' from '{sk_id}'"),
                    )
                } else {
                    (
                        format!("{feature}~{i}"),
                        format!(
                            "copy {} of the boss pattern on '{tgt}' from '{sk_id}'",
                            i + 1
                        ),
                    )
                };
                let t = place_tool(model, plane, copy, *reverse, Some(*depth), extent, "boss")?;
                stepped = stepped
                    .boss(&t.profile, t.start, t.end, &owner)
                    .map_err(|e| step_diag(e, &what))?;
            }
            finish(stepped)
        }
        FeatureSpec::Revolve {
            sketch: sk_id,
            profiles,
            axis,
            angle,
        } => {
            let (plane, sketch) = sketch_inputs()?;
            let mut found = find_profiles(sketch, sk_id, profiles)?;
            if found.len() != 1 {
                return Err(unsupported(format!(
                    "revolve takes one profile, but {} were given ({}); name one in `profiles`",
                    found.len(),
                    found
                        .iter()
                        .map(|(id, _)| id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
            let (_, p) = found.remove(0);
            let (origin, dir) = resolve_axis(sketch, sk_id, axis)?;
            if let Some(deg) = angle {
                // A partial turn is built directly as a B-rep (no layered model).
                let solid = dc_features::revolve_partial(plane, &p, origin, dir, *deg)
                    .map_err(|e| step_diag(e, &format!("revolve of '{sk_id}' about '{axis}'")))?;
                let summary = summarize(&solid);
                return Ok(Built {
                    solid: Arc::new(solid),
                    summary,
                    stepped: None,
                });
            }
            let stepped = dc_features::revolve(plane, &p, origin, dir, feature)
                .map_err(|e| step_diag(e, &format!("revolve of '{sk_id}' about '{axis}'")))?;
            finish(stepped)
        }
        FeatureSpec::Fillet {
            target: tgt,
            edges,
            radius,
        } => round_edges(
            feature,
            target.ok_or_else(|| missing_target(tgt))?,
            tgt,
            edges,
            *radius,
            Round::Fillet,
        ),
        FeatureSpec::Chamfer {
            target: tgt,
            edges,
            distance,
        } => round_edges(
            feature,
            target.ok_or_else(|| missing_target(tgt))?,
            tgt,
            edges,
            *distance,
            Round::Chamfer,
        ),
        FeatureSpec::Loft {
            sketch: sk_id,
            profiles,
            to_sketch,
            to_profiles,
        } => {
            let (plane, sketch) = sketch_inputs()?;
            let (plane2, sketch2) = second.ok_or_else(|| {
                Diagnostic::new(
                    DiagCode::NotFound,
                    format!("loft '{feature}' needs the sketch '{to_sketch}'"),
                )
            })?;
            let one = |sk: &Sketch, id: &str, wanted: &[String]| -> Result<Profile, Diagnostic> {
                let mut found = find_profiles(sk, id, wanted)?;
                if found.len() != 1 {
                    return Err(unsupported(format!(
                        "a loft takes one outline from each sketch, but '{id}' gave {} ({}); name one in the profiles",
                        found.len(),
                        found.iter().map(|(i, _)| i.as_str()).collect::<Vec<_>>().join(", ")
                    )));
                }
                Ok(found.remove(0).1)
            };
            let (p1, p2) = (
                one(sketch, sk_id, profiles)?,
                one(sketch2, to_sketch, to_profiles)?,
            );
            let corners = |p: &Profile,
                           pl: &Plane,
                           sk_name: &str|
             -> Result<Vec<dc_math::Vec3>, Diagnostic> {
                if !p.holes.is_empty() {
                    return Err(unsupported(format!(
                        "the outline in '{sk_name}' has holes: a loft joins plain outlines so far"
                    )));
                }
                p.outer
                    .edges
                    .iter()
                    .map(|e| {
                        if matches!(e.curve, Curve2::Line) {
                            Ok(pl.to_3d(Vec2::new(e.from[0], e.from[1])))
                        } else {
                            Err(unsupported(format!(
                                "edge '{}' of '{sk_name}' is a curve: a loft joins outlines of straight lines so far (curved sides would need ruled or free-form surfaces)",
                                e.id
                            )))
                        }
                    })
                    .collect()
            };
            let (bottom, top) = (
                corners(&p1, plane, sk_id)?,
                corners(&p2, plane2, to_sketch)?,
            );
            let names: Vec<String> = p1.outer.edges.iter().map(|e| e.id.clone()).collect();
            let solid = Solid::loft(&bottom, &top, &names).map_err(|m| {
                Diagnostic::new(
                    DiagCode::FeatureFailed,
                    format!("loft of '{sk_id}' to '{to_sketch}': {m}"),
                )
            })?;
            let summary = summarize(&solid);
            Ok(Built {
                solid: Arc::new(solid),
                summary,
                stepped: None,
            })
        }
        FeatureSpec::Sweep {
            sketch: sk_id,
            profiles,
            path_sketch,
            path,
        } => {
            let (plane, sketch) = sketch_inputs()?;
            let (pplane, psketch) = second.ok_or_else(|| {
                Diagnostic::new(
                    DiagCode::NotFound,
                    format!("sweep '{feature}' needs the path sketch '{path_sketch}'"),
                )
            })?;
            let (a, b) = match psketch.entity(path) {
                Some(dc_sketch::Entity::Line { a, b }) => (a.clone(), b.clone()),
                Some(_) => {
                    return Err(unsupported(format!(
                        "'{path}' is not a straight line: a sweep follows a straight line so far (a curved path would need tori and other surfaces)"
                    )))
                }
                None => {
                    return Err(Diagnostic::new(
                        DiagCode::NotFound,
                        format!("sketch '{path_sketch}' has no line '{path}'"),
                    ))
                }
            };
            let (pa, pb) = (
                psketch
                    .point(&a)
                    .ok_or_else(|| unsupported("the path has no start".into()))?,
                psketch
                    .point(&b)
                    .ok_or_else(|| unsupported("the path has no end".into()))?,
            );
            let dir = pplane.to_3d(Vec2::new(pb.0, pb.1)) - pplane.to_3d(Vec2::new(pa.0, pa.1));
            if dir.length() <= Tolerance::DEFAULT.linear {
                return Err(unsupported("the path has no length".into()));
            }
            let mut found = find_profiles(sketch, sk_id, profiles)?;
            if found.len() != 1 {
                return Err(unsupported(format!(
                    "a sweep takes one profile, but {} were given ({}); name one in `profiles`",
                    found.len(),
                    found
                        .iter()
                        .map(|(i, _)| i.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
            let prof = found.remove(0).1;
            let n = plane.normal();
            let solid = if Tolerance::DEFAULT.parallel(dir, n) {
                // Straight along the normal: any profile, as an extrusion.
                dc_features::extrude(plane, &prof, dir.length(), dir.dot(n) < 0.0).map_err(|e| {
                    Diagnostic::new(DiagCode::FeatureFailed, format!("sweep of '{sk_id}': {e}"))
                })?
            } else {
                if !prof.holes.is_empty() {
                    return Err(unsupported(
                        "an oblique sweep of a profile with holes is not supported yet".into(),
                    ));
                }
                if let Some(e) = prof
                    .outer
                    .edges
                    .iter()
                    .find(|e| !matches!(e.curve, Curve2::Line))
                {
                    return Err(unsupported(format!(
                        "edge '{}' is a curve: sweeping a curve along a slanted line makes an oblique cylinder, which the kernel does not have; only straight edges can be swept at an angle",
                        e.id
                    )));
                }
                let mut pts: Vec<dc_math::Vec3> = prof
                    .outer
                    .edges
                    .iter()
                    .map(|e| plane.to_3d(Vec2::new(e.from[0], e.from[1])))
                    .collect();
                let mut ids: Vec<String> = prof.outer.edges.iter().map(|e| e.id.clone()).collect();
                // The prism wants the outline counter-clockwise about the sweep direction.
                if dc_kernel::builder::newell(&pts).dot(dir) < 0.0 {
                    // Corner i starts edge i; reversed, corner i ends the edge that was i - 1.
                    pts.reverse();
                    ids.reverse();
                    ids.rotate_left(1);
                }
                let mut s = Solid::prism(&pts, &[], dir).ok_or_else(|| {
                    unsupported(
                        "the profile could not be swept (is the path in the profile's plane?)"
                            .into(),
                    )
                })?;
                for f in &mut s.faces {
                    f.name = match f.name.as_str() {
                        "bottom" => "cap_start".to_owned(),
                        "top" => "cap_end".to_owned(),
                        other => match other
                            .strip_prefix("side:")
                            .and_then(|i| i.parse::<usize>().ok())
                        {
                            Some(i) => format!("side:{}", ids[i]),
                            None => other.to_owned(),
                        },
                    };
                }
                let bases: Vec<String> = s.faces.iter().map(|f| f.name.clone()).collect();
                let (_, enames) = dc_kernel::naming::structural_names(&s, &bases);
                for (e, nm) in s.edges.iter_mut().zip(enames) {
                    e.name = nm;
                }
                s
            };
            let summary = summarize(&solid);
            Ok(Built {
                solid: Arc::new(solid),
                summary,
                stepped: None,
            })
        }
        FeatureSpec::Shell {
            target: tgt,
            faces,
            thickness,
        } => {
            let target = target.ok_or_else(|| missing_target(tgt))?;
            let solid = dc_features::shell_prism(&target.solid, faces, *thickness, feature)
                .map_err(|e| shell_diag(e, tgt))?;
            let summary = summarize(&solid);
            Ok(Built {
                solid: Arc::new(solid),
                summary,
                stepped: None,
            })
        }
    }
}

/// The two end faces of a prism-shaped solid (what a shell can leave open), if it is one.
pub fn shell_end_faces(solid: &Solid) -> Option<(String, String)> {
    dc_features::shell_ends(solid)
}

fn shell_diag(e: dc_features::ShellError, tgt: &str) -> Diagnostic {
    use dc_features::ShellError as S;
    let what = format!("shell of '{tgt}'");
    match e {
        S::NotFound(m) => Diagnostic::new(DiagCode::NotFound, format!("{what}: {m}")),
        S::Unsupported(m) => Diagnostic::new(DiagCode::Unsupported, format!("{what}: {m}")),
        S::TooThick(m) => Diagnostic::new(DiagCode::FeatureFailed, format!("{what}: {m}"))
            .suggest("use a thinner wall"),
        S::Bool(b) => Diagnostic::new(DiagCode::FeatureFailed, format!("{what}: {b}")),
    }
    .entity(feature_path(tgt))
}

/// The target's layered model if the sketch plane is parallel to its base sketch plane (so the
/// cut or boss runs along the part's axis); `None` sends the feature to the general boolean.
fn along_axis<'a>(target: &'a FeatureSlot, plane: &Plane) -> Option<&'a Stepped> {
    target
        .stepped
        .as_deref()
        .filter(|m| Tolerance::DEFAULT.parallel(plane.normal(), m.plane.normal()))
}

enum BoolTool<'a> {
    Cut {
        depth: Option<f64>,
        reverse: bool,
        extent: &'a Extent,
    },
    Boss {
        depth: f64,
        reverse: bool,
        extent: &'a Extent,
    },
}

fn tool_diag(e: ToolError, what: &str) -> Diagnostic {
    use dc_kernel::boolean::BoolError;
    match e {
        ToolError::Tool(m) => Diagnostic::new(DiagCode::FeatureFailed, format!("{what}: {m}")),
        ToolError::NoEffect(m) => Diagnostic::new(DiagCode::FeatureFailed, format!("{what}: {m}"))
            .suggest("if the sketch plane faces away from the part, set reverse to true; otherwise make the tool longer or move it onto the part"),
        ToolError::Floating(m) => Diagnostic::new(DiagCode::FeatureFailed, format!("{what}: {m}"))
            .suggest("place the boss sketch on a face of the part (or overlapping it) so it is attached"),
        ToolError::Bool(BoolError::Unsupported(m)) => {
            Diagnostic::new(DiagCode::Unsupported, format!("{what}: {m}"))
        }
        ToolError::Bool(BoolError::Empty) => Diagnostic::new(
            DiagCode::FeatureFailed,
            format!("{what}: nothing would be left of the part"),
        ),
        ToolError::Bool(BoolError::Degenerate(m)) => Diagnostic::new(
            DiagCode::FeatureFailed,
            format!("{what}: {m}"),
        )
        .suggest("move or resize the sketch so its outline clearly overlaps or clearly misses the part's faces and edges"),
        ToolError::Bool(BoolError::Invalid(v)) => Diagnostic::new(
            DiagCode::FeatureFailed,
            format!("{what} produced an invalid solid: {v:?}"),
        )
        .suggest("move or resize the sketch so its outline does not just touch an edge or a corner of the part"),
    }
}

/// A cut or boss through the general boolean: the tool is the extruded profile (every copy of
/// every profile), taken from or added to the target's solid.
#[allow(clippy::too_many_arguments)]
fn boolean_tool(
    feature: &str,
    target: &FeatureSlot,
    tgt: &str,
    plane: &Plane,
    sketch: &Sketch,
    sk_id: &str,
    profiles: &[String],
    pattern: Option<&FeaturePattern>,
    tool: BoolTool<'_>,
) -> Result<Built, Diagnostic> {
    let (depth, reverse, extent, is_cut) = match &tool {
        BoolTool::Cut {
            depth,
            reverse,
            extent,
        } => (*depth, *reverse, *extent, true),
        BoolTool::Boss {
            depth,
            reverse,
            extent,
        } => (Some(*depth), *reverse, *extent, false),
    };
    if extent.outside {
        return Err(unsupported(
            "keeping only what is inside the profile is not supported in this direction yet".into(),
        ));
    }
    let copies = tool_copies(sketch, sk_id, profiles, pattern)?;
    let bound = reach(&target.solid, plane);
    let dir = if reverse { -1.0 } else { 1.0 };
    let (from, to) = extent_span(dir, depth, extent);
    let clamp = |x: f64| {
        if x.is_infinite() {
            x.signum() * bound
        } else {
            x
        }
    };
    let (lo, hi) = (clamp(from).min(clamp(to)), clamp(from).max(clamp(to)));
    let mut solid: Solid = (*target.solid).clone();
    for (i, copy) in copies.into_iter().enumerate() {
        let (owner, what) = match (i, is_cut) {
            (0, true) => (feature.to_owned(), format!("cut of '{tgt}' by '{sk_id}'")),
            (0, false) => (
                feature.to_owned(),
                format!("boss on '{tgt}' from '{sk_id}'"),
            ),
            (_, true) => (
                format!("{feature}~{i}"),
                format!("copy {} of the cut pattern of '{tgt}' by '{sk_id}'", i + 1),
            ),
            (_, false) => (
                format!("{feature}~{i}"),
                format!(
                    "copy {} of the boss pattern on '{tgt}' from '{sk_id}'",
                    i + 1
                ),
            ),
        };
        let body = tool_solid(plane, &copy, lo, hi).map_err(|e| tool_diag(e, &what))?;
        solid = if is_cut {
            cut_with_tool(&solid, &body, &owner)
        } else {
            boss_with_tool(&solid, &body, &owner)
        }
        .map_err(|e| tool_diag(e, &what))?;
    }
    let summary = summarize(&solid);
    Ok(Built {
        solid: Arc::new(solid),
        summary,
        stepped: None,
    })
}

/// An axis of revolution in a sketch's 2D frame: a point on it and its direction.
fn resolve_axis(sketch: &Sketch, sketch_id: &str, axis: &str) -> Result<(Vec2, Vec2), Diagnostic> {
    match axis {
        "x" => Ok((Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0))),
        "y" => Ok((Vec2::new(0.0, 0.0), Vec2::new(0.0, 1.0))),
        id => {
            let bad = |m: String| Diagnostic::new(DiagCode::InvalidValue, m);
            let Some(dc_sketch::Entity::Line { a, b }) = sketch.entity(id) else {
                return Err(Diagnostic::new(
                    DiagCode::NotFound,
                    format!(
                        "the axis '{id}' is not \"x\", \"y\" or a line of sketch '{sketch_id}'"
                    ),
                )
                .suggest("draw a line (a centre line) in the sketch, or use \"x\" or \"y\""));
            };
            let (pa, pb) = (
                sketch
                    .point(a)
                    .ok_or_else(|| bad(format!("'{a}' has no position")))?,
                sketch
                    .point(b)
                    .ok_or_else(|| bad(format!("'{b}' has no position")))?,
            );
            let (o, d) = (Vec2::new(pa.0, pa.1), Vec2::new(pb.0 - pa.0, pb.1 - pa.1));
            if d.length() <= Tolerance::DEFAULT.linear {
                return Err(bad(format!("the axis line '{id}' has zero length")));
            }
            Ok((o, d))
        }
    }
}

/// The most edges one fillet or chamfer may name.
const MAX_ROUNDED: usize = 256;

/// Rounds the edges named by their two faces: each is found on the target's solid (it must be
/// parallel to the part's axis), then rounded in the layered model.
fn round_edges(
    feature: &str,
    target: &FeatureSlot,
    tgt: &str,
    edges: &[[String; 2]],
    size: f64,
    kind: Round,
) -> Result<Built, Diagnostic> {
    let word = if kind == Round::Fillet {
        "radius"
    } else {
        "distance"
    };
    if !size.is_finite() || size <= 0.0 {
        return Err(Diagnostic::new(
            DiagCode::InvalidValue,
            format!("the {word} must be finite and greater than zero, got {size}"),
        ));
    }
    if edges.is_empty() || edges.len() > MAX_ROUNDED {
        return Err(Diagnostic::new(
            DiagCode::InvalidValue,
            format!(
                "name between 1 and {MAX_ROUNDED} edges to round, got {}",
                edges.len()
            ),
        ));
    }
    let solid = &*target.solid;
    // Which of the named edges are circular rims (a flat end against a round wall)? Those are
    // rounded on the solid itself; the others, parallel to the axis, in the layered model.
    let is_rim = |fa: &str, fb: &str| -> bool {
        let (Some(a), Some(b)) = (
            solid.faces.iter().find(|f| f.name == fa),
            solid.faces.iter().find(|f| f.name == fb),
        ) else {
            return false;
        };
        let plane_and_cyl = matches!(
            (&a.surface, &b.surface),
            (
                dc_kernel::Surface::Plane { .. },
                dc_kernel::Surface::Cylinder { .. }
            ) | (
                dc_kernel::Surface::Cylinder { .. },
                dc_kernel::Surface::Plane { .. }
            )
        );
        if !plane_and_cyl {
            return false;
        }
        let ids = |f: &dc_kernel::Face| -> Vec<u32> {
            f.loops.iter().flatten().map(|c| c.edge).collect()
        };
        let eb = ids(b);
        ids(a).iter().any(|e| {
            eb.contains(e)
                && matches!(
                    solid.edges[*e as usize].curve,
                    dc_kernel::Curve::Circle { .. }
                )
        })
    };
    type Numbered<'a> = Vec<(usize, &'a [String; 2])>;
    let (rims, straight): (Numbered, Numbered) = edges
        .iter()
        .enumerate()
        .partition(|(_, [a, b])| is_rim(a, b));
    let mut working: Option<Stepped> = None;
    let mut brep_blended = false;
    let mut current: Arc<Solid> = target.solid.clone();
    // The layered model rounds edges that run along the part's axis (and keeps its face names);
    // any other straight edge between two flat faces, or any edge of a part that has no layered
    // model, is rounded on the B-rep itself.
    let along_axis = target.stepped.as_deref().is_some_and(|model| {
        let n = model.plane.normal();
        straight.iter().all(|(_, [fa, fb])| {
            let find = |name: &str| solid.faces.iter().find(|f| f.name == name);
            let (Some(a), Some(b)) = (find(fa), find(fb)) else {
                return true; // the layered path explains what is missing
            };
            let ids = |f: &dc_kernel::Face| -> Vec<u32> {
                f.loops.iter().flatten().map(|c| c.edge).collect()
            };
            let eb = ids(b);
            ids(a).iter().filter(|e| eb.contains(e)).all(|&eid| {
                let e = &solid.edges[eid as usize];
                let d = solid.vertices[e.v1 as usize] - solid.vertices[e.v0 as usize];
                matches!(e.curve, dc_kernel::Curve::Line)
                    && d.length() > Tolerance::DEFAULT.linear
                    && Tolerance::DEFAULT.parallel(d, n)
            })
        })
    });
    if !straight.is_empty() && !along_axis {
        let pairs: Vec<[String; 2]> = straight.iter().map(|(_, p)| (*p).clone()).collect();
        let blended = dc_kernel::blend_edges(
            solid,
            &pairs,
            if kind == Round::Fillet {
                dc_kernel::BlendKind::Fillet
            } else {
                dc_kernel::BlendKind::Chamfer
            },
            size,
            feature,
        )
        .map_err(|e| blend_diag(e, tgt, kind.label()))?;
        if rims.is_empty() {
            let summary = summarize(&blended);
            return Ok(Built {
                solid: Arc::new(blended),
                summary,
                stepped: None,
            });
        }
        current = Arc::new(blended);
        brep_blended = true;
    } else if !straight.is_empty() {
        let model = target.model(tgt, &format!("a {}", kind.label()))?;
        let n = model.plane.normal();
        let mut out = model.clone();
        for (i, [fa, fb]) in straight {
            let face = |name: &str| {
                solid.faces.iter().find(|f| f.name == name).ok_or_else(|| {
                    let sides: Vec<&str> = solid
                        .faces
                        .iter()
                        .filter(|f| f.name.starts_with("side:"))
                        .take(20)
                        .map(|f| f.name.as_str())
                        .collect();
                    Diagnostic::new(
                        DiagCode::NotFound,
                        format!(
                            "'{tgt}' has no face '{name}' (side faces: {})",
                            sides.join(", ")
                        ),
                    )
                    .entity(feature_path(tgt))
                })
            };
            let (a, b) = (face(fa)?, face(fb)?);
            let ids = |f: &dc_kernel::Face| -> Vec<u32> {
                f.loops.iter().flatten().map(|c| c.edge).collect()
            };
            let (ea, eb) = (ids(a), ids(b));
            let shared: Vec<u32> = ea.iter().copied().filter(|e| eb.contains(e)).collect();
            if shared.is_empty() {
                return Err(Diagnostic::new(
                    DiagCode::InvalidValue,
                    format!("faces '{fa}' and '{fb}' of '{tgt}' do not meet along an edge"),
                )
                .suggest("name the two faces that meet at the edge, as listed by feature.faces"));
            }
            for (k, &eid) in shared.iter().enumerate() {
                let e = &solid.edges[eid as usize];
                let (p0, p1) = (solid.vertices[e.v0 as usize], solid.vertices[e.v1 as usize]);
                let d = p1 - p0;
                let parallel = matches!(e.curve, dc_kernel::Curve::Line)
                    && d.length() > Tolerance::DEFAULT.linear
                    && Tolerance::DEFAULT.parallel(d, n);
                if !parallel {
                    return Err(unsupported(format!(
                        "the edge between '{fa}' and '{fb}' is neither parallel to the part's axis nor the circular rim of a round wall: other edges need more surface types (spheres, general blends)"
                    )));
                }
                let w = |p: dc_math::Vec3| (p - model.plane.origin).dot(n);
                let (lo, hi) = (w(p0).min(w(p1)), w(p0).max(w(p1)));
                let name = if shared.len() == 1 {
                    format!("{}:{feature}:{i}", kind.label())
                } else {
                    format!("{}:{feature}:{i}.{k}", kind.label())
                };
                out = out
                    .round_edge(model.plane.to_2d(p0), lo, hi, size, kind, &name)
                    .map_err(|e| {
                        step_diag(
                            e,
                            &format!("{} of the edge between '{fa}' and '{fb}'", kind.label()),
                        )
                    })?;
            }
        }
        if rims.is_empty() {
            return finish(out);
        }
        current = Arc::new(build_solid(&out).map_err(|e| step_diag(e, "this feature"))?);
        working = Some(out);
    }
    let rim_kind = if kind == Round::Fillet {
        RimRound::Fillet
    } else {
        RimRound::Chamfer
    };
    // The layered model keeps the rounded rims, so a later cut or boss is built on the layers
    // and the rims are rounded again on the result.
    let mut model_out: Option<Stepped> = if brep_blended {
        None
    } else {
        working.or_else(|| target.stepped.as_deref().cloned())
    };
    let mut solid_now: Solid = (*current).clone();
    for (i, [fa, fb]) in rims {
        let name = format!("{}:{feature}:{i}", kind.label());
        solid_now = solid_now
            .round_rim(fa, fb, rim_kind, size, &name)
            .map_err(|e| rim_diag(e, tgt, fa, fb, word))?;
        if let Some(m) = &mut model_out {
            m.rims.push(RimSpec {
                face_a: fa.clone(),
                face_b: fb.clone(),
                kind: rim_kind,
                size,
                name,
            });
        }
    }
    let summary = summarize(&solid_now);
    Ok(Built {
        solid: Arc::new(solid_now),
        summary,
        stepped: model_out.map(Arc::new),
    })
}

fn blend_diag(e: dc_kernel::BlendError, tgt: &str, what: &str) -> Diagnostic {
    use dc_kernel::BlendError as B;
    match e {
        B::NotFound(m) => Diagnostic::new(DiagCode::NotFound, format!("{what} of '{tgt}': {m}"))
            .entity(feature_path(tgt)),
        B::Unsupported(m) => {
            Diagnostic::new(DiagCode::Unsupported, format!("{what} of '{tgt}': {m}"))
                .entity(feature_path(tgt))
        }
        B::TooLarge(m) => {
            Diagnostic::new(DiagCode::FeatureFailed, format!("{what} of '{tgt}': {m}"))
                .suggest("use a smaller size")
                .entity(feature_path(tgt))
        }
        B::Invalid(v) => Diagnostic::new(
            DiagCode::FeatureFailed,
            format!("{what} of '{tgt}' produced an invalid solid: {v:?}"),
        )
        .suggest("this is a kernel bug; please report the part and the edge"),
    }
}

fn rim_diag(e: RimError, tgt: &str, fa: &str, fb: &str, word: &str) -> Diagnostic {
    let what = format!("the edge between '{fa}' and '{fb}' of '{tgt}'");
    match e {
        RimError::NotFound(n) => {
            Diagnostic::new(DiagCode::NotFound, format!("'{tgt}' has no face '{n}'"))
        }
        RimError::Unsupported(m) => Diagnostic::new(DiagCode::Unsupported, format!("{what}: {m}")),
        RimError::TooLarge(m) => Diagnostic::new(
            DiagCode::FeatureFailed,
            format!("{what}: the {word} does not fit: {m}"),
        )
        .suggest(format!("use a smaller {word}")),
        RimError::Invalid(v) => Diagnostic::new(
            DiagCode::FeatureFailed,
            format!(
                "rounding {what} produced an invalid solid: {}",
                v.join("; ")
            ),
        )
        .suggest("this is a kernel bug; please report the part and the edge"),
    }
}

/// Re-expresses a profile drawn on plane `from` in the 2D frame of plane `to` (the planes must be
/// parallel). Loops stay counter-clockwise: a mirror image (opposite normals) is flipped back.
fn transform_profile(p: &Profile, from: &Plane, to: &Plane) -> Profile {
    map_profile(p, &|q| to.to_2d(from.to_3d(q)))
}

/// Moves a profile by a rigid map of the plane (a translation, a rotation, a reflection, or
/// any combination; `map` must not scale). Loops stay counter-clockwise: a reflection reverses
/// their direction, so they are flipped back.
fn map_profile(p: &Profile, map: &dyn Fn(Vec2) -> Vec2) -> Profile {
    let o = map(Vec2::new(0.0, 0.0));
    let (ex, ey) = (map(Vec2::new(1.0, 0.0)) - o, map(Vec2::new(0.0, 1.0)) - o);
    let mirrored = ex.cross(ey) < 0.0;
    let phi = det::atan2(ex.y, ex.x);
    let pt = |a: [f64; 2]| {
        let m = map(Vec2::new(a[0], a[1]));
        [m.x, m.y]
    };
    let xf_loop = |l: &Loop| -> Loop {
        let single_circle = l.edges.len() == 1 && matches!(l.edges[0].curve, Curve2::Circle { .. });
        let mut edges: Vec<LoopEdge> = l
            .edges
            .iter()
            .map(|e| {
                let curve = match &e.curve {
                    Curve2::Line => Curve2::Line,
                    Curve2::Circle { center, radius } => Curve2::Circle {
                        center: pt(*center),
                        radius: *radius,
                    },
                    Curve2::Arc {
                        center,
                        radius,
                        start_angle,
                        sweep,
                    } => Curve2::Arc {
                        center: pt(*center),
                        radius: *radius,
                        start_angle: if mirrored {
                            phi - (start_angle + sweep)
                        } else {
                            start_angle + phi
                        },
                        sweep: *sweep,
                    },
                };
                let reversed = if mirrored && !matches!(e.curve, Curve2::Circle { .. }) {
                    !e.reversed
                } else {
                    e.reversed
                };
                // A circle's start point is by convention at angle 0 of the new frame, whatever
                // the rotation: re-anchor it instead of mapping the old start point.
                let (from, to) = match &curve {
                    Curve2::Circle { center, radius } => {
                        let p = [center[0] + radius, center[1]];
                        (p, p)
                    }
                    _ => (pt(e.from), pt(e.to)),
                };
                LoopEdge {
                    id: e.id.clone(),
                    reversed,
                    from,
                    to,
                    curve,
                }
            })
            .collect();
        if mirrored && !single_circle {
            edges.reverse();
            for e in &mut edges {
                e.reversed = !e.reversed;
                core::mem::swap(&mut e.from, &mut e.to);
            }
        }
        Loop {
            edges,
            area: l.area,
        }
    };
    Profile {
        id: p.id.clone(),
        outer: xf_loop(&p.outer),
        holes: p.holes.iter().map(xf_loop).collect(),
        area: p.area,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_math::Vec3;
    use dc_sketch::Entity;

    fn notched_block() -> Profile {
        let mut s = Sketch::new();
        for (id, x, y) in [
            ("a", 0.0, 0.0),
            ("b", 40.0, 0.0),
            ("c", 40.0, 20.0),
            ("d", 25.0, 20.0),
            ("e", 15.0, 20.0),
            ("f", 0.0, 20.0),
            ("o", 20.0, 20.0),
        ] {
            s.add_entity(id, Entity::Point { x, y }).unwrap();
        }
        for (id, p, q) in [
            ("ab", "a", "b"),
            ("bc", "b", "c"),
            ("cd", "c", "d"),
            ("ef", "e", "f"),
            ("fa", "f", "a"),
        ] {
            s.add_entity(
                id,
                Entity::Line {
                    a: p.into(),
                    b: q.into(),
                },
            )
            .unwrap();
        }
        s.add_entity(
            "notch",
            Entity::Arc {
                center: "o".into(),
                start: "e".into(),
                end: "d".into(),
            },
        )
        .unwrap();
        s.add_entity("k", Entity::Point { x: 10.0, y: 8.0 })
            .unwrap();
        s.add_entity(
            "hole",
            Entity::Circle {
                center: "k".into(),
                radius: 3.0,
            },
        )
        .unwrap();
        s.profiles().profiles.remove(0)
    }

    #[test]
    fn mapping_a_profile_between_planes_preserves_its_shape_for_translations_rotations_and_mirrors()
    {
        let t = Tolerance::DEFAULT;
        let from = Plane::new(Vec3::ZERO, Vec3::X, Vec3::Y, t).unwrap();
        let tos = [
            Plane::new(Vec3::new(5.0, -3.0, 9.0), Vec3::X, Vec3::Y, t).unwrap(), // translated
            Plane::new(Vec3::new(1.0, 2.0, 0.0), Vec3::Y, -Vec3::X, t).unwrap(), // rotated 90 degrees
            Plane::new(Vec3::new(0.0, 0.0, 0.0), Vec3::X, -Vec3::Y, t).unwrap(), // mirrored (normal flips)
            Plane::new(Vec3::new(7.0, 1.0, 4.0), -Vec3::X, Vec3::Y, t).unwrap(), // mirrored the other way
            Plane::new(
                Vec3::new(-2.0, 5.0, 0.0),
                Vec3::new(1.0, 1.0, 0.0),
                Vec3::new(-1.0, 1.0, 0.0),
                t,
            )
            .unwrap(), // 45 degrees
        ];
        let p = notched_block();
        let original = p.prepare();
        for to in tos {
            let q = transform_profile(&p, &from, &to);
            let mapped = q.prepare();
            assert!((q.area - p.area).abs() < 1e-12);
            for l in std::iter::once(&q.outer).chain(&q.holes) {
                assert!(l.area > 0.0);
                for (i, e) in l.edges.iter().enumerate() {
                    assert_eq!(
                        e.to,
                        l.edges[(i + 1) % l.edges.len()].from,
                        "loop stays connected"
                    );
                }
            }
            // Probe a grid: a point is in the mapped profile exactly when its preimage is in the original.
            let mut checked = 0;
            let mut y = -3.3;
            while y < 24.0 {
                let mut x = -4.1;
                while x < 45.0 {
                    let pre = Vec2::new(x, y);
                    let img = to.to_2d(from.to_3d(pre));
                    // Skip points within 0.05 of the notch/hole/outline, where polylines disagree.
                    let r1 = (pre - Vec2::new(20.0, 20.0)).length();
                    let r2 = (pre - Vec2::new(10.0, 8.0)).length();
                    let near = (r1 - 5.0).abs() < 0.05
                        || (r2 - 3.0).abs() < 0.05
                        || [0.0, 40.0].iter().any(|v| (x - v).abs() < 0.05)
                        || [0.0, 20.0].iter().any(|v| (y - v).abs() < 0.05);
                    if !near {
                        assert_eq!(
                            mapped.contains(img),
                            original.contains(pre),
                            "{to:?} at ({x}, {y})"
                        );
                        checked += 1;
                    }
                    x += 0.9;
                }
                y += 0.7;
            }
            assert!(checked > 500);
        }
    }
}
