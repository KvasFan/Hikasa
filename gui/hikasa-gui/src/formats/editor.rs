//! The format editor (M2 W11c; contract §1.11.27, FF-001, FF-020…FF-026, FF-031; D-319): a `DialogModel` over a
//! `hikasa_formats::FileFormat` value with one tab set per kind — Text: General, Conversion, Grammar, Misc; Table:
//! General, Conversion, Type, Regional; Hex and Picture: General, Conversion; Media, Version, Registry: General — and,
//! on Text formats, the preview (`preview.rs`: the sample tokenized by `Service::format_preview`, a job 300 ms after
//! the last change). OK checks the format with core (`format_check`: the message names the item) and writes it with
//! `format_save` (the canonical TOML `format edit` writes; a built-in gets its override file); a file changed on disk
//! since the editor read it asks W4c's *Overwrite* / *Reload* / *Cancel*. Cancel discards.
//!
//! The numbers are edited as text and read when the format is checked; every other field edits the value directly.

use std::any::Any;
use std::time::Instant;

use hikasa_core::JobKind;
use hikasa_formats::{
    Conversion, ElementInfo, ElementName, EncodingSetting, ExternalConversion, FileFormat,
    FilenameEncoding, FormatKind, GrammarItem, ItemKind, LineWeight, TableSettings,
};
use hikasa_session::api::FormatWriteDto;
use hikasa_session::api::format_editor::FormatForEdit;
use hikasa_session::{ApiError, Service, SessionError};

use super::grammar_item::{GrammarItemDialog, ItemDone};
use super::preview::Preview;
use super::{FormatsFollowup, Pending, queued};
use crate::dialog::{
    ButtonRole, DialogButton, DialogModel, DialogRequest, DialogStep, Followup, Prompt,
};
use crate::job::{Job, Wake};

/// The help topic of the editor.
pub const TOPIC: &str = "dialog-format-editor";

/// A tab of the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorTab {
    /// Name, id, mask, description.
    General,
    /// None, Sorted, External, Spreadsheet text.
    Conversion,
    /// The grammar items, the element settings, the line weights.
    Grammar,
    /// Tab size, word characters, encoding, the save options.
    Misc,
    /// The table type, delimiters, qualifier, widths, first line.
    Type,
    /// The table's regional settings.
    Regional,
}

impl EditorTab {
    /// The caption.
    pub fn caption(self) -> &'static str {
        match self {
            EditorTab::General => "General",
            EditorTab::Conversion => "Conversion",
            EditorTab::Grammar => "Grammar",
            EditorTab::Misc => "Misc",
            EditorTab::Type => "Type",
            EditorTab::Regional => "Regional",
        }
    }
}

/// The tabs of a kind (§1.11.27).
pub fn tabs_of(kind: FormatKind) -> &'static [EditorTab] {
    use EditorTab as T;
    match kind {
        FormatKind::Text => &[T::General, T::Conversion, T::Grammar, T::Misc],
        FormatKind::Table => &[T::General, T::Conversion, T::Type, T::Regional],
        FormatKind::Hex | FormatKind::Picture => &[T::General, T::Conversion],
        _ => &[T::General],
    }
}

/// The answer to a conflicting save (W4c).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictAnswer {
    /// Write over the file changed on disk.
    Overwrite,
    /// Read the file again (the edits go).
    Reload,
    /// Keep editing.
    Cancel,
}

/// The conversion choices of the Conversion tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversionChoice {
    /// Plain text.
    None,
    /// Sorted lines.
    Sorted,
    /// An external program.
    External,
    /// The built-in *Spreadsheet text*.
    Spreadsheet,
}

/// The numbers and characters of the format, as typed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Typed {
    /// Tab size (1–32).
    pub tab_size: String,
    /// Characters per line limit (`""`: none).
    pub cpl: String,
    /// The external conversion's timeout in seconds.
    pub timeout: String,
    /// The table's widths, `;`-separated.
    pub widths: String,
    /// The table's delimiter (one character; `tab` for a tab).
    pub delimiter: String,
    /// The table's text qualifier (`""`: none).
    pub qualifier: String,
    /// The line weight being typed: its priority (1–4).
    pub priority: String,
}

impl Typed {
    fn of(f: &FileFormat) -> Typed {
        let table = f.table.clone().unwrap_or_default();
        let timeout = match &f.conversion {
            Conversion::External(x) => x.timeout_secs.to_string(),
            _ => "60".to_owned(),
        };
        Typed {
            tab_size: f.text.tab_size.to_string(),
            cpl: f
                .text
                .chars_per_line_limit
                .map(|n| n.to_string())
                .unwrap_or_default(),
            timeout,
            widths: table
                .widths
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(";"),
            delimiter: char_word(table.delimiter),
            qualifier: table.qualifier.map(char_word).unwrap_or_default(),
            priority: "1".to_owned(),
        }
    }
}

/// A character as the field shows it (`tab` for a tab).
fn char_word(c: char) -> String {
    if c == '\t' {
        "tab".to_owned()
    } else {
        c.to_string()
    }
}

/// One character of a field (`tab` for a tab).
fn read_char(text: &str, what: &str) -> Result<char, String> {
    if text == "tab" || text == "\\t" {
        return Ok('\t');
    }
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Ok(c),
        _ => Err(format!("{what} must be one character, not `{text}`.")),
    }
}

/// A whole number.
fn read_number<T: std::str::FromStr>(text: &str, what: &str) -> Result<T, String> {
    text.trim()
        .parse()
        .map_err(|_| format!("{what} must be a whole number, not `{}`.", text.trim()))
}

/// The summary of a grammar item in the list.
pub fn summary(item: &GrammarItem) -> String {
    let flags = |regex: bool, case: bool| {
        let mut s = String::new();
        if regex {
            s.push_str(" (regex)");
        }
        if !case {
            s.push_str(" (any case)");
        }
        s
    };
    match &item.kind {
        ItemKind::Basic {
            text,
            regex,
            match_case,
        } => format!("Basic: {text}{}", flags(*regex, *match_case)),
        ItemKind::Delimited {
            from,
            to,
            stop_at_eol,
            escape,
            regex,
            match_case,
        } => {
            let mut s = format!(
                "Delimited: {from} … {}",
                to.as_deref().unwrap_or("end of line")
            );
            if *stop_at_eol && to.is_some() {
                s.push_str(" (stops at end of line)");
            }
            if let Some(e) = escape {
                s.push_str(&format!(" (escape {e})"));
            }
            s + &flags(*regex, *match_case)
        }
        ItemKind::List {
            tokens,
            regex,
            match_case,
        } => format!(
            "List: {} token{}{}",
            tokens.len(),
            if tokens.len() == 1 { "" } else { "s" },
            flags(*regex, *match_case)
        ),
        ItemKind::Columns {
            from_column,
            to_column,
        } => match to_column {
            Some(to) => format!("Columns {from_column}–{to}"),
            None => format!("Columns {from_column} to end of line"),
        },
        ItemKind::Lines {
            heading,
            regex,
            match_case,
            next_lines,
            or_line_one,
        } => {
            let mut s = format!("Lines: {heading} and the next {next_lines}");
            if *or_line_one {
                s.push_str(" (or line 1)");
            }
            s + &flags(*regex, *match_case)
        }
    }
}

/// The format editor.
pub struct FormatEditor {
    /// The format's id.
    pub id: String,
    /// The format as read (`None` while it loads).
    pub loaded: Option<FormatForEdit>,
    /// The working copy.
    pub format: FileFormat,
    /// The typed numbers and characters.
    pub typed: Typed,
    /// The tab shown.
    pub tab: EditorTab,
    /// The selected grammar item.
    pub item_selected: Option<usize>,
    /// The selected line weight.
    pub weight_selected: Option<usize>,
    /// The line weight being typed (*Add*).
    pub weight: LineWeight,
    /// The preview (Text formats).
    pub preview: Preview,
    /// Core's message (check, save) or a number that does not read.
    pub error: Option<String>,
    load: Option<Job<FormatForEdit>>,
    saving: Option<Pending<FormatWriteDto>>,
    requests: Vec<DialogRequest>,
    wake: Wake,
}

impl FormatEditor {
    /// The editor of format `id` (an id or a name): it reads the format on a job.
    pub fn open(svc: &Service, id: &str, wake: Wake) -> FormatEditor {
        let wanted = id.to_owned();
        let load = Job::spawn(
            svc,
            JobKind::VfsRead,
            format!("read the format {id}"),
            &wake,
            move |svc, _| svc.format_for_edit(&wanted),
        );
        FormatEditor {
            id: id.to_owned(),
            loaded: None,
            format: FileFormat::new(
                hikasa_formats::FormatId::new("x").unwrap_or_else(|_| unreachable_id()),
                "",
                FormatKind::Text,
            ),
            typed: Typed::default(),
            tab: EditorTab::General,
            item_selected: None,
            weight_selected: None,
            weight: LineWeight {
                pattern: String::new(),
                regex: false,
                match_case: true,
                priority: 1,
            },
            preview: Preview::default(),
            error: None,
            load: Some(load),
            saving: None,
            requests: Vec::new(),
            wake,
        }
    }

    /// Takes a format read (the load job's, or a reload).
    pub fn take(&mut self, read: FormatForEdit) {
        self.id = read.format.id.as_str().to_owned();
        self.format = read.format.clone();
        self.typed = Typed::of(&self.format);
        if !tabs_of(self.format.kind).contains(&self.tab) {
            self.tab = EditorTab::General;
        }
        self.loaded = Some(read);
        self.error = None;
        self.preview.touch(Instant::now());
    }

    /// True once the format is read.
    pub fn ready(&self) -> bool {
        self.loaded.is_some()
    }

    /// The tabs of this format's kind.
    pub fn tabs(&self) -> &'static [EditorTab] {
        tabs_of(self.format.kind)
    }

    /// The typed numbers and characters into `f`; a field that does not read is the error.
    fn apply_typed(&self, f: &mut FileFormat) -> Result<(), String> {
        let t = &self.typed;
        f.text.tab_size = read_number(&t.tab_size, "The tab size")?;
        f.text.chars_per_line_limit = if t.cpl.trim().is_empty() {
            None
        } else {
            Some(read_number(&t.cpl, "The characters per line limit")?)
        };
        if let Conversion::External(x) = &mut f.conversion {
            x.timeout_secs = read_number(&t.timeout, "The timeout")?;
        }
        if f.kind == FormatKind::Table || f.table.is_some() {
            let table = f.table.get_or_insert_with(TableSettings::default);
            table.delimiter = read_char(&t.delimiter, "The delimiter")?;
            table.qualifier = if t.qualifier.is_empty() {
                None
            } else {
                Some(read_char(&t.qualifier, "The text qualifier")?)
            };
            table.widths = t
                .widths
                .split(';')
                .map(str::trim)
                .filter(|w| !w.is_empty())
                .map(|w| read_number(w, "A width"))
                .collect::<Result<_, _>>()?;
        }
        Ok(())
    }

    /// The format as OK would write it (the typed fields read), or the message of a field that does not read.
    pub fn result(&self) -> Result<FileFormat, String> {
        let mut f = self.format.clone();
        self.apply_typed(&mut f)?;
        Ok(f)
    }

    // ---- Conversion ----

    /// The conversion choice shown.
    pub fn conversion(&self) -> ConversionChoice {
        match &self.format.conversion {
            Conversion::None => ConversionChoice::None,
            Conversion::Sorted => ConversionChoice::Sorted,
            Conversion::External(_) => ConversionChoice::External,
            Conversion::Spreadsheet => ConversionChoice::Spreadsheet,
        }
    }

    /// Picks a conversion (an external one starts empty, with the typed timeout).
    pub fn set_conversion(&mut self, choice: ConversionChoice) {
        if choice == self.conversion() {
            return;
        }
        self.format.conversion = match choice {
            ConversionChoice::None => Conversion::None,
            ConversionChoice::Sorted => Conversion::Sorted,
            ConversionChoice::Spreadsheet => Conversion::Spreadsheet,
            ConversionChoice::External => Conversion::External(ExternalConversion {
                load: String::new(),
                save: None,
                filename_encoding: FilenameEncoding::Unicode,
                timeout_secs: read_number(&self.typed.timeout, "").unwrap_or(60),
            }),
        };
        self.touched();
    }

    /// The external conversion, to edit (`None` for another choice).
    pub fn external_mut(&mut self) -> Option<&mut ExternalConversion> {
        match &mut self.format.conversion {
            Conversion::External(x) => Some(x),
            _ => None,
        }
    }

    /// *Disable editing* of an external conversion (FF-031): no save command line.
    pub fn set_disable_editing(&mut self, disabled: bool) {
        if let Some(x) = self.external_mut() {
            x.save = if disabled {
                None
            } else {
                Some(x.save.clone().unwrap_or_default())
            };
        }
    }

    // ---- Misc ----

    /// The encoding: fixed (`Some(label)`) or Auto with its fallback (`None`).
    pub fn set_encoding(&mut self, fixed: Option<String>, fallback: Option<String>) {
        self.format.encoding = match fixed {
            Some(label) => EncodingSetting::Fixed { label },
            None => EncodingSetting::Auto {
                fallback: fallback.filter(|f| !f.is_empty()),
            },
        };
    }

    // ---- Grammar ----

    /// Every element name of the grammar and the element table, in order of first use.
    pub fn element_names(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for n in self
            .format
            .grammar
            .iter()
            .map(|g| g.element.as_str().to_owned())
            .chain(self.format.elements.keys().map(|k| k.as_str().to_owned()))
        {
            if !out.contains(&n) {
                out.push(n);
            }
        }
        out
    }

    /// The settings of an element (the default when the table has no row).
    pub fn element_info(&self, name: &str) -> ElementInfo {
        self.format
            .elements
            .iter()
            .find(|(k, _)| k.as_str() == name)
            .map(|(_, v)| *v)
            .unwrap_or_default()
    }

    /// Sets an element's settings: a row equal to the default is no row.
    pub fn set_element_info(&mut self, name: &str, info: ElementInfo) {
        let Ok(key) = ElementName::new(name) else {
            return;
        };
        if info == ElementInfo::default() {
            self.format.elements.remove(&key);
        } else {
            self.format.elements.insert(key, info);
        }
        self.touched();
    }

    /// *Add* or *Edit*: the grammar item dialog over the editor (`index` `None`: a new item after the selected one).
    pub fn item_dialog(&mut self, index: Option<usize>) {
        let item = match index.and_then(|k| self.format.grammar.get(k)) {
            Some(item) => item.clone(),
            None => GrammarItem {
                element: ElementName::new("Comment").unwrap_or_else(|_| unreachable_name()),
                kind: ItemKind::Basic {
                    text: String::new(),
                    regex: false,
                    match_case: true,
                },
            },
        };
        let dialog = GrammarItemDialog::new(&self.format, index, self.new_item_at(), item);
        self.requests.push(DialogRequest::Push(Box::new(dialog)));
    }

    /// Where a new item goes: after the selected one, else last.
    fn new_item_at(&self) -> usize {
        let len = self.format.grammar.len();
        self.item_selected.map_or(len, |k| (k + 1).min(len))
    }

    /// The grammar item dialog's OK.
    pub fn item_done(&mut self, done: ItemDone) {
        let k = match done.index {
            Some(k) if k < self.format.grammar.len() => {
                self.format.grammar[k] = done.item.clone();
                k
            }
            _ => {
                let at = self.new_item_at();
                self.format.grammar.insert(at, done.item.clone());
                at
            }
        };
        self.item_selected = Some(k);
        let name = done.item.element.as_str().to_owned();
        let info = ElementInfo {
            case_sensitive: done.case_sensitive,
            ..self.element_info(&name)
        };
        self.set_element_info(&name, info);
        self.touched();
    }

    /// *Clone* of the selected item (after it).
    pub fn clone_item(&mut self) {
        if let Some(k) = self
            .item_selected
            .filter(|&k| k < self.format.grammar.len())
        {
            let item = self.format.grammar[k].clone();
            self.format.grammar.insert(k + 1, item);
            self.item_selected = Some(k + 1);
            self.touched();
        }
    }

    /// *Delete* of the selected item.
    pub fn delete_item(&mut self) {
        if let Some(k) = self
            .item_selected
            .filter(|&k| k < self.format.grammar.len())
        {
            self.format.grammar.remove(k);
            self.item_selected = None;
            self.touched();
        }
    }

    /// *Move Up* (`-1`) or *Move Down* (`1`) of the selected item (the first match wins, FF-026).
    pub fn move_item(&mut self, by: isize) {
        let Some(k) = self
            .item_selected
            .filter(|&k| k < self.format.grammar.len())
        else {
            return;
        };
        let to = k as isize + by;
        if to < 0 || to as usize >= self.format.grammar.len() {
            return;
        }
        self.format.grammar.swap(k, to as usize);
        self.item_selected = Some(to as usize);
        self.touched();
    }

    /// *Add* of the typed line weight (its priority read from the field).
    pub fn add_weight(&mut self) {
        if self.weight.pattern.is_empty() {
            return;
        }
        match read_number::<u8>(&self.typed.priority, "The priority") {
            Ok(p) => {
                let mut w = self.weight.clone();
                w.priority = p;
                self.format.line_weights.push(w);
                self.weight.pattern.clear();
                self.weight_selected = Some(self.format.line_weights.len() - 1);
                self.touched();
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// *Remove* of the selected line weight.
    pub fn remove_weight(&mut self) {
        if let Some(k) = self
            .weight_selected
            .filter(|&k| k < self.format.line_weights.len())
        {
            self.format.line_weights.remove(k);
            self.weight_selected = None;
            self.touched();
        }
    }

    /// A field changed: the preview runs again after its pause.
    pub fn touched(&mut self) {
        self.preview.touch(Instant::now());
    }

    // ---- OK ----

    /// OK: the typed fields read, core's check (the message names the item), then the save on the settings queue.
    fn ok(&mut self, svc: &Service, expected: Option<hikasa_session::settings::ChangeStamp>) {
        let format = match self.result() {
            Ok(f) => f,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        if let Err(e) = svc.format_check(&format) {
            self.error = Some(e.to_string());
            return;
        }
        self.error = None;
        let id = self.id.clone();
        let (request, pending) = queued(format!("save the format {id}"), move |svc| {
            let dto = svc.format_save(&id, &format, expected)?;
            let written = dto.written.iter().map(std::path::PathBuf::from).collect();
            Ok((dto, written))
        });
        self.requests.push(request);
        self.saving = Some(pending);
    }

    /// The answer to a conflicting save.
    pub fn conflict(&mut self, svc: &Service, answer: ConflictAnswer) {
        match answer {
            ConflictAnswer::Overwrite => self.ok(svc, None),
            ConflictAnswer::Reload => match svc.format_for_edit(&self.id) {
                Ok(read) => self.take(read),
                Err(e) => self.error = Some(e.to_string()),
            },
            ConflictAnswer::Cancel => {}
        }
    }

    /// The *Load sample…* answer: the first 64 KiB of the file become the sample (a job).
    pub fn load_sample(&mut self, svc: &Service, path: std::path::PathBuf) {
        self.preview.load_file(svc, path, &self.wake);
    }
}

/// An id that always parses (the placeholder of a format still loading).
fn unreachable_id() -> hikasa_formats::FormatId {
    hikasa_formats::FormatId::new("format").expect("a valid id")
}

/// A name that always parses.
fn unreachable_name() -> ElementName {
    ElementName::new("Element").expect("a valid name")
}

impl DialogModel for FormatEditor {
    fn title(&self) -> String {
        match &self.loaded {
            Some(l) => format!("Format: {} ({})", l.format.name, l.format.id),
            None => format!("Format: {}", self.id),
        }
    }

    fn help_topic(&self) -> &'static str {
        TOPIC
    }

    fn buttons(&self) -> Vec<DialogButton> {
        vec![
            DialogButton {
                label: "OK".to_owned(),
                role: ButtonRole::Ok,
                enabled: self.ready() && self.saving.is_none(),
                default: true,
            },
            DialogButton {
                label: "Cancel".to_owned(),
                role: ButtonRole::Cancel,
                enabled: true,
                default: false,
            },
        ]
    }

    fn on_button(&mut self, role: ButtonRole, svc: &Service) -> DialogStep {
        if role != ButtonRole::Ok {
            return DialogStep::Close;
        }
        let expected = self.loaded.as_ref().map(|l| l.stamp);
        self.ok(svc, expected);
        DialogStep::Stay
    }

    fn poll(&mut self, svc: &Service) -> bool {
        let mut changed = false;
        if let Some(load) = &self.load
            && let Some(result) = load.try_result()
        {
            self.load = None;
            changed = true;
            match result {
                Ok(read) => self.take(read),
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        if let Some(saving) = &self.saving
            && let Some(result) = saving.take()
        {
            self.saving = None;
            changed = true;
            match result {
                Ok(_) => {
                    self.requests.push(DialogRequest::FormatsChanged(vec![
                        self.format.name.clone(),
                    ]));
                    self.requests.push(DialogRequest::Close);
                }
                Err(ApiError::Session(SessionError::Conflict { path })) => {
                    let id = self.id.clone();
                    let answer = |answer| {
                        vec![Followup::Formats(FormatsFollowup::Conflict {
                            id: id.clone(),
                            answer,
                        })]
                    };
                    self.requests.push(DialogRequest::Ask(Box::new(Prompt::ask(
                        "Format Changed on Disk",
                        format!("{path} changed on disk since the editor read it."),
                        ButtonRole::Cancel,
                        vec![
                            (
                                ButtonRole::Other(1),
                                "Overwrite",
                                answer(ConflictAnswer::Overwrite),
                            ),
                            (
                                ButtonRole::Other(2),
                                "Reload (discard my edits)",
                                answer(ConflictAnswer::Reload),
                            ),
                            (ButtonRole::Cancel, "Cancel", answer(ConflictAnswer::Cancel)),
                        ],
                        TOPIC,
                    ))));
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        if self.ready() && self.format.kind == FormatKind::Text {
            let format = self.result().ok();
            changed |= self
                .preview
                .tick(svc, &self.wake, format.as_ref(), Instant::now());
        }
        changed
    }

    fn busy(&self) -> bool {
        self.load.is_some() || self.saving.is_some()
    }

    fn size(&self) -> Option<(f32, f32)> {
        Some((820.0, 520.0))
    }

    fn take_requests(&mut self) -> Vec<DialogRequest> {
        std::mem::take(&mut self.requests)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
