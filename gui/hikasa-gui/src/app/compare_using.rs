//! *Compare Files Using* (M2 W13c, D-369; Session ▸ Compare Files Using ▸) of a Text Compare, Table Compare, Hex
//! Compare or Picture Compare view: the view's current specs in a new tab of another of the four kinds, with the
//! comparison's FF-002 format overrides; a parent link carries over (D-311: the new view is a child of the same
//! folder row); unsaved text edits are not carried (a message panel line says so). The text view's binary notice
//! offers *Compare in Hex*, the same as *Compare Files Using ▸ Hex Compare*.

use std::collections::BTreeMap;
use std::ffi::OsString;

use hikasa_session::api::ViewRequest;
use hikasa_session::ipc::{ViewId, ViewKind};
use hikasa_session::sessions::OpaqueTable;

use super::flow::{StoreDone, load_value};
use super::ops::open_pair;
use super::{AppCore, OpenTarget, ViewModel};
use crate::status::Message;
use crate::window::{OpenWithItem, Tab, WindowId};

/// The view kind of an item of *Compare Files Using* ▸ (`None`: not one of the four file views).
pub fn compare_using_kind(item: OpenWithItem) -> Option<ViewKind> {
    match item {
        OpenWithItem::TextCompare => Some(ViewKind::TextCompare),
        OpenWithItem::TableCompare => Some(ViewKind::TableCompare),
        OpenWithItem::HexCompare => Some(ViewKind::HexCompare),
        OpenWithItem::PictureCompare => Some(ViewKind::PictureCompare),
        _ => None,
    }
}

/// The format overrides of a request (FF-002).
fn with_overrides(request: &mut ViewRequest, overrides: BTreeMap<String, bool>) {
    match request {
        ViewRequest::Text { request, .. } => request.format_overrides = overrides,
        ViewRequest::Table { request, .. } => request.format_overrides = overrides,
        ViewRequest::Hex { request, .. } => request.format_overrides = overrides,
        ViewRequest::Picture { request, .. } => request.format_overrides = overrides,
        _ => {}
    }
}

impl AppCore {
    /// *Compare Files Using ▸ <kind>* of `view` in `window`.
    pub(super) fn compare_using(&mut self, window: WindowId, view: ViewId, kind: ViewKind) {
        let found: Option<(OsString, OsString, BTreeMap<String, bool>, bool)> =
            match self.tab(view).and_then(Tab::model) {
                Some(ViewModel::Text(m)) if !m.is_edit() => {
                    let r = m.request();
                    let unsaved = m
                        .live_view()
                        .is_some_and(|v| v.docs.left.is_modified() || v.docs.right.is_modified());
                    Some((
                        r.left.clone(),
                        r.right.clone(),
                        r.format_overrides.clone(),
                        unsaved,
                    ))
                }
                Some(ViewModel::Table(m)) => {
                    let r = m.request();
                    Some((
                        r.left.clone(),
                        r.right.clone(),
                        r.format_overrides.clone(),
                        false,
                    ))
                }
                Some(ViewModel::Hex(m)) => {
                    let r = m.request();
                    Some((
                        r.left.clone(),
                        r.right.clone(),
                        r.format_overrides.clone(),
                        false,
                    ))
                }
                Some(ViewModel::Picture(m)) => {
                    let r = m.request();
                    Some((
                        r.left.clone(),
                        r.right.clone(),
                        r.format_overrides.clone(),
                        false,
                    ))
                }
                _ => None,
            };
        let Some((left, right, overrides, unsaved)) = found else {
            return;
        };
        if unsaved {
            self.set_message(
                view,
                Message::info(
                    "The unsaved edits are not in the new view: it compares the files as saved.",
                ),
            );
        }
        let parse = |s: &OsString| self.service.parse_spec(s);
        let specs = match (parse(&left), parse(&right)) {
            (Ok(a), Ok(b)) => [a, b],
            (Err(e), _) | (_, Err(e)) => {
                self.set_message(view, Message::warning(e.to_string()));
                return;
            }
        };
        let parent = self.parent_of(view);
        self.store("open the pair", None, move |svc, _| {
            let mut opened = open_pair(svc, &specs, Some(kind))?;
            with_overrides(&mut opened.request, overrides);
            let (mut session, _) = svc.session_for_view(
                &opened.request,
                OpaqueTable::new(),
                OpaqueTable::new(),
                parent,
            )?;
            if session.parent.is_some() {
                svc.session_resolve_parent(&mut session)?;
            }
            let value = load_value(svc, session)?;
            Ok(StoreDone::Child(
                Box::new(value),
                OpenTarget::Window(window),
                None,
            ))
        });
    }
}
