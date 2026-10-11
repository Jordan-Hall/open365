use std::path::{Path, PathBuf};

use open365_docx::{DocPos, Document, Snapshot};
use open365_opc::Limits;
use open365_xlsx::{ColumnsSnapshot, FormatSnapshot, RowHeightsSnapshot, Workbook};

use crate::command::{CellRef, Command, CommandLog, EffectClass, Outcome};
use crate::error::CommandError;
use crate::pdf::PdfOptions;
use crate::xlsx_state::{CellSnapshot, XlsxState};

#[derive(Debug)]
struct DocxState {
    doc: Document,
    /// The bytes the document was opened from; DOCX undo replays onto these.
    base: Vec<u8>,
}

#[derive(Debug)]
enum Content {
    Docx(Box<DocxState>),
    Xlsx(Box<XlsxState>),
}

#[derive(Debug)]
enum Inverse {
    /// Undo by reopening the base bytes and replaying the commands before this one. Only used
    /// when the affected paragraphs could not be captured.
    DocxReplay,
    /// Undo by putting the captured paragraphs back (newest capture last).
    DocxRestore(Vec<Snapshot>),
    /// Undo by writing the previous cell back; `before` is `None` when it cannot be
    /// restored.
    Cell {
        sheet: usize,
        row: u32,
        col: u32,
        before: Option<CellSnapshot>,
    },
    /// Undo by writing the previous cells back (newest last); a `None` cannot be restored.
    Cells(Vec<(CellRef, Option<CellSnapshot>)>),
    /// Undo by putting the whole workbook back (insert or delete of rows or columns).
    Workbook(Box<Workbook>),
    /// Undo by putting the previous `cols` element back.
    Columns {
        sheet: usize,
        before: ColumnsSnapshot,
    },
    /// Undo by putting the previous worksheet and style sheet back (a formatting action).
    Format(Box<FormatSnapshot>),
    /// A formatting action that changed nothing.
    Nothing,
    /// Undo by putting the previous row heights back.
    RowHeights {
        sheet: usize,
        before: RowHeightsSnapshot,
    },
}

#[derive(Debug)]
struct Entry {
    command: Command,
    inverse: Inverse,
    state_before: u64,
    state_after: u64,
}

/// An open document plus its command log and undo/redo history. All mutation goes
/// through [`apply`], [`Target::undo`] and [`Target::redo`].
#[derive(Debug)]
pub struct Target {
    content: Content,
    path: Option<PathBuf>,
    audit: CommandLog,
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    state: u64,
    next_state: u64,
    saved_state: u64,
    revision: u64,
    /// Edit-log entries the last edit added, taken by `apply`.
    notes: Vec<String>,
    /// Replacements the last replace-all made, taken by `apply`.
    replaced: usize,
    pdf_options: PdfOptions,
}

impl Target {
    fn new(content: Content, path: Option<PathBuf>) -> Self {
        Self {
            content,
            path,
            audit: CommandLog::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            state: 0,
            next_state: 1,
            saved_state: 0,
            revision: 0,
            notes: Vec::new(),
            replaced: 0,
            pdf_options: PdfOptions::default(),
        }
    }

    /// Sets the metadata, font policy and size limit `ExportPdf` uses. Not a document edit.
    pub fn set_pdf_options(&mut self, options: PdfOptions) {
        self.pdf_options = options;
    }

    pub fn open_docx(bytes: &[u8]) -> Result<Self, CommandError> {
        let doc = Document::open(bytes, &Limits::default())?;
        Ok(Self::new(
            Content::Docx(Box::new(DocxState {
                doc,
                base: bytes.to_vec(),
            })),
            None,
        ))
    }

    pub fn open_xlsx(bytes: &[u8]) -> Result<Self, CommandError> {
        let wb = Workbook::open(bytes, &Limits::default())?;
        Ok(Self::new(Content::Xlsx(Box::new(XlsxState::new(wb))), None))
    }

    /// Reads `path` and opens it as DOCX or XLSX by extension (case-insensitive): `.xlsx`,
    /// `.xlsm`, `.xltx` and `.xltm` are workbooks, anything else a Word document (`.docx`,
    /// `.docm`, `.dotx`, `.dotm`). A VBA project is kept byte for byte and never run.
    pub fn open_path(path: &Path) -> Result<Self, CommandError> {
        let bytes = std::fs::read(path).map_err(|source| CommandError::Io {
            op: "read",
            path: path.display().to_string(),
            source,
        })?;
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        let mut target = match ext.as_deref() {
            Some("xlsx" | "xlsm" | "xltx" | "xltm") => Self::open_xlsx(&bytes)?,
            _ => Self::open_docx(&bytes)?,
        };
        target.path = Some(path.to_path_buf());
        Ok(target)
    }

    /// The document's current content as package bytes, without touching its path, undo
    /// history or saved state. A copy to read from another thread can be reopened from them.
    ///
    /// # Errors
    /// The package cannot be written (for example an edit dropped content it cannot keep).
    pub fn snapshot_bytes(&mut self) -> Result<Vec<u8>, CommandError> {
        Ok(match &mut self.content {
            Content::Docx(s) => s.doc.save()?,
            Content::Xlsx(x) => x.workbook.save()?,
        })
    }

    pub fn docx(&self) -> Option<&Document> {
        match &self.content {
            Content::Docx(d) => Some(&d.doc),
            Content::Xlsx(_) => None,
        }
    }

    pub fn xlsx(&self) -> Option<&Workbook> {
        match &self.content {
            Content::Xlsx(x) => Some(&x.workbook),
            Content::Docx(_) => None,
        }
    }

    /// The file this document was opened from or last saved to.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Increases on every change to the document (apply, undo, redo). Views key caches
    /// on it.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// True when the document differs from what was last opened or saved.
    pub fn is_dirty(&self) -> bool {
        self.state != self.saved_state
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Every command ever applied successfully, in order, including saves. Append-only.
    pub fn audit_log(&self) -> &CommandLog {
        &self.audit
    }

    /// The commands currently in effect (applied and not undone). Replaying them on the
    /// same file reproduces the document; `Save` is not part of it.
    pub fn applied_log(&self) -> CommandLog {
        CommandLog(self.undo.iter().map(|e| e.command.clone()).collect())
    }

    /// Applies one command. A failed command changes nothing and is not logged.
    pub fn apply(&mut self, command: Command) -> Result<Outcome, CommandError> {
        let effect = command.effect(self.path.as_deref());
        let outcome = if let Command::Save { path } = &command {
            self.save_to(path, effect)?
        } else if let Command::ExportPdf { path, overwrite } = &command {
            self.export_pdf_to(path, *overwrite, effect)?
        } else {
            self.notes.clear();
            self.replaced = 0;
            let (inverse, changed_cells) = self.run_edit(&command)?;
            let state_before = self.state;
            self.state = self.next_state;
            self.next_state += 1;
            self.revision += 1;
            self.undo.push(Entry {
                command: command.clone(),
                inverse,
                state_before,
                state_after: self.state,
            });
            self.redo.clear();
            Outcome {
                effect,
                changed_cells,
                saved_to: None,
                bytes_written: 0,
                pages: 0,
                notes: std::mem::take(&mut self.notes),
                replaced: std::mem::take(&mut self.replaced),
            }
        };
        self.audit.0.push(command);
        Ok(outcome)
    }

    pub fn undo(&mut self) -> Result<Outcome, CommandError> {
        let Some(entry) = self.undo.last() else {
            return Err(CommandError::NothingToUndo);
        };
        let changed = match &entry.inverse {
            Inverse::DocxReplay => {
                let remaining: Vec<&Command> = self.undo[..self.undo.len() - 1]
                    .iter()
                    .map(|e| &e.command)
                    .collect();
                let Content::Docx(state) = &mut self.content else {
                    return Err(CommandError::NotUndoable);
                };
                let mut doc = Document::open(&state.base, &Limits::default())?;
                for c in remaining {
                    replay_docx(&mut doc, c)?;
                }
                state.doc = doc;
                Vec::new()
            }
            Inverse::DocxRestore(snapshots) => {
                let Content::Docx(state) = &mut self.content else {
                    return Err(CommandError::NotUndoable);
                };
                for snapshot in snapshots.iter().rev() {
                    state.doc.restore(snapshot)?;
                }
                Vec::new()
            }
            Inverse::Cell {
                sheet,
                row,
                col,
                before,
            } => {
                let Some(before) = before else {
                    return Err(CommandError::NotUndoable);
                };
                let Content::Xlsx(x) = &mut self.content else {
                    return Err(CommandError::NotUndoable);
                };
                x.restore(*sheet, *row, *col, before)?
            }
            Inverse::Cells(before) => {
                let Content::Xlsx(x) = &mut self.content else {
                    return Err(CommandError::NotUndoable);
                };
                x.undo_many(before)?
            }
            Inverse::Workbook(before) => {
                let Content::Xlsx(x) = &mut self.content else {
                    return Err(CommandError::NotUndoable);
                };
                x.restore_workbook((**before).clone());
                Vec::new()
            }
            Inverse::Columns { sheet, before } => {
                let Content::Xlsx(x) = &mut self.content else {
                    return Err(CommandError::NotUndoable);
                };
                x.workbook.restore_columns(*sheet, before)?;
                Vec::new()
            }
            Inverse::RowHeights { sheet, before } => {
                let Content::Xlsx(x) = &mut self.content else {
                    return Err(CommandError::NotUndoable);
                };
                x.workbook.restore_row_heights(*sheet, before)?;
                Vec::new()
            }
            Inverse::Format(before) => {
                let Content::Xlsx(x) = &mut self.content else {
                    return Err(CommandError::NotUndoable);
                };
                x.workbook.restore_format((**before).clone())?;
                Vec::new()
            }
            Inverse::Nothing => Vec::new(),
        };
        if let Some(entry) = self.undo.pop() {
            self.state = entry.state_before;
            self.redo.push(entry);
        }
        self.revision += 1;
        Ok(edit_outcome(changed))
    }

    pub fn redo(&mut self) -> Result<Outcome, CommandError> {
        let Some(entry) = self.redo.last() else {
            return Err(CommandError::NothingToRedo);
        };
        let command = entry.command.clone();
        let (inverse, changed_cells) = self.run_edit(&command)?;
        if let Some(mut entry) = self.redo.pop() {
            entry.inverse = inverse;
            self.state = entry.state_after;
            self.undo.push(entry);
        }
        self.revision += 1;
        Ok(edit_outcome(changed_cells))
    }

    /// Runs an edit command against the content and captures how to reverse it.
    fn run_edit(&mut self, command: &Command) -> Result<(Inverse, Vec<CellRef>), CommandError> {
        match (&mut self.content, command) {
            (Content::Docx(s), command) if is_docx_edit(command) => {
                let inverse = match capture_docx(&s.doc, command) {
                    Ok(snapshots) => Inverse::DocxRestore(snapshots),
                    Err(_) => Inverse::DocxReplay,
                };
                let log_len = s.doc.edit_log().len();
                self.replaced = replay_docx(&mut s.doc, command)?;
                self.notes = s.doc.edit_log()[log_len..]
                    .iter()
                    .map(|e| e.description.clone())
                    .collect();
                Ok((inverse, Vec::new()))
            }
            (Content::Xlsx(x), Command::XlsxReplaceAll { find, replacement }) => {
                let plan = crate::find::xlsx_replace_plan(&x.workbook, find, replacement);
                let entries: Vec<(CellRef, String)> =
                    plan.iter().map(|p| (p.cell, p.text.clone())).collect();
                let done = x.enter_many(&entries, true)?;
                self.replaced = done.applied.iter().map(|&i| plan[i].occurrences).sum();
                Ok((Inverse::Cells(done.before), done.changed))
            }
            (
                Content::Xlsx(x),
                Command::XlsxPaste {
                    sheet,
                    row,
                    col,
                    rows,
                },
            ) => {
                let entries = paste_entries(*sheet, *row, *col, rows)?;
                let done = x.enter_many(&entries, false)?;
                Ok((Inverse::Cells(done.before), done.changed))
            }
            (
                Content::Xlsx(x),
                Command::XlsxSetCellValue {
                    sheet,
                    row,
                    col,
                    value,
                },
            ) => {
                let before = x.capture(*sheet, *row, *col);
                let changed = x.set_value(*sheet, *row, *col, value)?;
                Ok((
                    Inverse::Cell {
                        sheet: *sheet,
                        row: *row,
                        col: *col,
                        before,
                    },
                    changed,
                ))
            }
            (
                Content::Xlsx(x),
                Command::XlsxSetCellFormula {
                    sheet,
                    row,
                    col,
                    formula,
                },
            ) => {
                let before = x.capture(*sheet, *row, *col);
                let changed = x.set_formula(*sheet, *row, *col, formula)?;
                Ok((
                    Inverse::Cell {
                        sheet: *sheet,
                        row: *row,
                        col: *col,
                        before,
                    },
                    changed,
                ))
            }
            (Content::Xlsx(x), command) if crate::structure::edit_of(command).is_some() => {
                let Some((sheet, edit)) = crate::structure::edit_of(command) else {
                    return Err(CommandError::NotUndoable);
                };
                let before = Box::new(x.workbook.clone());
                let (changed, notes) = x.edit_structure(sheet, edit)?;
                self.notes = notes;
                Ok((Inverse::Workbook(before), changed))
            }
            (
                Content::Xlsx(x),
                Command::SetColumnWidth {
                    sheet,
                    first,
                    last,
                    width,
                },
            ) => {
                let before = x.workbook.set_column_width(*sheet, *first, *last, *width)?;
                Ok((
                    Inverse::Columns {
                        sheet: *sheet,
                        before,
                    },
                    Vec::new(),
                ))
            }
            (
                Content::Xlsx(x),
                Command::SetRowHeight {
                    sheet,
                    first,
                    last,
                    points,
                },
            ) => {
                let before = x.workbook.set_row_height(*sheet, *first, *last, *points)?;
                Ok((
                    Inverse::RowHeights {
                        sheet: *sheet,
                        before,
                    },
                    Vec::new(),
                ))
            }
            (Content::Xlsx(x), Command::HideColumns { sheet, first, last }) => {
                let before = x.workbook.hide_columns(*sheet, *first, *last)?;
                Ok((
                    Inverse::Columns {
                        sheet: *sheet,
                        before,
                    },
                    Vec::new(),
                ))
            }
            (
                Content::Xlsx(x),
                Command::UnhideColumns {
                    sheet,
                    first,
                    last,
                    default_width,
                },
            ) => {
                let before = x
                    .workbook
                    .unhide_columns(*sheet, *first, *last, *default_width)?;
                Ok((
                    Inverse::Columns {
                        sheet: *sheet,
                        before,
                    },
                    Vec::new(),
                ))
            }
            (Content::Xlsx(x), Command::HideRows { sheet, first, last }) => {
                let before = x.workbook.hide_rows(*sheet, *first, *last)?;
                Ok((
                    Inverse::RowHeights {
                        sheet: *sheet,
                        before,
                    },
                    Vec::new(),
                ))
            }
            (Content::Xlsx(x), Command::UnhideRows { sheet, first, last }) => {
                let before = x.workbook.unhide_rows(*sheet, *first, *last)?;
                Ok((
                    Inverse::RowHeights {
                        sheet: *sheet,
                        before,
                    },
                    Vec::new(),
                ))
            }
            (Content::Xlsx(x), command) if crate::format::is_format_command(command) => {
                let before = crate::format::run(&mut x.workbook, command)?;
                Ok((
                    before.map_or(Inverse::Nothing, |b| Inverse::Format(Box::new(b))),
                    Vec::new(),
                ))
            }
            (content, command) => Err(CommandError::WrongTarget {
                command: command.name(),
                target: match content {
                    Content::Docx(_) => "DOCX",
                    Content::Xlsx(_) => "XLSX",
                },
            }),
        }
    }

    fn save_to(&mut self, path: &Path, effect: EffectClass) -> Result<Outcome, CommandError> {
        let bytes = match &mut self.content {
            Content::Docx(s) => s.doc.save()?,
            Content::Xlsx(x) => x.workbook.save()?,
        };
        write_atomic(path, &bytes)?;
        self.path = Some(path.to_path_buf());
        self.saved_state = self.state;
        Ok(Outcome {
            effect,
            changed_cells: Vec::new(),
            saved_to: Some(path.to_path_buf()),
            bytes_written: bytes.len(),
            pages: 0,
            notes: Vec::new(),
            replaced: 0,
        })
    }

    fn export_pdf_to(
        &self,
        path: &Path,
        overwrite: bool,
        effect: EffectClass,
    ) -> Result<Outcome, CommandError> {
        let Content::Docx(s) = &self.content else {
            return Err(CommandError::WrongTarget {
                command: "export_pdf",
                target: "XLSX",
            });
        };
        let rendered = crate::pdf::render(&s.doc, &self.pdf_options)?;
        crate::pdf::write_pdf(path, &rendered.bytes, overwrite, &self.pdf_options)?;
        Ok(Outcome {
            effect,
            changed_cells: Vec::new(),
            saved_to: Some(path.to_path_buf()),
            bytes_written: rendered.bytes.len(),
            pages: rendered.pages,
            notes: rendered.warnings,
            replaced: 0,
        })
    }
}

fn edit_outcome(changed_cells: Vec<CellRef>) -> Outcome {
    Outcome {
        effect: EffectClass::Edit,
        changed_cells,
        saved_to: None,
        bytes_written: 0,
        pages: 0,
        notes: Vec::new(),
        replaced: 0,
    }
}

/// Applies one command to a target. The single mutation entry point (ADR-002).
pub fn apply(target: &mut Target, command: Command) -> Result<Outcome, CommandError> {
    target.apply(command)
}

fn is_docx_edit(command: &Command) -> bool {
    matches!(
        command,
        Command::DocxInsertText { .. }
            | Command::DocxSetBold { .. }
            | Command::DocxDeleteRange { .. }
            | Command::DocxSplitParagraph { .. }
            | Command::DocxSetRunProps { .. }
            | Command::DocxSetParagraphStyle { .. }
            | Command::DocxSetAlignment { .. }
            | Command::DocxReplaceRange { .. }
            | Command::DocxReplaceAll { .. }
    )
}

fn ordered(a: DocPos, b: DocPos) -> (DocPos, DocPos) {
    if a <= b { (a, b) } else { (b, a) }
}

/// Saves the paragraphs `command` is going to change, for a cheap undo. Commands that keep
/// the paragraph count (formatting) save each paragraph on its own so a selection may span
/// table cells; commands that merge or split paragraphs save one contiguous range.
fn capture_docx(doc: &Document, command: &Command) -> Result<Vec<Snapshot>, CommandError> {
    let one = |para: usize| doc.capture(para, para);
    let snapshots = match command {
        Command::DocxInsertText { para, .. }
        | Command::DocxSetBold { para, .. }
        | Command::DocxSetParagraphStyle { para, .. }
        | Command::DocxSetAlignment { para, .. } => vec![one(*para)?],
        Command::DocxSplitParagraph { pos } => vec![one(pos.para)?],
        Command::DocxDeleteRange { start, end } | Command::DocxReplaceRange { start, end, .. } => {
            let (s, e) = ordered(*start, *end);
            vec![doc.capture(s.para, e.para)?]
        }
        Command::DocxSetRunProps { start, end, .. } => {
            let (s, e) = ordered(*start, *end);
            (s.para..=e.para).map(one).collect::<Result<Vec<_>, _>>()?
        }
        // Replacements stay inside their paragraph, so each affected paragraph is saved alone.
        Command::DocxReplaceAll {
            find,
            match_case,
            whole_word,
            wildcards,
            ..
        } => {
            let opts = open365_docx::FindOptions {
                pattern: find.clone(),
                match_case: *match_case,
                whole_word: *whole_word,
                wildcards: *wildcards,
            };
            let mut paras: Vec<usize> = doc.find_all(&opts)?.iter().map(|m| m.para).collect();
            paras.dedup();
            paras.into_iter().map(one).collect::<Result<Vec<_>, _>>()?
        }
        _ => return Err(CommandError::NotUndoable),
    };
    Ok(snapshots)
}

/// Replays a DOCX edit; gives the number of replacements for a replace-all, else 0.
fn replay_docx(doc: &mut Document, command: &Command) -> Result<usize, CommandError> {
    match command {
        Command::DocxReplaceAll {
            find,
            replacement,
            match_case,
            whole_word,
            wildcards,
        } => {
            let opts = open365_docx::FindOptions {
                pattern: find.clone(),
                match_case: *match_case,
                whole_word: *whole_word,
                wildcards: *wildcards,
            };
            return Ok(doc.replace_all(&opts, replacement)?);
        }
        Command::DocxInsertText { para, offset, text } => doc.insert_text(*para, *offset, text)?,
        Command::DocxSetBold {
            para,
            start,
            end,
            bold,
        } => doc.set_bold(*para, *start..*end, *bold)?,
        Command::DocxDeleteRange { start, end } => doc.delete_range(*start, *end)?,
        Command::DocxSplitParagraph { pos } => doc.split_paragraph(*pos)?,
        Command::DocxSetRunProps { start, end, patch } => doc.set_run_props(*start, *end, patch)?,
        Command::DocxSetParagraphStyle { para, style_id } => {
            doc.set_paragraph_style(*para, style_id)?;
        }
        Command::DocxSetAlignment { para, alignment } => doc.set_alignment(*para, *alignment)?,
        Command::DocxReplaceRange { start, end, text } => {
            doc.replace_range(*start, *end, text)?;
        }
        _ => {}
    }
    Ok(0)
}

/// Largest paste, in cells.
const MAX_PASTE_CELLS: usize = 1_000_000;

/// The cell/text pairs of a pasted block whose top-left cell is `(row, col)`.
fn paste_entries(
    sheet: usize,
    row: u32,
    col: u32,
    rows: &[Vec<String>],
) -> Result<Vec<(CellRef, String)>, CommandError> {
    let cells: usize = rows.iter().map(Vec::len).sum();
    if cells > MAX_PASTE_CELLS {
        return Err(CommandError::Paste(format!(
            "{cells} cells is more than the {MAX_PASTE_CELLS} a paste may hold"
        )));
    }
    let mut out = Vec::with_capacity(cells);
    for (dr, line) in rows.iter().enumerate() {
        for (dc, text) in line.iter().enumerate() {
            let (Some(r), Some(c)) = (
                u32::try_from(dr).ok().and_then(|d| row.checked_add(d)),
                u32::try_from(dc).ok().and_then(|d| col.checked_add(d)),
            ) else {
                return Err(CommandError::Paste(
                    "the block runs off the sheet".to_owned(),
                ));
            };
            if r > open365_xlsx::MAX_ROW || c > open365_xlsx::MAX_COL {
                return Err(CommandError::Paste(
                    "the block runs off the sheet".to_owned(),
                ));
            }
            out.push((
                CellRef {
                    sheet,
                    row: r,
                    col: c,
                },
                text.clone(),
            ));
        }
    }
    Ok(out)
}

/// Writes to a sibling temporary file, then renames over `path`, so a failed write never
/// leaves a truncated document.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), CommandError> {
    let io = |op: &'static str, source| CommandError::Io {
        op,
        path: path.display().to_string(),
        source,
    };
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".open365-tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes).map_err(|e| io("write", e))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io("rename", e)
    })
}
