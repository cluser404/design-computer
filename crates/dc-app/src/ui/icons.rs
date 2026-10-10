//! Icons. Generic ones (file, undo, eye, ...) are glyphs of the Phosphor icon font; the
//! CAD-specific ones are drawn here, in one consistent style, from a handful of primitives on a
//! 24 x 24 grid, so they stay sharp at any size and on any display (there are no bitmaps).
//!
//! Style: the drawing's existing geometry is dark, the geometry the command makes is blue, the
//! points that define it are orange, what gets removed is red and a result that is accepted is
//! green. A dark theme swaps the inks, not the drawings.

use super::theme::{icon_family, Colors};
use eframe::egui::{self, Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke};

/// What a command's button shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Icon {
    /// A glyph of the icon font (see `icon_font`).
    Glyph(char),
    /// A drawn CAD icon.
    Cad(CadIcon),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CadIcon {
    Select,
    Line,
    Centerline,
    Rectangle,
    CenterRectangle,
    ThreePointRect,
    ThreePointCentreRect,
    Parallelogram,
    Polygon,
    Slot,
    Circle,
    Arc,
    Arc3,
    TangentArc,
    Point,
    Dimension,
    Trim,
    Extend,
    Split,
    Move,
    Rotate,
    Scale,
    Mirror,
    LinearPattern,
    CircularPattern,
    Offset,
    Fillet,
    Chamfer,
    Sketch,
    ExitSketch,
    ExtrudedBoss,
    ExtrudedCut,
    Boss,
    Shell,
    Loft,
    Sweep,
    FullyDefine,
    CheckSketch,
    Repair,
    Construction,
    NormalTo,
    Fit,
    Origin,
    Relation,
    Snap,
    ViewFront,
    ViewBack,
    ViewLeft,
    ViewRight,
    ViewTop,
    ViewBottom,
    ViewIso,
}

impl CadIcon {
    pub const ALL: [CadIcon; 52] = [
        CadIcon::Select,
        CadIcon::Line,
        CadIcon::Centerline,
        CadIcon::Rectangle,
        CadIcon::CenterRectangle,
        CadIcon::ThreePointRect,
        CadIcon::ThreePointCentreRect,
        CadIcon::Parallelogram,
        CadIcon::Polygon,
        CadIcon::Slot,
        CadIcon::Circle,
        CadIcon::Arc,
        CadIcon::Arc3,
        CadIcon::TangentArc,
        CadIcon::Point,
        CadIcon::Dimension,
        CadIcon::Trim,
        CadIcon::Extend,
        CadIcon::Split,
        CadIcon::Move,
        CadIcon::Rotate,
        CadIcon::Scale,
        CadIcon::Mirror,
        CadIcon::LinearPattern,
        CadIcon::CircularPattern,
        CadIcon::Offset,
        CadIcon::Fillet,
        CadIcon::Chamfer,
        CadIcon::Sketch,
        CadIcon::ExitSketch,
        CadIcon::ExtrudedBoss,
        CadIcon::ExtrudedCut,
        CadIcon::Boss,
        CadIcon::Shell,
        CadIcon::Loft,
        CadIcon::Sweep,
        CadIcon::FullyDefine,
        CadIcon::CheckSketch,
        CadIcon::Repair,
        CadIcon::Construction,
        CadIcon::NormalTo,
        CadIcon::Fit,
        CadIcon::Origin,
        CadIcon::Relation,
        CadIcon::Snap,
        CadIcon::ViewFront,
        CadIcon::ViewBack,
        CadIcon::ViewLeft,
        CadIcon::ViewRight,
        CadIcon::ViewTop,
        CadIcon::ViewBottom,
        CadIcon::ViewIso,
    ];
}

/// Which colour a primitive is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    Dark,
    Blue,
    Orange,
    Green,
    Red,
    Grey,
    Paper,
}

/// One drawing primitive, on the 24 x 24 grid (y down). Angles are degrees, counter-clockwise as
/// seen on screen.
#[derive(Clone, Copy, Debug)]
pub enum P {
    Line(f32, f32, f32, f32, Ink),
    Dash(f32, f32, f32, f32, Ink),
    Circle(f32, f32, f32, Ink),
    Disc(f32, f32, f32, Ink),
    Rect(f32, f32, f32, f32, Ink),
    Fill(f32, f32, f32, f32, Ink),
    Arc(f32, f32, f32, f32, f32, Ink),
    /// A closed outline.
    Poly(&'static [(f32, f32)], Ink),
    /// A filled polygon with an outline in the other ink.
    PolyFill(&'static [(f32, f32)], Ink, Ink),
    /// An open polyline.
    Path(&'static [(f32, f32)], Ink),
}

use Ink::*;
use P::*;

/// A grey parallelogram standing for the sketch plane.
const PLANE: &[(f32, f32)] = &[(2.5, 19.0), (7.5, 9.0), (21.5, 9.0), (16.5, 19.0)];
/// The top, left and right faces of an isometric block.
const TOP: &[(f32, f32)] = &[(12.0, 3.0), (20.0, 7.0), (12.0, 11.0), (4.0, 7.0)];
const LEFT: &[(f32, f32)] = &[(4.0, 7.0), (12.0, 11.0), (12.0, 21.0), (4.0, 17.0)];
const RIGHT: &[(f32, f32)] = &[(12.0, 11.0), (20.0, 7.0), (20.0, 17.0), (12.0, 21.0)];

impl CadIcon {
    /// The drawing, as primitives in painting order.
    pub fn prims(self) -> &'static [P] {
        match self {
            CadIcon::Select => &[PolyFill(
                &[
                    (6.0, 3.0),
                    (6.0, 19.0),
                    (10.5, 15.0),
                    (13.5, 21.0),
                    (16.0, 20.0),
                    (13.0, 14.0),
                    (19.0, 14.0),
                ],
                Paper,
                Dark,
            )],
            CadIcon::Line => &[
                Line(4.0, 19.0, 20.0, 5.0, Blue),
                Disc(4.0, 19.0, 2.0, Orange),
                Disc(20.0, 5.0, 2.0, Orange),
            ],
            CadIcon::Centerline => &[
                Line(2.5, 12.0, 8.0, 12.0, Blue),
                Disc(12.0, 12.0, 1.1, Blue),
                Line(16.0, 12.0, 21.5, 12.0, Blue),
                Line(12.0, 5.0, 12.0, 8.0, Grey),
                Line(12.0, 16.0, 12.0, 19.0, Grey),
            ],
            CadIcon::Rectangle => &[
                Rect(4.0, 6.0, 16.0, 12.0, Blue),
                Disc(4.0, 6.0, 1.9, Orange),
                Disc(20.0, 18.0, 1.9, Orange),
            ],
            CadIcon::CenterRectangle => &[
                Rect(4.0, 6.0, 16.0, 12.0, Blue),
                Dash(4.0, 6.0, 20.0, 18.0, Grey),
                Dash(20.0, 6.0, 4.0, 18.0, Grey),
                Disc(12.0, 12.0, 1.9, Orange),
            ],
            CadIcon::ThreePointRect => &[
                Poly(
                    &[(4.0, 13.0), (11.0, 6.0), (18.0, 13.0), (11.0, 20.0)],
                    Blue,
                ),
                Disc(4.0, 13.0, 1.8, Orange),
                Disc(11.0, 6.0, 1.8, Orange),
                Disc(18.0, 13.0, 1.8, Orange),
            ],
            CadIcon::ThreePointCentreRect => &[
                Poly(
                    &[(4.0, 13.0), (11.0, 6.0), (18.0, 13.0), (11.0, 20.0)],
                    Blue,
                ),
                Dash(4.0, 13.0, 18.0, 13.0, Grey),
                Dash(11.0, 6.0, 11.0, 20.0, Grey),
                Disc(11.0, 13.0, 1.8, Orange),
            ],
            CadIcon::Parallelogram => &[
                Poly(&[(3.0, 18.0), (8.0, 6.0), (21.0, 6.0), (16.0, 18.0)], Blue),
                Disc(3.0, 18.0, 1.8, Orange),
                Disc(8.0, 6.0, 1.8, Orange),
                Disc(21.0, 6.0, 1.8, Orange),
            ],
            CadIcon::Polygon => &[
                Poly(
                    &[
                        (20.0, 12.0),
                        (16.0, 5.1),
                        (8.0, 5.1),
                        (4.0, 12.0),
                        (8.0, 18.9),
                        (16.0, 18.9),
                    ],
                    Blue,
                ),
                Disc(12.0, 12.0, 1.9, Orange),
            ],
            CadIcon::Slot => &[
                Line(8.0, 6.0, 16.0, 6.0, Blue),
                Line(8.0, 18.0, 16.0, 18.0, Blue),
                Arc(8.0, 12.0, 6.0, 90.0, 270.0, Blue),
                Arc(16.0, 12.0, 6.0, -90.0, 90.0, Blue),
                Disc(8.0, 12.0, 1.8, Orange),
                Disc(16.0, 12.0, 1.8, Orange),
            ],
            CadIcon::Circle => &[Circle(12.0, 12.0, 8.0, Blue), Disc(12.0, 12.0, 1.9, Orange)],
            CadIcon::Arc => &[
                Arc(12.0, 14.0, 8.0, 30.0, 150.0, Blue),
                Dash(12.0, 14.0, 18.9, 10.0, Grey),
                Dash(12.0, 14.0, 5.1, 10.0, Grey),
                Disc(12.0, 14.0, 1.8, Orange),
                Disc(18.9, 10.0, 1.6, Orange),
                Disc(5.1, 10.0, 1.6, Orange),
            ],
            CadIcon::Arc3 => &[
                Arc(12.0, 16.0, 9.0, 40.0, 140.0, Blue),
                Disc(18.9, 10.2, 1.8, Orange),
                Disc(5.1, 10.2, 1.8, Orange),
                Disc(12.0, 7.0, 1.5, Grey),
            ],
            CadIcon::TangentArc => &[
                Line(2.5, 18.0, 10.0, 18.0, Dark),
                Arc(10.0, 12.0, 6.0, -90.0, 20.0, Blue),
                Disc(15.6, 10.0, 1.8, Orange),
            ],
            CadIcon::Point => &[
                Line(12.0, 3.5, 12.0, 8.5, Dark),
                Line(12.0, 15.5, 12.0, 20.5, Dark),
                Line(3.5, 12.0, 8.5, 12.0, Dark),
                Line(15.5, 12.0, 20.5, 12.0, Dark),
                Disc(12.0, 12.0, 2.3, Orange),
            ],
            CadIcon::Dimension => &[
                Line(5.0, 5.0, 5.0, 18.0, Grey),
                Line(19.0, 5.0, 19.0, 18.0, Grey),
                Line(5.0, 14.0, 19.0, 14.0, Blue),
                PolyFill(&[(5.0, 14.0), (8.5, 12.3), (8.5, 15.7)], Blue, Blue),
                PolyFill(&[(19.0, 14.0), (15.5, 12.3), (15.5, 15.7)], Blue, Blue),
                Line(9.5, 8.0, 14.5, 8.0, Dark),
            ],
            CadIcon::Trim => &[
                Line(3.0, 12.0, 21.0, 12.0, Dark),
                Line(12.0, 3.0, 12.0, 12.0, Dark),
                Dash(12.0, 12.0, 12.0, 21.0, Red),
                Disc(12.0, 12.0, 1.6, Orange),
            ],
            CadIcon::Extend => &[
                Line(3.0, 12.0, 11.0, 12.0, Blue),
                Dash(11.0, 12.0, 18.0, 12.0, Blue),
                Line(19.5, 4.0, 19.5, 20.0, Dark),
            ],
            CadIcon::Split => &[
                Line(3.0, 12.0, 21.0, 12.0, Dark),
                Line(12.0, 6.5, 12.0, 17.5, Orange),
                Disc(12.0, 12.0, 2.0, Orange),
            ],
            CadIcon::Move => &[
                Line(12.0, 3.5, 12.0, 20.5, Dark),
                Line(3.5, 12.0, 20.5, 12.0, Dark),
                PolyFill(&[(12.0, 2.5), (9.0, 6.5), (15.0, 6.5)], Blue, Blue),
                PolyFill(&[(12.0, 21.5), (9.0, 17.5), (15.0, 17.5)], Blue, Blue),
                PolyFill(&[(2.5, 12.0), (6.5, 9.0), (6.5, 15.0)], Blue, Blue),
                PolyFill(&[(21.5, 12.0), (17.5, 9.0), (17.5, 15.0)], Blue, Blue),
            ],
            CadIcon::Rotate => &[
                Arc(12.0, 12.0, 7.5, 30.0, 330.0, Blue),
                PolyFill(&[(19.5, 12.0), (16.0, 10.0), (16.0, 14.5)], Blue, Blue),
                Disc(12.0, 12.0, 1.6, Orange),
            ],
            CadIcon::Scale => &[
                Rect(4.0, 12.0, 8.0, 8.0, Dark),
                Rect(4.0, 4.0, 16.0, 16.0, Blue),
                Line(12.0, 12.0, 19.0, 5.0, Orange),
            ],
            CadIcon::Mirror => &[
                Dash(12.0, 2.5, 12.0, 21.5, Grey),
                Poly(&[(4.0, 6.0), (10.0, 12.0), (4.0, 18.0)], Dark),
                Poly(&[(20.0, 6.0), (14.0, 12.0), (20.0, 18.0)], Blue),
            ],
            CadIcon::LinearPattern => &[
                Rect(2.5, 8.5, 5.0, 7.0, Dark),
                Rect(9.5, 8.5, 5.0, 7.0, Blue),
                Rect(16.5, 8.5, 5.0, 7.0, Blue),
            ],
            CadIcon::CircularPattern => &[
                Circle(12.0, 12.0, 8.0, Grey),
                Disc(12.0, 4.0, 2.4, Dark),
                Disc(20.0, 12.0, 2.4, Blue),
                Disc(12.0, 20.0, 2.4, Blue),
                Disc(4.0, 12.0, 2.4, Blue),
            ],
            CadIcon::Offset => &[
                Rect(2.5, 5.5, 19.0, 13.0, Dark),
                Rect(7.0, 9.0, 10.0, 6.0, Blue),
            ],
            CadIcon::Fillet => &[
                Line(4.0, 3.5, 4.0, 14.0, Dark),
                Arc(9.0, 14.0, 5.0, 180.0, 270.0, Blue),
                Line(9.0, 19.0, 21.0, 19.0, Dark),
            ],
            CadIcon::Chamfer => &[
                Line(4.0, 3.5, 4.0, 13.5, Dark),
                Line(4.0, 13.5, 9.5, 19.0, Blue),
                Line(9.5, 19.0, 21.0, 19.0, Dark),
            ],
            CadIcon::Sketch => &[
                PolyFill(PLANE, Paper, Grey),
                Path(&[(7.0, 16.0), (11.0, 11.5), (17.0, 14.0)], Blue),
                Disc(11.0, 11.5, 1.7, Orange),
                PolyFill(
                    &[(15.0, 7.0), (19.5, 2.5), (22.0, 5.0), (17.5, 9.5)],
                    Orange,
                    Dark,
                ),
            ],
            CadIcon::ExitSketch => &[
                PolyFill(PLANE, Paper, Grey),
                Path(&[(7.0, 15.0), (11.0, 18.5), (19.0, 6.0)], Green),
                Path(&[(7.2, 15.2), (11.0, 18.8), (19.2, 6.2)], Green),
            ],
            CadIcon::ExtrudedBoss => &[
                PolyFill(LEFT, Paper, Dark),
                PolyFill(RIGHT, Paper, Dark),
                PolyFill(TOP, Blue, Dark),
                Line(12.0, 11.0, 12.0, 21.0, Dark),
            ],
            CadIcon::ExtrudedCut => &[
                PolyFill(LEFT, Paper, Dark),
                PolyFill(RIGHT, Paper, Dark),
                PolyFill(TOP, Paper, Dark),
                PolyFill(
                    &[(12.0, 5.0), (16.0, 7.0), (12.0, 9.0), (8.0, 7.0)],
                    Red,
                    Dark,
                ),
            ],
            CadIcon::Shell => &[
                PolyFill(LEFT, Paper, Dark),
                PolyFill(RIGHT, Paper, Dark),
                PolyFill(TOP, Paper, Dark),
                PolyFill(
                    &[(12.0, 5.5), (17.0, 8.0), (12.0, 10.5), (7.0, 8.0)],
                    Blue,
                    Dark,
                ),
                Line(12.0, 10.5, 12.0, 15.0, Dark),
            ],
            CadIcon::Loft => &[
                Rect(3.5, 16.0, 8.0, 4.0, Dark),
                PolyFill(
                    &[(4.0, 8.0), (9.0, 5.0), (14.0, 8.0), (9.0, 11.0)],
                    Blue,
                    Dark,
                ),
                Line(4.0, 8.0, 3.5, 16.0, Dark),
                Line(14.0, 8.0, 11.5, 16.0, Dark),
            ],
            CadIcon::Sweep => &[
                PolyFill(
                    &[(3.0, 17.0), (8.0, 15.0), (8.0, 19.5), (3.0, 21.0)],
                    Blue,
                    Dark,
                ),
                Path(&[(8.0, 17.0), (12.0, 17.0), (15.0, 8.0), (21.0, 5.0)], Dark),
            ],
            CadIcon::Boss => &[
                PolyFill(LEFT, Paper, Dark),
                PolyFill(RIGHT, Paper, Dark),
                PolyFill(TOP, Paper, Dark),
                PolyFill(
                    &[(12.0, 1.5), (16.0, 3.5), (12.0, 5.5), (8.0, 3.5)],
                    Blue,
                    Dark,
                ),
                Line(8.0, 3.5, 8.0, 7.0, Dark),
                Line(16.0, 3.5, 16.0, 7.0, Dark),
            ],
            CadIcon::FullyDefine => &[
                Rect(3.5, 5.5, 13.0, 10.0, Dark),
                Path(&[(11.0, 15.0), (14.5, 19.0), (21.0, 9.0)], Green),
                Path(&[(11.2, 15.2), (14.5, 19.3), (21.2, 9.2)], Green),
            ],
            CadIcon::CheckSketch => &[
                Circle(10.0, 10.0, 6.5, Dark),
                Line(14.8, 14.8, 21.0, 21.0, Dark),
                Path(&[(7.0, 10.0), (9.5, 12.5), (13.5, 7.5)], Green),
            ],
            CadIcon::Repair => &[
                Line(4.5, 19.5, 14.5, 9.5, Dark),
                Circle(16.5, 7.5, 4.0, Blue),
                Disc(16.5, 7.5, 1.4, Orange),
            ],
            CadIcon::Construction => &[
                Dash(3.0, 19.0, 21.0, 5.0, Blue),
                Disc(3.0, 19.0, 1.7, Grey),
                Disc(21.0, 5.0, 1.7, Grey),
            ],
            CadIcon::NormalTo => &[
                PolyFill(PLANE, Paper, Grey),
                Line(12.0, 2.5, 12.0, 13.0, Blue),
                PolyFill(&[(12.0, 15.0), (9.0, 10.5), (15.0, 10.5)], Blue, Blue),
            ],
            CadIcon::Fit => &[
                Path(&[(3.0, 8.0), (3.0, 3.0), (8.0, 3.0)], Dark),
                Path(&[(16.0, 3.0), (21.0, 3.0), (21.0, 8.0)], Dark),
                Path(&[(21.0, 16.0), (21.0, 21.0), (16.0, 21.0)], Dark),
                Path(&[(8.0, 21.0), (3.0, 21.0), (3.0, 16.0)], Dark),
                Rect(8.0, 8.0, 8.0, 8.0, Blue),
            ],
            CadIcon::Origin => &[
                Line(6.0, 18.0, 6.0, 4.0, Green),
                Line(6.0, 18.0, 20.0, 18.0, Red),
                Line(6.0, 18.0, 13.0, 11.5, Blue),
                Disc(6.0, 18.0, 2.0, Orange),
            ],
            CadIcon::Relation => &[
                Line(3.5, 8.0, 13.0, 8.0, Dark),
                Line(11.0, 16.0, 20.5, 16.0, Dark),
                Line(8.0, 11.5, 16.0, 12.5, Blue),
                Disc(8.0, 11.5, 1.6, Orange),
                Disc(16.0, 12.5, 1.6, Orange),
            ],
            // The cube, with the face that is looked at in blue; a face on the far side is drawn as a
            // dashed outline (in an isometric drawing it lies right behind its opposite face).
            CadIcon::ViewFront => &[
                PolyFill(LEFT, Blue, Dark),
                PolyFill(RIGHT, Paper, Dark),
                PolyFill(TOP, Paper, Dark),
            ],
            CadIcon::ViewRight => &[
                PolyFill(LEFT, Paper, Dark),
                PolyFill(RIGHT, Blue, Dark),
                PolyFill(TOP, Paper, Dark),
            ],
            CadIcon::ViewTop => &[
                PolyFill(LEFT, Paper, Dark),
                PolyFill(RIGHT, Paper, Dark),
                PolyFill(TOP, Blue, Dark),
            ],
            CadIcon::ViewBack => &[
                PolyFill(LEFT, Paper, Dark),
                PolyFill(RIGHT, Paper, Dark),
                PolyFill(TOP, Paper, Dark),
                Dash(4.0, 7.0, 12.0, 11.0, Blue),
                Dash(12.0, 11.0, 12.0, 21.0, Blue),
                Dash(12.0, 21.0, 4.0, 17.0, Blue),
                Dash(4.0, 17.0, 4.0, 7.0, Blue),
            ],
            CadIcon::ViewLeft => &[
                PolyFill(LEFT, Paper, Dark),
                PolyFill(RIGHT, Paper, Dark),
                PolyFill(TOP, Paper, Dark),
                Dash(12.0, 11.0, 20.0, 7.0, Blue),
                Dash(20.0, 7.0, 20.0, 17.0, Blue),
                Dash(20.0, 17.0, 12.0, 21.0, Blue),
                Dash(12.0, 21.0, 12.0, 11.0, Blue),
            ],
            CadIcon::ViewBottom => &[
                PolyFill(LEFT, Paper, Dark),
                PolyFill(RIGHT, Paper, Dark),
                PolyFill(TOP, Paper, Dark),
                Dash(12.0, 3.0, 20.0, 7.0, Blue),
                Dash(20.0, 7.0, 12.0, 11.0, Blue),
                Dash(12.0, 11.0, 4.0, 7.0, Blue),
                Dash(4.0, 7.0, 12.0, 3.0, Blue),
            ],
            CadIcon::ViewIso => &[
                PolyFill(LEFT, Paper, Blue),
                PolyFill(RIGHT, Paper, Blue),
                PolyFill(TOP, Paper, Blue),
                Line(12.0, 11.0, 12.0, 21.0, Blue),
            ],
            CadIcon::Snap => &[
                Line(3.0, 12.0, 21.0, 12.0, Grey),
                Line(12.0, 3.0, 12.0, 21.0, Grey),
                Circle(12.0, 12.0, 4.5, Blue),
                Disc(12.0, 12.0, 1.8, Orange),
            ],
        }
    }
}

/// The colour of an ink in `colors`.
pub fn ink(colors: &Colors, i: Ink) -> Color32 {
    match i {
        Ink::Dark => colors.ink,
        Ink::Blue => colors.ink_blue,
        Ink::Orange => colors.ink_orange,
        Ink::Green => colors.ink_green,
        Ink::Red => colors.ink_red,
        Ink::Grey => colors.ink_grey,
        Ink::Paper => colors.paper,
    }
}

/// Draws `icon` filling `rect` (square). A disabled icon is faded.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, colors: &Colors, enabled: bool) {
    match icon {
        Icon::Glyph(c) => {
            // Until the icon font is installed (its first frame) there is nothing to draw with.
            if !painter
                .ctx()
                .fonts(|f| f.families().contains(&icon_family()))
            {
                return;
            }
            let col = if enabled {
                colors.ink
            } else {
                colors.ink.gamma_multiply(0.4)
            };
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                c.to_string(),
                FontId::new(rect.height() * 0.92, icon_family()),
                col,
            );
        }
        Icon::Cad(i) => paint_cad(painter, rect, i, colors, enabled),
    }
}

fn paint_cad(painter: &Painter, rect: Rect, icon: CadIcon, colors: &Colors, enabled: bool) {
    let s = rect.width().min(rect.height()) / 24.0;
    let origin = rect.center() - egui::vec2(12.0, 12.0) * s;
    let at = |x: f32, y: f32| origin + egui::vec2(x, y) * s;
    let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.38) };
    let width = (1.5 * s).clamp(1.1, 2.6);
    let stroke = |i: Ink| Stroke::new(width, fade(ink(colors, i)));
    for p in icon.prims() {
        match *p {
            P::Line(x0, y0, x1, y1, i) => {
                painter.line_segment([at(x0, y0), at(x1, y1)], stroke(i));
            }
            P::Dash(x0, y0, x1, y1, i) => {
                painter.extend(Shape::dashed_line(
                    &[at(x0, y0), at(x1, y1)],
                    stroke(i),
                    3.0 * s,
                    2.0 * s,
                ));
            }
            P::Circle(x, y, r, i) => {
                painter.circle_stroke(at(x, y), r * s, stroke(i));
            }
            P::Disc(x, y, r, i) => {
                painter.circle_filled(at(x, y), r * s, fade(ink(colors, i)));
            }
            P::Rect(x, y, w, h, i) => {
                painter.rect_stroke(
                    Rect::from_min_size(at(x, y), egui::vec2(w, h) * s),
                    0.0,
                    stroke(i),
                    egui::StrokeKind::Middle,
                );
            }
            P::Fill(x, y, w, h, i) => {
                painter.rect_filled(
                    Rect::from_min_size(at(x, y), egui::vec2(w, h) * s),
                    0.0,
                    fade(ink(colors, i)),
                );
            }
            P::Arc(cx, cy, r, a0, a1, i) => {
                let n = 28;
                let pts: Vec<Pos2> = (0..=n)
                    .map(|k| {
                        let a = (a0 + (a1 - a0) * k as f32 / n as f32).to_radians();
                        at(cx + r * a.cos(), cy - r * a.sin())
                    })
                    .collect();
                painter.add(Shape::line(pts, stroke(i)));
            }
            P::Poly(pts, i) => {
                let v: Vec<Pos2> = pts.iter().map(|&(x, y)| at(x, y)).collect();
                painter.add(Shape::closed_line(v, stroke(i)));
            }
            P::PolyFill(pts, fill, line) => {
                let v: Vec<Pos2> = pts.iter().map(|&(x, y)| at(x, y)).collect();
                painter.add(Shape::convex_polygon(
                    v,
                    fade(ink(colors, fill)),
                    stroke(line),
                ));
            }
            P::Path(pts, i) => {
                let v: Vec<Pos2> = pts.iter().map(|&(x, y)| at(x, y)).collect();
                painter.add(Shape::line(v, stroke(i)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coords(p: &P) -> Vec<(f32, f32)> {
        match *p {
            P::Line(a, b, c, d, _) | P::Dash(a, b, c, d, _) => vec![(a, b), (c, d)],
            P::Circle(x, y, r, _) | P::Disc(x, y, r, _) => {
                vec![(x - r, y - r), (x + r, y + r)]
            }
            P::Rect(x, y, w, h, _) | P::Fill(x, y, w, h, _) => vec![(x, y), (x + w, y + h)],
            P::Arc(x, y, r, a0, a1, _) => {
                let n = 28;
                (0..=n)
                    .map(|k| {
                        let a = (a0 + (a1 - a0) * k as f32 / n as f32).to_radians();
                        (x + r * a.cos(), y - r * a.sin())
                    })
                    .collect()
            }
            P::Poly(p, _) | P::PolyFill(p, _, _) | P::Path(p, _) => p.to_vec(),
        }
    }

    #[test]
    fn every_cad_icon_has_a_drawing_that_stays_on_the_grid() {
        for icon in CadIcon::ALL {
            let prims = icon.prims();
            assert!(!prims.is_empty(), "{icon:?} draws nothing");
            for p in prims {
                for (x, y) in coords(p) {
                    assert!(
                        (0.4..=23.6).contains(&x) && (0.4..=23.6).contains(&y),
                        "{icon:?}: {p:?} reaches ({x}, {y}) outside the 24 x 24 grid"
                    );
                }
            }
        }
    }

    #[test]
    fn the_list_of_icons_is_complete_and_has_no_repeats() {
        let mut seen = std::collections::HashSet::new();
        for i in CadIcon::ALL {
            assert!(seen.insert(i), "{i:?} listed twice");
        }
        // Every drawing differs from every other (no icon is a copy of another).
        let key = |i: CadIcon| format!("{:?}", i.prims());
        let mut drawings = std::collections::HashSet::new();
        for i in CadIcon::ALL {
            assert!(drawings.insert(key(i)), "{i:?} looks like another icon");
        }
    }

    #[test]
    fn painting_draws_shapes_in_both_themes_and_when_disabled() {
        use crate::ui::theme::ThemeMode;
        let ctx = egui::Context::default();
        crate::ui::theme::install(&ctx, ThemeMode::Light);
        // Fonts take effect on the next pass.
        let _ = ctx.run_ui(Default::default(), |_| {});
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let colors = Colors::of(mode);
            for enabled in [true, false] {
                let out = ctx.run_ui(Default::default(), |ui| {
                    let r = Rect::from_min_size(Pos2::new(10.0, 10.0), egui::vec2(32.0, 32.0));
                    for icon in CadIcon::ALL {
                        paint(ui.painter(), r, Icon::Cad(icon), &colors, enabled);
                    }
                    paint(
                        ui.painter(),
                        r,
                        Icon::Glyph(crate::ui::icon_font::GEAR),
                        &colors,
                        enabled,
                    );
                });
                assert!(out.shapes.len() >= CadIcon::ALL.len(), "{mode:?} {enabled}");
            }
        }
    }
}
