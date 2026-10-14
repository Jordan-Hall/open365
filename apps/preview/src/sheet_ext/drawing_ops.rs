//! Pictures, shapes and text boxes on the sheet model: Insert > Pictures, Shapes, Icons and Text Box,
//! and the edits of a selected object (move, resize, rotate, fill, outline, text, order, delete).
//!
//! Every change is one `SheetObjects` command, so one undo step. A new object is selected, as in
//! Excel. The pointer functions take positions in points from the corner of A1; a drag shows a frame
//! while it lasts (see [`selection_frame`]) and applies one command when the button is released.

use std::cell::RefCell;
use std::path::Path;

use open365_commands::objects_ops::{ObjectEdit, ObjectOrder, ObjectPlacement};
use open365_commands::{Command, ObjectsOp};
use open365_drawing::geom::P;
use open365_drawing::handles::{Frame, Handle, hit_handle, resize, rotation_to, translate};
use open365_drawing::{Kind, catalog};
use open365_xlsx::{Anchor, EditAs};

use super::Item;
use crate::sheet::SheetModel;

/// Largest picture file Insert > Pictures reads.
const MAX_PICTURE_BYTES: u64 = 64 * 1024 * 1024;
/// A pointer drag shorter than this (points) is a click.
const CLICK_SLOP_PT: f64 = 2.0;
/// Hit radius of a handle, points.
const HANDLE_HIT_PT: f64 = 5.0;

/// What an object is, for the UI (cursor, Format pane, tab).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    Picture,
    Shape,
    TextBox,
    Connector,
    Chart,
    Group,
    Other,
}

/// One drawing object of the active sheet.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectInfo {
    pub id: u32,
    pub name: String,
    pub kind: ObjectKind,
    /// Unrotated frame in points from the corner of A1, with the rotation.
    pub frame: Frame,
    pub flip_h: bool,
    pub flip_v: bool,
    /// Position in the stacking order, back to front.
    pub z: usize,
    pub placement: ObjectPlacement,
    /// The text of a shape or text box, paragraphs joined by newlines.
    pub text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DragKind {
    Move,
    Resize(Handle),
    Rotate,
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    kind: DragKind,
    start: P,
    origin: Frame,
    current: Frame,
}

#[derive(Debug, Default)]
struct State {
    sheet: usize,
    selected: Option<u32>,
    drag: Option<Drag>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

/// The frame to draw selection handles on for `sheet`: the selected object's, or the frame a drag
/// in progress would end in. `None` when nothing on `items` is selected.
pub(super) fn selection_frame(sheet: usize, items: &[Item]) -> Option<Frame> {
    with_state(|s| {
        if s.sheet != sheet {
            return None;
        }
        let id = s.selected?;
        if let Some(d) = s.drag.filter(|d| d.current != d.origin) {
            return Some(d.current);
        }
        let item = items.iter().find(|i| i.id == id)?;
        let k = 96.0 / 72.0;
        let (x, y, w, h) = item.rect;
        Some(Frame::new(x / k, y / k, w / k, h / k, item.rot_deg))
    })
}

fn bad(message: impl Into<String>) -> String {
    message.into()
}

impl SheetModel {
    /// Every drawing object of the active sheet, back to front.
    #[must_use]
    pub fn drawing_objects(&self) -> Vec<ObjectInfo> {
        let Some(wb) = self.target.xlsx() else {
            return Vec::new();
        };
        let Some(ws) = wb.sheet(self.sheet) else {
            return Vec::new();
        };
        let metrics = self.geometry().metrics();
        wb.sheet_drawing_objects(self.sheet)
            .into_iter()
            .map(|o| {
                let r = o.anchor.rect_px_pt(ws, metrics);
                let x = &o.object.xfrm;
                let (kind, text) = match &o.object.kind {
                    Kind::Picture(_) => (ObjectKind::Picture, None),
                    Kind::Shape(s) => {
                        let kind = if s.is_text_box {
                            ObjectKind::TextBox
                        } else if s.is_connector {
                            ObjectKind::Connector
                        } else {
                            ObjectKind::Shape
                        };
                        let text = s.text.as_ref().map(|t| {
                            open365_drawing::text::paragraphs(&t.story)
                                .into_iter()
                                .map(|p| p.text)
                                .collect::<Vec<_>>()
                                .join("\n")
                        });
                        (kind, text)
                    }
                    Kind::Chart { .. } => (ObjectKind::Chart, None),
                    Kind::Group(_) => (ObjectKind::Group, None),
                    Kind::Unknown { .. } => (ObjectKind::Other, None),
                };
                ObjectInfo {
                    id: o.object.id,
                    name: o.object.name.clone(),
                    kind,
                    frame: Frame::new(r.x, r.y, r.width, r.height, x.rot as f64 / 60_000.0),
                    flip_h: x.flip_h,
                    flip_v: x.flip_v,
                    z: o.z,
                    placement: match o.anchor {
                        Anchor::TwoCell {
                            edit_as: EditAs::TwoCell,
                            ..
                        } => ObjectPlacement::MoveAndSize,
                        Anchor::TwoCell {
                            edit_as: EditAs::OneCell,
                            ..
                        }
                        | Anchor::OneCell { .. } => ObjectPlacement::Move,
                        _ => ObjectPlacement::Free,
                    },
                    text,
                }
            })
            .collect()
    }

    /// The object with drawing id `id`.
    #[must_use]
    pub fn drawing_object(&self, id: u32) -> Option<ObjectInfo> {
        self.drawing_objects().into_iter().find(|o| o.id == id)
    }

    // ---- selection -------------------------------------------------------------------------

    /// The selected object's id, when it still exists on the active sheet.
    #[must_use]
    pub fn drawing_selected(&self) -> Option<u32> {
        let (sheet, id) = with_state(|s| (s.sheet, s.selected));
        let id = id?;
        (sheet == self.sheet && self.drawing_object(id).is_some()).then_some(id)
    }

    /// Selects object `id` (or clears the selection).
    pub fn drawing_select(&mut self, id: Option<u32>) {
        let sheet = self.sheet;
        with_state(|s| {
            s.sheet = sheet;
            s.selected = id;
            s.drag = None;
        });
    }

    /// The topmost object whose frame contains the point (points from A1).
    #[must_use]
    pub fn drawing_object_at(&self, x: f64, y: f64) -> Option<u32> {
        self.drawing_objects()
            .iter()
            .rev()
            .find(|o| o.frame.contains(P::new(x, y)))
            .map(|o| o.id)
    }

    // ---- insert ----------------------------------------------------------------------------

    fn drawing_ids(&self) -> Vec<u32> {
        self.drawing_objects().iter().map(|o| o.id).collect()
    }

    fn drawing_run(&mut self, op: ObjectsOp) -> Result<u32, String> {
        let before = self.drawing_ids();
        self.apply(Command::SheetObjects {
            sheet: self.sheet,
            op,
        })?;
        let id = self
            .drawing_ids()
            .into_iter()
            .find(|i| !before.contains(i))
            .ok_or_else(|| bad("The object was not added."))?;
        self.drawing_select(Some(id));
        Ok(id)
    }

    /// Insert > Pictures > This Device: the picture file `path` (PNG, JPEG, GIF, BMP or SVG) with its
    /// top-left corner at the active cell, at its natural size. Returns the new object's id.
    ///
    /// # Errors
    /// The file cannot be read, is too large, or is not a picture.
    pub fn insert_picture_file(&mut self, path: &Path) -> Result<u32, String> {
        let meta =
            std::fs::metadata(path).map_err(|e| format!("The file cannot be opened ({e})."))?;
        if meta.len() > MAX_PICTURE_BYTES {
            return Err(bad("The picture file is larger than 64 MB."));
        }
        let data = std::fs::read(path).map_err(|e| format!("The file cannot be read ({e})."))?;
        self.insert_picture_bytes(data)
    }

    /// Insert > Pictures with the picture's bytes in hand.
    ///
    /// # Errors
    /// The bytes are not a picture, or the sheet cannot take it.
    pub fn insert_picture_bytes(&mut self, data: Vec<u8>) -> Result<u32, String> {
        let (row, col) = self.active;
        self.drawing_run(ObjectsOp::InsertPicture {
            data,
            row,
            col,
            name: None,
        })
    }

    /// Insert > Icons: a bundled icon (see [`super::icons`]) as an SVG picture at the active cell.
    ///
    /// # Errors
    /// The name is not a bundled icon.
    pub fn insert_icon(&mut self, name: &str) -> Result<u32, String> {
        let svg = super::icons::icon_svg(name)
            .ok_or_else(|| format!("{name} is not an icon in the set."))?;
        self.insert_picture_bytes(svg)
    }

    /// Insert > Shapes: the gallery shape `preset` (see [`catalog`]) at the active cell, at the click size.
    ///
    /// # Errors
    /// The preset is not in the gallery.
    pub fn insert_shape(&mut self, preset: &str) -> Result<u32, String> {
        if catalog::label_of(preset).is_none() {
            return Err(format!("{preset} is not a shape in the gallery."));
        }
        let (row, col) = self.active;
        self.drawing_run(ObjectsOp::InsertShape {
            preset: preset.to_owned(),
            text_box: false,
            row,
            col,
            size: None,
        })
    }

    /// Insert > Text Box at the active cell, at the click size.
    ///
    /// # Errors
    /// The sheet cannot take the object.
    pub fn insert_text_box(&mut self) -> Result<u32, String> {
        let (row, col) = self.active;
        self.drawing_run(ObjectsOp::InsertShape {
            preset: "rect".to_owned(),
            text_box: true,
            row,
            col,
            size: None,
        })
    }

    // ---- edits -----------------------------------------------------------------------------

    /// Applies `edit` to object `id`.
    ///
    /// # Errors
    /// The object does not exist or the edit is not valid for it.
    pub fn drawing_edit(&mut self, id: u32, edit: ObjectEdit) -> Result<(), String> {
        self.apply(Command::SheetObjects {
            sheet: self.sheet,
            op: ObjectsOp::EditObject { object: id, edit },
        })
    }

    /// Moves object `id` by `(dx, dy)` points.
    ///
    /// # Errors
    /// The object does not exist.
    pub fn drawing_move_by(&mut self, id: u32, dx: f64, dy: f64) -> Result<(), String> {
        let o = self
            .drawing_object(id)
            .ok_or_else(|| bad("No such object."))?;
        let f = translate(&o.frame, dx, dy);
        self.drawing_edit(id, frame_edit(&f))
    }

    /// Arrange: Bring to Front, Send to Back, Bring Forward, Send Backward.
    ///
    /// # Errors
    /// The object does not exist.
    pub fn drawing_arrange(&mut self, id: u32, order: ObjectOrder) -> Result<(), String> {
        self.apply(Command::SheetObjects {
            sheet: self.sheet,
            op: ObjectsOp::ArrangeObject { object: id, order },
        })
    }

    /// Deletes object `id`.
    ///
    /// # Errors
    /// The object does not exist.
    pub fn drawing_delete(&mut self, id: u32) -> Result<(), String> {
        self.apply(Command::SheetObjects {
            sheet: self.sheet,
            op: ObjectsOp::DeleteObject { object: id },
        })?;
        if self.drawing_selected().is_none() {
            self.drawing_select(None);
        }
        Ok(())
    }

    /// The Delete key: removes the selected object. False when nothing is selected.
    pub fn drawing_delete_selected(&mut self) -> bool {
        match self.drawing_selected() {
            Some(id) => self.drawing_delete(id).is_ok(),
            None => false,
        }
    }

    // ---- pointer ---------------------------------------------------------------------------

    /// Button pressed at `(x, y)` points from A1. A handle of the selected object starts a resize or
    /// rotation; an object starts a move (and is selected); empty space clears the selection and
    /// returns false so the grid handles the click.
    pub fn drawing_pointer_down(&mut self, x: f64, y: f64, _shift: bool) -> bool {
        let p = P::new(x, y);
        let objects = self.drawing_objects();
        let selected = self
            .drawing_selected()
            .and_then(|id| objects.iter().find(|o| o.id == id));
        if let Some(o) = selected
            && let Some(h) = hit_handle(&o.frame, p, HANDLE_HIT_PT)
        {
            let kind = if h == Handle::Rotate {
                DragKind::Rotate
            } else {
                DragKind::Resize(h)
            };
            with_state(|s| {
                s.drag = Some(Drag {
                    kind,
                    start: p,
                    origin: o.frame,
                    current: o.frame,
                });
            });
            return true;
        }
        match objects.iter().rev().find(|o| o.frame.contains(p)) {
            Some(o) => {
                self.drawing_select(Some(o.id));
                with_state(|s| {
                    s.drag = Some(Drag {
                        kind: DragKind::Move,
                        start: p,
                        origin: o.frame,
                        current: o.frame,
                    });
                });
                true
            }
            None => {
                self.drawing_select(None);
                false
            }
        }
    }

    /// Pointer moved with the button down; true while a drag is in progress. `shift` keeps the aspect
    /// ratio of a corner resize, snaps a rotation to 15 degrees and locks a move to one axis.
    pub fn drawing_pointer_move(&mut self, x: f64, y: f64, shift: bool) -> bool {
        with_state(|s| {
            let Some(d) = s.drag.as_mut() else {
                return false;
            };
            let p = P::new(x, y);
            d.current = match d.kind {
                DragKind::Move => {
                    let (mut dx, mut dy) = (p.x - d.start.x, p.y - d.start.y);
                    if shift {
                        if dx.abs() >= dy.abs() {
                            dy = 0.0;
                        } else {
                            dx = 0.0;
                        }
                    }
                    translate(&d.origin, dx, dy)
                }
                DragKind::Resize(h) => resize(&d.origin, h, p, shift),
                DragKind::Rotate => Frame {
                    rot_deg: rotation_to(&d.origin, p, shift),
                    ..d.origin
                },
            };
            true
        })
    }

    /// Button released: a drag that moved applies one command (one undo step). True when an object
    /// owned the pointer.
    pub fn drawing_pointer_up(&mut self, x: f64, y: f64, shift: bool) -> bool {
        if !self.drawing_pointer_move(x, y, shift) {
            return false;
        }
        let Some(d) = with_state(|s| s.drag.take()) else {
            return false;
        };
        let Some(id) = with_state(|s| s.selected) else {
            return true;
        };
        let moved = (d.current.x - d.origin.x).hypot(d.current.y - d.origin.y) >= CLICK_SLOP_PT
            || (d.current.w - d.origin.w).abs() >= 0.01
            || (d.current.h - d.origin.h).abs() >= 0.01;
        let result = match d.kind {
            DragKind::Rotate if (d.current.rot_deg - d.origin.rot_deg).abs() >= 0.01 => self
                .drawing_edit(
                    id,
                    ObjectEdit::Rotate {
                        degrees: d.current.rot_deg,
                    },
                ),
            DragKind::Move | DragKind::Resize(_) if moved => {
                self.drawing_edit(id, frame_edit(&d.current))
            }
            _ => Ok(()),
        };
        if result.is_err() {
            // The status line carries the reason.
        }
        true
    }

    /// True while a pointer drag on an object is in progress (between press and release).
    #[must_use]
    pub fn drawing_dragging(&self) -> bool {
        with_state(|s| s.drag.is_some())
    }

    /// Escape during a drag: forgets it without a change.
    pub fn drawing_pointer_cancel(&mut self) {
        with_state(|s| s.drag = None);
    }
}

fn frame_edit(f: &Frame) -> ObjectEdit {
    ObjectEdit::Frame {
        x: f.x,
        y: f.y,
        width: f.w,
        height: f.h,
    }
}
