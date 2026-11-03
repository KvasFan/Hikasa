//! The checked script (§1.8.2, §1.8.6): commands with typed argument slots. A vocabulary word stays a word in its slot;
//! its meaning is the host's.

use serde::Serialize;

use crate::error::Pos;
use crate::expand::References;
use crate::lex::Word;

/// A command of the language (§1.8.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
#[allow(missing_docs)]
pub enum CommandName {
    Attrib,
    Beep,
    Collapse,
    Compare,
    Copy,
    #[serde(rename = "copyto")]
    CopyTo,
    Criteria,
    Delete,
    Expand,
    FileReport,
    Filter,
    FolderReport,
    HexReport,
    Load,
    Log,
    MediaReport,
    Move,
    #[serde(rename = "moveto")]
    MoveTo,
    Option,
    PictureReport,
    RegistryReport,
    Rename,
    Select,
    Snapshot,
    Sync,
    TableReport,
    TextReport,
    Touch,
    VersionReport,
}

impl CommandName {
    /// Every command, by name.
    pub const ALL: [CommandName; 29] = [
        CommandName::Attrib,
        CommandName::Beep,
        CommandName::Collapse,
        CommandName::Compare,
        CommandName::Copy,
        CommandName::CopyTo,
        CommandName::Criteria,
        CommandName::Delete,
        CommandName::Expand,
        CommandName::FileReport,
        CommandName::Filter,
        CommandName::FolderReport,
        CommandName::HexReport,
        CommandName::Load,
        CommandName::Log,
        CommandName::MediaReport,
        CommandName::Move,
        CommandName::MoveTo,
        CommandName::Option,
        CommandName::PictureReport,
        CommandName::RegistryReport,
        CommandName::Rename,
        CommandName::Select,
        CommandName::Snapshot,
        CommandName::Sync,
        CommandName::TableReport,
        CommandName::TextReport,
        CommandName::Touch,
        CommandName::VersionReport,
    ];

    /// The name as scripts write it.
    pub const fn as_str(self) -> &'static str {
        match self {
            CommandName::Attrib => "attrib",
            CommandName::Beep => "beep",
            CommandName::Collapse => "collapse",
            CommandName::Compare => "compare",
            CommandName::Copy => "copy",
            CommandName::CopyTo => "copyto",
            CommandName::Criteria => "criteria",
            CommandName::Delete => "delete",
            CommandName::Expand => "expand",
            CommandName::FileReport => "file-report",
            CommandName::Filter => "filter",
            CommandName::FolderReport => "folder-report",
            CommandName::HexReport => "hex-report",
            CommandName::Load => "load",
            CommandName::Log => "log",
            CommandName::MediaReport => "media-report",
            CommandName::Move => "move",
            CommandName::MoveTo => "moveto",
            CommandName::Option => "option",
            CommandName::PictureReport => "picture-report",
            CommandName::RegistryReport => "registry-report",
            CommandName::Rename => "rename",
            CommandName::Select => "select",
            CommandName::Snapshot => "snapshot",
            CommandName::Sync => "sync",
            CommandName::TableReport => "table-report",
            CommandName::TextReport => "text-report",
            CommandName::Touch => "touch",
            CommandName::VersionReport => "version-report",
        }
    }

    /// The command of `word` (ASCII case-insensitive; `data-report` is `table-report`).
    pub fn parse(word: &str) -> Option<CommandName> {
        if word.eq_ignore_ascii_case("data-report") {
            return Some(CommandName::TableReport);
        }
        CommandName::ALL
            .into_iter()
            .find(|name| name.as_str().eq_ignore_ascii_case(word))
    }

    /// True for a command that needs a loaded comparison (column *Needs* of §1.8.6).
    pub const fn needs_comparison(self) -> bool {
        matches!(
            self,
            CommandName::Expand
                | CommandName::Collapse
                | CommandName::Select
                | CommandName::Compare
                | CommandName::Copy
                | CommandName::Move
                | CommandName::CopyTo
                | CommandName::MoveTo
                | CommandName::Delete
                | CommandName::Rename
                | CommandName::Touch
                | CommandName::Attrib
                | CommandName::Sync
                | CommandName::FolderReport
        )
    }

    /// True for a command that writes the bases (and so builds a plan and asks).
    pub const fn mutates(self) -> bool {
        matches!(
            self,
            CommandName::Copy
                | CommandName::Move
                | CommandName::CopyTo
                | CommandName::MoveTo
                | CommandName::Delete
                | CommandName::Rename
                | CommandName::Touch
                | CommandName::Attrib
                | CommandName::Sync
        )
    }

    /// The report of a report command.
    pub const fn report(self) -> Option<ReportFor> {
        match self {
            CommandName::TextReport | CommandName::FileReport => Some(ReportFor::Text),
            CommandName::FolderReport => Some(ReportFor::Folder),
            CommandName::TableReport
            | CommandName::HexReport
            | CommandName::PictureReport
            | CommandName::MediaReport
            | CommandName::VersionReport
            | CommandName::RegistryReport => Some(ReportFor::Other),
            _ => None,
        }
    }
}

/// The kind of report a report command writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportFor {
    /// `text-report`, `file-report`.
    Text,
    /// `folder-report`.
    Folder,
    /// The reports this version does not write (D-221).
    Other,
}

/// What a word is (§1.8.2): the vocabulary a host checks it with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Slot {
    /// `left|right` (normalized).
    Side,
    /// `left|right|all` (normalized).
    Sides,
    /// `left->right|right->left` (normalized).
    Direction,
    /// A direction, or `all` (`update:`, `mirror:`).
    SyncDirection,
    /// `relative|base|none`.
    Structure,
    /// A select mask.
    SelectMask,
    /// A criteria list.
    Criteria,
    /// The name filter of `filter <masks>`.
    NameFilter,
    /// `filter cutoff:`.
    Cutoff,
    /// `filter size:`.
    SizeFilter,
    /// `filter attrib:`.
    AttribFilter,
    /// `filter unixtype:`.
    UnixTypeFilter,
    /// `option confirm:`.
    ConfirmMode,
    /// `log` level.
    LogLevel,
    /// `yes|no` of `recyclebin=`.
    Recycle,
    /// `now` or a time of `touch`.
    Time,
    /// `compare`'s method.
    ContentMethod,
    /// `rename`'s mask.
    RenameMask,
    /// `rename regexpr`'s expression.
    Regex,
    /// `attrib`'s change.
    AttribChange,
    /// `layout:` of a report.
    ReportLayout(ReportFor),
    /// `options:` of a report.
    ReportOptions(ReportFor),
    /// `output-options:` of a report.
    ReportOutputOptions(ReportFor),
    /// `output-to:` of a report.
    ReportTarget,
    /// A spec: a base, a report's input, a `copyto` target, a log file.
    Spec,
    /// A session name.
    Session,
    /// A row path below the bases (`expand`, `collapse`).
    Path,
    /// Free text (a title, a replacement, `snapshot`'s words).
    Text,
}

/// `load`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadArgs<W> {
    /// `load <session>`.
    Session(W),
    /// `load <default>`.
    Default,
    /// `load [create:SIDES] <left> <right>`.
    Folders {
        /// `create:`.
        create: Option<W>,
        /// The left base.
        left: W,
        /// The right base.
        right: W,
    },
}

/// `filter`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterArgs<W> {
    /// `filter <masks>`.
    Masks(W),
    /// `filter cutoff:<expr>|none` (`None`: `none`).
    Cutoff(Option<W>),
    /// `filter size:`.
    Size(Option<W>),
    /// `filter attrib:`.
    Attrib(Option<W>),
    /// `filter unixtype:`.
    UnixType(Option<W>),
    /// `filter exclude-protected`.
    ExcludeProtected,
    /// `filter include-protected`.
    IncludeProtected,
}

/// `expand`, `collapse`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpandArgs<W> {
    /// `all`.
    All,
    /// Row paths.
    Paths(Vec<W>),
}

/// `copyto`, `moveto`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CopyToArgs<W> {
    /// The sides (default `all`).
    pub sides: Option<W>,
    /// `path:` (default `relative`).
    pub structure: Option<W>,
    /// The target.
    pub target: W,
}

/// `rename`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RenameArgs<W> {
    /// `rename <new mask>`.
    Mask(W),
    /// `rename regexpr <old> <new>`.
    Regex {
        /// The expression.
        old: W,
        /// The replacement.
        new: W,
    },
}

/// `touch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TouchArgs<W> {
    /// `touch DIR`: the side the arrow points to gets the other side's times.
    Direction(W),
    /// `touch SIDES:now|<time>`.
    At {
        /// The sides.
        sides: W,
        /// `now` or a time.
        time: W,
    },
}

/// `update:` or `mirror:`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    /// `update:`.
    Update,
    /// `mirror:`.
    Mirror,
}

/// `sync`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SyncArgs<W> {
    /// `visible`.
    pub visible: bool,
    /// `create-empty`.
    pub create_empty: bool,
    /// `update:DIR|all`, `mirror:DIR` (`None`: the loaded session's preset).
    pub mode: Option<(SyncMode, W)>,
}

/// A report command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReportArgs<W> {
    /// `just-selection` (`folder-report`).
    pub just_selection: bool,
    /// `layout:`.
    pub layout: Option<W>,
    /// `options:`, in order.
    pub options: Vec<W>,
    /// `output-options:`, in order.
    pub output_options: Vec<W>,
    /// `title:`.
    pub title: Option<W>,
    /// `output-to:`.
    pub output_to: Option<W>,
    /// The positional words: a session, or two specs.
    pub specs: Vec<W>,
}

/// `option`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionArgs<W> {
    /// `option stop-on-error`.
    StopOnError,
    /// `option confirm:<mode>`.
    Confirm(W),
}

/// `log`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogArgs<W> {
    /// `none|normal|verbose`.
    pub level: Option<W>,
    /// The file.
    pub file: Option<W>,
    /// `append:`.
    pub append: bool,
}

/// The arguments of one command, as typed slots; `W` is a [`Word`] in a checked script and the expanded value when it
/// runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum Args<W> {
    Load(LoadArgs<W>),
    Criteria { words: Vec<W> },
    Filter(FilterArgs<W>),
    Expand(ExpandArgs<W>),
    Collapse(ExpandArgs<W>),
    Select { masks: Vec<W> },
    Compare { method: Option<W> },
    Copy { direction: W },
    Move { direction: W },
    CopyTo(CopyToArgs<W>),
    MoveTo(CopyToArgs<W>),
    Delete { recycle: Option<W>, sides: W },
    Rename(RenameArgs<W>),
    Touch(TouchArgs<W>),
    Attrib { changes: Vec<W> },
    Sync(SyncArgs<W>),
    Report(ReportArgs<W>),
    Snapshot { words: Vec<W> },
    Option(OptionArgs<W>),
    Log(LogArgs<W>),
    Beep,
}

/// Maps an optional word.
fn opt<'a, W, V, E>(
    word: &'a Option<W>,
    slot: Slot,
    f: &mut dyn FnMut(Slot, &'a W) -> Result<V, E>,
) -> Result<Option<V>, E> {
    word.as_ref().map(|w| f(slot, w)).transpose()
}

/// Maps a list of words.
fn list<'a, W, V, E>(
    words: &'a [W],
    slot: Slot,
    f: &mut dyn FnMut(Slot, &'a W) -> Result<V, E>,
) -> Result<Vec<V>, E> {
    words.iter().map(|w| f(slot, w)).collect()
}

impl<W> Args<W> {
    /// The same arguments with each word mapped by `f`, in grammar order, with its slot; the first error stops.
    pub fn try_map<'a, V, E>(
        &'a self,
        f: &mut dyn FnMut(Slot, &'a W) -> Result<V, E>,
    ) -> Result<Args<V>, E> {
        Ok(match self {
            Args::Load(LoadArgs::Session(w)) => Args::Load(LoadArgs::Session(f(Slot::Session, w)?)),
            Args::Load(LoadArgs::Default) => Args::Load(LoadArgs::Default),
            Args::Load(LoadArgs::Folders {
                create,
                left,
                right,
            }) => Args::Load(LoadArgs::Folders {
                create: opt(create, Slot::Sides, f)?,
                left: f(Slot::Spec, left)?,
                right: f(Slot::Spec, right)?,
            }),
            Args::Criteria { words } => Args::Criteria {
                words: list(words, Slot::Criteria, f)?,
            },
            Args::Filter(filter) => Args::Filter(match filter {
                FilterArgs::Masks(w) => FilterArgs::Masks(f(Slot::NameFilter, w)?),
                FilterArgs::Cutoff(w) => FilterArgs::Cutoff(opt(w, Slot::Cutoff, f)?),
                FilterArgs::Size(w) => FilterArgs::Size(opt(w, Slot::SizeFilter, f)?),
                FilterArgs::Attrib(w) => FilterArgs::Attrib(opt(w, Slot::AttribFilter, f)?),
                FilterArgs::UnixType(w) => FilterArgs::UnixType(opt(w, Slot::UnixTypeFilter, f)?),
                FilterArgs::ExcludeProtected => FilterArgs::ExcludeProtected,
                FilterArgs::IncludeProtected => FilterArgs::IncludeProtected,
            }),
            Args::Expand(args) => Args::Expand(map_expand(args, f)?),
            Args::Collapse(args) => Args::Collapse(map_expand(args, f)?),
            Args::Select { masks } => Args::Select {
                masks: list(masks, Slot::SelectMask, f)?,
            },
            Args::Compare { method } => Args::Compare {
                method: opt(method, Slot::ContentMethod, f)?,
            },
            Args::Copy { direction } => Args::Copy {
                direction: f(Slot::Direction, direction)?,
            },
            Args::Move { direction } => Args::Move {
                direction: f(Slot::Direction, direction)?,
            },
            Args::CopyTo(args) => Args::CopyTo(map_copy_to(args, f)?),
            Args::MoveTo(args) => Args::MoveTo(map_copy_to(args, f)?),
            Args::Delete { recycle, sides } => Args::Delete {
                recycle: opt(recycle, Slot::Recycle, f)?,
                sides: f(Slot::Sides, sides)?,
            },
            Args::Rename(RenameArgs::Mask(w)) => {
                Args::Rename(RenameArgs::Mask(f(Slot::RenameMask, w)?))
            }
            Args::Rename(RenameArgs::Regex { old, new }) => Args::Rename(RenameArgs::Regex {
                old: f(Slot::Regex, old)?,
                new: f(Slot::Text, new)?,
            }),
            Args::Touch(TouchArgs::Direction(w)) => {
                Args::Touch(TouchArgs::Direction(f(Slot::Direction, w)?))
            }
            Args::Touch(TouchArgs::At { sides, time }) => Args::Touch(TouchArgs::At {
                sides: f(Slot::Sides, sides)?,
                time: f(Slot::Time, time)?,
            }),
            Args::Attrib { changes } => Args::Attrib {
                changes: list(changes, Slot::AttribChange, f)?,
            },
            Args::Sync(args) => Args::Sync(SyncArgs {
                visible: args.visible,
                create_empty: args.create_empty,
                mode: match &args.mode {
                    Some((mode, w)) => Some((*mode, f(Slot::SyncDirection, w)?)),
                    None => None,
                },
            }),
            Args::Report(args) => {
                // The kind is not known here: the slots name it from the words' command, see `Command::slots`.
                Args::Report(map_report(args, ReportFor::Text, f)?)
            }
            Args::Snapshot { words } => Args::Snapshot {
                words: list(words, Slot::Text, f)?,
            },
            Args::Option(OptionArgs::StopOnError) => Args::Option(OptionArgs::StopOnError),
            Args::Option(OptionArgs::Confirm(w)) => {
                Args::Option(OptionArgs::Confirm(f(Slot::ConfirmMode, w)?))
            }
            Args::Log(args) => Args::Log(LogArgs {
                level: opt(&args.level, Slot::LogLevel, f)?,
                file: opt(&args.file, Slot::Spec, f)?,
                append: args.append,
            }),
            Args::Beep => Args::Beep,
        })
    }

    /// [`Args::try_map`] with the report kind of `name` in the report slots.
    pub fn try_map_for<'a, V, E>(
        &'a self,
        name: CommandName,
        f: &mut dyn FnMut(Slot, &'a W) -> Result<V, E>,
    ) -> Result<Args<V>, E> {
        match (self, name.report()) {
            (Args::Report(args), Some(kind)) => Ok(Args::Report(map_report(args, kind, f)?)),
            _ => self.try_map(f),
        }
    }
}

fn map_expand<'a, W, V, E>(
    args: &'a ExpandArgs<W>,
    f: &mut dyn FnMut(Slot, &'a W) -> Result<V, E>,
) -> Result<ExpandArgs<V>, E> {
    Ok(match args {
        ExpandArgs::All => ExpandArgs::All,
        ExpandArgs::Paths(paths) => ExpandArgs::Paths(list(paths, Slot::Path, f)?),
    })
}

fn map_copy_to<'a, W, V, E>(
    args: &'a CopyToArgs<W>,
    f: &mut dyn FnMut(Slot, &'a W) -> Result<V, E>,
) -> Result<CopyToArgs<V>, E> {
    Ok(CopyToArgs {
        sides: opt(&args.sides, Slot::Sides, f)?,
        structure: opt(&args.structure, Slot::Structure, f)?,
        target: f(Slot::Spec, &args.target)?,
    })
}

fn map_report<'a, W, V, E>(
    args: &'a ReportArgs<W>,
    kind: ReportFor,
    f: &mut dyn FnMut(Slot, &'a W) -> Result<V, E>,
) -> Result<ReportArgs<V>, E> {
    let spec_slot = if args.specs.len() == 1 {
        Slot::Session
    } else {
        Slot::Spec
    };
    Ok(ReportArgs {
        just_selection: args.just_selection,
        layout: opt(&args.layout, Slot::ReportLayout(kind), f)?,
        options: list(&args.options, Slot::ReportOptions(kind), f)?,
        output_options: list(&args.output_options, Slot::ReportOutputOptions(kind), f)?,
        title: opt(&args.title, Slot::Text, f)?,
        output_to: opt(&args.output_to, Slot::ReportTarget, f)?,
        specs: list(&args.specs, spec_slot, f)?,
    })
}

/// One checked command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Command {
    /// The position of its name.
    pub pos: Pos,
    /// Its last physical line.
    pub end_line: u32,
    /// The command.
    pub name: CommandName,
    /// Its arguments.
    pub args: Args<Word>,
    /// Every word as lexed, the name first (the JSON `words`).
    pub words: Vec<Word>,
}

impl Command {
    /// The words as lexed, before expansion.
    pub fn word_texts(&self) -> Vec<String> {
        self.words.iter().map(|word| word.text.clone()).collect()
    }

    /// The command as a script writes it, on one line.
    pub fn source(&self) -> String {
        let words: Vec<String> = self.words.iter().map(Word::to_source).collect();
        words.join(" ")
    }

    /// Each word of the arguments with its slot, in grammar order.
    pub fn slots(&self) -> Vec<(Slot, &Word)> {
        let mut out = Vec::new();
        let _ = self
            .args
            .try_map_for(self.name, &mut |slot, word| -> Result<(), ()> {
                out.push((slot, word));
                Ok(())
            });
        out
    }
}

/// A checked script.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Script {
    /// The commands, in order.
    pub commands: Vec<Command>,
    /// What it references.
    pub references: References,
}
