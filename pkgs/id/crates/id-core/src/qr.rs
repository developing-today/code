//! QR codes as SVG, so an invite can be scanned from a screen.

use anyhow::{Result, anyhow};
use qrcode::{Color, QrCode};
use std::fmt::Write;

const QUIET_ZONE: usize = 4;

/// The text as a QR code in SVG, with a quiet zone. Every dark module is one
/// `M x y h1 v1 h-1 z` square in a single path, so the output can be read
/// back by the same geometry it was written with.
///
/// # Errors
///
/// Fails if the text is too long for a QR code.
pub fn svg(text: &str) -> Result<String> {
    let code = QrCode::new(text.as_bytes())
        .map_err(|error| anyhow!("the text does not fit in a QR code: {error:?}"))?;
    let width = code.width();
    let path = code
        .to_colors()
        .into_iter()
        .enumerate()
        .filter(|(_, color)| *color == Color::Dark)
        .fold(String::new(), |mut path, (index, _)| {
            let _ = write!(path, "M{} {}h1v1h-1z", index % width, index / width);
            path
        });
    let size = width + 2 * QUIET_ZONE;
    Ok(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {size} {size}\" \
         width=\"{size}\" height=\"{size}\" shape-rendering=\"crispEdges\">\
         <rect width=\"{size}\" height=\"{size}\" fill=\"#fff\"/>\
         <path transform=\"translate({QUIET_ZONE} {QUIET_ZONE})\" fill=\"#000\" d=\"{path}\"/></svg>"
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const SCALE: usize = 4;

    /// Rasterize our own SVG the way a camera would see it, then decode it.
    fn decode_svg(svg: &str) -> String {
        let size: usize = svg
            .split("viewBox=\"0 0 ")
            .nth(1)
            .unwrap()
            .split(' ')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let path = svg
            .split(" d=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let modules_per_side = size - 2 * QUIET_ZONE;
        let mut dark = vec![false; size * size];
        for square in path.split('M').filter(|square| !square.is_empty()) {
            let (x, rest) = square.split_once(' ').unwrap();
            let y = rest.split('h').next().unwrap();
            let x: usize = x.parse().unwrap();
            let y: usize = y.parse().unwrap();
            assert!(x < modules_per_side && y < modules_per_side);
            dark[(y + QUIET_ZONE) * size + (x + QUIET_ZONE)] = true;
        }
        let pixels = size * SCALE;
        let mut image = rqrr::PreparedImage::prepare_from_greyscale(pixels, pixels, |x, y| {
            if dark[(y / SCALE) * size + x / SCALE] {
                0
            } else {
                255
            }
        });
        let grids = image.detect_grids();
        assert_eq!(grids.len(), 1, "the QR code should be found once");
        let (_, content) = grids[0].decode().unwrap();
        content
    }

    #[test]
    fn an_invite_string_decodes_back_to_itself() {
        let key = "aa11".repeat(16);
        let node = "bb22".repeat(16);
        let url = format!("id:invite?key={key}&node={node}&name=Ann%20Lee%20%26%20Co.");
        let svg = svg(&url).unwrap();
        assert!(svg.starts_with("<svg "));
        assert_eq!(decode_svg(&svg), url);
    }

    #[test]
    fn a_short_text_decodes_back_to_itself() {
        let svg = svg("hello").unwrap();
        assert_eq!(decode_svg(&svg), "hello");
    }
}
