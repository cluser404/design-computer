//! The native `.dcad` file: a ZIP containing the command log (the source of truth, see
//! AGENTS.md §7) and a manifest.
//!
//! ```text
//! manifest.json   format name + version, engine version, units, final revision and state hash
//! history.jsonl   one committed command per line, with its state hash
//! <anything else> preserved verbatim on read and write (future plugins, caches, thumbnails)
//! ```
//!
//! Loading replays the log and verifies the state hash after every command, then checks the
//! result against the manifest, so truncation, tampering or a kernel that computes something
//! differently are all caught.

use crate::zip::{self, ZipError};
use dc_doc::{Authority, Hash128, Policy, ReplayError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Bumped on any incompatible change; older files get migrations, newer files are refused.
pub const FORMAT_VERSION: u32 = 1;
const FORMAT_NAME: &str = "designcomputer";
const MANIFEST: &str = "manifest.json";
const HISTORY: &str = "history.jsonl";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub format_version: u32,
    /// Engine that wrote the file (informational: files never depend on it, since the history
    /// is replayed).
    pub engine: String,
    pub units: String,
    pub revision: u64,
    pub state_hash: Hash128,
}

#[derive(Debug)]
pub enum DcadError {
    Zip(ZipError),
    MissingEntry(&'static str),
    BadManifest(String),
    NotDcad(String),
    /// Written by a newer version of the format than this build understands.
    TooNew {
        found: u32,
        supported: u32,
    },
    Replay(ReplayError),
    /// The history replays cleanly but does not end where the manifest says it should.
    ManifestMismatch(String),
}

impl fmt::Display for DcadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DcadError::Zip(e) => write!(f, "{e}"),
            DcadError::MissingEntry(n) => write!(f, "the file has no '{n}'"),
            DcadError::BadManifest(m) => write!(f, "manifest is unreadable: {m}"),
            DcadError::NotDcad(n) => write!(f, "not a Design Computer file (format is '{n}')"),
            DcadError::TooNew { found, supported } => write!(
                f,
                "this file uses format version {found}, but this build only understands up to {supported}; update Design Computer"
            ),
            DcadError::Replay(e) => write!(f, "the history cannot be replayed: {e}"),
            DcadError::ManifestMismatch(m) => write!(f, "the file is inconsistent: {m}"),
        }
    }
}

impl std::error::Error for DcadError {}

/// A loaded document plus any entries this build does not understand.
pub struct Dcad {
    pub authority: Authority,
    pub manifest: Manifest,
    /// Unknown entries, kept so saving does not silently drop other tools' data.
    pub extras: BTreeMap<String, Vec<u8>>,
}

/// Serializes a document. Deterministic: the same document always yields the same bytes.
pub fn write(authority: &Authority, extras: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let manifest = Manifest {
        format: FORMAT_NAME.into(),
        format_version: FORMAT_VERSION,
        engine: env!("CARGO_PKG_VERSION").into(),
        units: "mm".into(),
        revision: authority.revision().0,
        state_hash: authority.state_hash(),
    };
    let manifest = serde_json::to_vec_pretty(&manifest).expect("manifest serializes");
    let history = authority.log_jsonl().into_bytes();
    let mut entries: Vec<(&str, &[u8])> = vec![(MANIFEST, &manifest), (HISTORY, &history)];
    for (name, data) in extras {
        if name != MANIFEST && name != HISTORY {
            entries.push((name, data));
        }
    }
    zip::write(&entries)
}

/// Loads a document, verifying everything on the way.
pub fn read(bytes: &[u8], policy: Policy) -> Result<Dcad, DcadError> {
    let mut entries: BTreeMap<String, Vec<u8>> = zip::read(bytes)
        .map_err(DcadError::Zip)?
        .into_iter()
        .collect();
    let manifest_bytes = entries
        .remove(MANIFEST)
        .ok_or(DcadError::MissingEntry(MANIFEST))?;
    // Check the format identity before anything else, so a newer file gets "too new", not a
    // confusing parse error about fields we have never heard of.
    let raw: serde_json::Value = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| DcadError::BadManifest(e.to_string()))?;
    let name = raw.get("format").and_then(|v| v.as_str()).unwrap_or("");
    if name != FORMAT_NAME {
        return Err(DcadError::NotDcad(name.to_owned()));
    }
    let version = raw
        .get("format_version")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as u32;
    if version > FORMAT_VERSION {
        return Err(DcadError::TooNew {
            found: version,
            supported: FORMAT_VERSION,
        });
    }
    let manifest: Manifest =
        serde_json::from_value(raw).map_err(|e| DcadError::BadManifest(e.to_string()))?;
    let history = entries
        .remove(HISTORY)
        .ok_or(DcadError::MissingEntry(HISTORY))?;
    let history = String::from_utf8(history)
        .map_err(|_| DcadError::BadManifest("history is not UTF-8".into()))?;
    let authority = Authority::replay(&history, policy).map_err(DcadError::Replay)?;
    if authority.revision().0 != manifest.revision {
        return Err(DcadError::ManifestMismatch(format!(
            "history ends at revision {} but the manifest says {} (the file was truncated or edited)",
            authority.revision().0,
            manifest.revision
        )));
    }
    if authority.state_hash() != manifest.state_hash {
        return Err(DcadError::ManifestMismatch(
            "the replayed state does not match the manifest's hash".into(),
        ));
    }
    Ok(Dcad {
        authority,
        manifest,
        extras: entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_doc::{Actor, Command, CommandKind as K, Expected};

    fn doc() -> Authority {
        let mut a = Authority::default();
        let who = Actor::Human("ann".into());
        for (i, kind) in [
            K::ParamSet {
                name: "x".into(),
                value: 1.5,
                expected: Expected::Any,
            },
            K::SketchCreate {
                sketch: "s".into(),
                plane: Default::default(),
            },
            K::ParamSet {
                name: "y".into(),
                value: -2.0,
                expected: Expected::Any,
            },
            K::HistoryUndo { revision: None },
        ]
        .into_iter()
        .enumerate()
        {
            let c = Command {
                id: format!("c{i}"),
                actor: who.clone(),
                base_revision: a.revision(),
                kind,
            };
            a.submit(c).unwrap();
        }
        a
    }

    #[test]
    fn round_trips_state_history_and_undo_bookkeeping() {
        let a = doc();
        let bytes = write(&a, &BTreeMap::new());
        let loaded = read(&bytes, Policy::default()).unwrap();
        assert_eq!(loaded.authority.state_hash(), a.state_hash());
        assert_eq!(loaded.authority.log_jsonl(), a.log_jsonl());
        assert_eq!(loaded.manifest.revision, 4);
        assert_eq!(loaded.manifest.units, "mm");
        // Undo history survived: the undone parameter can be redone after loading.
        let mut b = loaded.authority;
        let c = Command {
            id: "r".into(),
            actor: Actor::Human("ann".into()),
            base_revision: b.revision(),
            kind: K::HistoryRedo { revision: None },
        };
        b.submit(c).unwrap();
        assert_eq!(b.state().get("y"), Some(-2.0));
    }

    #[test]
    fn output_is_deterministic_and_empty_documents_work() {
        let a = doc();
        assert_eq!(write(&a, &BTreeMap::new()), write(&a, &BTreeMap::new()));
        let empty = Authority::default();
        let loaded = read(&write(&empty, &BTreeMap::new()), Policy::default()).unwrap();
        assert_eq!(loaded.authority.revision().0, 0);
    }

    #[test]
    fn unknown_entries_survive_a_load_and_save() {
        let mut extras = BTreeMap::new();
        extras.insert("plugins/thing.bin".to_string(), vec![1, 2, 3]);
        extras.insert("thumbnail.png".to_string(), b"not really a png".to_vec());
        let bytes = write(&doc(), &extras);
        let loaded = read(&bytes, Policy::default()).unwrap();
        assert_eq!(loaded.extras, extras);
        assert_eq!(
            write(&loaded.authority, &loaded.extras),
            bytes,
            "resaving changes nothing"
        );
    }

    fn rebuild(entries: Vec<(&str, Vec<u8>)>) -> Vec<u8> {
        let refs: Vec<(&str, &[u8])> = entries.iter().map(|(n, d)| (*n, d.as_slice())).collect();
        zip::write(&refs)
    }

    #[test]
    fn damage_is_caught_at_every_layer() {
        let a = doc();
        let good = write(&a, &BTreeMap::new());
        let manifest =
            serde_json::to_string_pretty(&read(&good, Policy::default()).unwrap().manifest)
                .unwrap();
        let history = a.log_jsonl();

        // A flipped byte anywhere: zip checksum.
        let mut bad = good.clone();
        let mid = bad.len() / 2;
        bad[mid] ^= 1;
        assert!(read(&bad, Policy::default()).is_err());

        // History edited and re-zipped (valid CRCs): the replayed state hash catches it.
        let edited = history.replacen("1.5", "2.5", 1);
        let e = read(
            &rebuild(vec![
                (MANIFEST, manifest.clone().into_bytes()),
                (HISTORY, edited.into_bytes()),
            ]),
            Policy::default(),
        );
        assert!(matches!(e, Err(DcadError::Replay(_))), "{:?}", e.err());

        // History truncated to fewer commands (each line still valid): the manifest catches it.
        let truncated: String = history.lines().take(2).map(|l| format!("{l}\n")).collect();
        let e = read(
            &rebuild(vec![
                (MANIFEST, manifest.clone().into_bytes()),
                (HISTORY, truncated.into_bytes()),
            ]),
            Policy::default(),
        );
        assert!(
            matches!(e, Err(DcadError::ManifestMismatch(_))),
            "{:?}",
            e.err()
        );

        // Missing entries.
        let e = read(
            &rebuild(vec![(HISTORY, history.clone().into_bytes())]),
            Policy::default(),
        );
        assert!(matches!(e, Err(DcadError::MissingEntry(MANIFEST))));
        let e = read(
            &rebuild(vec![(MANIFEST, manifest.clone().into_bytes())]),
            Policy::default(),
        );
        assert!(matches!(e, Err(DcadError::MissingEntry(HISTORY))));

        // Not ours at all.
        assert!(matches!(
            read(b"hello", Policy::default()),
            Err(DcadError::Zip(_))
        ));
        let other = manifest.replace("designcomputer", "somethingelse");
        let e = read(
            &rebuild(vec![
                (MANIFEST, other.into_bytes()),
                (HISTORY, history.clone().into_bytes()),
            ]),
            Policy::default(),
        );
        assert!(matches!(e, Err(DcadError::NotDcad(_))));
    }

    #[test]
    fn newer_format_versions_are_refused_with_a_clear_message() {
        let a = doc();
        let good = write(&a, &BTreeMap::new());
        let manifest =
            serde_json::to_string_pretty(&read(&good, Policy::default()).unwrap().manifest)
                .unwrap();
        let newer = manifest
            .replace("\"format_version\": 1", "\"format_version\": 2")
            .replace("\"units\"", "\"future_field\": true,\n  \"units\"");
        let e = read(
            &rebuild(vec![
                (MANIFEST, newer.into_bytes()),
                (HISTORY, a.log_jsonl().into_bytes()),
            ]),
            Policy::default(),
        );
        match e {
            Err(
                err @ DcadError::TooNew {
                    found: 2,
                    supported: 1,
                },
            ) => {
                assert!(err.to_string().contains("update Design Computer"));
            }
            other => panic!("{:?}", other.err()),
        }
    }
}
