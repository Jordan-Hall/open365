//! Writer pane: the Mailings tab. Mail merge state (type, recipient list, field matching,
//! preview), merge fields and rules, Address Block and Greeting Line, the merge into a new
//! document, envelopes and labels.
//!
//! Word asks for options in dialogs first (Select Table, Insert Address Block, Envelope Options).
//! The pane has no dialog host for them yet, so each method takes Word's defaults and a dialog
//! can call the same method with the person's choices.
//!
//! The merge state is per thread like the other tab states, not per document. A merged result
//! (Edit Individual Documents, a label sheet for Create > Labels) goes to the outbox; the shell
//! opens its bytes as a new document with [`WriterPane::take_new_documents`].

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use open365_commands::mailings_ops::{
    self, EnvelopeSize, FieldMap, LabelDef, MAP_PARTS, MergeKind, Recipients,
};
use open365_commands::{Command, MailingsOp, Target};
use open365_docx::DocPos;

use super::WriterPane;

/// Chooses an existing recipient file; `None` when cancelled.
pub type ListPicker = fn() -> Option<PathBuf>;

fn pick_list() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Recipient list", &["csv", "txt", "tsv", "xlsx"])
        .set_title("Select Data Source")
        .pick_file()
}

/// The merge state the ribbon shows.
pub struct MailState {
    /// The merge type chosen in Start Mail Merge; `None` before a merge started.
    pub kind: Option<MergeKind>,
    pub recipients: Option<Recipients>,
    pub map: FieldMap,
    /// Preview Results is on.
    pub preview: bool,
    /// Position in the list of ticked records (0-based) the preview shows.
    pub record: usize,
    pub highlight: bool,
    /// Set by Edit Recipient List / Match Fields for a dialog to pick up.
    pub dialog_requested: Option<&'static str>,
    picker: ListPicker,
    /// The real document while a preview replaces it.
    saved: Option<Target>,
    outbox: Vec<Vec<u8>>,
}

impl std::fmt::Debug for MailState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MailState")
            .field("kind", &self.kind)
            .field("preview", &self.preview)
            .field("record", &self.record)
            .finish_non_exhaustive()
    }
}

impl Default for MailState {
    fn default() -> Self {
        Self {
            kind: None,
            recipients: None,
            map: FieldMap::default(),
            preview: false,
            record: 0,
            highlight: false,
            dialog_requested: None,
            picker: pick_list,
            saved: None,
            outbox: Vec::new(),
        }
    }
}

thread_local! {
    static STATE: RefCell<MailState> = RefCell::new(MailState::default());
}

/// The text of a recipient file as `Recipients` reads it: `.xlsx` first sheet, anything else
/// as delimited text.
fn read_list(path: &Path) -> Result<Recipients, String> {
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase());
    if ext.as_deref() == Some("xlsx") {
        let sheets = Recipients::xlsx_sheets(&bytes).map_err(|e| e.to_string())?;
        let first = (0..sheets.len())
            .find(|i| Recipients::from_xlsx(&bytes, *i, &name).is_ok())
            .ok_or_else(|| "the workbook has no worksheet".to_owned())?;
        return Recipients::from_xlsx(&bytes, first, &name).map_err(|e| e.to_string());
    }
    Ok(Recipients::from_csv(
        &String::from_utf8_lossy(&bytes),
        &name,
    ))
}

impl WriterPane {
    /// Reads or changes the merge state.
    pub fn mail<R>(&self, f: impl FnOnce(&mut MailState) -> R) -> R {
        STATE.with(|s| f(&mut s.borrow_mut()))
    }

    /// A merge type was chosen.
    #[must_use]
    pub fn mail_started(&self) -> bool {
        self.mail(|m| m.kind.is_some())
    }

    /// A recipient list is attached.
    #[must_use]
    pub fn mail_has_recipients(&self) -> bool {
        self.mail(|m| m.recipients.is_some())
    }

    /// The columns of the recipient list.
    #[must_use]
    pub fn merge_columns(&self) -> Vec<String> {
        self.mail(|m| {
            m.recipients
                .as_ref()
                .map(|r| r.columns.clone())
                .unwrap_or_default()
        })
    }

    /// The columns of the attached list, for menus that have no pane at hand.
    #[must_use]
    pub fn current_merge_columns() -> Vec<String> {
        STATE.with(|s| {
            s.borrow()
                .recipients
                .as_ref()
                .map(|r| r.columns.clone())
                .unwrap_or_default()
        })
    }

    /// Lets a test or dialog choose the recipient file without a file chooser.
    pub fn set_list_picker(&mut self, picker: ListPicker) {
        self.mail(|m| m.picker = picker);
    }

    /// Ends the merge state (a new document, Normal Document).
    pub fn mail_reset(&mut self) {
        self.stop_preview();
        self.mail(|m| *m = MailState::default());
    }

    // ---- start and recipients ----------------------------------------------------------------

    /// Start Mail Merge: chooses the merge type. Envelopes and Labels also lay the page out
    /// (Word asks for the size first; the defaults here are Size 10 and Avery 5160). Normal
    /// Document ends the merge.
    pub fn start_mail_merge(&mut self, kind: MergeKind) -> bool {
        self.stop_preview();
        if kind == MergeKind::Normal {
            self.mail(|m| {
                m.kind = None;
                m.preview = false;
            });
            self.status = Some("Normal document".to_owned());
            return true;
        }
        let laid_out = match kind {
            MergeKind::Envelopes => self.insert_envelope("", "", &EnvelopeSize::named("Size 10")),
            MergeKind::Labels => self.apply_mailings(MailingsOp::LabelSheet {
                def: LabelDef::named("5160"),
                content: Vec::new(),
                merge: true,
            }),
            _ => true,
        };
        if laid_out {
            self.mail(|m| {
                m.kind = Some(kind);
                m.record = 0;
            });
            self.status = Some(format!("Mail merge: {}", kind.label()));
        }
        laid_out
    }

    fn apply_mailings(&mut self, op: MailingsOp) -> bool {
        self.run(Command::DocxMailings { op }).is_some()
    }

    /// Forgets the layout of the former document after the target was swapped.
    fn fresh_layout(&mut self) {
        self.revision = None;
        self.engine = super::IncrementalLayout::new();
    }

    /// Characters in paragraph `para`.
    fn mail_para_len(&self, para: usize) -> usize {
        self.paragraph_text(para).map_or(0, |t| t.chars().count())
    }

    /// Runs `op` for a field at the caret and puts the caret after what it inserted.
    fn insert_at_caret(&mut self, op: impl FnOnce(DocPos) -> MailingsOp) -> bool {
        let at = self.selection().1;
        let before = self.mail_para_len(at.para);
        let ok = self.apply_mailings(op(at));
        if ok {
            let grown = self.mail_para_len(at.para).saturating_sub(before);
            let to = self.clamp(DocPos {
                para: at.para,
                offset: at.offset + grown,
            });
            self.caret = to;
            self.anchor = to;
        }
        ok
    }

    /// Attaches `list` as the data source; the fields are matched to its columns.
    pub fn use_recipients(&mut self, list: Recipients) {
        self.stop_preview();
        let n = list.active().len();
        self.mail(|m| {
            m.map = FieldMap::auto(&list);
            m.recipients = Some(list);
            m.record = 0;
        });
        self.status = Some(format!("{n} recipients"));
    }

    /// Use an Existing List: reads a CSV/TSV/text file or the first worksheet of an `.xlsx`.
    pub fn use_recipients_file(&mut self, path: &Path) -> bool {
        match read_list(path) {
            Ok(list) => {
                self.use_recipients(list);
                true
            }
            Err(e) => {
                self.status = Some(format!("Could not read the list: {e}"));
                false
            }
        }
    }

    /// Use an Existing List with the file chooser.
    pub fn choose_recipients(&mut self) -> bool {
        match self.mail(|m| m.picker)() {
            Some(path) => self.use_recipients_file(&path),
            None => false,
        }
    }

    /// Type a New List: Word's columns and one empty record.
    pub fn new_recipient_list(&mut self) {
        self.use_recipients(Recipients::blank());
        self.mail(|m| m.dialog_requested = Some("edit-recipient-list"));
    }

    /// Changes the recipient list (Edit Recipient List) and re-matches the address parts.
    pub fn edit_recipients(&mut self, f: impl FnOnce(&mut Recipients)) -> bool {
        self.mail(|m| {
            let Some(list) = m.recipients.as_mut() else {
                return false;
            };
            f(list);
            let keep = m.map.clone();
            let mut map = FieldMap::auto(list);
            for (key, col) in keep.0 {
                if col.as_ref().is_some_and(|c| list.column(c).is_some()) {
                    map.set(&key, col.as_deref());
                }
            }
            m.map = map;
            let n = list.active().len();
            m.record = m.record.min(n.saturating_sub(1));
            true
        })
    }

    /// Match Fields: maps an address part (`_FIRST0_`...) to a column, or clears it.
    pub fn match_field(&mut self, key: &str, column: Option<&str>) -> bool {
        self.mail(|m| {
            if m.recipients.is_none() || !MAP_PARTS.iter().any(|(k, _, _)| *k == key) {
                return false;
            }
            m.map.set(key, column);
            true
        })
    }

    // ---- fields and rules --------------------------------------------------------------------

    fn field_at_caret(&mut self, instruction: &str, placeholder: &str) -> bool {
        self.insert_at_caret(|at| MailingsOp::InsertField {
            at,
            instruction: instruction.to_owned(),
            placeholder: placeholder.to_owned(),
        })
    }

    fn runs_at_caret(&mut self, xml: String) -> bool {
        self.insert_at_caret(|at| MailingsOp::InsertRuns { at, xml })
    }

    /// Insert Merge Field: a `MERGEFIELD` for `column` at the caret.
    pub fn insert_merge_field(&mut self, column: &str) -> bool {
        if !self.merge_columns().iter().any(|c| c == column) {
            self.status = Some(format!("The list has no field {column}"));
            return false;
        }
        self.field_at_caret(
            &mailings_ops::merge_field_instruction(column),
            &format!("\u{ab}{column}\u{bb}"),
        )
    }

    /// Address Block with Word's default format.
    pub fn insert_address_block(&mut self) -> bool {
        let format = "<<_TITLE0_>> <<_FIRST0_>> <<_LAST0_>> <<_SUFFIX0_>>\n<<_COMPANY_>>\n<<_STREET1_>>\n<<_STREET2_>>\n<<_CITY_>>, <<_STATE_>> <<_POSTAL_>>\n<<_COUNTRY_>>";
        self.field_at_caret(
            &format!("ADDRESSBLOCK \\f \"{format}\" \\e"),
            "\u{ab}AddressBlock\u{bb}",
        )
    }

    /// Greeting Line: "Dear Mr. Smith," with the fallback "Dear Sir or Madam,".
    pub fn insert_greeting_line(&mut self) -> bool {
        self.field_at_caret(
            "GREETINGLINE \\f \"Dear <<_TITLE0_>> <<_LAST0_>>,\" \\e \"Dear Sir or Madam,\"",
            "\u{ab}GreetingLine\u{bb}",
        )
    }

    /// Rules menu: inserts the rule with the values its dialog starts with. Comparisons use the
    /// first column of the list, "Equal to" and an empty text.
    pub fn insert_rule(&mut self, rule: &str) -> bool {
        let Some(first) = self.merge_columns().into_iter().next() else {
            return false;
        };
        match rule {
            "ask" => self.field_at_caret("ASK Bookmark1 \"Enter text\" \\o", ""),
            "fill-in" => self.field_at_caret("FILLIN \"Enter text\"", ""),
            "if-then-else" => {
                let xml = mailings_ops::nested_field("IF", &first, "= \"\" \"\" \"\"", "");
                self.runs_at_caret(xml)
            }
            "merge-record" => self.field_at_caret("MERGEREC", "1"),
            "merge-sequence" => self.field_at_caret("MERGESEQ", "1"),
            "next-record" => self.field_at_caret("NEXT", "\u{ab}Next Record\u{bb}"),
            "next-record-if" => {
                let xml = mailings_ops::nested_field(
                    "NEXTIF",
                    &first,
                    "= \"\"",
                    "\u{ab}Next Record If\u{bb}",
                );
                self.runs_at_caret(xml)
            }
            "set-bookmark" => self.field_at_caret("SET Bookmark1 \"Text\"", "Text"),
            "skip-record-if" => {
                let xml = mailings_ops::nested_field(
                    "SKIPIF",
                    &first,
                    "= \"\"",
                    "\u{ab}Skip Record If\u{bb}",
                );
                self.runs_at_caret(xml)
            }
            _ => false,
        }
    }

    /// Highlight Merge Fields on or off (a display option; the canvas reads [`MailState`]).
    pub fn toggle_highlight_merge_fields(&mut self) {
        self.mail(|m| m.highlight = !m.highlight);
    }

    // ---- preview -----------------------------------------------------------------------------

    fn merge_source(&mut self) -> Option<Vec<u8>> {
        if let Some(mut saved) = self.mail(|m| m.saved.take()) {
            let bytes = saved.snapshot_bytes().ok();
            self.mail(|m| m.saved = Some(saved));
            bytes
        } else {
            self.target.snapshot_bytes().ok()
        }
    }

    /// The records a merge or preview starts from: positions in the list, ticked ones only.
    fn merge_order(&self) -> Vec<usize> {
        self.mail(|m| {
            m.recipients
                .as_ref()
                .map(Recipients::active)
                .unwrap_or_default()
        })
    }

    /// Number of ticked records.
    #[must_use]
    pub fn merge_record_count(&self) -> usize {
        self.merge_order().len()
    }

    fn merged_bytes(&mut self, order: &[usize]) -> Option<mailings_ops::MergeOutput> {
        let source = self.merge_source()?;
        let (rec, map, kind) = self.mail(|m| {
            (
                m.recipients.clone(),
                m.map.clone(),
                m.kind.unwrap_or_default(),
            )
        });
        let rec = rec?;
        match mailings_ops::merge_documents(
            &source,
            &rec,
            &map,
            order,
            kind,
            super::ext_insert::now_utc(),
        ) {
            Ok(out) => Some(out),
            Err(e) => {
                self.status = Some(format!("Merge failed: {e}"));
                None
            }
        }
    }

    fn show_record(&mut self) -> bool {
        let order = self.merge_order();
        let at = self.mail(|m| m.record).min(order.len().saturating_sub(1));
        // A label sheet shows from the record on; other types show one record.
        let slice: Vec<usize> = if self.mail(|m| m.kind) == Some(MergeKind::Labels) {
            order[at..].to_vec()
        } else {
            order.get(at).copied().into_iter().collect()
        };
        if slice.is_empty() {
            return false;
        }
        let Some(out) = self.merged_bytes(&slice) else {
            return false;
        };
        let Ok(preview) = Target::open_docx(&out.bytes) else {
            return false;
        };
        let original = std::mem::replace(&mut self.target, preview);
        self.fresh_layout();
        // The first preview keeps the real document; later ones replace the previous result.
        self.mail(|m| {
            if m.saved.is_none() {
                m.saved = Some(original);
            }
            m.preview = true;
        });
        self.sync();
        self.status = Some(format!("Preview: record {} of {}", at + 1, order.len()));
        true
    }

    /// Preview Results on or off. While it is on the document shows the merged record;
    /// changes made then are not kept.
    pub fn toggle_preview(&mut self) -> bool {
        if self.mail(|m| m.preview) {
            self.stop_preview();
            return true;
        }
        if !self.mail_has_recipients() || self.merge_order().is_empty() {
            self.status = Some("Choose recipients first".to_owned());
            return false;
        }
        self.show_record()
    }

    /// Puts the real document back after a preview.
    pub fn stop_preview(&mut self) {
        if let Some(saved) = self.mail(|m| {
            m.preview = false;
            m.saved.take()
        }) {
            self.target = saved;
            self.fresh_layout();
            self.sync();
        }
    }

    /// Goes to record `n` (1-based among the ticked records) and shows it when previewing.
    pub fn goto_record(&mut self, n: usize) -> bool {
        let count = self.merge_order().len();
        if count == 0 {
            return false;
        }
        let at = n.clamp(1, count) - 1;
        self.mail(|m| m.record = at);
        if self.mail(|m| m.preview) {
            return self.show_record();
        }
        true
    }

    /// The record shown (1-based).
    #[must_use]
    pub fn merge_record(&self) -> usize {
        self.mail(|m| m.record) + 1
    }

    /// Find Recipient: the next record (after the current one, wrapping) containing `text` in
    /// any field, or in `column` when given. Moves to it.
    pub fn find_recipient(&mut self, text: &str, column: Option<&str>) -> bool {
        let order = self.merge_order();
        let found = self.mail(|m| {
            let rec = m.recipients.as_ref()?;
            let col = match column {
                Some(c) => Some(rec.column(c)?),
                None => None,
            };
            let from = m.record + 1;
            (0..order.len())
                .map(|k| (from + k) % order.len())
                .find(|i| rec.find(text, col, order[*i]) == Some(order[*i]))
        });
        match found {
            Some(i) => self.goto_record(i + 1),
            None => {
                self.status = Some(format!("Cannot find \"{text}\""));
                false
            }
        }
    }

    /// Check for Errors: the problems a merge would meet. Empty when there are none.
    pub fn check_merge_errors(&mut self) -> Vec<String> {
        let mut problems = Vec::new();
        let Some(columns) = self.mail(|m| m.recipients.as_ref().map(|r| r.columns.clone())) else {
            problems.push("No data source is attached.".to_owned());
            return problems;
        };
        if self.merge_order().is_empty() {
            problems.push("No recipient is selected.".to_owned());
        }
        let fields = self
            .merge_source()
            .and_then(|b| Target::open_docx(&b).ok())
            .and_then(|t| t.docx().map(open365_docx::Document::fields))
            .unwrap_or_default();
        for f in fields.iter().filter(|f| f.instruction.name == "MERGEFIELD") {
            if let Some(name) = f.instruction.first_arg()
                && !columns.iter().any(|c| c.eq_ignore_ascii_case(name))
            {
                problems.push(format!("The field {name} is not in the data source."));
            }
        }
        self.status = Some(
            problems
                .first()
                .cloned()
                .unwrap_or_else(|| "No errors found".to_owned()),
        );
        problems
    }

    // ---- finish ------------------------------------------------------------------------------

    /// Finish & Merge > Edit Individual Documents (all ticked records): the merged document goes
    /// to the outbox. `from` and `to` (1-based) restrict the records, as the dialog's From/To do.
    pub fn finish_merge(&mut self, range: Option<(usize, usize)>) -> bool {
        let all = self.merge_order();
        if all.is_empty() {
            self.status = Some("There are no recipients to merge".to_owned());
            return false;
        }
        let order: Vec<usize> = match range {
            Some((from, to)) => all
                .iter()
                .copied()
                .skip(from.max(1) - 1)
                .take(to.saturating_sub(from.max(1)) + 1)
                .collect(),
            None => all,
        };
        let Some(out) = self.merged_bytes(&order) else {
            return false;
        };
        self.status = Some(format!("Merged {} records", out.merged));
        self.mail(|m| m.outbox.push(out.bytes));
        true
    }

    /// The documents merges and label sheets produced; the shell opens each as a new document.
    pub fn take_new_documents(&mut self) -> Vec<Vec<u8>> {
        self.mail(|m| std::mem::take(&mut m.outbox))
    }

    // ---- envelopes and labels ----------------------------------------------------------------

    /// The selected text as address lines.
    fn selected_address(&self) -> String {
        self.selected_text()
            .replace(['\r', '\u{b}'], "\n")
            .trim()
            .to_owned()
    }

    /// Envelopes > Add to Document: the addresses in a new first section of the envelope's size.
    pub fn insert_envelope(
        &mut self,
        delivery: &str,
        return_address: &str,
        size: &EnvelopeSize,
    ) -> bool {
        self.apply_mailings(MailingsOp::Envelope {
            delivery: delivery.to_owned(),
            return_address: return_address.to_owned(),
            size: size.clone(),
        })
    }

    /// Create > Envelopes with the dialog's defaults: the selected text is the delivery address,
    /// Size 10, no return address.
    pub fn create_envelope(&mut self) -> bool {
        let delivery = self.selected_address();
        self.insert_envelope(&delivery, "", &EnvelopeSize::named("Size 10"))
    }

    /// Create > Labels > New Document: a full sheet of the label `address` (the selected text by
    /// default) on `def`; the sheet goes to the outbox.
    pub fn create_labels(&mut self, address: Option<&str>, def: &LabelDef) -> bool {
        let address = address.map_or_else(|| self.selected_address(), str::to_owned);
        let content: Vec<String> = address
            .lines()
            .map(|l| format!("<w:p><w:pPr><w:spacing w:after=\"0\"/></w:pPr><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>", open365_docx::escape_xml(l)))
            .collect();
        let Ok(mut target) = Target::open_docx(&crate::doc::blank::docx()) else {
            return false;
        };
        let op = MailingsOp::LabelSheet {
            def: def.clone(),
            content,
            merge: false,
        };
        if target.apply(Command::DocxMailings { op }).is_err() {
            return false;
        }
        match target.snapshot_bytes() {
            Ok(bytes) => {
                self.mail(|m| m.outbox.push(bytes));
                self.status = Some("Label sheet created".to_owned());
                true
            }
            Err(_) => false,
        }
    }

    /// Update Labels: copies the first label into the other labels of the sheet.
    pub fn update_labels(&mut self) -> bool {
        self.apply_mailings(MailingsOp::UpdateLabels)
    }
}
