//! Writer ribbon: the Help tab (`help.*`). Offline stubs: each control names the local
//! documentation instead of opening a web page.

use crate::ribbon::{Handled, RibbonCtx};

/// The control ids this module handles (ids as in the spec, without `=` or `#` suffixes).
pub const IDS: &[&str] = &[
    "help.help.help",
    "help.help.feedback",
    "help.help.show-training",
    "help.help.what-s-new",
    "help.mobile.get-mobile-app",
];

/// Runs command `id`; [`Handled::No`] when this module does not know it.
pub fn handle(ctx: &mut RibbonCtx<'_>, id: &str) -> Handled {
    let text = match id {
        "help.help.help" => {
            "Help: the documentation is in the docs folder (docs/architecture/app-shell.md)"
        }
        "help.help.feedback" => "Feedback: file an issue in the project repository",
        "help.help.show-training" => "Training: see docs/testing for the guided walkthroughs",
        "help.help.what-s-new" => "What is new: see the roadmap in docs/roadmap",
        "help.mobile.get-mobile-app" => "Mobile app: not available yet, see docs/roadmap",
        _ => return Handled::No,
    };
    if let Some(p) = ctx.writer.as_deref_mut() {
        p.status = Some(text.to_owned());
    }
    Handled::Yes
}
