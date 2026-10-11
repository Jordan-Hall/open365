//! One permission function for accounts and links (ADR-013): effective rights never exceed the grant.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Viewer,
    Editor,
    Owner,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Editor => "editor",
            Self::Owner => "owner",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkScope {
    View,
    Edit,
    ViewNoDownload,
}

impl LinkScope {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "view" => Some(Self::View),
            "edit" => Some(Self::Edit),
            "view-no-download" => Some(Self::ViewNoDownload),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::View => "view",
            Self::Edit => "edit",
            Self::ViewNoDownload => "view-no-download",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grant {
    Role(Role),
    Link(LinkScope),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rights {
    /// Metadata and version list.
    pub read: bool,
    /// Content bytes.
    pub download: bool,
    /// New versions.
    pub write: bool,
    /// ACL, links, audit view.
    pub manage: bool,
}

pub fn rights(g: Grant) -> Rights {
    match g {
        Grant::Role(Role::Viewer) | Grant::Link(LinkScope::View) => Rights {
            read: true,
            download: true,
            write: false,
            manage: false,
        },
        Grant::Link(LinkScope::ViewNoDownload) => Rights {
            read: true,
            download: false,
            write: false,
            manage: false,
        },
        Grant::Role(Role::Editor) | Grant::Link(LinkScope::Edit) => Rights {
            read: true,
            download: true,
            write: true,
            manage: false,
        },
        Grant::Role(Role::Owner) => Rights {
            read: true,
            download: true,
            write: true,
            manage: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Grant; 6] = [
        Grant::Role(Role::Viewer),
        Grant::Role(Role::Editor),
        Grant::Role(Role::Owner),
        Grant::Link(LinkScope::View),
        Grant::Link(LinkScope::Edit),
        Grant::Link(LinkScope::ViewNoDownload),
    ];

    #[test]
    fn links_never_manage() {
        for g in ALL {
            if matches!(g, Grant::Link(_)) {
                assert!(!rights(g).manage, "{g:?}");
            }
        }
    }

    #[test]
    fn write_implies_read_and_manage_implies_write() {
        for g in ALL {
            let r = rights(g);
            assert!(!r.write || r.read, "{g:?}");
            assert!(!r.manage || r.write, "{g:?}");
            assert!(r.read, "every grant can read metadata: {g:?}");
        }
    }

    #[test]
    fn view_no_download_cannot_download_or_write() {
        let r = rights(Grant::Link(LinkScope::ViewNoDownload));
        assert!(!r.download && !r.write);
    }

    #[test]
    fn link_rights_never_exceed_editor() {
        let e = rights(Grant::Role(Role::Editor));
        for s in [LinkScope::View, LinkScope::Edit, LinkScope::ViewNoDownload] {
            let r = rights(Grant::Link(s));
            assert!(
                (!r.read || e.read)
                    && (!r.download || e.download)
                    && (!r.write || e.write)
                    && !r.manage
            );
        }
    }
}
