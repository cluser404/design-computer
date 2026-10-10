//! Mouse navigation presets: which drag rotates, pans or zooms the view. Pure logic so each
//! preset is tested, and a command can switch between them.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NavPreset {
    /// Middle rotates, Ctrl + middle pans, Shift + middle zooms (what SolidWorks users know).
    #[default]
    SolidWorks,
    /// Middle pans, Shift + middle rotates, Ctrl + middle zooms (Fusion / Onshape).
    Fusion,
    /// Middle rotates, Shift + middle pans, Ctrl + middle zooms (Blender).
    Blender,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavAction {
    Rotate,
    Pan,
    Zoom,
}

impl NavPreset {
    pub const ALL: [NavPreset; 3] = [NavPreset::SolidWorks, NavPreset::Fusion, NavPreset::Blender];

    pub fn name(self) -> &'static str {
        match self {
            NavPreset::SolidWorks => "SolidWorks",
            NavPreset::Fusion => "Fusion / Onshape",
            NavPreset::Blender => "Blender",
        }
    }

    pub fn next(self) -> NavPreset {
        match self {
            NavPreset::SolidWorks => NavPreset::Fusion,
            NavPreset::Fusion => NavPreset::Blender,
            NavPreset::Blender => NavPreset::SolidWorks,
        }
    }

    /// What a drag with the middle button does with these modifiers held.
    pub fn middle_drag(self, ctrl: bool, shift: bool) -> NavAction {
        use NavAction::{Pan, Rotate, Zoom};
        match (self, ctrl, shift) {
            (NavPreset::SolidWorks, true, _) => Pan,
            (NavPreset::SolidWorks, _, true) => Zoom,
            (NavPreset::SolidWorks, ..) => Rotate,
            (NavPreset::Fusion, _, true) => Rotate,
            (NavPreset::Fusion, true, _) => Zoom,
            (NavPreset::Fusion, ..) => Pan,
            (NavPreset::Blender, _, true) => Pan,
            (NavPreset::Blender, true, _) => Zoom,
            (NavPreset::Blender, ..) => Rotate,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use NavAction::*;

    #[test]
    fn each_preset_maps_the_middle_button_as_documented_and_names_are_distinct() {
        let t = |p: NavPreset| {
            [
                p.middle_drag(false, false),
                p.middle_drag(true, false),
                p.middle_drag(false, true),
            ]
        };
        assert_eq!(t(NavPreset::SolidWorks), [Rotate, Pan, Zoom]);
        assert_eq!(t(NavPreset::Fusion), [Pan, Zoom, Rotate]);
        assert_eq!(t(NavPreset::Blender), [Rotate, Zoom, Pan]);
        // Cycling visits all three and returns.
        let mut p = NavPreset::SolidWorks;
        let mut seen = vec![p];
        for _ in 0..3 {
            p = p.next();
            seen.push(p);
        }
        assert_eq!(seen[3], NavPreset::SolidWorks);
        assert_ne!(seen[0].name(), seen[1].name());
    }
}
