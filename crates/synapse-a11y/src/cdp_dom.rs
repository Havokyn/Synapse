//! CDP DOM/accessibility tree → queryable `AccessibleNode` mapping (#685).
//!
//! When CDP is attached to a Chromium-family foreground, this module pulls the
//! page's `Accessibility.getFullAXTree` (+ `DOM.getBoxModel` for bounds) and maps
//! each node into the same [`AccessibleNode`] model the UIA path uses, so an
//! agent can `find(role="button", name_substring="Apply")` on a web page exactly
//! as it does on native UI.
//!
//! The pure mapping ([`build_accessible_nodes`]) is unit-tested with real CDP
//! response shapes; the async fetch ([`fetch_dom_snapshot`]) is the I/O wrapper
//! verified manually against a live Chrome (see `examples/cdp_axtree_probe.rs`).
//!
//! ## Element id scheme
//!
//! Web nodes get an [`ElementId`] of `<hwnd_hex>:cdcd<backendNodeId-hex>`. The
//! `cdcd` sentinel lets the action layer (#686) recognise a CDP-resolved node and
//! route it back through CDP (`DOM.scrollIntoViewIfNeeded` + box model +
//! `Input.dispatch*`) instead of UIA re-resolution. The backendNodeId round-trips
//! out of the id with no side registry.
//!
//! ## bbox semantics
//!
//! Web-node `bbox` is the element's CSS-pixel rectangle in page-layout
//! coordinates (from `DOM.getBoxModel`), NOT screen pixels. Actions do not rely
//! on it — they re-resolve the live box model at click time after scrolling the
//! node into view — but it lets an agent reason about on-page layout.

use synapse_core::{AccessibleNode, ElementId, Rect, element_id};

/// Sentinel prefix in the runtime-id portion of a web node's [`ElementId`].
/// Hex-only so the id still parses; `cdcd` is vanishingly unlikely to collide
/// with a real UIA runtime id.
pub const CDP_RUNTIME_PREFIX: &str = "cdcd";

/// One node distilled from a CDP `AXNode` (+ its resolved box model). This is the
/// crate-internal, browser-free representation the pure mapper consumes so it can
/// be unit-tested without a live Chrome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CdpDomNode {
    /// `DOM.BackendNodeId` of the associated DOM node (stable within a document).
    pub backend_node_id: i64,
    /// Backend id of this node's nearest mapped ancestor, if any.
    pub parent_backend_node_id: Option<i64>,
    /// Computed ARIA/AX role (e.g. `button`, `link`, `textbox`, `heading`).
    pub role: String,
    /// Accessible name.
    pub name: String,
    /// Accessible value (form fields, sliders), if any.
    pub value: Option<String>,
    /// Element rectangle in CSS px / page-layout coords, if a box model resolved.
    pub bbox: Option<Rect>,
    /// Number of mapped child nodes.
    pub child_count: u32,
    /// Whether the node is enabled (defaults true unless AX reports disabled).
    pub enabled: bool,
    /// Whether the node is focused.
    pub focused: bool,
}

/// Builds an [`ElementId`] for a web node from the browser window `hwnd` and the
/// DOM `backend_node_id`.
#[must_use]
pub fn cdp_element_id(hwnd: i64, backend_node_id: i64) -> ElementId {
    // Backend ids are non-negative and small; mask to a stable unsigned hex.
    let unsigned = u64::from_ne_bytes(backend_node_id.to_ne_bytes());
    element_id(hwnd, &format!("{CDP_RUNTIME_PREFIX}{unsigned:012x}"))
}

/// If `id` is a CDP web-node id (`…:cdcd<hex>`), returns its `backend_node_id`.
/// Returns `None` for ordinary UIA element ids so the action layer can tell them
/// apart.
#[must_use]
pub fn cdp_backend_from_element_id(id: &ElementId) -> Option<i64> {
    let parts = id.parts().ok()?;
    let hex = parts.runtime_id_hex.strip_prefix(CDP_RUNTIME_PREFIX)?;
    let unsigned = u64::from_str_radix(hex, 16).ok()?;
    Some(i64::from_ne_bytes(unsigned.to_ne_bytes()))
}

/// Pure mapping: CDP nodes → `AccessibleNode`s with stable ids, depth computed
/// from the parent-backend chain, and bounds carried through. Nodes are returned
/// in input order, capped at `max_nodes`. The mapping never invents data — a node
/// with no box model gets a zero `bbox` (callers may filter on it).
#[must_use]
pub fn build_accessible_nodes(
    hwnd: i64,
    nodes: &[CdpDomNode],
    max_nodes: usize,
) -> Vec<AccessibleNode> {
    use std::collections::HashMap;

    // Depth = chain length from a root (a node whose parent is absent or not in
    // the set). Memoised so a deep tree stays linear.
    let by_backend: HashMap<i64, &CdpDomNode> = nodes
        .iter()
        .map(|node| (node.backend_node_id, node))
        .collect();
    let mut depth_cache: HashMap<i64, u32> = HashMap::new();

    nodes
        .iter()
        .take(max_nodes)
        .map(|node| {
            let depth = depth_of(node.backend_node_id, &by_backend, &mut depth_cache, 256);
            AccessibleNode {
                element_id: cdp_element_id(hwnd, node.backend_node_id),
                parent: node
                    .parent_backend_node_id
                    .filter(|parent| by_backend.contains_key(parent))
                    .map(|parent| cdp_element_id(hwnd, parent)),
                name: node.name.clone(),
                role: node.role.clone(),
                automation_id: Some(format!("cdp:backendNodeId={}", node.backend_node_id)),
                value: node.value.clone(),
                bbox: node.bbox.unwrap_or(Rect {
                    x: 0,
                    y: 0,
                    w: 0,
                    h: 0,
                }),
                enabled: node.enabled,
                focused: node.focused,
                patterns: Vec::new(),
                children_count: node.child_count,
                depth,
            }
        })
        .collect()
}

/// Depth of `backend` = chain length to a root, memoised in `cache`. `guard`
/// bounds recursion against a malformed (cyclic) parent chain.
fn depth_of(
    backend: i64,
    by_backend: &std::collections::HashMap<i64, &CdpDomNode>,
    cache: &mut std::collections::HashMap<i64, u32>,
    guard: u32,
) -> u32 {
    if let Some(found) = cache.get(&backend) {
        return *found;
    }
    if guard == 0 {
        return 0;
    }
    let depth = by_backend
        .get(&backend)
        .and_then(|node| node.parent_backend_node_id)
        .filter(|parent| by_backend.contains_key(parent))
        .map_or(0, |parent| {
            depth_of(parent, by_backend, cache, guard - 1) + 1
        });
    cache.insert(backend, depth);
    depth
}

/// Axis-aligned bounding rect of a CDP box-model content quad
/// (`[x1,y1,x2,y2,x3,y3,x4,y4]`). Returns `None` for a malformed quad.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    reason = "page coordinates are rounded then cast into the i32 Rect space"
)]
pub fn rect_from_quad(quad: &[f64]) -> Option<Rect> {
    if quad.len() < 8 {
        return None;
    }
    let xs = [quad[0], quad[2], quad[4], quad[6]];
    let ys = [quad[1], quad[3], quad[5], quad[7]];
    let min_x = xs.iter().copied().fold(f64::INFINITY, f64::min);
    let min_y = ys.iter().copied().fold(f64::INFINITY, f64::min);
    let max_x = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let max_y = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !(min_x.is_finite() && min_y.is_finite() && max_x.is_finite() && max_y.is_finite()) {
        return None;
    }
    Some(Rect {
        x: min_x.round() as i32,
        y: min_y.round() as i32,
        w: (max_x - min_x).round().max(0.0) as i32,
        h: (max_y - min_y).round().max(0.0) as i32,
    })
}

/// A fully-resolved CDP DOM snapshot ready to fold into observation elements.
#[derive(Clone, Debug)]
pub struct CdpDomSnapshot {
    /// Mapped, queryable web nodes.
    pub nodes: Vec<AccessibleNode>,
    /// Total non-ignored AX nodes the page exposed (before `max_nodes` capping).
    pub total_ax_nodes: u32,
    /// Page URL the snapshot came from (diagnostics).
    pub page_url: String,
}

/// Attaches CDP at `endpoint`, selects the page matching `foreground_title`
/// (falling back to the first page), pulls `Accessibility.getFullAXTree` and
/// per-node box models, and maps everything into queryable [`AccessibleNode`]s
/// owned by `hwnd`.
///
/// Fail-loud: any attach/tree failure returns an `A11yError` with a specific
/// code (`A11Y_CDP_ATTACH_FAILED` / `A11Y_CDP_AXTREE_FAILED`) — never a silent
/// empty tree.
///
/// # Errors
///
/// Returns [`crate::A11yError::CdpAttachFailed`] when the client cannot connect
/// or no page target exists, and [`crate::A11yError::CdpAxtreeFailed`] when the
/// accessibility tree cannot be retrieved.
#[cfg(windows)]
#[allow(clippy::future_not_send)]
pub async fn fetch_dom_snapshot(
    endpoint: &str,
    hwnd: i64,
    foreground_title: &str,
    max_nodes: usize,
) -> crate::A11yResult<CdpDomSnapshot> {
    use std::collections::HashMap;

    use chromiumoxide::Browser;
    use chromiumoxide::cdp::browser_protocol::accessibility::{EnableParams, GetFullAxTreeParams};
    use chromiumoxide::cdp::browser_protocol::dom::{BackendNodeId, GetBoxModelParams};
    use futures_util::StreamExt as _;

    use crate::A11yError;

    let (browser, mut handler) =
        Browser::connect(endpoint)
            .await
            .map_err(|err| A11yError::CdpAttachFailed {
                detail: format!("connect {endpoint}: {err}"),
            })?;
    let handler_task = tokio::spawn(async move { while handler.next().await.is_some() {} });

    let result = async {
        let pages = browser
            .pages()
            .await
            .map_err(|err| A11yError::CdpAttachFailed {
                detail: format!("list pages: {err}"),
            })?;
        if pages.is_empty() {
            return Err(A11yError::CdpAttachFailed {
                detail: "no page targets attached to the browser".to_owned(),
            });
        }

        // Prefer the page whose title matches the foreground window title; else the
        // first page target.
        let mut chosen = None;
        for page in &pages {
            if let Ok(Some(title)) = page.get_title().await
                && !title.is_empty()
                && foreground_title.contains(title.as_str())
            {
                chosen = Some(page.clone());
                break;
            }
        }
        let page = chosen.unwrap_or_else(|| pages[0].clone());
        let page_url = page.url().await.ok().flatten().unwrap_or_default();

        page.execute(EnableParams::default())
            .await
            .map_err(|err| A11yError::CdpAxtreeFailed {
                detail: format!("Accessibility.enable: {err}"),
            })?;
        let tree = page
            .execute(GetFullAxTreeParams::default())
            .await
            .map_err(|err| A11yError::CdpAxtreeFailed {
                detail: format!("Accessibility.getFullAXTree: {err}"),
            })?;

        let ax_nodes = &tree.result.nodes;
        // Index by AX node id so we can resolve the nearest backend-bearing ancestor.
        let by_ax_id: HashMap<&str, &_> = ax_nodes
            .iter()
            .map(|node| (node.node_id.inner().as_str(), node))
            .collect();

        let mut dom_nodes: Vec<CdpDomNode> = Vec::new();
        let mut total_ax_nodes = 0_u32;
        for node in ax_nodes {
            if node.ignored {
                continue;
            }
            total_ax_nodes = total_ax_nodes.saturating_add(1);
            let Some(backend) = node.backend_dom_node_id.as_ref().map(|id| *id.inner()) else {
                continue;
            };
            let role = ax_value_string(node.role.as_ref());
            if role.is_empty() {
                continue;
            }
            let name = ax_value_string(node.name.as_ref());
            let value = {
                let value = ax_value_string(node.value.as_ref());
                (!value.is_empty()).then_some(value)
            };
            let parent_backend = nearest_backend_ancestor(node, &by_ax_id);
            let child_count = node
                .child_ids
                .as_ref()
                .map_or(0, |ids| u32::try_from(ids.len()).unwrap_or(u32::MAX));

            let bbox = if dom_nodes.len() < max_nodes {
                let params = GetBoxModelParams::builder()
                    .backend_node_id(BackendNodeId::new(backend))
                    .build();
                page.execute(params)
                    .await
                    .ok()
                    .and_then(|response| rect_from_quad(response.result.model.content.inner()))
            } else {
                None
            };

            dom_nodes.push(CdpDomNode {
                backend_node_id: backend,
                parent_backend_node_id: parent_backend,
                role,
                name,
                value,
                bbox,
                child_count,
                enabled: true,
                focused: false,
            });
        }

        let nodes = build_accessible_nodes(hwnd, &dom_nodes, max_nodes);
        Ok(CdpDomSnapshot {
            nodes,
            total_ax_nodes,
            page_url,
        })
    }
    .await;

    handler_task.abort();
    result
}

/// Extracts the string value of a CDP `AxValue` (role/name/value), empty if none.
#[cfg(windows)]
fn ax_value_string(
    value: Option<&chromiumoxide::cdp::browser_protocol::accessibility::AxValue>,
) -> String {
    value
        .and_then(|value| value.value.as_ref())
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Walks an AX node's `parentId` chain to the first ancestor that has a backend
/// DOM node id, so the mapped tree stays connected even across generic AX nodes.
#[cfg(windows)]
fn nearest_backend_ancestor(
    node: &chromiumoxide::cdp::browser_protocol::accessibility::AxNode,
    by_ax_id: &std::collections::HashMap<
        &str,
        &chromiumoxide::cdp::browser_protocol::accessibility::AxNode,
    >,
) -> Option<i64> {
    let mut current = node.parent_id.as_ref()?.inner().as_str();
    for _ in 0..256 {
        let parent = by_ax_id.get(current)?;
        if let Some(backend) = parent.backend_dom_node_id.as_ref() {
            return Some(*backend.inner());
        }
        current = parent.parent_id.as_ref()?.inner().as_str();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cdp_element_id_round_trips_backend_node_id() {
        let hwnd = 0x000A_BCDE;
        let id = cdp_element_id(hwnd, 42);
        println!("readback=cdp_element_id before=backend:42 after=id:{id}");
        assert!(id.to_string().contains(":cdcd"));
        assert_eq!(cdp_backend_from_element_id(&id), Some(42));
    }

    #[test]
    fn uia_element_ids_are_not_mistaken_for_cdp() {
        let uia = element_id(0x1234, "0000002a00000001");
        println!(
            "readback=cdp_detect edge=uia id:{uia} backend:{:?}",
            cdp_backend_from_element_id(&uia)
        );
        assert_eq!(cdp_backend_from_element_id(&uia), None);
    }

    #[test]
    fn rect_from_quad_computes_axis_aligned_bounds() {
        // The real Apply-button quad observed against Chrome 149 in FSV.
        let quad = [
            16.0, 69.4375, 49.359, 69.4375, 49.359, 84.4375, 16.0, 84.4375,
        ];
        let rect = rect_from_quad(&quad).expect("valid quad yields a rect");
        println!("readback=rect_from_quad before=quad:{quad:?} after=rect:{rect:?}");
        assert_eq!(rect.x, 16);
        assert_eq!(rect.y, 69);
        assert_eq!(rect.w, 33);
        assert_eq!(rect.h, 15);
    }

    #[test]
    fn rect_from_quad_rejects_short_quad() {
        assert_eq!(rect_from_quad(&[1.0, 2.0, 3.0]), None);
    }

    #[test]
    fn build_accessible_nodes_assigns_depth_and_ids() {
        let nodes = vec![
            CdpDomNode {
                backend_node_id: 1,
                parent_backend_node_id: None,
                role: "RootWebArea".to_owned(),
                name: "Apply to YC".to_owned(),
                value: None,
                bbox: Some(Rect {
                    x: 0,
                    y: 0,
                    w: 1600,
                    h: 900,
                }),
                child_count: 1,
                enabled: true,
                focused: false,
            },
            CdpDomNode {
                backend_node_id: 6,
                parent_backend_node_id: Some(1),
                role: "button".to_owned(),
                name: "Apply".to_owned(),
                value: None,
                bbox: Some(Rect {
                    x: 16,
                    y: 69,
                    w: 33,
                    h: 15,
                }),
                child_count: 0,
                enabled: true,
                focused: true,
            },
        ];
        let mapped = build_accessible_nodes(0x2200, &nodes, 60);
        println!(
            "readback=build_nodes after=count:{} roles:{:?}",
            mapped.len(),
            mapped
                .iter()
                .map(|node| (node.role.clone(), node.depth))
                .collect::<Vec<_>>()
        );
        assert_eq!(mapped.len(), 2);
        assert_eq!(mapped[0].depth, 0);
        assert_eq!(mapped[1].depth, 1);
        assert_eq!(mapped[1].role, "button");
        assert_eq!(mapped[1].name, "Apply");
        // Child's parent id is the root's cdp id, enabling tree navigation.
        assert_eq!(mapped[1].parent.as_ref(), Some(&cdp_element_id(0x2200, 1)));
        // Backend id recovers from the mapped element id (action routing #686).
        assert_eq!(cdp_backend_from_element_id(&mapped[1].element_id), Some(6));
    }

    #[test]
    fn build_accessible_nodes_caps_at_max() {
        let nodes: Vec<CdpDomNode> = (0..10)
            .map(|index| CdpDomNode {
                backend_node_id: index,
                parent_backend_node_id: None,
                role: "link".to_owned(),
                name: format!("link-{index}"),
                value: None,
                bbox: None,
                child_count: 0,
                enabled: true,
                focused: false,
            })
            .collect();
        let mapped = build_accessible_nodes(0x10, &nodes, 4);
        println!(
            "readback=build_nodes_cap before=in:10 max:4 after=out:{}",
            mapped.len()
        );
        assert_eq!(mapped.len(), 4);
    }
}
