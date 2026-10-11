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

#[cfg(test)]
mod prop {
    use super::*;
    use proptest::prelude::*;

    fn any_scope() -> impl Strategy<Value = LinkScope> {
        prop_oneof![
            Just(LinkScope::View),
            Just(LinkScope::Edit),
            Just(LinkScope::ViewNoDownload)
        ]
    }

    fn any_role() -> impl Strategy<Value = Role> {
        prop_oneof![Just(Role::Viewer), Just(Role::Editor), Just(Role::Owner)]
    }

    fn any_grant() -> impl Strategy<Value = Grant> {
        prop_oneof![
            any_role().prop_map(Grant::Role),
            any_scope().prop_map(Grant::Link)
        ]
    }

    fn within(a: Rights, b: Rights) -> bool {
        (!a.read || b.read)
            && (!a.download || b.download)
            && (!a.write || b.write)
            && (!a.manage || b.manage)
    }

    proptest! {
        #[test]
        fn rights_are_internally_consistent(g in any_grant()) {
            let r = rights(g);
            prop_assert!(r.read);
            prop_assert!(!r.write || r.download, "writers can read the content");
            prop_assert!(!r.manage || r.write);
            if matches!(g, Grant::Link(_)) {
                prop_assert!(!r.manage, "a link never manages access");
            }
        }

        #[test]
        fn a_link_never_exceeds_the_editor_role_and_roles_are_ordered(s in any_scope()) {
            let link = rights(Grant::Link(s));
            prop_assert!(within(link, rights(Grant::Role(Role::Editor))));
            prop_assert!(within(rights(Grant::Role(Role::Viewer)), rights(Grant::Role(Role::Editor))));
            prop_assert!(within(rights(Grant::Role(Role::Editor)), rights(Grant::Role(Role::Owner))));
        }

        #[test]
        fn only_edit_scopes_write_and_only_the_owner_manages(g in any_grant()) {
            let r = rights(g);
            let may_write = matches!(g, Grant::Role(Role::Editor | Role::Owner) | Grant::Link(LinkScope::Edit));
            prop_assert_eq!(r.write, may_write);
            prop_assert_eq!(r.manage, g == Grant::Role(Role::Owner));
        }

        #[test]
        fn scope_names_round_trip_and_nothing_else_parses(s in r"\PC{0,24}") {
            match LinkScope::parse(&s) {
                Some(scope) => prop_assert_eq!(scope.as_str(), s.as_str()),
                None => prop_assert!(!["view", "edit", "view-no-download"].contains(&s.as_str())),
            }
        }
    }
}
