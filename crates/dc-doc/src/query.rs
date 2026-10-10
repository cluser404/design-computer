use crate::feature::{FaceInfo, FeatureSpec, SolidSummary};
use crate::{Hash128, LogEntry, Revision};
use dc_sketch::{Constraint, Entity, PlaneSpec, ProfileProblem, SolveReport, SolveStatus};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Hard upper bound on any page size. Unbounded queries do not exist (AGENTS.md §4.7).
pub const MAX_PAGE: usize = 1000;
pub const DEFAULT_PAGE: usize = 100;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum Query {
    /// Cheap overview: revision, entry count, state hash. Start here on large models.
    #[serde(rename = "summary")]
    Summary,
    /// How the document is organised: labels, folders, what is suppressed, the rollback bar and
    /// what is therefore off right now. Bounded: at most `MAX_PAGE` entries of each list.
    #[serde(rename = "organization")]
    Organization,
    #[serde(rename = "param")]
    Param { name: String },
    /// Paginated listing, ordered by name.
    #[serde(rename = "params")]
    Params {
        #[serde(default)]
        prefix: String,
        after: Option<String>,
        limit: Option<usize>,
    },
    /// Paginated list of sketches with their solve status and degrees of freedom.
    #[serde(rename = "sketches")]
    Sketches {
        #[serde(default)]
        prefix: String,
        after: Option<String>,
        limit: Option<usize>,
    },
    /// One sketch's overview and full solver analysis (DOF, under-constrained, redundant ...).
    #[serde(rename = "sketch.summary")]
    SketchSummary { sketch: String },
    /// Paginated entities of a sketch, ordered by ID.
    #[serde(rename = "sketch.entities")]
    SketchEntities {
        sketch: String,
        after: Option<String>,
        limit: Option<usize>,
    },
    /// Paginated constraints of a sketch, ordered by ID.
    #[serde(rename = "sketch.constraints")]
    SketchConstraints {
        sketch: String,
        after: Option<String>,
        limit: Option<usize>,
    },
    /// The closed profiles of a sketch (what can be extruded) and what blocks other geometry from
    /// forming one (open ends, branches, crossings), paginated by profile ID.
    #[serde(rename = "sketch.profiles")]
    SketchProfiles {
        sketch: String,
        after: Option<String>,
        limit: Option<usize>,
    },
    /// Paginated list of features with their exact solid measurements.
    #[serde(rename = "features")]
    Features {
        #[serde(default)]
        prefix: String,
        after: Option<String>,
        limit: Option<usize>,
    },
    #[serde(rename = "feature.summary")]
    FeatureSummary { feature: String },
    /// Paginated faces of a feature's solid, by stable name.
    #[serde(rename = "feature.faces")]
    FeatureFaces {
        feature: String,
        after: Option<String>,
        limit: Option<usize>,
    },
    /// Paginated history; `from` is the first revision wanted (1-based).
    #[serde(rename = "log")]
    Log { from: u64, limit: Option<usize> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum QueryResult {
    #[serde(rename = "organization")]
    Organization {
        /// `feature/<id>` or `sketch/<id>` and the name shown for it.
        labels: Vec<(String, String)>,
        folders: Vec<(String, Vec<String>)>,
        suppressed: Vec<String>,
        rollback: Option<String>,
        /// Everything that is off: suppressed, after the bar, or built on such a feature.
        hidden: Vec<String>,
        /// Features in build order, as the design tree lists them.
        order: Vec<String>,
    },
    #[serde(rename = "summary")]
    Summary {
        revision: Revision,
        param_count: usize,
        sketch_count: usize,
        feature_count: usize,
        state_hash: Hash128,
    },
    #[serde(rename = "param")]
    Param { name: String, value: Option<f64> },
    #[serde(rename = "params")]
    Params {
        items: Vec<(String, f64)>,
        next: Option<String>,
    },
    #[serde(rename = "sketches")]
    Sketches {
        items: Vec<SketchInfo>,
        next: Option<String>,
    },
    #[serde(rename = "sketch.summary")]
    SketchSummary {
        info: SketchInfo,
        report: SolveReport,
    },
    #[serde(rename = "sketch.entities")]
    SketchEntities {
        items: Vec<(String, Entity)>,
        next: Option<String>,
        /// Which of the entities on this page are construction geometry.
        #[serde(default)]
        construction: Vec<String>,
        /// Which of the construction lines on this page are infinite.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        infinite: Vec<String>,
    },
    #[serde(rename = "sketch.constraints")]
    SketchConstraints {
        items: Vec<(String, Constraint)>,
        next: Option<String>,
        /// Where the dimensions on this page are drawn, where they were placed.
        #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
        placements: std::collections::BTreeMap<String, [f64; 2]>,
        /// Which of the dimensions on this page are references (they measure, constrain nothing).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        driven: Vec<String>,
        /// How dimensions on this page are written, where it differs from the default.
        #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
        styles: std::collections::BTreeMap<String, dc_sketch::DimStyle>,
    },
    #[serde(rename = "sketch.profiles")]
    SketchProfiles {
        profiles: Vec<ProfileInfo>,
        next: Option<String>,
        /// The first problems found (at most 100); see `problem_count` for the total.
        problems: Vec<ProfileProblem>,
        problem_count: usize,
    },
    #[serde(rename = "features")]
    Features {
        items: Vec<FeatureInfo>,
        next: Option<String>,
    },
    #[serde(rename = "feature.summary")]
    FeatureSummary { info: FeatureInfo },
    #[serde(rename = "feature.faces")]
    FeatureFaces {
        items: Vec<FaceInfo>,
        next: Option<String>,
    },
    #[serde(rename = "log")]
    Log {
        entries: Vec<LogEntry>,
        next: Option<u64>,
    },
}

/// A closed region of a sketch that can be extruded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProfileInfo {
    pub id: String,
    /// Outer area minus hole areas, in mm².
    pub area: f64,
    /// Sketch entity IDs of the outer boundary, in order.
    pub outer: Vec<String>,
    /// Entity IDs of each hole's boundary.
    pub holes: Vec<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FeatureInfo {
    pub id: String,
    pub kind: String,
    pub spec: FeatureSpec,
    pub summary: SolidSummary,
}

/// One-line description of a sketch: what a human reads off the feature tree and DOF display.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SketchInfo {
    pub id: String,
    pub entities: usize,
    pub constraints: usize,
    pub status: SolveStatus,
    pub dof: usize,
    pub fully_constrained: bool,
    pub plane: PlaneSpec,
    /// Where the sketch actually is in 3D right now (resolved for face-attached sketches), or
    /// `None` if its face no longer exists.
    pub frame: Option<PlaneFrame>,
}

/// A sketch plane in world coordinates: a point on it, its in-plane axes and its normal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlaneFrame {
    pub origin: [f64; 3],
    pub x_axis: [f64; 3],
    pub y_axis: [f64; 3],
    pub normal: [f64; 3],
}

pub(crate) fn clamp_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE)
}
