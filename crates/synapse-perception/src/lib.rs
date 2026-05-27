mod error;
mod observe;
mod ocr;
mod template_match;

pub use error::{PerceptionError, PerceptionResult};
pub use observe::{
    A11yTreeSummary, ObservationAssembler, ObservationInput, ObserveInclude, assemble,
    assemble_from_input, auto_mode, auto_mode_with_a11y, bounded_sensor_latency,
    is_known_game_process, parse_perception_mode,
};
pub use ocr::{OcrProvider, TextRegion, is_empty_region, read_text, read_text_with_provider};
pub use template_match::{
    HudTemplate, TemplateCounterConfig, TemplateCounterReading, TemplateSlotReading,
    extract_template_counter_from_frame, extract_template_counter_from_region,
};

#[cfg(windows)]
pub use ocr::read_text_from_software_bitmap;
