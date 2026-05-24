use std::fmt::Display;

use synapse_core::{ElementId, Point};

#[cfg(windows)]
use synapse_a11y::{UIElement, uiautomation::types::Rect as UiaRect};

use crate::{ActionError, ActionResult};

use super::CoordinateFallbackPlan;

#[cfg(windows)]
pub(super) fn resolve_element(element_id: &ElementId) -> ActionResult<UIElement> {
    synapse_a11y::re_resolve(element_id).map_err(element_not_resolved)
}

#[cfg(windows)]
pub(super) fn coordinate_fallback_plan(
    element_id: &ElementId,
    element: &UIElement,
) -> ActionResult<CoordinateFallbackPlan> {
    let parts = element_id.parts().map_err(target_invalid)?;
    let rect = element.get_bounding_rectangle().map_err(|err| {
        target_invalid(format!(
            "element {element_id} bounding rectangle unavailable: {err}"
        ))
    })?;
    let screen_point = center_from_rect_edges(RectEdges::from(rect))?;
    let window_point =
        synapse_capture::screen_to_window(screen_point, parts.hwnd).map_err(|err| {
            target_invalid(format!(
                "element {element_id} screen_to_window failed for {screen_point:?}: {err}"
            ))
        })?;

    Ok(CoordinateFallbackPlan {
        screen_point,
        window_point,
    })
}

#[must_use]
#[cfg(any(test, windows))]
pub(super) fn element_not_resolved(error: impl Display) -> ActionError {
    ActionError::ElementNotResolved {
        detail: error.to_string(),
    }
}

#[must_use]
#[cfg(any(test, windows))]
pub(super) fn invoke_pattern_unavailable(
    element_id: &ElementId,
    error: impl Display,
) -> ActionError {
    ActionError::TargetInvalid {
        detail: format!("element {element_id} does not expose InvokePattern: {error}"),
    }
}

#[must_use]
#[cfg(any(test, windows))]
pub(super) fn invoke_pattern_failed(element_id: &ElementId, error: impl Display) -> ActionError {
    ActionError::TargetInvalid {
        detail: format!("InvokePattern.invoke failed for element {element_id}: {error}"),
    }
}

#[must_use]
#[cfg(any(test, windows))]
pub(super) fn target_invalid(error: impl Display) -> ActionError {
    ActionError::TargetInvalid {
        detail: error.to_string(),
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[cfg(any(test, windows))]
pub(super) struct RectEdges {
    pub(super) left: i32,
    pub(super) top: i32,
    pub(super) right: i32,
    pub(super) bottom: i32,
}

#[cfg(windows)]
impl From<UiaRect> for RectEdges {
    fn from(value: UiaRect) -> Self {
        Self {
            left: value.get_left(),
            top: value.get_top(),
            right: value.get_right(),
            bottom: value.get_bottom(),
        }
    }
}

#[cfg(any(test, windows))]
pub(super) fn center_from_rect_edges(rect: RectEdges) -> ActionResult<Point> {
    if rect.right <= rect.left || rect.bottom <= rect.top {
        return Err(ActionError::TargetInvalid {
            detail: format!("element bounding rectangle is empty or inverted: {rect:?}"),
        });
    }

    let width = i64::from(rect.right) - i64::from(rect.left);
    let height = i64::from(rect.bottom) - i64::from(rect.top);
    let x = i64::from(rect.left) + width / 2;
    let y = i64::from(rect.top) + height / 2;

    Ok(Point {
        x: i32::try_from(x).map_err(target_invalid)?,
        y: i32::try_from(y).map_err(target_invalid)?,
    })
}
