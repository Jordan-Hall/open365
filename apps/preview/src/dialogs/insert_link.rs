//! Insert Link (Insert > Link, Ctrl+K): turns the text of the dialog's address box into a link
//! target. `#Sheet2!A1` is Place in This Document, `mailto:` an e-mail address, anything else a
//! web address (a bare `www.` host gets `http://`).

use open365_commands::objects_ops::LinkKind;

/// The link target a typed address means.
///
/// # Errors
/// The address is empty.
pub fn parse_target(text: &str) -> Result<LinkKind, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err("Type the address of the link.".to_owned());
    }
    if let Some(place) = t.strip_prefix('#') {
        return Ok(LinkKind::Place {
            reference: place.to_owned(),
        });
    }
    if let Some(rest) = t.strip_prefix("mailto:") {
        let (address, query) = rest.split_once('?').unwrap_or((rest, ""));
        let subject = query
            .split('&')
            .find_map(|p| p.strip_prefix("subject="))
            .map(str::to_owned);
        return Ok(LinkKind::Email {
            address: address.to_owned(),
            subject,
        });
    }
    let has_scheme = t.split_once(':').is_some_and(|(s, _)| {
        s.len() > 1
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    });
    if has_scheme || t.contains(['\\', ' ']) || !t.contains('.') {
        return Ok(LinkKind::Url {
            address: t.to_owned(),
        });
    }
    if t.contains('@') {
        return Ok(LinkKind::Email {
            address: t.to_owned(),
            subject: None,
        });
    }
    Ok(LinkKind::Url {
        address: format!("http://{t}"),
    })
}

/// The text a link shows when the cell is empty and no text was typed: the address.
#[must_use]
pub fn default_text(target: &LinkKind) -> String {
    match target {
        LinkKind::Url { address } | LinkKind::Email { address, .. } => address.clone(),
        LinkKind::Place { reference } => reference.clone(),
    }
}
