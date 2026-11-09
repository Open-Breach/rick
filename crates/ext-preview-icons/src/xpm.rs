//! XPM (X `PixMap`, version 3), parsed here: the C array of strings with
//! the values line, the colour table and the pixel rows. One and two
//! characters a pixel (up to four are taken); a `c` colour key wins over
//! `g`, `g4` and `m`; a colour is `#rgb`, `#rrggbb` or `#rrrrggggbbbb`,
//! `None` for transparent, or an X11 colour name from `rgb.txt`
//! ([`color`]).

use std::collections::HashMap;

use crate::x11::X11_COLORS;

/// What the file opens with, after any whitespace.
pub const MAGIC: &[u8] = b"/* XPM */";

/// The largest picture parsed, per axis.
pub const MAX_SIDE: u32 = 16384;

/// The most characters a pixel may take.
pub const MAX_CHARS_PER_PIXEL: usize = 4;

/// A decoded XPM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Xpm {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Colours in the table.
    pub colors: usize,
    /// Characters per pixel in the rows.
    pub chars_per_pixel: usize,
    /// RGBA, row-major.
    pub pixels: Vec<u8>,
}

/// Whether `sniff` starts an XPM: the `/* XPM */` magic after any
/// whitespace or byte order mark.
#[must_use]
pub fn is_xpm(sniff: &[u8]) -> bool {
    let rest = sniff.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(sniff);
    let start = rest
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(rest.len());
    rest[start..].starts_with(MAGIC)
}

/// Parses the text of an XPM file.
///
/// # Errors
///
/// A message naming what is wrong: a values line that is not four
/// numbers, a size out of range, a colour that is neither a hex triple
/// nor `None` nor an X11 name, a pixel whose characters name no colour,
/// or a file cut short.
pub fn parse(text: &str) -> Result<Xpm, String> {
    let strings = strings(text);
    let values = strings.first().ok_or("no values line")?;
    let numbers: Vec<u32> = values
        .split_whitespace()
        .take(4)
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(|_| format!("the values line is not 'width height colours chars': {values:?}"))?;
    let [width, height, colors, chars_per_pixel] = numbers[..] else {
        return Err(format!(
            "the values line is not 'width height colours chars': {values:?}"
        ));
    };
    if !(1..=MAX_SIDE).contains(&width) || !(1..=MAX_SIDE).contains(&height) {
        return Err(format!("size {width} × {height} out of range"));
    }
    let chars_per_pixel = chars_per_pixel as usize;
    if !(1..=MAX_CHARS_PER_PIXEL).contains(&chars_per_pixel) {
        return Err(format!("{chars_per_pixel} characters per pixel"));
    }
    let colors = colors as usize;
    if colors == 0 {
        return Err("no colours".to_owned());
    }
    let needed = 1 + colors + height as usize;
    if strings.len() < needed {
        return Err(format!(
            "cut short: {} strings of the {needed} needed",
            strings.len()
        ));
    }
    let mut table: HashMap<String, [u8; 4]> = HashMap::with_capacity(colors);
    for line in &strings[1..=colors] {
        let (chars, rgba) = color_line(line, chars_per_pixel)?;
        table.insert(chars, rgba);
    }
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for (row, line) in strings[1 + colors..needed].iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        if chars.len() < width as usize * chars_per_pixel {
            return Err(format!("row {} is shorter than the width", row + 1));
        }
        for chunk in chars.chunks_exact(chars_per_pixel).take(width as usize) {
            let key: String = chunk.iter().collect();
            let rgba = table
                .get(&key)
                .ok_or_else(|| format!("pixel {key:?} in row {} names no colour", row + 1))?;
            pixels.extend_from_slice(rgba);
        }
    }
    Ok(Xpm {
        width,
        height,
        colors,
        chars_per_pixel,
        pixels,
    })
}

/// The characters and the colour of one line of the colour table: the
/// first `chars_per_pixel` characters, then key and value pairs, the `c`
/// key first, then `g`, `g4` and `m`; `s` names a symbol and is skipped.
fn color_line(line: &str, chars_per_pixel: usize) -> Result<(String, [u8; 4]), String> {
    let chars: String = line.chars().take(chars_per_pixel).collect();
    if chars.chars().count() < chars_per_pixel {
        return Err(format!("colour line {line:?} is shorter than a pixel"));
    }
    let rest: String = line.chars().skip(chars_per_pixel).collect();
    let mut keyed: Vec<(&str, Vec<&str>)> = Vec::new();
    for token in rest.split_whitespace() {
        match token {
            "c" | "m" | "g" | "g4" | "s" => keyed.push((token, Vec::new())),
            word => match keyed.last_mut() {
                Some((_, words)) => words.push(word),
                None => return Err(format!("colour line {line:?} has no key")),
            },
        }
    }
    let value = ["c", "g", "g4", "m"]
        .iter()
        .find_map(|wanted| {
            keyed
                .iter()
                .find(|(key, words)| key == wanted && !words.is_empty())
        })
        .map(|(_, words)| words.join(" "))
        .ok_or_else(|| format!("colour line {line:?} has no colour"))?;
    let rgba = color(&value).ok_or_else(|| format!("unknown colour {value:?}"))?;
    Ok((chars, rgba))
}

/// The RGBA of an XPM colour value: `None` is transparent; `#rgb`,
/// `#rrggbb` and `#rrrrggggbbbb` are hex triples; anything else is an X11
/// colour name, matched without regard to case and spaces.
#[must_use]
pub fn color(value: &str) -> Option<[u8; 4]> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("none") {
        return Some([0, 0, 0, 0]);
    }
    if let Some(hex) = value.strip_prefix('#') {
        let digits = hex.len();
        if digits == 0 || digits % 3 != 0 || digits / 3 > 4 || !hex.is_ascii() {
            return None;
        }
        let per_channel = digits / 3;
        let mut rgba = [0, 0, 0, 255];
        for (i, slot) in rgba.iter_mut().take(3).enumerate() {
            let channel = &hex[i * per_channel..(i + 1) * per_channel];
            // The high byte of each channel; a single digit stands for itself twice.
            let high: String = if per_channel == 1 {
                channel.repeat(2)
            } else {
                channel[..2].to_owned()
            };
            *slot = u8::from_str_radix(&high, 16).ok()?;
        }
        return Some(rgba);
    }
    let name: String = value
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    X11_COLORS
        .binary_search_by(|(known, _)| (*known).cmp(name.as_str()))
        .ok()
        .map(|i| {
            let [r, g, b] = X11_COLORS[i].1;
            [r, g, b, 255]
        })
}

/// The string literals of the C source, in order, with `/* */` and `//`
/// comments skipped.
fn strings(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut last = ' ';
                for c in chars.by_ref() {
                    if last == '*' && c == '/' {
                        break;
                    }
                    last = c;
                }
            }
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '"' => {
                let mut literal = String::new();
                loop {
                    match chars.next() {
                        None | Some('"') => break,
                        Some('\\') => {
                            if let Some(escaped) = chars.next() {
                                literal.push(escaped);
                            }
                        }
                        Some(c) => literal.push(c),
                    }
                }
                out.push(literal);
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_come_as_hex_none_or_x11_names() {
        assert_eq!(color("None"), Some([0, 0, 0, 0]));
        assert_eq!(color("none"), Some([0, 0, 0, 0]));
        assert_eq!(color("#F80"), Some([255, 136, 0, 255]));
        assert_eq!(color("#00FF00"), Some([0, 255, 0, 255]));
        assert_eq!(color("#FFFF80000000"), Some([255, 128, 0, 255]));
        assert_eq!(color("red"), Some([255, 0, 0, 255]));
        assert_eq!(color("Light Slate Blue"), Some([132, 112, 255, 255]));
        assert_eq!(color("LightSlateBlue"), Some([132, 112, 255, 255]));
        assert_eq!(color("gray50"), Some([127, 127, 127, 255]));
        assert_eq!(color("grey"), Some([190, 190, 190, 255]));
        assert_eq!(color("#12"), None);
        assert_eq!(color("#GGG"), None);
        assert_eq!(color("nosuchcolour"), None);
        assert_eq!(color("%1 2 3"), None);
    }

    #[test]
    fn the_table_is_sorted_for_the_binary_search() {
        assert!(X11_COLORS.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn a_small_picture_parses_with_every_key_kind() {
        let text = "/* XPM */\nstatic char *x[] = {\n/* values */\n\"3 2 3 1\",\n\"a c red m black\",\n\"b m black g gray50 c None\",\n\"c s border g4 #ABC\",\n// rows\n\"abc\",\n\"cba\"\n};\n";
        let xpm = parse(text).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            (xpm.width, xpm.height, xpm.colors, xpm.chars_per_pixel),
            (3, 2, 3, 1)
        );
        let px = |x: usize, y: usize| {
            let i = (y * 3 + x) * 4;
            [
                xpm.pixels[i],
                xpm.pixels[i + 1],
                xpm.pixels[i + 2],
                xpm.pixels[i + 3],
            ]
        };
        assert_eq!(px(0, 0), [255, 0, 0, 255], "c wins over m");
        assert_eq!(px(1, 0), [0, 0, 0, 0], "c None wins over m and g");
        assert_eq!(px(2, 0), [0xAA, 0xBB, 0xCC, 255], "g4 when there is no c");
        assert_eq!(px(0, 1), [0xAA, 0xBB, 0xCC, 255]);
        assert_eq!(px(2, 1), [255, 0, 0, 255]);
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        assert!(parse("").is_err());
        assert!(parse("/* XPM */ static char *x[] = { \"a b\" };").is_err());
        assert!(
            parse("\"2 2 1 1\", \"a c red\", \"aa\"").is_err(),
            "a row short"
        );
        assert!(
            parse("\"2 1 1 1\", \"a c red\", \"ab\"").is_err(),
            "unknown pixel"
        );
        assert!(
            parse("\"2 1 1 1\", \"a c #12345\", \"aa\"").is_err(),
            "bad hex"
        );
        assert!(parse("\"2 1 1 1\", \"a\", \"aa\"").is_err(), "no key");
        assert!(
            parse("\"0 1 1 1\", \"a c red\", \"\"").is_err(),
            "zero size"
        );
        assert!(
            parse("\"2 1 1 9\", \"a c red\", \"aa\"").is_err(),
            "nine chars a pixel"
        );
        assert!(
            parse("\"99999 1 1 1\", \"a c red\", \"a\"").is_err(),
            "too wide"
        );
        assert!(is_xpm(b"  /* XPM */\nstatic"));
        assert!(is_xpm(b"\xEF\xBB\xBF/* XPM */"));
        assert!(!is_xpm(b"/* XPM2 */"));
        assert!(!is_xpm(b"static char"));
    }
}
