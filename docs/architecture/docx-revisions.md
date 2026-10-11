# DOCX revisions and comments: Word parity

Extends `docx-review.md` (model and API). Code: `crates/docx/src/{review,revisions,comments}.rs`.
Oracle: `tools/office/make-revision-refs.ps1` (local developer tool, never CI). Test: `crates/docx/tests/revisions.rs`.

## Method
The script starts its own `Word.Application` (aborts when the instance's PID, read from the document window handle, existed
before; `AutomationSecurity = 3`; Quit in `finally`, `Stop-Process` on that PID only). It creates documents through
`TrackRevisions` / `Comments.Add` (`fixtures/docx/rev-*`, `cmt-*`), rewrites the stamped user name to "Author A" and scrubs
`docProps` and `people.xml`, then records per fixture (`fixtures/reference/docx/<name>/review.tsv`): the revision list
(type, author, date, text), the comments, and the paragraph state (text, alignment, style, list, run-length character
formatting bold/italic/underline/size) and table cell state (position, width, shading, text) after `AcceptAll`,
`RejectAll`, each revision accepted / rejected alone (fresh open each time) and `Range.Revisions` of each paragraph.
Hand-written fixtures (`rev-cell-*`, `rev-numbering-xml`) cover markup Word 16 never emits itself.

## Measured Word behaviour now modelled
- A paragraph's `pPrChange` is listed before the paragraph's own content revisions (sorted by range start).
- Every table, grid, row-exception, row and cell property change of one table (`tblPrChange`, `tblGridChange`,
  `tblPrExChange`, `trPrChange`, `tcPrChange`) is ONE `TableProperty` revision; accepting or rejecting it resolves all.
- The paragraph mark of a cell's last paragraph is the end-of-cell mark: Word does not list a revision on it.
- Moves: a new `moveFromRangeStart`/`moveToRangeStart` always starts a new revision, adjacent moved paragraphs are not
  merged, and the two halves are paired by the move range `w:name` (`Revision::move_name`), not by order. Body-level
  `moveFromRangeEnd` markers between paragraphs do not block joining paragraphs on accept/reject.
- Word does not track column insert/delete or cell merge/split as revisions (`rev-table-cols`, `rev-table-merge`: 0).
- `Range.Revisions.Accept/Reject` resolves only the part inside the range; a move's other half goes with it.
  API: `accept/reject_revisions_in_paragraphs(first, last)` (atoms located in those paragraphs).
- New API: `revisions()` now has `last_paragraph`, `move_name`; `accept_revisions(&[usize])`, `reject_revisions(&[usize])`,
  `revisions_in_paragraphs`, `RevisionKind::word_type_name()`, `paragraph_format_runs(index, view)`,
  `paragraph_style_name(index)`, `people()` (`word/people.xml`; created and extended when a comment by a new author is added).

- Cell revisions: `cellIns`, `cellDel` and `cellMerge` are listed as `CellInsertion`, `CellDeletion` and `CellMerge`
  (`RevisionKind::CellInserted/CellDeleted/CellMerged`), after the cell's own content revisions. Resolving one also
  resolves the cell's own `w:ins` (cellIns) or `w:del` (cellDel). Rejecting a cellIns or accepting a cellDel removes the
  cell; the previous cell takes its `tcW` and `gridSpan` (the next cell when the first one goes; not measured).
  `cellMerge` sets the vertical merge to its `vMerge` (accept) or `vMergeOrig` (reject): `cont` adds `w:vMerge`, `rest`
  removes a continuation. Word's `Range.Revisions` for a cell revision touches the previous cell through the end of
  the row. Word's `Paragraphs` and `Cells` leave out a vertical merge's continuation cell (the test maps indices).
- `w:numberingChange` and `w:numPr/w:ins`: Word lists no revision and drops both on accept and reject (the numbering
  stays). They are hidden atoms: resolved by accept/reject all and by ranges, never listed.
- Moves: accepting a move also accepts the moves that start inside its range (the range of the second of two adjacent
  moves starts before the first one ends; `Revision::link_moves`). Rejecting does not (measured on `rev-move`).
- Table revisions through `Range.Revisions`: the single TableProperty revision is split per row. A range that touches
  any paragraph of a row resolves that row's `tblPrEx`, `trPr` and `tcPr` changes and every other revision in the row
  (`rev-table-cells`: accepting a cell in the last row accepts the deleted text of that row). Inserted and deleted rows
  are hit by any paragraph of the row. Table and grid changes are hit by any paragraph of the table.
- Comment scope: `Comment::scope_text` is the markup view (deleted text included), as Word's `Scope.Text`; it is
  compared for every fixture, revised ones too.
- Word's object model refuses comments outside the main story ("Comments, endnotes and footnotes can only be added to
  the main story", measured with `cmt-stories` for footnotes, text boxes, headers and footers), so such comments only
  exist in files written by other producers.

## Parity numbers (this machine, Word 16.0, 18 generated + 4 hand-written fixtures, plus the 6 earlier review-* fixtures)
- Accept/reject states: 490 compared by `revisions.rs` over 22 `rev-*`/`cmt-*` fixtures; all 490 exact, `KNOWN_GAPS` is empty
  (it was 310 of 322 with 12 gaps and 4 excluded fixtures)
- Revision lists (type, author, date, ins/del/move text) match for every fixture (67 revisions), including `rev-two-authors` (two
  authors alternate over adjacent revisions) and the cell and numbering fixtures.
- Comments (author, initials, scope incl. deleted text, text, done, parent, reply counts) match in all `cmt-*` fixtures;
  the five comment parts round-trip byte-identical; `people.xml` survives and gains new authors.
- New fixtures: `rev-two-authors`, `rev-numbering-deep`, `rev-section-deep`, `cmt-stories` (note, text box, header and
  footer present; Word refuses comments there).

## Open
- Cell revisions at the edge of a row (first cell removed: which neighbour takes the width; range for the first and last
  cell) and adjacent cell revisions (one or several Word revisions) are unmeasured: the hand-written `rev-cell-edge`
  document (4x4 table, first/last/adjacent cells, a `cont`/`rest` merge) made Word hang in the measure step and was
  dropped; build it smaller before retrying.
- Comments in headers, footers, notes and text boxes (hand-written markup) and `w:sectPrChange` inside headers are not
  covered; `Document::comments()` anchors only in the main story.
- Reject-side move linking, and the per-row rule for cell revisions, rest on one fixture each.
