//! `ASC`, `DBCS`, `PHONETIC` and `ENCODEURL`.
//!
//! Measured on an Excel without an East Asian editing language (en-GB):
//! * `ASC` and `DBCS` return their argument as text unchanged (numbers and booleans are converted to text, an
//!   error propagates). The full-width/half-width conversion happens only when a double-byte language is enabled.
//! * `PHONETIC` is `#N/A` for every reference: no cell carries phonetic (furigana) data. Excel refuses a literal
//!   argument and more than one argument when it parses the formula; this engine does not.
//! * `JIS` is not a function in this Excel (`#NAME?`) and is left unregistered.
//! * `ENCODEURL` percent-encodes the UTF-8 bytes of the text. Unreserved are the ASCII letters, digits and
//!   `-`, `_`, `.`; every other byte is `%XX` in upper case (`~`, `!`, `*`, `'`, `(` and `)` included). A result
//!   longer than 32,767 characters is `#VALUE!`.

use super::{Res, arity, text_arg};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value};

/// Excel's maximum cell text length.
const MAX_TEXT: usize = 32_767;

pub(super) fn asc(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    Ok(Value::Text(text_arg(ctx, &args[0])?))
}

pub(super) fn phonetic(_ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    Err(ErrorKind::NA)
}

pub(super) fn encodeurl(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    let text = text_arg(ctx, &args[0])?;
    let mut out = String::with_capacity(text.len());
    for &b in text.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    if out.len() > MAX_TEXT {
        return Err(ErrorKind::Value);
    }
    Ok(Value::Text(out))
}
