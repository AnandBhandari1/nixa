use crate::layout::ICON_ASSET;
use crate::run;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const MASCOT: &str = "crates/app/src/screens/nixa_screen/mascot.svg";
pub const ICON_SVG: &str = "crates/xtask/assets/icon.svg";
pub const SIZES: [(u32, &str); 10] = [
    (16, "icon_16x16"),
    (32, "icon_16x16@2x"),
    (32, "icon_32x32"),
    (64, "icon_32x32@2x"),
    (128, "icon_128x128"),
    (256, "icon_128x128@2x"),
    (256, "icon_256x256"),
    (512, "icon_256x256@2x"),
    (512, "icon_512x512"),
    (1024, "icon_512x512@2x"),
];

const FRAME_TOP: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="1024" height="1024">
<defs><linearGradient id="nixa-icon-bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#2A2F5E"/><stop offset="1" stop-color="#0E1126"/></linearGradient></defs>
<rect x="100" y="100" width="824" height="824" rx="185" fill="url(#nixa-icon-bg)"/>
<g transform="translate(152 140) scale(1.8)">
"##;
const FRAME_END: &str = "</g>\n</svg>\n";

const RENDER_JS: &str = r#"ObjC.import('AppKit');
function run(argv) {
  const size = parseInt(argv[2], 10);
  const image = $.NSImage.alloc.initWithContentsOfFile(argv[0]);
  if (!image || image.isNil()) return 'no image';
  const rep = $.NSBitmapImageRep.alloc.initWithBitmapDataPlanesPixelsWidePixelsHighBitsPerSampleSamplesPerPixelHasAlphaIsPlanarColorSpaceNameBytesPerRowBitsPerPixel(null, size, size, 8, 4, true, false, $.NSDeviceRGBColorSpace, 0, 0);
  rep.size = $.NSMakeSize(size, size);
  $.NSGraphicsContext.saveGraphicsState;
  $.NSGraphicsContext.setCurrentContext($.NSGraphicsContext.graphicsContextWithBitmapImageRep(rep));
  image.drawInRectFromRectOperationFraction($.NSMakeRect(0, 0, size, size), $.NSZeroRect, $.NSCompositingOperationSourceOver, 1.0);
  $.NSGraphicsContext.restoreGraphicsState;
  const data = rep.representationUsingTypeProperties($.NSBitmapImageFileTypePNG, $({}));
  return data.writeToFileAtomically(argv[1], true) ? 'ok' : 'not written';
}
"#;

pub fn mascot_body(svg: &str) -> Option<&str> {
    let open = svg.find("<svg")?;
    let start = open + svg[open..].find('>')? + 1;
    let end = svg.rfind("</svg>")?;
    (start <= end).then(|| svg[start..end].trim())
}

pub fn icon_svg(mascot: &str) -> Result<String, String> {
    let body = mascot_body(mascot).ok_or("the mascot file is not an svg")?;
    Ok(format!("{FRAME_TOP}{body}\n{FRAME_END}"))
}

fn render(script: &Path, svg: &Path, png: &Path, size: u32) -> Result<(), String> {
    let said = run::text(Command::new("/usr/bin/osascript").args(["-l", "JavaScript"]).arg(script).arg(svg).arg(png).arg(size.to_string()))?;
    if said.trim() == "ok" && png.is_file() {
        Ok(())
    } else {
        Err(format!("the icon was not drawn at {size} px: {}", said.trim()))
    }
}

pub fn make(root: &Path) -> Result<PathBuf, String> {
    let mascot_path = root.join(MASCOT);
    let mascot = fs::read_to_string(&mascot_path).map_err(|e| format!("cannot read {}: {e}", mascot_path.display()))?;
    let svg_path = root.join(ICON_SVG);
    let svg = icon_svg(&mascot)?;
    if let Some(parent) = svg_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot make {}: {e}", parent.display()))?;
    }
    fs::write(&svg_path, svg).map_err(|e| format!("cannot write {}: {e}", svg_path.display()))?;
    let work = std::env::temp_dir().join(format!("mx-icon-{}", std::process::id()));
    fs::remove_dir_all(&work).ok();
    let set = work.join("AppIcon.iconset");
    fs::create_dir_all(&set).map_err(|e| format!("cannot make {}: {e}", set.display()))?;
    let script = work.join("render.js");
    fs::write(&script, RENDER_JS).map_err(|e| format!("cannot write {}: {e}", script.display()))?;
    let icns = root.join(ICON_ASSET);
    let done = SIZES
        .iter()
        .try_for_each(|(size, name)| render(&script, &svg_path, &set.join(format!("{name}.png")), *size))
        .and_then(|()| run::text(Command::new("/usr/bin/iconutil").args(["-c", "icns"]).arg(&set).arg("-o").arg(&icns)).map(|_| ()));
    fs::remove_dir_all(&work).ok();
    done.map(|()| icns)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mascot_is_framed() {
        let svg = "<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 400 400\">\n<circle r=\"4\"/>\n</svg>\n";
        assert_eq!(mascot_body(svg), Some("<circle r=\"4\"/>"));
        let framed = icon_svg(svg).unwrap();
        assert!(framed.starts_with("<svg xmlns"));
        assert!(framed.contains("rx=\"185\""));
        assert!(framed.contains("<g transform=\"translate(152 140) scale(1.8)\">\n<circle r=\"4\"/>\n</g>"));
        assert_eq!(framed.matches("<svg").count(), 1);
        assert!(icon_svg("not an svg").is_err());
        assert_eq!(SIZES.len(), 10);
        assert_eq!(SIZES.iter().filter(|(s, _)| *s == 1024).count(), 1);
    }

    #[test]
    fn the_repo_mascot_and_icon_are_there() {
        let root = crate::opts::default_root();
        let mascot = fs::read_to_string(root.join(MASCOT)).unwrap();
        assert!(mascot_body(&mascot).is_some_and(|b| b.len() > 100));
        let icns = fs::read(root.join(ICON_ASSET)).unwrap();
        assert_eq!(&icns[..4], b"icns");
    }
}
