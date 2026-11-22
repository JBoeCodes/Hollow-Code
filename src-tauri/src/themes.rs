// Custom theme files (kitty-style `key = "#hex"` palette files).
//
// Theme files live in `dirs::config_dir() / Hollow / themes / *.theme`
// — that resolves to `~/Library/Application Support/Hollow/themes/`
// on macOS and `~/.config/Hollow/themes/` on Linux, matching the same
// convention the main config uses.
//
// The parser is permissive on purpose: kitty themes pasted as-is
// (`accent #c8761a`) work alongside the `=` form (`accent = "#c8761a"`)
// the README documents. Only `accent`, `foreground`, and `background`
// are required; everything else (cursor, color0..15, selection_*) is
// optional and downstream code falls back to derived values.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const FILE_EXT: &str = "theme";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomTheme {
    pub name: String,
    /// Raw file contents — surfaces in the editor preview / export.
    pub source: String,
    pub is_dark: bool,
    pub accent: String,
    pub cursor: Option<String>,
    pub foreground: String,
    pub background: String,
    pub selection_foreground: Option<String>,
    pub selection_background: Option<String>,
    /// 16-slot ANSI palette (color0..color15). Empty slots stay `None`
    /// so the frontend can fall back to derived values.
    pub colors: Vec<Option<String>>,
    pub success: Option<String>,
    pub warning: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomThemeMeta {
    pub name: String,
    pub path: String,
    pub is_dark: bool,
    pub accent: String,
    pub background: String,
    pub foreground: String,
}

pub fn themes_dir() -> Result<PathBuf, String> {
    let base = dirs::config_dir().ok_or_else(|| "config dir not found".to_string())?;
    Ok(base
        .join(crate::instance::current().support_name())
        .join("themes"))
}

/// Ensure the themes directory exists. On first creation, seed it with
/// two starter files so the picker isn't empty out of the box and the
/// user has a concrete example to clone.
pub fn ensure_themes_dir() -> Result<PathBuf, String> {
    let dir = themes_dir()?;
    let was_missing = !dir.exists();
    fs::create_dir_all(&dir).map_err(|e| format!("create themes dir: {e}"))?;
    if was_missing {
        let _ = fs::write(dir.join("Graphite.theme"), GRAPHITE_STARTER);
        let _ = fs::write(dir.join("Solarized Dark.theme"), SOLARIZED_DARK_STARTER);
    }
    Ok(dir)
}

pub fn list_themes() -> Result<Vec<CustomThemeMeta>, String> {
    let dir = ensure_themes_dir()?;
    let mut out = Vec::new();
    let rd = fs::read_dir(&dir).map_err(|e| format!("read dir: {e}"))?;
    for entry in rd.flatten() {
        let p = entry.path();
        if p.extension().and_then(|x| x.to_str()) != Some(FILE_EXT) {
            continue;
        }
        let stem = match p.file_stem().and_then(|x| x.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let text = match fs::read_to_string(&p) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let theme = match parse_theme(&stem, &text) {
            Ok(t) => t,
            Err(_) => continue,
        };
        out.push(CustomThemeMeta {
            name: theme.name,
            path: p.to_string_lossy().into_owned(),
            is_dark: theme.is_dark,
            accent: theme.accent,
            background: theme.background,
            foreground: theme.foreground,
        });
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    Ok(out)
}

pub fn read_theme(name: &str) -> Result<CustomTheme, String> {
    let dir = ensure_themes_dir()?;
    let path = locate_theme_file(&dir, name)?;
    let text = fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let stem = path.file_stem().and_then(|x| x.to_str()).unwrap_or(name);
    parse_theme(stem, &text)
}

pub fn write_theme(name: &str, content: &str) -> Result<CustomTheme, String> {
    let dir = ensure_themes_dir()?;
    let parsed = parse_theme(name, content)?;
    let sanitized = sanitize_filename(name);
    if sanitized.is_empty() {
        return Err("theme name must contain at least one character".into());
    }
    let path = dir.join(format!("{sanitized}.{FILE_EXT}"));
    crate::vault::atomic_write(&path, content.as_bytes())
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(parsed)
}

pub fn delete_theme(name: &str) -> Result<(), String> {
    let dir = ensure_themes_dir()?;
    let path = locate_theme_file(&dir, name)?;
    fs::remove_file(&path).map_err(|e| format!("delete {}: {e}", path.display()))
}

pub fn reveal_themes_dir() -> Result<(), String> {
    let dir = ensure_themes_dir()?;
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&dir)
            .spawn()
            .map_err(|e| format!("open: {e}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = dir;
    }
    Ok(())
}

/// Parse a kitty-style theme file. Accepts both `key = "value"` and
/// `key value` forms. Comments start with `#` or `//`. Values are
/// optionally quoted with `"` or `'`.
pub fn parse_theme(default_name: &str, source: &str) -> Result<CustomTheme, String> {
    let mut map = HashMap::<String, String>::new();
    for raw in source.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
            continue;
        }
        // The equals form clearly fences the value, so we can safely strip
        // an inline `# trailing comment`. The kitty space form (`accent
        // #c8761a`) carries the hash in the value — never strip there.
        let (k, v) = match trimmed.split_once('=') {
            Some((k, v)) => (k, strip_inline_comment(v)),
            None => match trimmed.split_once(char::is_whitespace) {
                Some(pair) => pair,
                None => continue,
            },
        };
        let key = k.trim().to_lowercase();
        let raw_val = v.trim();
        let unquoted = raw_val
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .or_else(|| {
                raw_val
                    .strip_prefix('\'')
                    .and_then(|s| s.strip_suffix('\''))
            })
            .unwrap_or(raw_val);
        let val = unquoted.trim().to_string();
        if !key.is_empty() && !val.is_empty() {
            map.insert(key, val);
        }
    }

    let take_required = |k: &str| -> Result<String, String> {
        let val = map
            .get(k)
            .cloned()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("missing required field: {k}"))?;
        validate_color(k, &val)
    };
    let take_optional = |k: &str| -> Result<Option<String>, String> {
        match map.get(k) {
            Some(val) => validate_color(k, val).map(Some),
            None => Ok(None),
        }
    };
    let accent = take_required("accent")?;
    let foreground = take_required("foreground")?;
    let background = take_required("background")?;
    let cursor = take_optional("cursor")?;
    let selection_foreground = take_optional("selection_foreground")?;
    let selection_background = take_optional("selection_background")?;
    let success = take_optional("success")?;
    let warning = take_optional("warning")?;
    let error = take_optional("error")?;

    let mut colors = Vec::with_capacity(16);
    for i in 0..16 {
        let key = format!("color{i}");
        colors.push(take_optional(&key)?);
    }

    let is_dark = match map.get("is_dark").map(|s| s.to_lowercase()) {
        Some(s) if s == "true" || s == "1" || s == "yes" => true,
        Some(s) if s == "false" || s == "0" || s == "no" => false,
        _ => is_color_dark(&background),
    };

    let display_name = map
        .get("name")
        .cloned()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default_name.to_string());

    Ok(CustomTheme {
        name: display_name,
        source: source.to_string(),
        is_dark,
        accent,
        cursor,
        foreground,
        background,
        selection_foreground,
        selection_background,
        colors,
        success,
        warning,
        error,
    })
}

/// Strictly validate a theme color value so `.theme` files cannot inject CSS
/// or HTML into rendered styles or sandboxed artifacts. Accepts only safe hex
/// literals (`#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA`) or standard CSS named
/// color keywords (pure ASCII letters, no punctuation, quotes, angle brackets,
/// braces, semicolons, parentheses, or functional notations such as `url()`).
fn validate_color(field: &str, value: &str) -> Result<String, String> {
    if is_valid_color(value) {
        Ok(value.to_string())
    } else {
        Err(format!("invalid color value for {field}"))
    }
}

fn is_valid_color(value: &str) -> bool {
    if value.is_empty() || value.len() > 32 {
        return false;
    }
    if let Some(hex) = value.strip_prefix('#') {
        return matches!(hex.len(), 3 | 4 | 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit());
    }
    if !value.bytes().all(|b| b.is_ascii_alphabetic()) {
        return false;
    }
    let lower = value.to_ascii_lowercase();
    CSS_NAMED_COLORS.binary_search(&lower.as_str()).is_ok()
}

/// Standard CSS Color Module Level 4 named colors plus `transparent` (sorted
/// for binary search).
#[rustfmt::skip]
const CSS_NAMED_COLORS: &[&str] = &[
    "aliceblue", "antiquewhite", "aqua", "aquamarine", "azure", "beige", "bisque", "black",
    "blanchedalmond", "blue", "blueviolet", "brown", "burlywood", "cadetblue", "chartreuse",
    "chocolate", "coral", "cornflowerblue", "cornsilk", "crimson", "cyan", "darkblue", "darkcyan",
    "darkgoldenrod", "darkgray", "darkgreen", "darkgrey", "darkkhaki", "darkmagenta",
    "darkolivegreen", "darkorange", "darkorchid", "darkred", "darksalmon", "darkseagreen",
    "darkslateblue", "darkslategray", "darkslategrey", "darkturquoise", "darkviolet", "deeppink",
    "deepskyblue", "dimgray", "dimgrey", "dodgerblue", "firebrick", "floralwhite", "forestgreen",
    "fuchsia", "gainsboro", "ghostwhite", "gold", "goldenrod", "gray", "green", "greenyellow",
    "grey", "honeydew", "hotpink", "indianred", "indigo", "ivory", "khaki", "lavender",
    "lavenderblush", "lawngreen", "lemonchiffon", "lightblue", "lightcoral", "lightcyan",
    "lightgoldenrodyellow", "lightgray", "lightgreen", "lightgrey", "lightpink", "lightsalmon",
    "lightseagreen", "lightskyblue", "lightslategray", "lightslategrey", "lightsteelblue",
    "lightyellow", "lime", "limegreen", "linen", "magenta", "maroon", "mediumaquamarine",
    "mediumblue", "mediumorchid", "mediumpurple", "mediumseagreen", "mediumslateblue",
    "mediumspringgreen", "mediumturquoise", "mediumvioletred", "midnightblue", "mintcream",
    "mistyrose", "moccasin", "navajowhite", "navy", "oldlace", "olive", "olivedrab", "orange",
    "orangered", "orchid", "palegoldenrod", "palegreen", "paleturquoise", "palevioletred",
    "papayawhip", "peachpuff", "peru", "pink", "plum", "powderblue", "purple", "rebeccapurple",
    "red", "rosybrown", "royalblue", "saddlebrown", "salmon", "sandybrown", "seagreen", "seashell",
    "sienna", "silver", "skyblue", "slateblue", "slategray", "slategrey", "snow", "springgreen",
    "steelblue", "tan", "teal", "thistle", "tomato", "transparent", "turquoise", "violet", "wheat",
    "white", "whitesmoke", "yellow", "yellowgreen",
];

/// Strip a `# trailing comment` from the right-hand side of an equals
/// expression. Only called when we've already seen `=`, so the entire
/// argument is the value side — meaning we can safely treat
/// quote-delimited regions as opaque and chop at the first bare `#`.
fn strip_inline_comment(value_side: &str) -> &str {
    let bytes = value_side.as_bytes();
    let mut in_quote: Option<u8> = None;
    let mut last_was_space = true; // leading whitespace counts as "after space"
    for (i, b) in bytes.iter().enumerate() {
        match in_quote {
            Some(q) => {
                if *b == q {
                    in_quote = None;
                }
                last_was_space = false;
            }
            None => {
                if *b == b'"' || *b == b'\'' {
                    in_quote = Some(*b);
                    last_was_space = false;
                } else if *b == b'#' && last_was_space {
                    // A `#` after whitespace is a comment — but only when
                    // it's not the very first non-space char (so an
                    // unquoted `#hex` literal still works).
                    let before = value_side[..i].trim();
                    if !before.is_empty() {
                        return value_side[..i].trim_end();
                    }
                    last_was_space = false;
                } else {
                    last_was_space = b.is_ascii_whitespace();
                }
            }
        }
    }
    value_side
}

fn is_color_dark(hex: &str) -> bool {
    let (r, g, b) = match hex_to_rgb(hex) {
        Some(v) => v,
        None => return false,
    };
    let lum = (0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32) / 255.0;
    lum < 0.5
}

fn hex_to_rgb(hex: &str) -> Option<(u8, u8, u8)> {
    let h = hex.trim().trim_start_matches('#');
    let parse = |s: &str| u8::from_str_radix(s, 16).ok();
    match h.len() {
        6 => Some((parse(&h[0..2])?, parse(&h[2..4])?, parse(&h[4..6])?)),
        3 => {
            let r = parse(&h[0..1].repeat(2))?;
            let g = parse(&h[1..2].repeat(2))?;
            let b = parse(&h[2..3].repeat(2))?;
            Some((r, g, b))
        }
        _ => None,
    }
}

fn locate_theme_file(dir: &Path, name: &str) -> Result<PathBuf, String> {
    let sanitized = sanitize_filename(name);
    let direct = dir.join(format!("{sanitized}.{FILE_EXT}"));
    if direct.exists() {
        return Ok(direct);
    }
    let rd = fs::read_dir(dir).map_err(|e| format!("read dir: {e}"))?;
    let needle = name.to_lowercase();
    for entry in rd.flatten() {
        let p = entry.path();
        if p.extension().and_then(|x| x.to_str()) != Some(FILE_EXT) {
            continue;
        }
        if let Some(stem) = p.file_stem().and_then(|x| x.to_str()) {
            if stem.to_lowercase() == needle {
                return Ok(p);
            }
        }
        if let Ok(t) = fs::read_to_string(&p) {
            let stem = p.file_stem().and_then(|x| x.to_str()).unwrap_or("");
            if let Ok(parsed) = parse_theme(stem, &t) {
                if parsed.name.to_lowercase() == needle {
                    return Ok(p);
                }
            }
        }
    }
    Err(format!("theme not found: {name}"))
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .filter(|c| !matches!(c, '/' | '\\' | ':' | '\0'))
        .collect::<String>()
        .trim()
        .to_string()
}

const GRAPHITE_STARTER: &str = r##"# Graphite — calm monochrome scheme. The default starter file —
# delete it any time. Drop kitty terminal themes here as-is; both
# `accent = "#hex"` and the kitty-style `accent #hex` forms parse.
name = "Graphite"
is_dark = true

accent = "#9e9e9e"
cursor = "#e0e0e0"
foreground = "#e0e0e0"
background = "#121212"
selection_foreground = "#121212"
selection_background = "#e0e0e0"

color0  = "#121212"
color1  = "#8a8a8a"
color2  = "#767676"
color3  = "#9e9e9e"
color4  = "#626262"
color5  = "#8a8a8a"
color6  = "#767676"
color7  = "#e0e0e0"
color8  = "#8a8a8a"
color9  = "#9e9e9e"
color10 = "#767676"
color11 = "#b2b2b2"
color12 = "#626262"
color13 = "#8a8a8a"
color14 = "#9e9e9e"
color15 = "#ffffff"
"##;

const SOLARIZED_DARK_STARTER: &str = r##"# Solarized Dark — Ethan Schoonover's classic.
name = "Solarized Dark"
is_dark = true

accent = "#b58900"
cursor = "#93a1a1"
foreground = "#93a1a1"
background = "#002b36"
selection_foreground = "#002b36"
selection_background = "#93a1a1"

color0  = "#073642"
color1  = "#dc322f"
color2  = "#859900"
color3  = "#b58900"
color4  = "#268bd2"
color5  = "#d33682"
color6  = "#2aa198"
color7  = "#eee8d5"
color8  = "#586e75"
color9  = "#cb4b16"
color10 = "#586e75"
color11 = "#657b83"
color12 = "#839496"
color13 = "#6c71c4"
color14 = "#93a1a1"
color15 = "#fdf6e3"
"##;

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r##"
        # comment
        name = "Sample"
        is_dark = true
        accent = "#626262"
        foreground = "#e0e0e0"
        background = "#121212"
        cursor = "#e0e0e0"
        color0 = "#121212"
        color1 = "#8a8a8a"
        color15 = "#ffffff"
    "##;

    #[test]
    fn parses_equals_form() {
        let t = parse_theme("Sample", SAMPLE).unwrap();
        assert_eq!(t.name, "Sample");
        assert_eq!(t.accent, "#626262");
        assert_eq!(t.background, "#121212");
        assert_eq!(t.foreground, "#e0e0e0");
        assert!(t.is_dark);
        assert_eq!(t.colors[0].as_deref(), Some("#121212"));
        assert_eq!(t.colors[15].as_deref(), Some("#ffffff"));
        assert!(t.colors[2].is_none());
    }

    #[test]
    fn parses_kitty_space_form() {
        let txt = "accent #c8761a\nforeground #1c1408\nbackground #f4ecd8\n";
        let t = parse_theme("Kitty", txt).unwrap();
        assert_eq!(t.accent, "#c8761a");
        assert_eq!(t.background, "#f4ecd8");
        assert!(!t.is_dark);
    }

    #[test]
    fn missing_required_field_errors() {
        assert!(parse_theme("X", "background = \"#000\"").is_err());
        assert!(parse_theme("X", "accent = \"#000\"\nforeground = \"#fff\"").is_err());
    }

    #[test]
    fn autodetects_dark_when_unset() {
        let dark = parse_theme(
            "D",
            "accent = \"#fff\"\nforeground = \"#eee\"\nbackground = \"#000\"",
        )
        .unwrap();
        let light = parse_theme(
            "L",
            "accent = \"#000\"\nforeground = \"#111\"\nbackground = \"#fff\"",
        )
        .unwrap();
        assert!(dark.is_dark);
        assert!(!light.is_dark);
    }

    #[test]
    fn ignores_inline_comment_after_value() {
        let txt = "accent = \"#abc\" # trailing\nforeground = \"#def\"\nbackground = \"#012\"\n";
        let t = parse_theme("C", txt).unwrap();
        assert_eq!(t.accent, "#abc");
    }

    #[test]
    fn keeps_hash_inside_quoted_value() {
        let txt = "accent = \"#c8761a\"\nforeground=\"#fff\"\nbackground=\"#000\"\n";
        let t = parse_theme("Q", txt).unwrap();
        assert_eq!(t.accent, "#c8761a");
    }

    #[test]
    fn sanitize_filename_strips_separators() {
        assert_eq!(sanitize_filename("Hello / World"), "Hello  World");
        assert_eq!(sanitize_filename("  spaces  "), "spaces");
        assert!(sanitize_filename("///").is_empty());
    }

    #[test]
    fn starters_round_trip() {
        let t = parse_theme("Graphite", GRAPHITE_STARTER).unwrap();
        assert_eq!(t.name, "Graphite");
        let s = parse_theme("Solarized Dark", SOLARIZED_DARK_STARTER).unwrap();
        assert_eq!(s.name, "Solarized Dark");
        assert!(s.is_dark);
    }

    #[test]
    fn css_named_colors_list_is_sorted() {
        for pair in CSS_NAMED_COLORS.windows(2) {
            assert!(pair[0] < pair[1], "unsorted: {} >= {}", pair[0], pair[1]);
        }
    }

    #[test]
    fn accepts_safe_hex_and_css_named_colors() {
        let txt = r##"
            accent = "#abc"
            foreground = "#abcd"
            background = "#123456"
            cursor = "#12345678"
            selection_foreground = "White"
            selection_background = "rebeccapurple"
            color0 = "transparent"
            success = "#00ff88"
        "##;
        let t = parse_theme("Safe", txt).unwrap();
        assert_eq!(t.accent, "#abc");
        assert_eq!(t.foreground, "#abcd");
        assert_eq!(t.background, "#123456");
        assert_eq!(t.cursor.as_deref(), Some("#12345678"));
        assert_eq!(t.selection_foreground.as_deref(), Some("White"));
        assert_eq!(t.selection_background.as_deref(), Some("rebeccapurple"));
        assert_eq!(t.colors[0].as_deref(), Some("transparent"));
    }

    #[test]
    fn rejects_css_and_html_injection_in_theme_colors() {
        let payloads = [
            "#fff; } </style><script>alert(1)</script>",
            "red; background: url(https://evil.example/x)",
            "url(https://evil.example/a.png)",
            "rgb(255, 0, 0)",
            "expression(alert(1))",
            "#12345",
            "#gggggg",
            "#fff\" onclick=\"alert(1)",
            "#fff' }",
            "#fff<>",
            "#fff{}",
            "notacolor",
            "\"#fff",
        ];
        for bad in payloads {
            let required =
                format!("accent = \"{bad}\"\nforeground = \"#eee\"\nbackground = \"#111\"\n");
            assert!(
                parse_theme("Bad", &required).is_err(),
                "required field accepted dangerous value: {bad}"
            );
            let optional = format!(
                "accent = \"#123456\"\nforeground = \"#eee\"\nbackground = \"#111\"\ncolor0 = \"{bad}\"\n"
            );
            assert!(
                parse_theme("BadOpt", &optional).is_err(),
                "optional color0 accepted dangerous value: {bad}"
            );
            let error_field = format!(
                "accent = \"#123456\"\nforeground = \"#eee\"\nbackground = \"#111\"\nerror = \"{bad}\"\n"
            );
            assert!(
                parse_theme("BadErr", &error_field).is_err(),
                "optional error accepted dangerous value: {bad}"
            );
        }
    }
}
