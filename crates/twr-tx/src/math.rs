//! The animation-key derivation math, ported **verbatim** from
//! `iSarabjitDhiman/XClientTransaction` (MIT) via `tmp/_research/agentic-x/src/agentic_x/transaction.py`.
//!
//! Kept byte-for-byte faithful to the Python port rather than "improved" —
//! see `lib.rs`'s module docs for why. Every function here has a direct
//! Python counterpart with the same name (minus the leading underscore).

/// JavaScript's `Math.round`, which differs from Python's/Rust's default
/// rounding on exact `.5` (round-half-away-from-zero, not round-half-even).
/// The derived key is wrong if this is not matched exactly.
pub(crate) fn js_round(num: f64) -> f64 {
    let mut x = num.floor();
    if (num - x) >= 0.5 {
        x = num.ceil();
    }
    x.copysign(num)
}

/// Python's `round(x)` (no ndigits): round-half-to-even, returned as i64.
pub(crate) fn py_round_int(x: f64) -> i64 {
    let floor = x.floor();
    let diff = x - floor;
    if (diff - 0.5).abs() < 1e-9 {
        let fi = floor as i64;
        if fi % 2 == 0 {
            fi
        } else {
            fi + 1
        }
    } else {
        x.round() as i64
    }
}

/// Python's `round(x, 2)`: round-half-to-even to 2 decimal places.
pub(crate) fn py_round2(x: f64) -> f64 {
    let scaled = x * 100.0;
    let floor = scaled.floor();
    let diff = scaled - floor;
    let rounded = if (diff - 0.5).abs() < 1e-9 {
        let fi = floor as i64;
        (if fi % 2 == 0 { fi } else { fi + 1 }) as f64
    } else {
        scaled.round()
    };
    rounded / 100.0
}

pub(crate) fn is_odd(num: i64) -> f64 {
    if num % 2 != 0 {
        -1.0
    } else {
        0.0
    }
}

/// Upstream's bespoke float->hex encoder (not standard hex formatting —
/// this is why it can't be replaced with `format!("{:x}", ...)`).
pub(crate) fn float_to_hex(x_in: f64) -> String {
    let mut result: Vec<char> = Vec::new();
    let mut quotient = x_in as i64;
    let fraction_start = x_in - quotient as f64;
    let mut x = x_in;
    while quotient > 0 {
        quotient = (x / 16.0) as i64;
        let remainder = (x - (quotient as f64 * 16.0)) as i64;
        let ch = if remainder > 9 {
            (b'A' + (remainder - 10) as u8) as char
        } else {
            (b'0' + remainder as u8) as char
        };
        result.insert(0, ch);
        x = quotient as f64;
    }
    if fraction_start == 0.0 {
        return result.into_iter().collect();
    }
    result.push('.');
    let mut fraction = fraction_start;
    while fraction > 0.0 {
        fraction *= 16.0;
        let integer = fraction as i64;
        fraction -= integer as f64;
        let ch = if integer > 9 {
            (b'A' + (integer - 10) as u8) as char
        } else {
            (b'0' + integer as u8) as char
        };
        result.push(ch);
    }
    result.into_iter().collect()
}

pub(crate) fn cubic_value(curves: &[f64; 4], t: f64) -> f64 {
    let (mut start_gradient, mut end_gradient) = (0.0_f64, 0.0_f64);
    let (mut start, mut mid, end0) = (0.0_f64, 0.0_f64, 1.0_f64);
    let mut end = end0;

    if t <= 0.0 {
        if curves[0] > 0.0 {
            start_gradient = curves[1] / curves[0];
        } else if curves[1] == 0.0 && curves[2] > 0.0 {
            start_gradient = curves[3] / curves[2];
        }
        return start_gradient * t;
    }
    if t >= 1.0 {
        if curves[2] < 1.0 {
            end_gradient = (curves[3] - 1.0) / (curves[2] - 1.0);
        } else if curves[2] == 1.0 && curves[0] < 1.0 {
            end_gradient = (curves[1] - 1.0) / (curves[0] - 1.0);
        }
        return 1.0 + end_gradient * (t - 1.0);
    }

    let calculate = |a: f64, b: f64, m: f64| -> f64 {
        3.0 * a * (1.0 - m) * (1.0 - m) * m + 3.0 * b * (1.0 - m) * m * m + m * m * m
    };

    while start < end {
        mid = (start + end) / 2.0;
        let x_estimate = calculate(curves[0], curves[2], mid);
        if (t - x_estimate).abs() < 0.00001 {
            return calculate(curves[1], curves[3], mid);
        }
        if x_estimate < t {
            start = mid;
        } else {
            end = mid;
        }
    }
    calculate(curves[1], curves[3], mid)
}

pub(crate) fn interpolate(from_list: &[f64], to_list: &[f64], f: f64) -> Vec<f64> {
    from_list
        .iter()
        .zip(to_list.iter())
        .map(|(a, b)| a * (1.0 - f) + b * f)
        .collect()
}

pub(crate) fn rotation_matrix(rotation: f64) -> [f64; 4] {
    let rad = rotation.to_radians();
    [rad.cos(), -rad.sin(), rad.sin(), rad.cos()]
}

pub(crate) fn solve(value: f64, min_val: f64, max_val: f64, rounding: bool) -> f64 {
    let result = value * (max_val - min_val) / 255.0 + min_val;
    if rounding {
        result.floor()
    } else {
        py_round2(result)
    }
}

/// One "frame row" (12 numbers: 3 from-color + 3 to-color + 1 rotation + 4 curve
/// control points) -> the hex-ish animation-key fragment for that row.
pub(crate) fn animate(frame_row: &[i64], target_time: f64) -> String {
    let from_color: Vec<f64> = frame_row[..3]
        .iter()
        .map(|&v| v as f64)
        .chain(std::iter::once(1.0))
        .collect();
    let to_color: Vec<f64> = frame_row[3..6]
        .iter()
        .map(|&v| v as f64)
        .chain(std::iter::once(1.0))
        .collect();
    let to_rotation = solve(frame_row[6] as f64, 60.0, 360.0, true);
    let curves_vec: Vec<f64> = frame_row[7..]
        .iter()
        .enumerate()
        .map(|(counter, &item)| solve(item as f64, is_odd(counter as i64), 1.0, false))
        .collect();
    let curves = [curves_vec[0], curves_vec[1], curves_vec[2], curves_vec[3]];

    let value = cubic_value(&curves, target_time);
    let color: Vec<f64> = interpolate(&from_color, &to_color, value)
        .into_iter()
        .map(|item| item.clamp(0.0, 255.0))
        .collect();
    let rotation = interpolate(&[0.0], &[to_rotation], value);
    let matrix = rotation_matrix(rotation[0]);

    let mut parts: Vec<String> = color[..color.len() - 1]
        .iter()
        .map(|item| format!("{:x}", py_round_int(*item)))
        .collect();
    for item in matrix.iter() {
        let rounded = py_round2(*item).abs();
        let hex_value = float_to_hex(rounded);
        if hex_value.starts_with('.') {
            parts.push(format!("0{hex_value}").to_lowercase());
        } else if hex_value.is_empty() {
            parts.push("0".to_string());
        } else {
            parts.push(hex_value);
        }
    }
    parts.push("0".to_string());
    parts.push("0".to_string());

    parts
        .concat()
        .chars()
        .filter(|c| *c != '.' && *c != '-')
        .collect()
}
