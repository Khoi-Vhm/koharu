use koharu_ml::pp_ocr_v6::det::TextDetections;

use super::*;

pub(super) fn supplement(
    output: &mut KoharuLayoutDetections,
    lines: TextDetections,
    preserved: &[KoharuLayoutDetection],
) {
    for line in lines.detections {
        let points = line.polygon.map(|[x, y]| Point {
            x: x.into(),
            y: y.into(),
        });
        let Some(mask) = polygon_mask(&points, output.image_width, output.image_height) else {
            continue;
        };
        let bounds = [
            mask.x as f32,
            mask.y as f32,
            (mask.x + mask.width) as f32,
            (mask.y + mask.height) as f32,
        ];
        if preserved
            .iter()
            .any(|old| mask_containment(&old.mask, &mask, bounds) >= TEXT_COVERAGE_THRESHOLD)
            || output.detections.iter().any(|existing| {
                (existing.label == "text"
                    && containment(existing.bbox, bounds) >= TEXT_COVERAGE_THRESHOLD
                    && (existing.area == 0
                        || mask_containment(&existing.mask, &mask, bounds) > 0.0))
                    || (existing.label == "onomatopoeia"
                        && existing.score >= 0.5
                        && (intersection_over_union(existing.bbox, bounds) >= 0.5
                            || (containment(existing.bbox, bounds) >= 0.8
                                && mask_containment(&existing.mask, &mask, bounds) > 0.0)))
            })
        {
            continue;
        }
        let area = mask.pixels.iter().filter(|&&pixel| pixel != 0).count() as u32;
        output.detections.push(KoharuLayoutDetection {
            label_id: 0,
            label: "text".to_owned(),
            score: line.score,
            bbox: bounds,
            area,
            mask,
        });
    }
}

// PP-OCR supplies quadrilaterals rather than instance masks. Rasterize only
// their local extent so both region matching and cleanup retain the line angle.
pub(super) fn polygon_mask(points: &[Point], width: u32, height: u32) -> Option<KoharuLayoutMask> {
    if points.len() < 3 || points.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return None;
    }
    let left = points
        .iter()
        .map(|p| p.x)
        .fold(f64::INFINITY, f64::min)
        .floor()
        .clamp(0.0, width as f64) as u32;
    let top = points
        .iter()
        .map(|p| p.y)
        .fold(f64::INFINITY, f64::min)
        .floor()
        .clamp(0.0, height as f64) as u32;
    let right = points
        .iter()
        .map(|p| p.x)
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil()
        .clamp(0.0, width as f64) as u32;
    let bottom = points
        .iter()
        .map(|p| p.y)
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil()
        .clamp(0.0, height as f64) as u32;
    if right <= left || bottom <= top {
        return None;
    }
    let mut image = GrayImage::new(right - left, bottom - top);
    let mut polygon = points
        .iter()
        .map(|p| {
            imageproc::point::Point::new(
                (p.x - left as f64).round() as i32,
                (p.y - top as f64).round() as i32,
            )
        })
        .collect::<Vec<_>>();
    polygon.dedup();
    if polygon.first() == polygon.last() {
        polygon.pop();
    }
    if polygon.len() < 3 {
        return None;
    }
    imageproc::drawing::draw_polygon_mut(&mut image, &polygon, Luma([255]));
    Some(KoharuLayoutMask {
        x: left,
        y: top,
        width: image.width(),
        height: image.height(),
        pixels: image.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use koharu_ml::pp_ocr_v6::det::TextDetection;

    #[test]
    fn weak_sound_effect_classification_does_not_hide_a_confident_text_line() {
        for (sound_score, count) in [(0.25, 2), (0.9, 1)] {
            let sound = super::super::tests::detection(
                "onomatopoeia",
                sound_score,
                [10.0, 10.0, 90.0, 30.0],
            );
            let mut output = KoharuLayoutDetections {
                image_width: 100,
                image_height: 100,
                detections: vec![sound],
            };
            supplement(
                &mut output,
                TextDetections {
                    detections: vec![TextDetection {
                        polygon: [[10.0, 10.0], [90.0, 10.0], [90.0, 30.0], [10.0, 30.0]],
                        score: 0.9,
                    }],
                },
                &[],
            );
            assert_eq!(output.detections.len(), count);
        }
    }

    #[test]
    fn adds_uncovered_lines_without_duplicating_blocks_or_sound_effects() {
        let mut block = super::super::tests::detection("text", 0.8, [0.0, 0.0, 40.0, 40.0]);
        block.area = 1600;
        block.mask = KoharuLayoutMask {
            x: 0,
            y: 0,
            width: 40,
            height: 40,
            pixels: vec![255; 1600],
        };
        let mut sound =
            super::super::tests::detection("onomatopoeia", 0.9, [60.0, 0.0, 90.0, 30.0]);
        sound.mask = KoharuLayoutMask {
            x: 60,
            y: 0,
            width: 30,
            height: 30,
            pixels: {
                let mut pixels = vec![0; 900];
                pixels[7 * 30 + 10] = 255;
                pixels
            },
        };
        let mut output = KoharuLayoutDetections {
            image_width: 100,
            image_height: 100,
            detections: vec![block, sound],
        };
        let lines = TextDetections {
            detections: vec![
                TextDetection {
                    polygon: [[5.0, 5.0], [35.0, 5.0], [35.0, 15.0], [5.0, 15.0]],
                    score: 0.9,
                },
                TextDetection {
                    polygon: [[65.0, 5.0], [85.0, 5.0], [85.0, 10.0], [65.0, 10.0]],
                    score: 0.9,
                },
                TextDetection {
                    polygon: [[10.0, 60.0], [70.0, 70.0], [68.0, 82.0], [8.0, 72.0]],
                    score: 0.9,
                },
            ],
        };
        supplement(&mut output, lines.clone(), &[]);
        assert_eq!(output.detections.len(), 3);
        let line = &output.detections[2];
        assert!(line.mask.contains(30, 70));
        assert!(!line.mask.contains(8, 60));
        assert_eq!(line.label, "text");
        supplement(&mut output, lines, &[]);
        assert_eq!(output.detections.len(), 3);
    }
}
