use synapse_core::Key;

use crate::ActionError;

pub(super) fn hid_key_code(key: &Key) -> Result<u8, ActionError> {
    super::keymap::hid_usage(key)
}

pub(super) fn hid_text_key_code(ch: char) -> Result<u8, ActionError> {
    super::keymap::hid_usage_for_text_char(ch)
}
