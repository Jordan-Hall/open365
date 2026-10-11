# DOCX review model: comments and tracked changes

Code: `crates/docx/src/{review,track,comments}.rs`. Oracle: `tools/office/review-oracle.ps1`
(`-Make` builds `fixtures/docx/review-*.docx`; default run writes
`fixtures/reference/docx/review-*/review.tsv` from Word 16). Test: `crates/docx/tests/review.rs`.

## Model
Nothing new is stored. `w:ins/del/moveFrom/moveTo` are `Group`s, `w:delText` is raw run content,
`*PrChange` and paragraph-mark `ins/del` stay in the raw property elements, comment ranges are raw
inlines. Untouched parts therefore save byte-identical (tested). Comment parts
(`comments`, `commentsExtended`, `commentsIds`, `commentsExtensible`) are parsed at open and rewritten
only when a comment call changed them; missing parts are created with relationship and content type.

Measured finding: Word writes the text of a `w:moveFrom` as `w:t` (it rejects `w:delText` there). The
reader keeps that text as raw run content so it is invisible to every text and offset calculation.

## API (`Document`)
- `revisions() -> Vec<Revision>`; `accept_revision(i)`, `reject_revision(i)`, `accept_all_revisions()`,
  `reject_all_revisions()`. Adjacent same-kind, same-author changes are one revision (Word counts them
  so); a move resolves both halves; an inserted/deleted row absorbs its content changes.
- Views: `Paragraph::text()` / `text_in_view(TextView::Final|Markup)`, `Paragraph::spans(view)` (text with
  author/kind/format flags), `Document::final_view_paragraphs()` (paragraphs whose mark is deleted join the
  next one, as Word's final view does), `Paragraph::mark_revision()`. Layout integration is the layout
  owner's: `Inline::Group` children of `w:del` carry `delText` (no characters); `moveFrom` text is raw.
- Recording: `set_track_changes(Some(TrackChanges::new("Author")))` makes `insert_text`, `delete_range`,
  `replace_range`, `split_paragraph` record `w:ins`/`w:del`; deleting the author's own insertion removes it.
- Comments: `comments()`, `comment(id)`, `comment_replies(id)`, `add_comment(start, end, &NewComment)`,
  `reply_to_comment`, `set_comment_resolved` (resolves the thread, as Word does), `delete_comment`.

## Oracle results
See the test; Word and Open365 agree on revision count, type, author and text; markup and final text;
AcceptAll/RejectAll text and paragraph alignment; each single accept/reject; comment count, author,
initials, scope text, text, done state, parent and reply counts, on 6 fixtures.

## Not covered
`w:tblPrChange` is modelled but Word 16 drops it in the probe, so it is not compared; `trPrChange`,
`tcPrChange`, `tblGridChange`, cell insert/merge/delete revisions are kept verbatim and not listed;
tracked deletion of non-text content (drawings, fields) is not recorded; comments only in the main story.
