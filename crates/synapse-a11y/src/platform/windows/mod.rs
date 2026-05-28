mod common;
mod events;
mod resolve;
mod snapshot;
mod window;

pub use events::{WinEventSubscription, subscribe_win_events};
pub use resolve::{expand_state_of, re_resolve};
pub use snapshot::{find_by_name_and_pattern, snapshot};
pub use window::{
    close_window, current_foreground_context, element_from_point, focus_window, focused_element,
    focused_window, foreground_context, is_window_minimized, visible_top_level_window_contexts,
    window_for_process, window_from_hwnd,
};
