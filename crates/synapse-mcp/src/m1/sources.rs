use std::{collections::BTreeMap, time::Instant};

use chrono::Utc;
use rmcp::ErrorData;
use sha2::{Digest as _, Sha256};
use synapse_action::{ClipboardFormat, read_clipboard_text};
use synapse_core::{
    AccessibleNode, AudioContext, ClipboardSummary, DetectedEntity, FocusedElement,
    ForegroundContext, HudReading, HudReadings, HudValue, PerceptionMode, Rect, SensorStatus,
    UiaPattern, element_id, entity_id,
};
use synapse_perception::ObservationInput;

pub fn synthetic_notepad_input() -> ObservationInput {
    let at = Utc::now();
    let focused_id = element_id(0x1234, "0000002a00000001");
    let elements = vec![
        node(0, 0, "Notepad", "Window", false),
        node(1, 1, "Document", "Edit", true),
        node(2, 1, "File", "MenuItem", false),
        node(3, 1, "Edit", "MenuItem", false),
        node(4, 1, "View", "MenuItem", false),
        node(5, 1, "Status", "Text", false),
    ];
    let mut latency = BTreeMap::new();
    latency.insert("a11y".to_owned(), 1.25);
    latency.insert("capture".to_owned(), 0.50);
    ObservationInput {
        foreground: ForegroundContext {
            hwnd: 0x1234,
            pid: 44,
            process_name: "notepad.exe".to_owned(),
            process_path: "C:\\Windows\\System32\\notepad.exe".to_owned(),
            window_title: "manual.txt - Notepad".to_owned(),
            window_bounds: Rect {
                x: 10,
                y: 20,
                w: 800,
                h: 600,
            },
            monitor_index: 0,
            dpi_scale: 1.0,
            profile_id: None,
            steam_appid: None,
            is_fullscreen: false,
            is_dwm_composed: true,
        },
        focused: Some(FocusedElement {
            element_id: focused_id,
            name: "Document".to_owned(),
            role: "Edit".to_owned(),
            automation_id: Some("15".to_owned()),
            bbox: Rect {
                x: 12,
                y: 80,
                w: 760,
                h: 480,
            },
            enabled: true,
            patterns: vec![UiaPattern::Text, UiaPattern::Value],
            value: Some("Synthetic Synapse text".to_owned()),
            selected_text: None,
        }),
        elements,
        entities: vec![DetectedEntity {
            entity_id: entity_id(9),
            track_id: 9,
            class_label: "cursor".to_owned(),
            bbox: Rect {
                x: 40,
                y: 90,
                w: 8,
                h: 20,
            },
            confidence: 0.80,
            first_seen_at: at,
            last_seen_at: at,
            velocity_px_per_s: None,
        }],
        hud: HudReadings::default(),
        audio: AudioContext::default(),
        recent_events: Vec::new(),
        clipboard_summary: None,
        fs_recent: Vec::new(),
        sensor_latency_ms: latency,
        a11y_status: SensorStatus::Healthy,
        capture_status: SensorStatus::Healthy,
        detection_status: SensorStatus::Disabled,
        audio_status: SensorStatus::Disabled,
        mode_override: None,
    }
}

pub fn populate_clipboard_summary(input: &mut ObservationInput) {
    let started = Instant::now();
    let summary = match read_clipboard_text(ClipboardFormat::Unicode) {
        Ok(text) => clipboard_summary_from_text(&text),
        Err(error) => {
            tracing::debug!(
                code = "OBSERVE_CLIPBOARD_READ_FAILED",
                error = %error,
                "clipboard summary read failed"
            );
            ClipboardSummary {
                formats: Vec::new(),
                text_len: None,
                text_excerpt: None,
                redacted: true,
            }
        }
    };
    input.sensor_latency_ms.insert(
        "clipboard".to_owned(),
        started.elapsed().as_secs_f32() * 1000.0,
    );
    input.clipboard_summary = Some(summary);
}

fn clipboard_summary_from_text(text: &str) -> ClipboardSummary {
    if text.is_empty() {
        return ClipboardSummary {
            formats: Vec::new(),
            text_len: None,
            text_excerpt: None,
            redacted: true,
        };
    }
    ClipboardSummary {
        formats: vec!["text/plain".to_owned(), "text/unicode".to_owned()],
        text_len: Some(len_to_u32(text.chars().count())),
        text_excerpt: Some(sha256_hex(text.as_bytes())),
        redacted: true,
    }
}

fn len_to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{}", hex_lower(&digest))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_summary_hashes_text_without_raw_excerpt() {
        let summary = clipboard_summary_from_text("issue544-known-input");

        assert_eq!(
            summary.formats,
            vec!["text/plain".to_owned(), "text/unicode".to_owned()]
        );
        assert_eq!(summary.text_len, Some(20));
        assert_eq!(summary.redacted, true);
        let excerpt = summary.text_excerpt.as_deref().unwrap_or_default();
        assert!(excerpt.starts_with("sha256:"));
        assert!(!excerpt.contains("issue544-known-input"));
    }

    #[test]
    fn clipboard_summary_empty_text_is_redacted_empty_metadata() {
        let summary = clipboard_summary_from_text("");

        assert!(summary.formats.is_empty());
        assert_eq!(summary.text_len, None);
        assert_eq!(summary.text_excerpt, None);
        assert_eq!(summary.redacted, true);
    }
}

fn node(sequence: u32, depth: u32, name: &str, role: &str, focused: bool) -> AccessibleNode {
    let depth_i32 = i32::try_from(depth).unwrap_or(0);
    let sequence_i32 = i32::try_from(sequence).unwrap_or(0);
    AccessibleNode {
        element_id: element_id(0x1234, &format!("0000002a{sequence:08x}")),
        parent: (depth > 0).then(|| element_id(0x1234, "0000002a00000000")),
        name: name.to_owned(),
        role: role.to_owned(),
        automation_id: None,
        bbox: Rect {
            x: 10 + depth_i32,
            y: 20 + sequence_i32.saturating_mul(10),
            w: 100,
            h: 30,
        },
        enabled: true,
        focused,
        patterns: Vec::new(),
        children_count: 0,
        depth,
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn platform_input(_depth: u32, mode: PerceptionMode) -> Result<ObservationInput, ErrorData> {
    linux_x11::platform_input(mode)
}

#[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
pub fn platform_input(_depth: u32, _mode: PerceptionMode) -> Result<ObservationInput, ErrorData> {
    Err(crate::m1::mcp_error(
        synapse_core::error_codes::OBSERVE_NO_PERCEPTION_AVAILABLE,
        "UIA foreground window lookup requires Windows",
    ))
}

#[cfg(all(unix, not(target_os = "macos")))]
mod linux_x11 {
    use std::{collections::BTreeMap, fs, path::PathBuf, time::Instant};

    use chrono::Utc;
    use rmcp::ErrorData;
    use synapse_core::{
        AccessibleNode, DetectedEntity, ForegroundContext, PerceptionMode, Rect, SensorStatus,
        element_id, entity_id, error_codes,
    };
    use synapse_perception::ObservationInput;
    use x11rb::{
        connection::Connection,
        protocol::xproto::{Atom, AtomEnum, ConnectionExt as _, Window},
        rust_connection::RustConnection,
    };

    use crate::m1::mcp_error;

    pub fn platform_input(mode: PerceptionMode) -> Result<ObservationInput, ErrorData> {
        let started = Instant::now();
        let (conn, screen_num) = RustConnection::connect(None)
            .map_err(|err| unavailable(format!("X11 connect failed: {err}")))?;
        let screen =
            conn.setup().roots.get(screen_num).ok_or_else(|| {
                unavailable(format!("X11 screen index {screen_num} was not found"))
            })?;
        let root_bounds = Rect {
            x: 0,
            y: 0,
            w: i32::from(screen.width_in_pixels),
            h: i32::from(screen.height_in_pixels),
        };
        let active = active_window(&conn, screen.root);
        let window = active.unwrap_or(screen.root);
        let bounds = window_bounds(&conn, window).unwrap_or(root_bounds);
        let pid = window_pid(&conn, window).unwrap_or_default();
        let title = window_title(&conn, window)
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| {
                if active.is_some() {
                    format!("X11 window 0x{window:x}")
                } else {
                    format!(
                        "X11 display {}",
                        std::env::var("DISPLAY").unwrap_or_default()
                    )
                }
            });
        let (process_name, process_path) = process_metadata(pid);
        let foreground = ForegroundContext {
            hwnd: i64::from(window),
            pid,
            process_name,
            process_path,
            window_title: title,
            window_bounds: bounds,
            monitor_index: u32::try_from(screen_num).unwrap_or(u32::MAX),
            dpi_scale: 1.0,
            profile_id: None,
            steam_appid: None,
            is_fullscreen: bounds.x <= root_bounds.x
                && bounds.y <= root_bounds.y
                && bounds.w >= root_bounds.w
                && bounds.h >= root_bounds.h,
            is_dwm_composed: false,
        };
        let mut input = ObservationInput::new(foreground.clone());
        input.elements = vec![window_node(&foreground)];
        input.focused = None;
        input.entities = cursor_entity().into_iter().collect();
        input.a11y_status = SensorStatus::DegradedSensorFailed {
            reason_code: "LINUX_X11_WINDOW_METADATA_ONLY".to_owned(),
        };
        input.capture_status = SensorStatus::Healthy;
        input.detection_status = SensorStatus::Disabled;
        input.audio_status = SensorStatus::Disabled;
        input.mode_override = Some(mode);
        input.sensor_latency_ms = BTreeMap::from([(
            "x11_window_metadata".to_owned(),
            started.elapsed().as_secs_f32() * 1000.0,
        )]);
        Ok(input)
    }

    fn active_window(conn: &RustConnection, root: Window) -> Option<Window> {
        let atom = intern_atom(conn, b"_NET_ACTIVE_WINDOW").ok()?;
        let reply = conn
            .get_property(false, root, atom, AtomEnum::WINDOW, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        let window = reply.value32()?.next()?;
        if window == 0 || window_bounds(conn, window).is_none() {
            None
        } else {
            Some(window)
        }
    }

    fn window_bounds(conn: &RustConnection, window: Window) -> Option<Rect> {
        let geometry = conn.get_geometry(window).ok()?.reply().ok()?;
        Some(Rect {
            x: i32::from(geometry.x),
            y: i32::from(geometry.y),
            w: i32::from(geometry.width),
            h: i32::from(geometry.height),
        })
    }

    fn window_title(conn: &RustConnection, window: Window) -> Option<String> {
        let utf8 = intern_atom(conn, b"UTF8_STRING").ok()?;
        let net_wm_name = intern_atom(conn, b"_NET_WM_NAME").ok()?;
        read_string_property(conn, window, net_wm_name, utf8).or_else(|| {
            read_string_property(
                conn,
                window,
                AtomEnum::WM_NAME.into(),
                AtomEnum::STRING.into(),
            )
        })
    }

    fn window_pid(conn: &RustConnection, window: Window) -> Option<u32> {
        let atom = intern_atom(conn, b"_NET_WM_PID").ok()?;
        let reply = conn
            .get_property(false, window, atom, AtomEnum::CARDINAL, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        reply.value32()?.next()
    }

    fn read_string_property(
        conn: &RustConnection,
        window: Window,
        property: Atom,
        property_type: Atom,
    ) -> Option<String> {
        let reply = conn
            .get_property(false, window, property, property_type, 0, 4096)
            .ok()?
            .reply()
            .ok()?;
        let bytes = trim_nul(reply.value);
        if bytes.is_empty() {
            return None;
        }
        String::from_utf8(bytes).ok()
    }

    fn intern_atom(conn: &RustConnection, name: &[u8]) -> Result<Atom, ErrorData> {
        conn.intern_atom(false, name)
            .map_err(|err| unavailable(format!("X11 intern_atom failed: {err}")))?
            .reply()
            .map(|reply| reply.atom)
            .map_err(|err| unavailable(format!("X11 intern_atom reply failed: {err}")))
    }

    fn trim_nul(mut bytes: Vec<u8>) -> Vec<u8> {
        while bytes.last() == Some(&0) {
            bytes.pop();
        }
        bytes
    }

    fn window_node(foreground: &ForegroundContext) -> AccessibleNode {
        AccessibleNode {
            element_id: element_id(foreground.hwnd, "0000000000000000"),
            parent: None,
            name: foreground.window_title.clone(),
            role: "Window".to_owned(),
            automation_id: None,
            bbox: foreground.window_bounds,
            enabled: true,
            focused: false,
            patterns: Vec::new(),
            children_count: 0,
            depth: 0,
        }
    }

    fn cursor_entity() -> Option<DetectedEntity> {
        let point = synapse_action::backend::software::cursor_position().ok()?;
        let at = Utc::now();
        Some(DetectedEntity {
            entity_id: entity_id(0),
            track_id: 0,
            class_label: "cursor".to_owned(),
            bbox: Rect {
                x: point.x,
                y: point.y,
                w: 1,
                h: 1,
            },
            confidence: 1.0,
            first_seen_at: at,
            last_seen_at: at,
            velocity_px_per_s: None,
        })
    }

    fn process_metadata(pid: u32) -> (String, String) {
        if pid == 0 {
            return (
                "x11".to_owned(),
                std::env::var("DISPLAY").unwrap_or_default(),
            );
        }
        let comm_path = PathBuf::from(format!("/proc/{pid}/comm"));
        let name = fs::read_to_string(comm_path)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| format!("pid:{pid}"));
        let process_path = fs::read_link(format!("/proc/{pid}/exe"))
            .ok()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        (name, process_path)
    }

    fn unavailable(detail: String) -> ErrorData {
        mcp_error(error_codes::OBSERVE_NO_PERCEPTION_AVAILABLE, detail)
    }
}

#[cfg(windows)]
pub fn platform_input(depth: u32, mode: PerceptionMode) -> Result<ObservationInput, ErrorData> {
    let root = synapse_a11y::focused_window().map_err(|err| a11y_error(&err))?;
    let tree = synapse_a11y::snapshot(&root, depth).map_err(|err| a11y_error(&err))?;
    let hwnd = tree
        .root
        .parts()
        .map_err(|err| {
            crate::m1::mcp_error(synapse_core::error_codes::OBSERVE_INTERNAL, err.to_string())
        })?
        .hwnd;
    let foreground = windows_foreground_context(hwnd)?;
    let focused = tree
        .nodes
        .iter()
        .find(|node| node.focused)
        .or_else(|| tree.nodes.first())
        .map(focused_from_node);
    let mut input = ObservationInput::new(foreground);
    input.focused = focused;
    input.elements = tree.nodes;
    input.a11y_status = SensorStatus::Healthy;
    if mode == PerceptionMode::A11yOnly {
        input.capture_status = SensorStatus::Disabled;
    } else {
        populate_window_capture_baseline(&mut input);
    }
    if mode != PerceptionMode::Auto {
        input.mode_override = Some(mode);
    }
    Ok(input)
}

#[cfg(windows)]
fn populate_window_capture_baseline(input: &mut ObservationInput) {
    let started = Instant::now();
    let Some(probe_region) = one_pixel_region(input.foreground.window_bounds) else {
        input.capture_status = SensorStatus::DegradedSensorFailed {
            reason_code: "WINDOW_CAPTURE_EMPTY".to_owned(),
        };
        return;
    };
    match synapse_capture::screen_region_to_bgra_bitmap(probe_region) {
        Ok(_probe) => {
            input.capture_status = SensorStatus::Healthy;
            input.sensor_latency_ms.insert(
                "capture".to_owned(),
                started.elapsed().as_secs_f32() * 1000.0,
            );
            if is_luanti_foreground(&input.foreground) {
                populate_luanti_visible_baseline(input);
            }
        }
        Err(error) => {
            tracing::debug!(
                code = "OBSERVE_CAPTURE_PROBE_FAILED",
                error = %error,
                "foreground window capture probe failed"
            );
            input.capture_status = SensorStatus::DegradedSensorFailed {
                reason_code: "WINDOW_CAPTURE_FAILED".to_owned(),
            };
        }
    }
}

#[cfg(windows)]
fn populate_luanti_visible_baseline(input: &mut ObservationInput) {
    let Some(crosshair_region) = centered_region(input.foreground.window_bounds, 48, 48) else {
        return;
    };
    if let Some(reading) = contrast_reading("luanti.crosshair_contrast", crosshair_region) {
        let confidence = reading.confidence;
        input
            .hud
            .by_name
            .insert("luanti.crosshair_contrast".to_owned(), reading);
        input.entities.push(DetectedEntity {
            entity_id: entity_id(10_001),
            track_id: 10_001,
            class_label: "luanti_crosshair_region".to_owned(),
            bbox: crosshair_region,
            confidence,
            first_seen_at: Utc::now(),
            last_seen_at: Utc::now(),
            velocity_px_per_s: None,
        });
    }

    let Some(hotbar_region) = hotbar_region(input.foreground.window_bounds) else {
        return;
    };
    if let Some(reading) = contrast_reading("luanti.hotbar_contrast", hotbar_region) {
        let confidence = reading.confidence;
        input
            .hud
            .by_name
            .insert("luanti.hotbar_contrast".to_owned(), reading);
        input.entities.push(DetectedEntity {
            entity_id: entity_id(10_002),
            track_id: 10_002,
            class_label: "luanti_hotbar_region".to_owned(),
            bbox: hotbar_region,
            confidence,
            first_seen_at: Utc::now(),
            last_seen_at: Utc::now(),
            velocity_px_per_s: None,
        });
    }
}

#[cfg(windows)]
fn contrast_reading(name: &str, region: Rect) -> Option<HudReading> {
    let captured = synapse_capture::screen_region_to_bgra_bitmap(region).ok()?;
    let score = bgra_contrast_score(&captured.bytes);
    Some(HudReading {
        raw_text: format!(
            "{name} contrast={score:.3} region={}x{}@{},{}",
            captured.width, captured.height, captured.region.x, captured.region.y
        ),
        parsed: HudValue::Number(f64::from(score)),
        confidence: score.clamp(0.0, 1.0),
        stale_ms: 0,
    })
}

#[cfg(windows)]
fn bgra_contrast_score(bytes: &[u8]) -> f32 {
    let mut count = 0.0_f32;
    let mut sum = 0.0_f32;
    let mut sum_sq = 0.0_f32;
    for pixel in bytes.chunks_exact(4) {
        let b = f32::from(pixel[0]);
        let g = f32::from(pixel[1]);
        let r = f32::from(pixel[2]);
        let luma = 0.0722_f32.mul_add(b, 0.7152_f32.mul_add(g, 0.2126_f32 * r));
        count += 1.0;
        sum += luma;
        sum_sq += luma * luma;
    }
    if count <= 0.0 {
        return 0.0;
    }
    let mean = sum / count;
    let variance = mean.mul_add(-mean, sum_sq / count).max(0.0);
    (variance.sqrt() / 128.0).clamp(0.0, 1.0)
}

#[cfg(windows)]
fn one_pixel_region(bounds: Rect) -> Option<Rect> {
    (bounds.w > 0 && bounds.h > 0).then_some(Rect {
        x: bounds.x,
        y: bounds.y,
        w: 1,
        h: 1,
    })
}

#[cfg(windows)]
const fn centered_region(bounds: Rect, w: i32, h: i32) -> Option<Rect> {
    if bounds.w < w || bounds.h < h || w <= 0 || h <= 0 {
        return None;
    }
    Some(Rect {
        x: bounds.x + ((bounds.w - w) / 2),
        y: bounds.y + ((bounds.h - h) / 2),
        w,
        h,
    })
}

#[cfg(windows)]
fn hotbar_region(bounds: Rect) -> Option<Rect> {
    if bounds.w <= 0 || bounds.h <= 0 {
        return None;
    }
    let w = (bounds.w / 3).clamp(180, 520).min(bounds.w);
    let h = (bounds.h / 12).clamp(48, 96).min(bounds.h);
    centered_bottom_region(bounds, w, h, 28)
}

#[cfg(windows)]
fn centered_bottom_region(bounds: Rect, w: i32, h: i32, y_offset: i32) -> Option<Rect> {
    if bounds.w < w || bounds.h < h || w <= 0 || h <= 0 {
        return None;
    }
    Some(Rect {
        x: bounds.x + ((bounds.w - w) / 2),
        y: bounds.y + bounds.h - h - y_offset.clamp(0, bounds.h - h),
        w,
        h,
    })
}

#[cfg(windows)]
fn is_luanti_foreground(foreground: &ForegroundContext) -> bool {
    foreground.process_name.eq_ignore_ascii_case("luanti.exe")
        && foreground.window_title.starts_with("Luanti ")
}

#[cfg(windows)]
fn a11y_error(err: &synapse_a11y::A11yError) -> ErrorData {
    match err {
        synapse_a11y::A11yError::NoForeground { .. }
        | synapse_a11y::A11yError::NotAvailable { .. } => crate::m1::mcp_error(
            synapse_core::error_codes::OBSERVE_NO_PERCEPTION_AVAILABLE,
            err.to_string(),
        ),
        _ => crate::m1::mcp_error(synapse_core::error_codes::OBSERVE_INTERNAL, err.to_string()),
    }
}

#[cfg(windows)]
fn focused_from_node(node: &AccessibleNode) -> FocusedElement {
    FocusedElement {
        element_id: node.element_id.clone(),
        name: node.name.clone(),
        role: node.role.clone(),
        automation_id: node.automation_id.clone(),
        bbox: node.bbox,
        enabled: node.enabled,
        patterns: node.patterns.clone(),
        value: None,
        selected_text: None,
    }
}

#[cfg(windows)]
fn windows_foreground_context(hwnd: i64) -> Result<ForegroundContext, ErrorData> {
    synapse_a11y::foreground_context(hwnd).map_err(|err| a11y_error(&err))
}
