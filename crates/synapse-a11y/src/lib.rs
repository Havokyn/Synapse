#![allow(unsafe_code)]

mod cdp;
mod cdp_action;
mod cdp_dom;
mod error;
mod events;
mod ids;
mod platform;
mod re_resolve;
mod snapshot;
mod ui_element;
mod window;

pub use cdp::*;
#[cfg(windows)]
pub use cdp_action::*;
pub use cdp_dom::*;
pub use error::*;
pub use events::*;
pub use ids::*;
pub use re_resolve::*;
pub use snapshot::*;
pub use ui_element::*;
pub use window::*;

#[cfg(test)]
mod tests;
