//! In-house vector icons (M2 W4a, D-114; UI-010): each icon is a constant list of drawing operations on a
//! 16 × 16 design grid, painted with egui's painter at the window's current scale in one colour, so it is sharp
//! at 100/125/150/200 % and any other scale, in light and dark schemes. No image files, no icon font, no
//! dependency. Original drawings (docs/17 §5). Later waves add icons to the same table.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke};
use hikasa_session::sessions::SessionType;

/// The stroke width in design units.
pub const STROKE: f32 = 1.5;

/// One drawing operation; coordinates in `0.0..=16.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Op {
    /// A line.
    Line([f32; 2], [f32; 2]),
    /// An open polyline.
    Polyline(&'static [[f32; 2]]),
    /// A closed outline.
    Polygon(&'static [[f32; 2]]),
    /// A filled convex polygon.
    FilledPolygon(&'static [[f32; 2]]),
    /// A circle outline (centre, radius).
    Circle([f32; 2], f32),
    /// A filled circle.
    FilledCircle([f32; 2], f32),
    /// A rounded rectangle outline (min, max).
    Rect([f32; 2], [f32; 2]),
    /// A filled rounded rectangle.
    FilledRect([f32; 2], [f32; 2]),
}

/// Every icon (§1.4.5 inventory; the views of later waves use the rest).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IconId {
    /// The `FolderCompare` icon.
    FolderCompare,
    /// The `FolderSync` icon.
    FolderSync,
    /// The `TextCompare` icon.
    TextCompare,
    /// The `TextEdit` icon.
    TextEdit,
    /// The `TableCompare` icon.
    TableCompare,
    /// The `HexCompare` icon.
    HexCompare,
    /// The `PictureCompare` icon.
    PictureCompare,
    /// The `Home` icon.
    Home,
    /// The `NewTab` icon.
    NewTab,
    /// The `NewWindow` icon.
    NewWindow,
    /// The `Close` icon.
    Close,
    /// The `Open` icon.
    Open,
    /// The `Save` icon.
    Save,
    /// The `SaveAs` icon.
    SaveAs,
    /// The `Settings` icon.
    Settings,
    /// The `Lock` icon.
    Lock,
    /// The `Unlock` icon.
    Unlock,
    /// The `Favorite` icon.
    Favorite,
    /// The `Folder` icon.
    Folder,
    /// The `Session` icon.
    Session,
    /// The `AutoSaved` icon.
    AutoSaved,
    /// The `Defaults` icon.
    Defaults,
    /// The `Workspace` icon.
    Workspace,
    /// The `Stop` icon.
    Stop,
    /// The `Reload` icon.
    Reload,
    /// The `Expand` icon.
    Expand,
    /// The `Collapse` icon.
    Collapse,
    /// The `ExpandAll` icon.
    ExpandAll,
    /// The `CollapseAll` icon.
    CollapseAll,
    /// The `OpenPair` icon.
    OpenPair,
    /// The `NextDiff` icon.
    NextDiff,
    /// The `PrevDiff` icon.
    PrevDiff,
    /// The `NextSection` icon.
    NextSection,
    /// The `PrevSection` icon.
    PrevSection,
    /// The `Swap` icon.
    Swap,
    /// The `Rules` icon.
    Rules,
    /// The `Search` icon.
    Search,
    /// The `FilterAll` icon.
    FilterAll,
    /// The `FilterDiff` icon.
    FilterDiff,
    /// The `FilterSame` icon.
    FilterSame,
    /// The `FilterOrphans` icon.
    FilterOrphans,
    /// The `FilterLeftOrphan` icon.
    FilterLeftOrphan,
    /// The `FilterRightOrphan` icon.
    FilterRightOrphan,
    /// The `FilterLeftNewer` icon.
    FilterLeftNewer,
    /// The `FilterRightNewer` icon.
    FilterRightNewer,
    /// The `FilterNone` icon.
    FilterNone,
    /// The `Context` icon.
    Context,
    /// The `Unimportant` icon.
    Unimportant,
    /// The `Log` icon.
    Log,
    /// The `Thumbnail` icon.
    Thumbnail,
    /// The `FontLarger` icon.
    FontLarger,
    /// The `FontSmaller` icon.
    FontSmaller,
    /// The `FontReset` icon.
    FontReset,
    /// The `Info` icon.
    Info,
    /// The `Warning` icon.
    Warning,
    /// The `Error` icon.
    Error,
    /// The `Notice` icon.
    Notice,
    /// The `Drop` icon.
    Drop,
    /// The `Options` icon.
    Options,
    /// The `Export` icon.
    Export,
    /// The `Import` icon.
    Import,
    /// The `Restore` icon.
    Restore,
    /// The `Help` icon.
    Help,
    /// The `Zipper` icon: an archive row of the folder tree (D-153).
    Zipper,
}

impl IconId {
    /// Every icon, in inventory order.
    #[cfg_attr(not(test), allow(dead_code))]
    pub const ALL: &'static [IconId] = &[
        IconId::FolderCompare,
        IconId::FolderSync,
        IconId::TextCompare,
        IconId::TextEdit,
        IconId::TableCompare,
        IconId::HexCompare,
        IconId::PictureCompare,
        IconId::Home,
        IconId::NewTab,
        IconId::NewWindow,
        IconId::Close,
        IconId::Open,
        IconId::Save,
        IconId::SaveAs,
        IconId::Settings,
        IconId::Lock,
        IconId::Unlock,
        IconId::Favorite,
        IconId::Folder,
        IconId::Session,
        IconId::AutoSaved,
        IconId::Defaults,
        IconId::Workspace,
        IconId::Stop,
        IconId::Reload,
        IconId::Expand,
        IconId::Collapse,
        IconId::ExpandAll,
        IconId::CollapseAll,
        IconId::OpenPair,
        IconId::NextDiff,
        IconId::PrevDiff,
        IconId::NextSection,
        IconId::PrevSection,
        IconId::Swap,
        IconId::Rules,
        IconId::Search,
        IconId::FilterAll,
        IconId::FilterDiff,
        IconId::FilterSame,
        IconId::FilterOrphans,
        IconId::FilterLeftOrphan,
        IconId::FilterRightOrphan,
        IconId::FilterLeftNewer,
        IconId::FilterRightNewer,
        IconId::FilterNone,
        IconId::Context,
        IconId::Unimportant,
        IconId::Log,
        IconId::Thumbnail,
        IconId::FontLarger,
        IconId::FontSmaller,
        IconId::FontReset,
        IconId::Info,
        IconId::Warning,
        IconId::Error,
        IconId::Notice,
        IconId::Drop,
        IconId::Options,
        IconId::Export,
        IconId::Import,
        IconId::Restore,
        IconId::Help,
        IconId::Zipper,
    ];

    /// The icon's operations.
    pub fn ops(self) -> &'static [Op] {
        match self {
            IconId::FolderCompare => &[
                Op::Polygon(&[
                    [1.0, 4.0],
                    [3.5, 4.0],
                    [4.5, 5.0],
                    [7.3, 5.0],
                    [7.3, 12.0],
                    [1.0, 12.0],
                ]),
                Op::Polygon(&[
                    [8.7, 4.0],
                    [11.2, 4.0],
                    [12.2, 5.0],
                    [15.0, 5.0],
                    [15.0, 12.0],
                    [8.7, 12.0],
                ]),
            ],
            IconId::FolderSync => &[
                Op::Polygon(&[
                    [1.5, 3.0],
                    [5.5, 3.0],
                    [7.0, 4.5],
                    [14.5, 4.5],
                    [14.5, 13.0],
                    [1.5, 13.0],
                ]),
                Op::Line([4.5, 9.0], [11.0, 9.0]),
                Op::Polyline(&[[9.0, 7.0], [11.0, 9.0], [9.0, 11.0]]),
            ],
            IconId::TextCompare => &[
                Op::Rect([1.0, 2.0], [7.0, 14.0]),
                Op::Rect([9.0, 2.0], [15.0, 14.0]),
                Op::Line([2.5, 5.0], [5.5, 5.0]),
                Op::Line([2.5, 8.0], [5.5, 8.0]),
                Op::Line([10.5, 5.0], [13.5, 5.0]),
                Op::Line([10.5, 8.0], [13.5, 8.0]),
                Op::Line([10.5, 11.0], [12.5, 11.0]),
            ],
            IconId::TextEdit => &[
                Op::Rect([2.0, 1.5], [10.0, 14.5]),
                Op::Line([4.0, 5.0], [8.0, 5.0]),
                Op::Line([4.0, 8.0], [7.0, 8.0]),
                Op::FilledPolygon(&[
                    [9.0, 13.0],
                    [14.0, 8.0],
                    [15.5, 9.5],
                    [10.5, 14.5],
                    [9.0, 14.5],
                ]),
            ],
            IconId::TableCompare => &[
                Op::Rect([1.5, 2.5], [14.5, 13.5]),
                Op::Line([1.5, 6.0], [14.5, 6.0]),
                Op::Line([1.5, 9.5], [14.5, 9.5]),
                Op::Line([6.0, 2.5], [6.0, 13.5]),
                Op::Line([10.5, 2.5], [10.5, 13.5]),
            ],
            IconId::HexCompare => &[
                Op::Polygon(&[
                    [8.0, 1.5],
                    [14.0, 5.0],
                    [14.0, 11.0],
                    [8.0, 14.5],
                    [2.0, 11.0],
                    [2.0, 5.0],
                ]),
                Op::Line([5.5, 8.0], [10.5, 8.0]),
            ],
            IconId::PictureCompare => &[
                Op::Rect([1.5, 3.0], [14.5, 13.0]),
                Op::Polyline(&[
                    [1.5, 12.0],
                    [6.0, 7.0],
                    [9.0, 10.0],
                    [11.0, 8.0],
                    [14.5, 11.5],
                ]),
                Op::FilledCircle([11.0, 5.5], 1.0),
            ],
            IconId::Home => &[
                Op::Polyline(&[[2.0, 8.0], [8.0, 2.5], [14.0, 8.0]]),
                Op::Polyline(&[[4.0, 7.0], [4.0, 14.0], [12.0, 14.0], [12.0, 7.0]]),
                Op::Rect([7.0, 10.0], [9.0, 14.0]),
            ],
            IconId::NewTab => &[
                Op::Polyline(&[[1.0, 13.0], [1.0, 4.0], [8.5, 4.0], [8.5, 13.0]]),
                Op::Line([0.5, 13.0], [15.5, 13.0]),
                Op::Line([12.5, 4.5], [12.5, 10.5]),
                Op::Line([9.5, 7.5], [15.5, 7.5]),
            ],
            IconId::NewWindow => &[
                Op::Rect([1.0, 3.0], [10.5, 12.0]),
                Op::Line([1.0, 5.5], [10.5, 5.5]),
                Op::Line([13.5, 8.0], [13.5, 14.0]),
                Op::Line([10.5, 11.0], [16.0, 11.0]),
            ],
            IconId::Close => &[
                Op::Line([3.5, 3.5], [12.5, 12.5]),
                Op::Line([12.5, 3.5], [3.5, 12.5]),
            ],
            IconId::Open => &[
                Op::Polyline(&[
                    [1.5, 13.0],
                    [1.5, 3.5],
                    [5.0, 3.5],
                    [6.5, 5.0],
                    [12.0, 5.0],
                    [12.0, 7.0],
                ]),
                Op::Polygon(&[[1.5, 13.0], [4.0, 7.0], [15.0, 7.0], [12.5, 13.0]]),
            ],
            IconId::Save => &[
                Op::Polygon(&[
                    [2.0, 2.0],
                    [12.0, 2.0],
                    [14.0, 4.0],
                    [14.0, 14.0],
                    [2.0, 14.0],
                ]),
                Op::Rect([5.0, 2.0], [11.0, 6.0]),
                Op::Rect([4.5, 9.0], [11.5, 14.0]),
            ],
            IconId::SaveAs => &[
                Op::Polygon(&[
                    [1.0, 1.0],
                    [9.0, 1.0],
                    [11.0, 3.0],
                    [11.0, 11.0],
                    [1.0, 11.0],
                ]),
                Op::Rect([3.5, 1.0], [8.0, 4.0]),
                Op::Line([9.5, 14.5], [15.0, 9.0]),
                Op::Line([8.5, 15.5], [9.5, 14.5]),
            ],
            IconId::Settings => &[
                Op::Circle([8.0, 8.0], 3.0),
                Op::Line([8.0, 1.0], [8.0, 3.5]),
                Op::Line([8.0, 12.5], [8.0, 15.0]),
                Op::Line([1.0, 8.0], [3.5, 8.0]),
                Op::Line([12.5, 8.0], [15.0, 8.0]),
                Op::Line([3.0, 3.0], [4.8, 4.8]),
                Op::Line([11.2, 11.2], [13.0, 13.0]),
                Op::Line([13.0, 3.0], [11.2, 4.8]),
                Op::Line([4.8, 11.2], [3.0, 13.0]),
            ],
            IconId::Lock => &[
                Op::Rect([3.0, 7.0], [13.0, 14.5]),
                Op::Polyline(&[
                    [5.0, 7.0],
                    [5.0, 4.5],
                    [6.0, 2.5],
                    [8.0, 1.8],
                    [10.0, 2.5],
                    [11.0, 4.5],
                    [11.0, 7.0],
                ]),
                Op::Line([8.0, 9.5], [8.0, 12.0]),
            ],
            IconId::Unlock => &[
                Op::Rect([3.0, 7.0], [13.0, 14.5]),
                Op::Polyline(&[
                    [5.0, 7.0],
                    [5.0, 4.5],
                    [6.0, 2.5],
                    [8.0, 1.8],
                    [10.0, 2.5],
                    [11.0, 4.0],
                ]),
                Op::Line([8.0, 9.5], [8.0, 12.0]),
            ],
            IconId::Favorite => &[Op::Polygon(&[
                [8.0, 1.9],
                [9.65, 6.13],
                [14.18, 6.39],
                [10.66, 9.27],
                [11.82, 13.66],
                [8.0, 11.2],
                [4.18, 13.66],
                [5.34, 9.27],
                [1.82, 6.39],
                [6.35, 6.13],
            ])],
            IconId::Folder => &[Op::Polygon(&[
                [1.5, 3.0],
                [5.5, 3.0],
                [7.0, 4.5],
                [14.5, 4.5],
                [14.5, 13.0],
                [1.5, 13.0],
            ])],
            IconId::Session => &[
                Op::Rect([3.0, 1.5], [13.0, 14.5]),
                Op::Line([5.5, 5.0], [10.5, 5.0]),
                Op::Line([5.5, 8.0], [10.5, 8.0]),
                Op::Line([5.5, 11.0], [8.5, 11.0]),
            ],
            IconId::AutoSaved => &[
                Op::Circle([8.0, 8.0], 6.0),
                Op::Polyline(&[[8.0, 4.0], [8.0, 8.0], [11.0, 10.0]]),
            ],
            IconId::Defaults => &[
                Op::Rect([2.0, 2.0], [14.0, 14.0]),
                Op::Polyline(&[[4.5, 8.0], [7.0, 10.5], [11.5, 5.5]]),
            ],
            IconId::Workspace => &[
                Op::Rect([1.0, 4.0], [11.0, 13.5]),
                Op::Line([1.0, 6.5], [11.0, 6.5]),
                Op::Polyline(&[
                    [5.0, 4.0],
                    [5.0, 1.5],
                    [15.0, 1.5],
                    [15.0, 10.0],
                    [11.0, 10.0],
                ]),
            ],
            IconId::Stop => &[Op::FilledRect([4.0, 4.0], [12.0, 12.0])],
            IconId::Reload => &[
                Op::Polyline(&[
                    [10.75, 3.74],
                    [12.51, 5.35],
                    [13.42, 7.54],
                    [13.31, 9.92],
                    [12.21, 12.04],
                    [10.32, 13.48],
                    [8.0, 14.0],
                    [5.68, 13.48],
                    [3.79, 12.04],
                    [2.69, 9.92],
                    [2.58, 7.54],
                    [3.49, 5.35],
                    [5.25, 3.74],
                ]),
                Op::FilledPolygon(&[[3.05, 3.15], [6.85, 1.34], [5.85, 5.34]]),
            ],
            IconId::Expand => &[Op::FilledPolygon(&[[6.0, 3.5], [11.0, 8.0], [6.0, 12.5]])],
            IconId::Collapse => &[Op::FilledPolygon(&[[3.5, 6.0], [12.5, 6.0], [8.0, 11.0]])],
            IconId::ExpandAll => &[
                Op::Rect([2.0, 2.0], [14.0, 14.0]),
                Op::Line([5.0, 8.0], [11.0, 8.0]),
                Op::Line([8.0, 5.0], [8.0, 11.0]),
            ],
            IconId::CollapseAll => &[
                Op::Rect([2.0, 2.0], [14.0, 14.0]),
                Op::Line([5.0, 8.0], [11.0, 8.0]),
            ],
            IconId::OpenPair => &[
                Op::Rect([1.5, 3.0], [6.5, 13.0]),
                Op::Rect([9.5, 3.0], [14.5, 13.0]),
                Op::Line([4.0, 8.0], [12.0, 8.0]),
                Op::Polyline(&[[10.0, 6.0], [12.0, 8.0], [10.0, 10.0]]),
            ],
            IconId::NextDiff => &[
                Op::Line([8.0, 2.0], [8.0, 13.0]),
                Op::Polyline(&[[3.5, 8.5], [8.0, 13.0], [12.5, 8.5]]),
            ],
            IconId::PrevDiff => &[
                Op::Line([8.0, 14.0], [8.0, 3.0]),
                Op::Polyline(&[[3.5, 7.5], [8.0, 3.0], [12.5, 7.5]]),
            ],
            IconId::NextSection => &[
                Op::Line([8.0, 1.5], [8.0, 11.0]),
                Op::Polyline(&[[4.0, 7.0], [8.0, 11.0], [12.0, 7.0]]),
                Op::Line([3.0, 14.0], [13.0, 14.0]),
            ],
            IconId::PrevSection => &[
                Op::Line([8.0, 14.5], [8.0, 5.0]),
                Op::Polyline(&[[4.0, 9.0], [8.0, 5.0], [12.0, 9.0]]),
                Op::Line([3.0, 2.0], [13.0, 2.0]),
            ],
            IconId::Swap => &[
                Op::Line([2.0, 5.0], [13.0, 5.0]),
                Op::Polyline(&[[10.0, 2.5], [13.0, 5.0], [10.0, 7.5]]),
                Op::Line([14.0, 11.0], [3.0, 11.0]),
                Op::Polyline(&[[6.0, 8.5], [3.0, 11.0], [6.0, 13.5]]),
            ],
            IconId::Rules => &[
                Op::Line([6.0, 4.0], [14.0, 4.0]),
                Op::Line([6.0, 8.0], [14.0, 8.0]),
                Op::Line([6.0, 12.0], [14.0, 12.0]),
                Op::FilledCircle([3.0, 4.0], 1.0),
                Op::FilledCircle([3.0, 8.0], 1.0),
                Op::FilledCircle([3.0, 12.0], 1.0),
            ],
            IconId::Search => &[
                Op::Circle([6.5, 6.5], 4.5),
                Op::Line([10.0, 10.0], [14.5, 14.5]),
            ],
            IconId::FilterAll => &[
                Op::FilledRect([2.0, 3.0], [14.0, 5.0]),
                Op::FilledRect([2.0, 7.0], [14.0, 9.0]),
                Op::FilledRect([2.0, 11.0], [14.0, 13.0]),
            ],
            IconId::FilterDiff => &[
                Op::Line([3.0, 6.0], [13.0, 6.0]),
                Op::Line([3.0, 10.0], [13.0, 10.0]),
                Op::Line([10.5, 2.5], [5.5, 13.5]),
            ],
            IconId::FilterSame => &[
                Op::Line([3.0, 6.0], [13.0, 6.0]),
                Op::Line([3.0, 10.0], [13.0, 10.0]),
            ],
            IconId::FilterOrphans => &[
                Op::Rect([2.0, 3.0], [14.0, 13.0]),
                Op::FilledRect([2.0, 3.0], [7.0, 8.0]),
                Op::FilledRect([9.0, 8.0], [14.0, 13.0]),
            ],
            IconId::FilterLeftOrphan => &[
                Op::Rect([2.0, 3.0], [14.0, 13.0]),
                Op::FilledRect([2.0, 3.0], [8.0, 13.0]),
            ],
            IconId::FilterRightOrphan => &[
                Op::Rect([2.0, 3.0], [14.0, 13.0]),
                Op::FilledRect([8.0, 3.0], [14.0, 13.0]),
            ],
            IconId::FilterLeftNewer => {
                &[Op::FilledPolygon(&[[11.0, 3.0], [11.0, 13.0], [4.0, 8.0]])]
            }
            IconId::FilterRightNewer => {
                &[Op::FilledPolygon(&[[5.0, 3.0], [5.0, 13.0], [12.0, 8.0]])]
            }
            IconId::FilterNone => &[
                Op::Circle([8.0, 8.0], 6.0),
                Op::Line([3.8, 12.2], [12.2, 3.8]),
            ],
            IconId::Context => &[
                Op::Line([2.0, 3.0], [14.0, 3.0]),
                Op::Line([2.0, 13.0], [14.0, 13.0]),
                Op::FilledRect([2.0, 6.5], [14.0, 9.5]),
            ],
            IconId::Unimportant => &[
                Op::Polyline(&[[3.0, 6.0], [5.0, 5.0], [8.0, 6.5], [11.0, 5.0], [13.0, 6.0]]),
                Op::Polyline(&[
                    [3.0, 10.5],
                    [5.0, 9.5],
                    [8.0, 11.0],
                    [11.0, 9.5],
                    [13.0, 10.5],
                ]),
            ],
            IconId::Log => &[
                Op::Rect([2.0, 2.0], [14.0, 14.0]),
                Op::Line([4.5, 5.5], [11.5, 5.5]),
                Op::Line([4.5, 8.0], [11.5, 8.0]),
                Op::Line([4.5, 10.5], [9.0, 10.5]),
            ],
            IconId::Thumbnail => &[
                Op::Rect([5.0, 1.5], [11.0, 14.5]),
                Op::FilledRect([5.0, 5.0], [11.0, 8.0]),
            ],
            IconId::FontLarger => &[
                Op::Polyline(&[[1.5, 14.0], [5.5, 3.0], [9.5, 14.0]]),
                Op::Line([3.0, 10.0], [8.0, 10.0]),
                Op::Line([13.0, 4.0], [13.0, 10.0]),
                Op::Line([10.0, 7.0], [16.0, 7.0]),
            ],
            IconId::FontSmaller => &[
                Op::Polyline(&[[1.5, 14.0], [5.5, 3.0], [9.5, 14.0]]),
                Op::Line([3.0, 10.0], [8.0, 10.0]),
                Op::Line([10.0, 7.0], [16.0, 7.0]),
            ],
            IconId::FontReset => &[
                Op::Polyline(&[[1.5, 14.0], [5.5, 3.0], [9.5, 14.0]]),
                Op::Line([3.0, 10.0], [8.0, 10.0]),
                Op::Line([10.5, 6.0], [15.5, 6.0]),
                Op::Line([10.5, 9.0], [15.5, 9.0]),
            ],
            IconId::Info => &[
                Op::Circle([8.0, 8.0], 6.5),
                Op::Line([8.0, 7.0], [8.0, 11.5]),
                Op::FilledCircle([8.0, 4.6], 0.9),
            ],
            IconId::Warning => &[
                Op::Polygon(&[[8.0, 1.5], [15.0, 14.0], [1.0, 14.0]]),
                Op::Line([8.0, 6.0], [8.0, 10.0]),
                Op::FilledCircle([8.0, 12.0], 0.8),
            ],
            IconId::Error => &[
                Op::Circle([8.0, 8.0], 6.5),
                Op::Line([5.5, 5.5], [10.5, 10.5]),
                Op::Line([10.5, 5.5], [5.5, 10.5]),
            ],
            IconId::Notice => &[
                Op::Polyline(&[
                    [3.0, 12.0],
                    [4.5, 10.0],
                    [4.5, 6.5],
                    [6.0, 4.0],
                    [8.0, 3.3],
                    [10.0, 4.0],
                    [11.5, 6.5],
                    [11.5, 10.0],
                    [13.0, 12.0],
                    [3.0, 12.0],
                ]),
                Op::FilledCircle([8.0, 14.0], 1.0),
            ],
            IconId::Drop => &[
                Op::Polyline(&[[2.0, 10.0], [2.0, 14.0], [14.0, 14.0], [14.0, 10.0]]),
                Op::Line([8.0, 1.5], [8.0, 10.0]),
                Op::Polyline(&[[4.5, 6.5], [8.0, 10.0], [11.5, 6.5]]),
            ],
            IconId::Options => &[
                Op::Line([2.0, 4.0], [14.0, 4.0]),
                Op::Line([2.0, 8.0], [14.0, 8.0]),
                Op::Line([2.0, 12.0], [14.0, 12.0]),
                Op::FilledCircle([5.0, 4.0], 1.6),
                Op::FilledCircle([10.5, 8.0], 1.6),
                Op::FilledCircle([7.0, 12.0], 1.6),
            ],
            IconId::Export => &[
                Op::Polyline(&[
                    [5.0, 6.0],
                    [2.0, 6.0],
                    [2.0, 14.0],
                    [14.0, 14.0],
                    [14.0, 6.0],
                    [11.0, 6.0],
                ]),
                Op::Line([8.0, 10.0], [8.0, 1.5]),
                Op::Polyline(&[[5.0, 4.5], [8.0, 1.5], [11.0, 4.5]]),
            ],
            IconId::Import => &[
                Op::Polyline(&[
                    [5.0, 6.0],
                    [2.0, 6.0],
                    [2.0, 14.0],
                    [14.0, 14.0],
                    [14.0, 6.0],
                    [11.0, 6.0],
                ]),
                Op::Line([8.0, 1.5], [8.0, 10.0]),
                Op::Polyline(&[[5.0, 7.0], [8.0, 10.0], [11.0, 7.0]]),
            ],
            IconId::Restore => &[
                Op::Polyline(&[
                    [5.25, 3.74],
                    [7.52, 3.02],
                    [9.88, 3.33],
                    [11.89, 4.61],
                    [13.17, 6.62],
                    [13.48, 8.98],
                    [12.76, 11.25],
                    [11.15, 13.01],
                    [8.96, 13.92],
                    [6.58, 13.81],
                    [4.46, 12.71],
                    [3.02, 10.82],
                    [2.5, 8.5],
                ]),
                Op::FilledPolygon(&[[3.65, 1.34], [7.45, 3.15], [4.65, 5.34]]),
                Op::FilledCircle([8.0, 8.5], 1.2),
            ],
            IconId::Zipper => &[
                Op::Polygon(&[[3.0, 1.5], [13.0, 1.5], [13.0, 14.5], [3.0, 14.5]]),
                Op::Line([8.0, 1.5], [8.0, 3.0]),
                Op::Line([6.5, 3.0], [8.0, 3.0]),
                Op::Line([8.0, 4.5], [9.5, 4.5]),
                Op::Line([6.5, 6.0], [8.0, 6.0]),
                Op::Line([8.0, 7.5], [9.5, 7.5]),
                Op::Rect([6.5, 9.0], [9.5, 12.5]),
            ],
            IconId::Help => &[
                Op::Circle([8.0, 8.0], 6.5),
                Op::Polyline(&[
                    [5.8, 6.0],
                    [6.3, 4.6],
                    [8.0, 4.0],
                    [9.7, 4.6],
                    [10.2, 6.0],
                    [9.5, 7.3],
                    [8.0, 8.3],
                    [8.0, 9.8],
                ]),
                Op::FilledCircle([8.0, 12.0], 0.9),
            ],
        }
    }

    /// The icon of a session type (Home, W4b).
    #[allow(dead_code)]
    pub fn of_session_type(kind: SessionType) -> IconId {
        match kind {
            SessionType::FolderCompare => IconId::FolderCompare,
            SessionType::FolderSync => IconId::FolderSync,
            SessionType::TextCompare => IconId::TextCompare,
            SessionType::TextEdit => IconId::TextEdit,
            SessionType::TableCompare => IconId::TableCompare,
            SessionType::HexCompare => IconId::HexCompare,
            SessionType::PictureCompare => IconId::PictureCompare,
        }
    }
}

/// The shapes of an icon fitted into `rect` (square; the stroke stays inside) in `colour`.
pub fn shapes(rect: Rect, id: IconId, colour: Color32) -> Vec<Shape> {
    let side = rect.width().min(rect.height());
    let unit = side / (16.0 + STROKE);
    let origin =
        rect.center() - egui::vec2(side, side) / 2.0 + egui::vec2(unit, unit) * (STROKE / 2.0);
    let at = |p: [f32; 2]| -> Pos2 { origin + egui::vec2(p[0], p[1]) * unit };
    let stroke = Stroke::new(STROKE * unit, colour);
    let corner = (unit * 1.0).min(4.0);
    id.ops()
        .iter()
        .map(|op| match *op {
            Op::Line(a, b) => Shape::line_segment([at(a), at(b)], stroke),
            Op::Polyline(p) => Shape::line(p.iter().map(|q| at(*q)).collect(), stroke),
            Op::Polygon(p) => Shape::closed_line(p.iter().map(|q| at(*q)).collect(), stroke),
            Op::FilledPolygon(p) => {
                Shape::convex_polygon(p.iter().map(|q| at(*q)).collect(), colour, Stroke::NONE)
            }
            Op::Circle(c, r) => Shape::circle_stroke(at(c), r * unit, stroke),
            Op::FilledCircle(c, r) => Shape::circle_filled(at(c), r * unit, colour),
            Op::Rect(a, b) => Shape::rect_stroke(
                Rect::from_min_max(at(a), at(b)),
                corner,
                stroke,
                egui::StrokeKind::Middle,
            ),
            Op::FilledRect(a, b) => {
                Shape::rect_filled(Rect::from_min_max(at(a), at(b)), corner, colour)
            }
        })
        .collect()
}

/// Paints an icon into `rect` in `colour`.
pub fn paint(painter: &Painter, rect: Rect, id: IconId, colour: Color32) {
    painter.extend(shapes(rect, id, colour));
}

/// A square of `size` points at `min`.
pub fn square(min: Pos2, size: f32) -> Rect {
    Rect::from_min_size(min, egui::vec2(size, size))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coords(op: &Op) -> Vec<f32> {
        match *op {
            Op::Line(a, b) | Op::Rect(a, b) | Op::FilledRect(a, b) => vec![a[0], a[1], b[0], b[1]],
            Op::Polyline(p) | Op::Polygon(p) | Op::FilledPolygon(p) => {
                p.iter().flat_map(|q| [q[0], q[1]]).collect()
            }
            Op::Circle(c, r) | Op::FilledCircle(c, r) => {
                vec![c[0] - r, c[1] - r, c[0] + r, c[1] + r]
            }
        }
    }

    /// A4.6: the inventory, operations inside the grid, no two icons alike.
    #[test]
    fn the_inventory_is_complete_and_original() {
        assert_eq!(IconId::ALL.len(), 64);
        for (i, id) in IconId::ALL.iter().enumerate() {
            assert!(!id.ops().is_empty(), "{id:?}");
            for op in id.ops() {
                for v in coords(op) {
                    assert!((0.0..=16.0).contains(&v), "{id:?}: {op:?}");
                }
            }
            for other in &IconId::ALL[..i] {
                assert_ne!(id.ops(), other.ops(), "{id:?} = {other:?}");
            }
        }
        for kind in SessionType::ALL {
            assert!(IconId::ALL.contains(&IconId::of_session_type(kind)));
        }
    }

    /// A4.6: painted through a headless context at 1.0, 1.25, 1.5 and 2.0 points per pixel, the shapes stay
    /// inside the target rectangle and use the given colour.
    #[test]
    fn icons_paint_inside_their_rectangle_at_every_scale() {
        let colour = Color32::from_rgb(10, 120, 200);
        for ppp in [1.0_f32, 1.25, 1.5, 2.0] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(ppp);
            for size in [16.0_f32, 20.0, 24.0, 32.0] {
                let rect = square(egui::pos2(10.0, 10.0), size);
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    for id in IconId::ALL {
                        paint(ui.painter(), rect, *id, colour);
                    }
                });
                let mut count = 0;
                for clipped in &output.shapes {
                    let shape = &clipped.shape;
                    let bounds = shape.visual_bounding_rect();
                    if !bounds.is_positive() {
                        continue;
                    }
                    count += 1;
                    assert!(
                        rect.expand(0.01).contains_rect(bounds),
                        "{bounds:?} outside {rect:?} at {ppp}"
                    );
                    let colours = shape_colours(shape);
                    assert!(
                        !colours.is_empty() && colours.iter().all(|c| *c == colour),
                        "{colours:?}"
                    );
                }
                output.textures_delta.clear();
                let ops: usize = IconId::ALL.iter().map(|i| i.ops().len()).sum();
                assert_eq!(count, ops, "every operation is one shape");
            }
        }
    }

    fn shape_colours(shape: &Shape) -> Vec<Color32> {
        match shape {
            Shape::LineSegment { stroke, .. } => vec![stroke.color],
            Shape::Path(p) => {
                let mut v = Vec::new();
                if p.fill != Color32::TRANSPARENT {
                    v.push(p.fill);
                }
                if p.stroke.width > 0.0
                    && let egui::epaint::ColorMode::Solid(c) = p.stroke.color
                {
                    v.push(c);
                }
                v
            }
            Shape::Circle(c) => {
                let mut v = Vec::new();
                if c.fill != Color32::TRANSPARENT {
                    v.push(c.fill);
                }
                if c.stroke.width > 0.0 {
                    v.push(c.stroke.color);
                }
                v
            }
            Shape::Rect(r) => {
                let mut v = Vec::new();
                if r.fill != Color32::TRANSPARENT {
                    v.push(r.fill);
                }
                if r.stroke.width > 0.0 {
                    v.push(r.stroke.color);
                }
                v
            }
            other => panic!("unexpected shape {other:?}"),
        }
    }
}
