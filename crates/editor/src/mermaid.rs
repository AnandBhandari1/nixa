use crate::markdown::Block;
use gpui::{Image, ImageFormat};
use merman::model::resources::{InputResourceLimitId, InputResourcePolicy};
use merman::svg::export::{EmbeddedImageLimit, RasterFitBox, RasterOptions, RasterSizeLimit};
use merman::{OperationControl, PngRequest, RenderError, RenderOutput, RenderRequest, Renderer, SvgRequest};
use std::{collections::HashMap, sync::Arc, time::Duration};

const MAX_SOURCE: usize = 16 * 1024;
const MAX_LINES: usize = 256;
const MAX_DIAGRAMS: usize = 6;
const MAX_SIDE: u32 = 1200;

pub(super) type Previews = HashMap<usize, Result<Picture, &'static str>>;
pub(super) type Inputs = Vec<(usize, Result<String, &'static str>)>;

pub(super) struct Picture {
    pub image: Arc<Image>,
    pub size: (u32, u32),
}

pub(super) const TOO_MANY: &str = "Only the first six Mermaid diagrams are drawn in this preview.";
const TOO_LARGE: &str = "This Mermaid diagram exceeds the preview size limit.";
const TIMED_OUT: &str = "Mermaid preview timed out.";

pub(super) fn is_fence(info: &str) -> bool {
    info.split_whitespace().next().is_some_and(|word| word.eq_ignore_ascii_case("mermaid"))
}

fn checked_source(source: &str) -> Result<String, &'static str> {
    if source.len() > MAX_SOURCE || source.lines().nth(MAX_LINES).is_some() {
        return Err(TOO_LARGE);
    }
    Ok(source.to_owned())
}

pub(super) fn inputs(blocks: &[Block], standalone: Option<&str>) -> Inputs {
    if let Some(source) = standalone {
        return vec![(0, checked_source(source))];
    }
    blocks.iter().enumerate().filter_map(|(index, block)| match block {
        Block::Code { info, text } if is_fence(info) => Some((index, text)),
        _ => None,
    }).take(MAX_DIAGRAMS).map(|(i, s)| (i, checked_source(s))).collect()
}

pub(super) fn draw(inputs: Inputs, cancel: &OperationControl) -> Previews {
    let renderer = Renderer::new();
    inputs.into_iter().map(|(i, source)| {
        (i, source.and_then(|source| {
            if cancel.is_cancelled() { Err(TIMED_OUT) } else { draw_one(&renderer, &source, cancel) }
        }))
    }).collect()
}

fn draw_one(renderer: &Renderer, source: &str, control: &OperationControl) -> Result<Picture, &'static str> {
    use InputResourceLimitId::*;
    let mut policy = InputResourcePolicy::default();
    for (limit, value) in [(MaxSourceBytes, MAX_SOURCE), (MaxModelItems, 256), (MaxModelTextBytes, MAX_SOURCE), (MaxModelNestingDepth, 32)] {
        policy.apply_limit(limit, value).expect("positive preview budget");
    }
    let request = PngRequest {
        svg: SvgRequest::default(),
        options: RasterOptions {
            scale: 1.5,
            matte: Some("#ffffff".into()),
            fit_to: Some(RasterFitBox::new(Some(MAX_SIDE), Some(MAX_SIDE))),
            size_limit: RasterSizeLimit::new(Some(MAX_SIDE), Some(MAX_SIDE), Some(u64::from(MAX_SIDE).pow(2))),
            embedded_image_limit: EmbeddedImageLimit::new(Some(1), Some(1), Some(1), Some(1)),
            ..RasterOptions::default()
        },
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        renderer.render(RenderRequest::png(source, control.clone(), request).with_resource_policy(policy))
    }));
    let bytes = match result {
        Ok(Ok(RenderOutput::Png(Some(png)))) => png.into_bytes(),
        Ok(Err(RenderError::Cancelled(_))) => return Err(TIMED_OUT),
        Ok(Err(RenderError::ResourceLimitExceeded(_))) => return Err(TOO_LARGE),
        other => {
            #[cfg(test)]
            if let Ok(Err(ref diagnostic)) = other {
                eprintln!("Mermaid fixture failed: {diagnostic}");
            }
            let _ = other;
            return Err("Could not draw this Mermaid diagram. Check its syntax in Edit.");
        }
    };
    let size = crate::image::dims(&bytes).ok_or("Mermaid preview could not read the rendered image.")?;
    Ok(Picture { image: Arc::new(Image::from_bytes(ImageFormat::Png, bytes)), size })
}

pub(super) struct Batch {
    pub rev: u64,
    pub result: Option<Previews>,
    pub cancel: OperationControl,
}

impl Batch {
    pub fn new(rev: u64) -> Self {
        Self { rev, result: None, cancel: OperationControl::new().with_deadline(Duration::from_secs(2)) }
    }
}

impl Drop for Batch {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_flowchart_and_sequence_to_bounded_images() {
        for source in ["flowchart TD\nA[Start] --> B[Done]", "sequenceDiagram\nEditor->>Renderer: Draw"] {
            let picture = draw_one(&Renderer::new(), source, &OperationControl::new()).unwrap();
            assert!(picture.size.0 > 20 && picture.size.1 > 20);
            assert!(picture.size.0 <= MAX_SIDE && picture.size.1 <= MAX_SIDE);
        }
    }

    #[test]
    fn recognizes_fences_but_only_schedules_six() {
        let markdown = "```rust\nfn main() {}\n```\n".to_owned() + &"~~~MERMAID preview\nflowchart LR\nA-->B\n~~~\n".repeat(10);
        let blocks = crate::markdown::parse(&markdown);
        let scheduled = inputs(&blocks, None);
        assert_eq!(scheduled.len(), MAX_DIAGRAMS);
        assert_eq!(scheduled[0].0, 1);
        assert_eq!(scheduled[0].1.as_deref(), Ok("flowchart LR\nA-->B"));
    }

    #[test]
    fn refuses_oversized_invalid_and_cancelled_work() {
        let big = "a".repeat(MAX_SOURCE + 1);
        assert_eq!(inputs(&[], Some(&big))[0].1, Err(TOO_LARGE));
        assert_eq!(checked_source(&"x\n".repeat(MAX_LINES + 1)), Err(TOO_LARGE));
        assert!(draw_one(&Renderer::new(), "not a diagram", &OperationControl::new()).is_err());
        let many = format!("flowchart TD\n{}", (0..180).map(|i| format!("A{i} --> B{i}\n")).collect::<String>());
        assert!(matches!(draw_one(&Renderer::new(), &many, &OperationControl::new()), Err(TOO_LARGE)));
        let batch = Batch::new(0);
        let control = batch.cancel.clone();
        drop(batch);
        assert!(matches!(draw(inputs(&[], Some("flowchart LR\nA-->B")), &control).get(&0), Some(Err(TIMED_OUT))));
    }
}
