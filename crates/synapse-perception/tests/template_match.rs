use std::{
    error::Error,
    io::{self, Write},
};

use image::{DynamicImage, GrayImage, Luma};
use synapse_core::{Rect, error_codes};
use synapse_perception::{
    HudTemplate, TemplateCounterConfig, extract_template_counter_from_frame,
    extract_template_counter_from_region,
};

type TestResult = Result<(), Box<dyn Error>>;

fn regression_log(args: std::fmt::Arguments<'_>) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_fmt(args)?;
    stdout.write_all(b"\n")
}

#[test]
fn template_match_counts_synthetic_minecraft_hearts() -> TestResult {
    let templates = status_templates()?;
    let region = synthetic_region(&[2, 2, 2, 2, 2, 2, 2, 0, 0, 0]);
    regression_log(format_args!(
        "regression_check=hud_template edge=seven_full before=region:{}x{} templates:{}",
        region.width(),
        region.height(),
        templates.len()
    ))?;
    let reading = extract_template_counter_from_region(
        &region,
        &templates,
        TemplateCounterConfig::default(),
    )?;
    regression_log(format_args!(
        "regression_check=hud_template edge=seven_full after=value:{} confidence:{:.3} slots:{}",
        reading.value,
        reading.confidence,
        reading.slots.len()
    ))?;
    assert_eq!(reading.value, 14);
    assert!(reading.confidence >= 0.99);
    assert_eq!(reading.slots.len(), 10);
    Ok(())
}

#[test]
fn template_match_distinguishes_half_and_empty_slots() -> TestResult {
    let templates = status_templates()?;
    let region = synthetic_region(&[2, 2, 1, 0, 0, 0, 0, 0, 0, 0]);
    regression_log(format_args!(
        "regression_check=hud_template edge=half_slot before=values=[2,2,1,0...]"
    ))?;
    let reading = extract_template_counter_from_region(
        &region,
        &templates,
        TemplateCounterConfig::default(),
    )?;
    regression_log(format_args!(
        "regression_check=hud_template edge=half_slot after=value:{} labels={:?}",
        reading.value,
        reading
            .slots
            .iter()
            .take(4)
            .map(|slot| slot.label.as_str())
            .collect::<Vec<_>>()
    ))?;
    assert_eq!(reading.value, 5);
    assert_eq!(
        reading.slots.get(2).map(|slot| slot.label.as_str()),
        Some("half")
    );
    Ok(())
}

#[test]
fn template_match_crops_from_frame_region() -> TestResult {
    let templates = status_templates()?;
    let hud = synthetic_region(&[2, 2, 2, 0, 0, 0, 0, 0, 0, 0]);
    let mut frame = GrayImage::from_pixel(220, 80, Luma([4]));
    blit(&mut frame, &hud, 20, 30);
    let frame = DynamicImage::ImageLuma8(frame);
    let region = Rect {
        x: 20,
        y: 30,
        w: 180,
        h: 16,
    };
    regression_log(format_args!(
        "regression_check=hud_template edge=frame_crop before=region:{region:?}"
    ))?;
    let reading = extract_template_counter_from_frame(
        &frame,
        region,
        &templates,
        TemplateCounterConfig::default(),
    )?;
    regression_log(format_args!(
        "regression_check=hud_template edge=frame_crop after=value:{} confidence:{:.3}",
        reading.value, reading.confidence
    ))?;
    assert_eq!(reading.value, 6);
    assert!(reading.confidence >= 0.99);
    Ok(())
}

#[test]
fn template_match_fails_closed_for_structural_edges() -> TestResult {
    let templates = status_templates()?;
    let region = synthetic_region(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    regression_log(format_args!(
        "regression_check=hud_template edge=no_templates before=templates:0"
    ))?;
    let no_templates =
        extract_template_counter_from_region(&region, &[], TemplateCounterConfig::default());
    regression_log(format_args!(
        "regression_check=hud_template edge=no_templates after={no_templates:?}"
    ))?;
    assert_eq!(
        no_templates.err().map(|err| err.code()),
        Some(error_codes::HUD_EXTRACTION_FAILED)
    );

    let invalid_slots = TemplateCounterConfig {
        slots: 0,
        ..TemplateCounterConfig::default()
    };
    regression_log(format_args!(
        "regression_check=hud_template edge=zero_slots before=config:{invalid_slots:?}"
    ))?;
    let zero_slots = extract_template_counter_from_region(&region, &templates, invalid_slots);
    regression_log(format_args!(
        "regression_check=hud_template edge=zero_slots after={zero_slots:?}"
    ))?;
    assert_eq!(
        zero_slots.err().map(|err| err.code()),
        Some(error_codes::HUD_EXTRACTION_FAILED)
    );

    let blank = GrayImage::from_pixel(180, 16, Luma([0]));
    regression_log(format_args!(
        "regression_check=hud_template edge=blank_region before=region:{}x{}",
        blank.width(),
        blank.height()
    ))?;
    let blank_reading =
        extract_template_counter_from_region(&blank, &templates, TemplateCounterConfig::default());
    regression_log(format_args!(
        "regression_check=hud_template edge=blank_region after={blank_reading:?}"
    ))?;
    assert_eq!(
        blank_reading.err().map(|err| err.code()),
        Some(error_codes::HUD_EXTRACTION_FAILED)
    );
    Ok(())
}

fn status_templates() -> synapse_perception::PerceptionResult<Vec<HudTemplate>> {
    Ok(vec![
        HudTemplate::from_gray("full", 2, full_template())?,
        HudTemplate::from_gray("half", 1, half_template())?,
        HudTemplate::from_gray("empty", 0, empty_template())?,
    ])
}

fn synthetic_region(values: &[u32; 10]) -> GrayImage {
    let full = full_template();
    let half = half_template();
    let empty = empty_template();
    let mut region = GrayImage::from_pixel(180, 16, Luma([8]));
    for (index, value) in values.iter().enumerate() {
        let slot_x = u32::try_from(index).map_or(0, |item| item.saturating_mul(18));
        let x = slot_x.saturating_add(4);
        let template = match value {
            2 => &full,
            1 => &half,
            _ => &empty,
        };
        blit(&mut region, template, x, 3);
    }
    region
}

fn full_template() -> GrayImage {
    GrayImage::from_fn(9, 9, |x, y| {
        if heart_fill(x, y) {
            Luma([230])
        } else if heart_outline(x, y) {
            Luma([120])
        } else {
            Luma([24])
        }
    })
}

fn half_template() -> GrayImage {
    GrayImage::from_fn(9, 9, |x, y| {
        if heart_fill(x, y) && x <= 4 {
            Luma([230])
        } else if heart_outline(x, y) {
            Luma([120])
        } else {
            Luma([24])
        }
    })
}

fn empty_template() -> GrayImage {
    GrayImage::from_fn(9, 9, |x, y| {
        if heart_outline(x, y) {
            Luma([190])
        } else {
            Luma([24])
        }
    })
}

const fn heart_fill(x: u32, y: u32) -> bool {
    matches!(
        (x, y),
        (2..=3 | 5..=6, 1..=2) | (1..=7, 3..=4) | (2..=6, 5) | (3..=5, 6) | (4, 7)
    )
}

const fn heart_outline(x: u32, y: u32) -> bool {
    matches!(
        (x, y),
        (1..=3 | 5..=7, 0)
            | (0 | 8, 2..=4)
            | (1 | 7, 5)
            | (2 | 6, 6)
            | (3 | 5, 7)
            | (4, 8)
    )
}

fn blit(target: &mut GrayImage, source: &GrayImage, x: u32, y: u32) {
    for source_y in 0..source.height() {
        for source_x in 0..source.width() {
            let target_x = x.saturating_add(source_x);
            let target_y = y.saturating_add(source_y);
            if target_x < target.width() && target_y < target.height() {
                target.put_pixel(target_x, target_y, *source.get_pixel(source_x, source_y));
            }
        }
    }
}
