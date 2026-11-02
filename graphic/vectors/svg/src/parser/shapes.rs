use super::*;

pub(super) fn element_path(
    tag: &str,
    attrs: &std::collections::HashMap<String, String>,
) -> Result<Vec<SvgPathCommand>, SvgParseError> {
    let number = |name: &str, default: f32| {
        attrs
            .get(name)
            .map(|s| parse_length(s).ok_or(SvgParseError::InvalidNumber))
            .transpose()
            .map(|v| v.unwrap_or(default))
    };
    match tag {
        "path" => parse_path(attrs.get("d").ok_or(SvgParseError::InvalidPath)?),
        "rect" => {
            let x = number("x", 0.0)?;
            let y = number("y", 0.0)?;
            let w = number("width", 0.0)?;
            let h = number("height", 0.0)?;
            if w < 0.0 || h < 0.0 {
                return Err(SvgParseError::InvalidNumber);
            }
            let rx = attrs.get("rx").map(|_| number("rx", 0.0)).transpose()?;
            let ry = attrs.get("ry").map(|_| number("ry", 0.0)).transpose()?;
            let rx = rx.or(ry).unwrap_or(0.0);
            let ry = ry.unwrap_or(rx);
            if rx < 0.0 || ry < 0.0 {
                return Err(SvgParseError::InvalidNumber);
            }
            if w == 0.0 || h == 0.0 {
                return Ok(Vec::new());
            }
            Ok(rectangle_path(x, y, w, h, rx.min(w * 0.5), ry.min(h * 0.5)))
        }
        "circle" | "ellipse" => {
            let cx = number("cx", 0.0)?;
            let cy = number("cy", 0.0)?;
            let rx = if tag == "circle" {
                number("r", 0.0)?
            } else {
                number("rx", 0.0)?
            };
            let ry = if tag == "circle" {
                rx
            } else {
                number("ry", 0.0)?
            };
            if rx < 0.0 || ry < 0.0 {
                return Err(SvgParseError::InvalidNumber);
            }
            ellipse_path(cx, cy, rx, ry)
        }
        "line" => Ok(vec![
            m(number("x1", 0.0)?, number("y1", 0.0)?),
            l(number("x2", 0.0)?, number("y2", 0.0)?),
        ]),
        "polyline" | "polygon" => {
            let points = parse_numbers(attrs.get("points").map(String::as_str).unwrap_or(""));
            if points.len() < 4 || points.len() % 2 != 0 {
                return Err(SvgParseError::InvalidPath);
            }
            let mut p = vec![m(points[0], points[1])];
            for pair in points[2..].chunks_exact(2) {
                p.push(l(pair[0], pair[1]));
            }
            if tag == "polygon" {
                p.push(SvgPathCommand::Close);
            }
            Ok(p)
        }
        _ => Err(SvgParseError::Unsupported("shape")),
    }
}


fn rectangle_path(x: f32, y: f32, w: f32, h: f32, rx: f32, ry: f32) -> Vec<SvgPathCommand> {
    if rx == 0.0 || ry == 0.0 {
        return vec![m(x, y), l(x + w, y), l(x + w, y + h), l(x, y + h), SvgPathCommand::Close];
    }
    // The same quarter-ellipse approximation used for circles and ellipses.
    let k = 0.552_284_8;
    vec![
        m(x + rx, y),
        l(x + w - rx, y),
        c(x + w - rx + k * rx, y, x + w, y + ry - k * ry, x + w, y + ry),
        l(x + w, y + h - ry),
        c(x + w, y + h - ry + k * ry, x + w - rx + k * rx, y + h, x + w - rx, y + h),
        l(x + rx, y + h),
        c(x + rx - k * rx, y + h, x, y + h - ry + k * ry, x, y + h - ry),
        l(x, y + ry),
        c(x, y + ry - k * ry, x + rx - k * rx, y, x + rx, y),
        SvgPathCommand::Close,
    ]
}
