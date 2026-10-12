//! The command catalogue: one entry per `open365_commands::Command`, with a runnable example.
//!
//! The CLI (`open365 commands`, `open365 run`) and the MCP server (`list_commands`, the
//! `open365://commands` resource) both list this table, so there is one place to describe a
//! command. A test reads the `Command` enum's source and fails when a variant has no entry
//! here, so a new command cannot ship without CLI and MCP exposure. See
//! `docs/architecture/agent-control.md`.

use open365_commands::{Command, EffectClass};
use serde_json::{Value, json};

/// Which kind of document a command works on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Domain {
    Docx,
    Xlsx,
    /// Works on either.
    Any,
}

impl Domain {
    pub fn name(self) -> &'static str {
        match self {
            Self::Docx => "docx",
            Self::Xlsx => "xlsx",
            Self::Any => "any",
        }
    }
}

/// How a surface runs a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route {
    /// `open365 run <name>` runs every command.
    pub cli: &'static str,
    /// The MCP tool that runs it.
    pub mcp: &'static str,
}

/// One command's catalogue entry.
#[derive(Debug, Clone, Copy)]
pub struct CommandSpec {
    /// `Command::name()`, the `command` tag in the JSON form.
    pub name: &'static str,
    pub domain: Domain,
    pub summary: &'static str,
    /// A complete, valid example in the JSON form (the `command` tag included).
    pub example: fn() -> Value,
}

impl CommandSpec {
    /// The example parsed as a command.
    pub fn example_command(&self) -> Option<Command> {
        serde_json::from_value((self.example)()).ok()
    }

    /// The effect class of the example. Some commands change class with their arguments
    /// (`docx_delete_range` over an empty range is `edit`), so this is the typical class.
    pub fn example_effect(&self) -> Option<EffectClass> {
        self.example_command().map(|c| c.effect(None))
    }

    pub fn route(&self) -> Route {
        match self.name {
            "save" => Route {
                cli: "open365 run save",
                mcp: "save_as",
            },
            "export_pdf" => Route {
                cli: "open365 run export_pdf",
                mcp: "export_pdf",
            },
            "share_send_copy" => Route {
                cli: "open365 share send-copy",
                mcp: "share_send_copy",
            },
            "share_send_pdf" => Route {
                cli: "open365 share send-pdf",
                mcp: "share_send_pdf",
            },
            "share_inspect" => Route {
                cli: "open365 share inspect",
                mcp: "share_inspect",
            },
            "share_sanitize_copy" => Route {
                cli: "open365 share sanitize-copy",
                mcp: "share_sanitize_copy",
            },
            "share_os_sheet" => Route {
                cli: "open365 share os-sheet",
                mcp: "share_os_sheet",
            },
            _ => Route {
                cli: "open365 run",
                mcp: "apply_commands",
            },
        }
    }

    pub fn to_json(&self) -> Value {
        let route = self.route();
        let effect = self.example_effect().map(crate::tools::effect_name);
        json!({
            "name": self.name,
            "domain": self.domain.name(),
            "summary": self.summary,
            "example_effect": effect,
            "example": (self.example)(),
            "cli": route.cli,
            "mcp": route.mcp,
        })
    }
}

fn red() -> Value {
    json!({"rgb": "FF0000"})
}

/// Every command, in the order of the `Command` enum.
pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "docx_insert_text",
        domain: Domain::Docx,
        summary: "Insert text into a paragraph at a character offset.",
        example: || json!({"command": "docx_insert_text", "para": 0, "offset": 0, "text": "Hello "}),
    },
    CommandSpec {
        name: "docx_set_bold",
        domain: Domain::Docx,
        summary: "Set or clear bold on a character range of one paragraph.",
        example: || json!({"command": "docx_set_bold", "para": 0, "start": 0, "end": 3, "bold": true}),
    },
    CommandSpec {
        name: "docx_delete_range",
        domain: Domain::Docx,
        summary: "Delete the text between two positions, merging paragraphs across the range.",
        example: || {
            json!({"command": "docx_delete_range",
                "start": {"para": 0, "offset": 0}, "end": {"para": 0, "offset": 2}})
        },
    },
    CommandSpec {
        name: "docx_split_paragraph",
        domain: Domain::Docx,
        summary: "Split a paragraph at a position (the Enter key).",
        example: || json!({"command": "docx_split_paragraph", "pos": {"para": 0, "offset": 1}}),
    },
    CommandSpec {
        name: "docx_set_run_props",
        domain: Domain::Docx,
        summary: "Change run formatting (bold, italic, underline, size, colour, font) over a range.",
        example: || {
            json!({"command": "docx_set_run_props",
                "start": {"para": 0, "offset": 0}, "end": {"para": 0, "offset": 3},
                "patch": {"italic": true, "color": "FF0000"}})
        },
    },
    CommandSpec {
        name: "docx_set_paragraph_style",
        domain: Domain::Docx,
        summary: "Apply a paragraph style by its style id.",
        example: || json!({"command": "docx_set_paragraph_style", "para": 0, "style_id": "Heading1"}),
    },
    CommandSpec {
        name: "docx_set_alignment",
        domain: Domain::Docx,
        summary: "Set paragraph alignment: left, center, right or justify.",
        example: || json!({"command": "docx_set_alignment", "para": 0, "alignment": "center"}),
    },
    CommandSpec {
        name: "docx_set_indents",
        domain: Domain::Docx,
        summary: "Set the left, first-line (negative: hanging) and right indent of a paragraph, in points.",
        example: || {
            json!({"command": "docx_set_indents", "para": 0,
                "left_pt": 36.0, "first_line_pt": -18.0, "right_pt": 0.0})
        },
    },
    CommandSpec {
        name: "docx_add_tab",
        domain: Domain::Docx,
        summary: "Add a custom tab stop (left, center, right, decimal or bar) at a distance in points.",
        example: || json!({"command": "docx_add_tab", "para": 0, "pos_pt": 144.0, "kind": "left"}),
    },
    CommandSpec {
        name: "docx_move_tab",
        domain: Domain::Docx,
        summary: "Move the tab stop at one position to another, keeping its alignment.",
        example: || json!({"command": "docx_move_tab", "para": 0, "from_pt": 36.0, "to_pt": 72.0}),
    },
    CommandSpec {
        name: "docx_clear_tab",
        domain: Domain::Docx,
        summary: "Clear the tab stop at a position.",
        example: || json!({"command": "docx_clear_tab", "para": 0, "pos_pt": 36.0}),
    },
    CommandSpec {
        name: "docx_replace_range",
        domain: Domain::Docx,
        summary: "Replace the text between two positions (typing over a selection).",
        example: || {
            json!({"command": "docx_replace_range",
                "start": {"para": 0, "offset": 0}, "end": {"para": 0, "offset": 2}, "text": "XY"})
        },
    },
    CommandSpec {
        name: "docx_replace_all",
        domain: Domain::Docx,
        summary: "Replace every match of a text in the document (Word's Replace All).",
        example: || {
            json!({"command": "docx_replace_all", "find": "a", "replacement": "b",
                "match_case": false, "whole_word": false, "wildcards": false})
        },
    },
    CommandSpec {
        name: "docx_replace_one",
        domain: Domain::Docx,
        summary: "Replace one match of Find and Replace (the range docx find gave) with the expanded replacement.",
        example: || {
            json!({"command": "docx_replace_one", "op": {"find": "F", "replacement": "G",
                "para": 0, "start": 0, "end": 1}})
        },
    },
    CommandSpec {
        name: "docx_section_margins",
        domain: Domain::Docx,
        summary: "Set the page margins of one section, in points (ruler drag, Page Setup).",
        example: || {
            json!({"command": "docx_section_margins", "op": {"section": 0,
                "top_pt": 72.0, "bottom_pt": 72.0, "left_pt": 72.0, "right_pt": 72.0}})
        },
    },
    CommandSpec {
        name: "xlsx_replace_all",
        domain: Domain::Xlsx,
        summary: "Replace every match of a text over cell formulas and constants (Excel's Replace All).",
        example: || {
            json!({"command": "xlsx_replace_all",
                "find": {"text": "a", "scope": {"scope": "workbook"}}, "replacement": "b"})
        },
    },
    CommandSpec {
        name: "xlsx_paste",
        domain: Domain::Xlsx,
        summary: "Paste a block of typed cell text (a leading = makes a formula) at a cell.",
        example: || {
            json!({"command": "xlsx_paste", "sheet": 0, "row": 1, "col": 1,
                "rows": [["1", "2"], ["=A1+B1", "text"]]})
        },
    },
    CommandSpec {
        name: "xlsx_set_cell_value",
        domain: Domain::Xlsx,
        summary: "Set a cell to a number, text, boolean or empty.",
        example: || {
            json!({"command": "xlsx_set_cell_value", "sheet": 0, "row": 1, "col": 1,
                "value": {"type": "number", "value": 42.0}})
        },
    },
    CommandSpec {
        name: "xlsx_set_cell_formula",
        domain: Domain::Xlsx,
        summary: "Set a cell's formula; dependent cells are recalculated.",
        example: || {
            json!({"command": "xlsx_set_cell_formula", "sheet": 0, "row": 1, "col": 3,
                "formula": "=1+2"})
        },
    },
    CommandSpec {
        name: "set_column_width",
        domain: Domain::Xlsx,
        summary: "Set the width of columns first..=last in characters (0..=255).",
        example: || json!({"command": "set_column_width", "sheet": 0, "first": 2, "last": 3, "width": 24.5}),
    },
    CommandSpec {
        name: "set_row_height",
        domain: Domain::Xlsx,
        summary: "Set the height of rows first..=last in points (0..=409.5).",
        example: || json!({"command": "set_row_height", "sheet": 0, "first": 2, "last": 2, "points": 40.0}),
    },
    CommandSpec {
        name: "insert_rows",
        domain: Domain::Xlsx,
        summary: "Insert blank rows before a row; formulas that point at the sheet are rewritten.",
        example: || json!({"command": "insert_rows", "sheet": 0, "at": 2, "count": 1}),
    },
    CommandSpec {
        name: "delete_rows",
        domain: Domain::Xlsx,
        summary: "Delete rows; references to deleted cells become #REF!.",
        example: || json!({"command": "delete_rows", "sheet": 0, "at": 2, "count": 1}),
    },
    CommandSpec {
        name: "insert_columns",
        domain: Domain::Xlsx,
        summary: "Insert blank columns before a column.",
        example: || json!({"command": "insert_columns", "sheet": 0, "at": 2, "count": 1}),
    },
    CommandSpec {
        name: "delete_columns",
        domain: Domain::Xlsx,
        summary: "Delete columns; references to deleted cells become #REF!.",
        example: || json!({"command": "delete_columns", "sheet": 0, "at": 2, "count": 1}),
    },
    CommandSpec {
        name: "hide_columns",
        domain: Domain::Xlsx,
        summary: "Hide columns first..=last, keeping their widths.",
        example: || json!({"command": "hide_columns", "sheet": 0, "first": 2, "last": 2}),
    },
    CommandSpec {
        name: "unhide_columns",
        domain: Domain::Xlsx,
        summary: "Show hidden columns first..=last.",
        example: || json!({"command": "unhide_columns", "sheet": 0, "first": 2, "last": 2}),
    },
    CommandSpec {
        name: "hide_rows",
        domain: Domain::Xlsx,
        summary: "Hide rows first..=last, keeping their heights.",
        example: || json!({"command": "hide_rows", "sheet": 0, "first": 2, "last": 2}),
    },
    CommandSpec {
        name: "unhide_rows",
        domain: Domain::Xlsx,
        summary: "Show hidden rows first..=last.",
        example: || json!({"command": "unhide_rows", "sheet": 0, "first": 2, "last": 2}),
    },
    CommandSpec {
        name: "format_cells",
        domain: Domain::Xlsx,
        summary: "Set font, fill, alignment, number format or protection on a range.",
        example: || {
            json!({"command": "format_cells", "sheet": 0,
                "range": {"first_row": 1, "first_col": 1, "last_row": 2, "last_col": 2},
                "change": {"font": {"bold": true, "color": red()},
                           "number_format": {"kind": "code", "value": "0.00"}}})
        },
    },
    CommandSpec {
        name: "set_borders",
        domain: Domain::Xlsx,
        summary: "Set outline, inside and single-edge borders of a range.",
        example: || {
            json!({"command": "set_borders", "sheet": 0,
                "range": {"first_row": 1, "first_col": 1, "last_row": 2, "last_col": 2},
                "borders": {"top": {"style": "thin"}, "bottom": {"style": "thin"}}})
        },
    },
    CommandSpec {
        name: "apply_cell_style",
        domain: Domain::Xlsx,
        summary: "Apply a named cell style (Good, Heading 1, Percent, Currency, ...).",
        example: || {
            json!({"command": "apply_cell_style", "sheet": 0,
                "range": {"first_row": 1, "first_col": 1, "last_row": 1, "last_col": 2},
                "style": "Good"})
        },
    },
    CommandSpec {
        name: "clear_formats",
        domain: Domain::Xlsx,
        summary: "Clear formats of a range back to the Normal style.",
        example: || {
            json!({"command": "clear_formats", "sheet": 0,
                "range": {"first_row": 1, "first_col": 1, "last_row": 2, "last_col": 2}})
        },
    },
    CommandSpec {
        name: "copy_format",
        domain: Domain::Xlsx,
        summary: "Copy the formats of one range onto another (Format Painter).",
        example: || {
            json!({"command": "copy_format", "sheet": 0,
                "from": {"first_row": 1, "first_col": 1, "last_row": 1, "last_col": 1},
                "to": {"first_row": 2, "first_col": 1, "last_row": 3, "last_col": 2}})
        },
    },
    CommandSpec {
        name: "change_decimals",
        domain: Domain::Xlsx,
        summary: "Increase (delta > 0) or decrease the decimals shown in a range.",
        example: || {
            json!({"command": "change_decimals", "sheet": 0,
                "range": {"first_row": 1, "first_col": 1, "last_row": 2, "last_col": 2},
                "delta": 1})
        },
    },
    CommandSpec {
        name: "merge_cells",
        domain: Domain::Xlsx,
        summary: "Merge & Center, Merge Across, Merge Cells or Unmerge a range; merging keeps only the upper-left value of each area (destructive when it would drop values).",
        example: || {
            json!({"command": "merge_cells", "sheet": 0,
                "range": {"first_row": 40, "first_col": 1, "last_row": 40, "last_col": 3},
                "mode": "cells"})
        },
    },
    CommandSpec {
        name: "docx_references",
        domain: Domain::Docx,
        summary: "Writer References tab: tables of contents and figures, captions, footnotes and endnotes, index, authorities, field updates.",
        example: || {
            json!({"command": "docx_references",
                "op": {"update_fields": {"scope": "toc", "pages": []}}})
        },
    },
    CommandSpec {
        name: "docx_home",
        domain: Domain::Docx,
        summary: "Writer Home tab operations beyond the text model: highlight, underline style, change case, lists, borders.",
        example: || {
            json!({"command": "docx_home",
                "op": {"op": "underline", "start": {"para": 0, "offset": 0},
                    "end": {"para": 0, "offset": 5}, "style": "single"}})
        },
    },
    CommandSpec {
        name: "docx_insert",
        domain: Domain::Docx,
        summary: "Writer Insert tab: page and section breaks, tables, links, bookmarks, pictures, headers and footers.",
        example: || {
            json!({"command": "docx_insert",
                "op": {"op": "page_break", "pos": {"para": 0, "offset": 0}}})
        },
    },
    CommandSpec {
        name: "docx_review",
        domain: Domain::Docx,
        summary: "Writer Review tab: track changes on or off, comments, accept and reject revisions.",
        example: || {
            json!({"command": "docx_review",
                "op": {"track_changes": {"on": true, "author": "Agent"}}})
        },
    },
    CommandSpec {
        name: "docx_layout",
        domain: Domain::Docx,
        summary: "Writer Layout tab: margins, orientation, size, columns, breaks, line numbers, hyphenation, indent and spacing.",
        example: || {
            json!({"command": "docx_layout",
                "op": {"op": "orientation", "sections": [0], "landscape": true}})
        },
    },
    CommandSpec {
        name: "docx_design",
        domain: Domain::Docx,
        summary: "Writer Design tab: themes, colours, fonts, effects, style sets, page colour and borders, watermark.",
        example: || json!({"command": "docx_design", "op": {"op": "theme_effects", "level": 1}}),
    },
    CommandSpec {
        name: "home_op",
        domain: Domain::Xlsx,
        summary: "Sheets Home tab operations on whole structures: Clear All, custom Sort, AutoFilter (destructive when it removes content).",
        example: || {
            json!({"command": "home_op", "sheet": 0,
                "op": {"op": "clear_all",
                    "range": {"first_row": 40, "first_col": 1, "last_row": 40, "last_col": 3}}})
        },
    },
    CommandSpec {
        name: "data_op",
        domain: Domain::Xlsx,
        summary: "Sheets Data tab: sort, AutoFilter and criteria, Advanced Filter, Text to Columns, validation, outline, subtotals.",
        example: || {
            json!({"command": "data_op", "sheet": 0,
                "op": {"op": "auto_filter",
                    "range": {"first_row": 1, "first_col": 1, "last_row": 3, "last_col": 3}}})
        },
    },
    CommandSpec {
        name: "review_op",
        domain: Domain::Xlsx,
        summary: "Sheets Review tab: protect or unprotect a sheet or workbook, notes.",
        example: || json!({"command": "review_op", "sheet": 0, "op": {"op": "protect_workbook"}}),
    },
    CommandSpec {
        name: "sheet_objects",
        domain: Domain::Xlsx,
        summary: "Sheets Insert and Formulas tabs and sheet management: tables, names, links, charts, pivots, conditional formats, sheet operations, cell shifts, recalculation.",
        example: || json!({"command": "sheet_objects", "sheet": 0, "op": {"op": "calculate_now"}}),
    },
    CommandSpec {
        name: "save",
        domain: Domain::Any,
        summary: "Write the document to a path (effect export to a new file, destructive over an existing one).",
        example: || json!({"command": "save", "path": "saved.out"}),
    },
    CommandSpec {
        name: "export_pdf",
        domain: Domain::Docx,
        summary: "Lay a DOCX out and write it as a tagged PDF (effect export).",
        example: || json!({"command": "export_pdf", "path": "export.pdf", "overwrite": false}),
    },
    CommandSpec {
        name: "docx_correct_word",
        domain: Domain::Docx,
        summary: "Replace a flagged misspelled word; fails without changes when the paragraph no longer holds the expected word there (destructive).",
        example: || {
            json!({"command": "docx_correct_word", "para": 0, "start": 0, "end": 5,
                "expected": "First", "replacement": "Frist"})
        },
    },
    CommandSpec {
        name: "auto_fit_columns",
        domain: Domain::Xlsx,
        summary: "Store measured auto-fit widths for (1-based column, width) pairs, as AutoFit Column Width does.",
        example: || json!({"command": "auto_fit_columns", "sheet": 0, "columns": [[1, 12.5]]}),
    },
    CommandSpec {
        name: "auto_fit_rows",
        domain: Domain::Xlsx,
        summary: "Store measured auto-fit heights for (1-based row, points) pairs, as AutoFit Row Height does.",
        example: || json!({"command": "auto_fit_rows", "sheet": 0, "rows": [[1, 18.0]]}),
    },
    CommandSpec {
        name: "share_send_copy",
        domain: Domain::Any,
        summary: "Write a copy of the document, as it is now, to a new file (or a temporary folder) for an attachment; never the document's own file (effect export).",
        example: || json!({"command": "share_send_copy", "path": "attachment.docx", "overwrite": false}),
    },
    CommandSpec {
        name: "share_send_pdf",
        domain: Domain::Docx,
        summary: "Write the document as a PDF to send, through the PDF export (effect export).",
        example: || json!({"command": "share_send_pdf", "path": "attachment.pdf", "overwrite": false}),
    },
    CommandSpec {
        name: "share_inspect",
        domain: Domain::Any,
        summary: "List comments, tracked changes, hidden text, properties and author names a copy would carry (effect read).",
        example: || json!({"command": "share_inspect"}),
    },
    CommandSpec {
        name: "share_sanitize_copy",
        domain: Domain::Any,
        summary: "Write a copy without the chosen content: comments, tracked_changes, hidden_text, properties, personal_info; the original is unchanged (effect export).",
        example: || {
            json!({"command": "share_sanitize_copy", "path": "clean.docx", "overwrite": false,
                "remove": ["comments", "tracked_changes", "personal_info"]})
        },
    },
    CommandSpec {
        name: "share_os_sheet",
        domain: Domain::Any,
        summary: "Build the request for the operating system share sheet for a file; the app performs the share (effect egress).",
        example: || json!({"command": "share_os_sheet", "path": "attachment.docx"}),
    },
];

/// The catalogue as the JSON the CLI and the MCP server return.
pub fn catalogue_json() -> Value {
    json!({
        "schema": "open365.commands/1",
        "count": COMMANDS.len(),
        "commands": COMMANDS.iter().map(CommandSpec::to_json).collect::<Vec<_>>(),
    })
}

pub fn find(name: &str) -> Option<&'static CommandSpec> {
    COMMANDS.iter().find(|c| c.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The variant names of `enum Command`, read from its source.
    fn enum_variants() -> Vec<String> {
        let source = include_str!("../../commands/src/command.rs");
        let start = source.find("pub enum Command {").expect("enum Command");
        let body = &source[start..];
        let end = body.find("\n}").expect("end of enum");
        body[..end]
            .lines()
            .skip(1)
            .filter(|l| l.starts_with("    ") && !l.starts_with("     "))
            .filter_map(|l| {
                let word: String = l
                    .trim()
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric())
                    .collect();
                word.chars()
                    .next()
                    .filter(char::is_ascii_uppercase)
                    .map(|_| word)
            })
            .collect()
    }

    fn snake(camel: &str) -> String {
        let mut out = String::new();
        for (i, c) in camel.chars().enumerate() {
            if c.is_ascii_uppercase() {
                if i > 0 {
                    out.push('_');
                }
                out.push(c.to_ascii_lowercase());
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn every_command_has_a_catalogue_entry() {
        let variants = enum_variants();
        assert!(variants.len() >= 31, "found only {variants:?}");
        let missing: Vec<String> = variants
            .iter()
            .map(|v| snake(v))
            .filter(|n| find(n).is_none())
            .collect();
        assert!(
            missing.is_empty(),
            "commands without CLI/MCP exposure (add them to registry::COMMANDS): {missing:?}"
        );
        let stale: Vec<&str> = COMMANDS
            .iter()
            .map(|c| c.name)
            .filter(|n| !variants.iter().any(|v| snake(v) == *n))
            .collect();
        assert!(stale.is_empty(), "entries for removed commands: {stale:?}");
        assert_eq!(COMMANDS.len(), variants.len());
    }

    #[test]
    fn examples_parse_and_carry_their_own_name() {
        for spec in COMMANDS {
            let command = spec
                .example_command()
                .unwrap_or_else(|| panic!("{} example does not parse", spec.name));
            assert_eq!(command.name(), spec.name);
        }
    }
}
