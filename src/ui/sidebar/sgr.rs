use ratatui::style::{Color, Modifier, Style};

use super::super::text::{display_width, take_prefix_width};

/// A run of plain text paired with the style parsed from any SGR sequence preceding it.
pub(super) struct StyledSegment {
    pub text: String,
    pub style: Style,
}

/// The escape-free text across all segments, for width-budget purposes.
pub(super) fn plain_text(segments: &[StyledSegment]) -> String {
    segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect()
}

/// Truncates `segments` to `max_width` display columns, mirroring `truncate_end`'s
/// prefix-plus-ellipsis behavior but preserving per-segment style boundaries.
pub(super) fn truncate_segments(
    segments: &[StyledSegment],
    max_width: usize,
) -> Vec<(String, Style)> {
    let total_width: usize = segments
        .iter()
        .map(|segment| display_width(&segment.text))
        .sum();
    if total_width <= max_width {
        return segments
            .iter()
            .map(|segment| (segment.text.clone(), segment.style))
            .collect();
    }
    if max_width == 0 {
        return Vec::new();
    }
    if max_width == 1 {
        let style = segments
            .first()
            .map_or(Style::default(), |segment| segment.style);
        return vec![("…".to_string(), style)];
    }

    let mut remaining = max_width - 1;
    let mut result = Vec::new();
    for segment in segments {
        if remaining == 0 {
            break;
        }
        let prefix = take_prefix_width(&segment.text, remaining);
        remaining -= display_width(&prefix);
        if !prefix.is_empty() {
            result.push((prefix, segment.style));
        }
    }
    match result.last_mut() {
        Some(last) => last.0.push('…'),
        None => result.push(("…".to_string(), Style::default())),
    }
    result
}

/// Splits `text` into SGR-styled segments, dropping escape bytes from the visible
/// content. Returns `None` if `text` has no escape sequences, so callers can keep
/// treating it as plain text.
///
/// `normalize_metadata_tokens` (`src/app/api_helpers.rs`) only ever lets well-formed
/// `ESC [ <digits/;>* m` sequences reach storage, so an unterminated or non-`m` CSI
/// sequence here would mean that guarantee broke; skip it defensively rather than
/// panic or misrender.
pub(super) fn parse_sgr_segments(text: &str) -> Option<Vec<StyledSegment>> {
    if !text.contains('\u{1b}') {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut segments = Vec::new();
    let mut current_text = String::new();
    let mut current_style = Style::default();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\u{1b}' && chars.get(i + 1) == Some(&'[') {
            let mut j = i + 2;
            while j < chars.len() && matches!(chars[j], '0'..='9' | ';') {
                j += 1;
            }
            let Some(&final_byte) = chars.get(j) else {
                break;
            };
            if final_byte != 'm' {
                i = j + 1;
                continue;
            }
            let params: String = chars[i + 2..j].iter().collect();
            if !current_text.is_empty() {
                segments.push(StyledSegment {
                    text: std::mem::take(&mut current_text),
                    style: current_style,
                });
            }
            current_style = apply_sgr_params(current_style, &params);
            i = j + 1;
            continue;
        }
        current_text.push(chars[i]);
        i += 1;
    }
    if !current_text.is_empty() {
        segments.push(StyledSegment {
            text: current_text,
            style: current_style,
        });
    }
    Some(segments)
}

/// Applies one SGR parameter list to `style`. Foreground and text-attribute codes
/// only; background codes are recognized just enough to skip their extended-color
/// parameters, not applied, per this feature's foreground-only scope (sidebar rows
/// own their selection/highlight background).
fn apply_sgr_params(mut style: Style, params: &str) -> Style {
    let codes: Vec<i64> = if params.is_empty() {
        vec![0]
    } else {
        params.split(';').map(|p| p.parse().unwrap_or(0)).collect()
    };
    let mut i = 0;
    while i < codes.len() {
        match codes[i] {
            0 => style = Style::default(),
            1 => style = style.add_modifier(Modifier::BOLD),
            2 => style = style.add_modifier(Modifier::DIM),
            3 => style = style.add_modifier(Modifier::ITALIC),
            4 => style = style.add_modifier(Modifier::UNDERLINED),
            22 => {
                style = style
                    .remove_modifier(Modifier::BOLD)
                    .remove_modifier(Modifier::DIM);
            }
            23 => style = style.remove_modifier(Modifier::ITALIC),
            24 => style = style.remove_modifier(Modifier::UNDERLINED),
            30..=37 => style = style.fg(Color::Indexed((codes[i] - 30) as u8)),
            90..=97 => style = style.fg(Color::Indexed((codes[i] - 90 + 8) as u8)),
            38 => match codes.get(i + 1) {
                Some(5) => {
                    if let Some(&index) = codes.get(i + 2) {
                        style = style.fg(Color::Indexed(index as u8));
                        i += 2;
                    }
                }
                Some(2) => {
                    if let (Some(&r), Some(&g), Some(&b)) =
                        (codes.get(i + 2), codes.get(i + 3), codes.get(i + 4))
                    {
                        style = style.fg(Color::Rgb(r as u8, g as u8, b as u8));
                        i += 4;
                    }
                }
                _ => {}
            },
            40..=49 => {
                if codes[i] == 48 {
                    match codes.get(i + 1) {
                        Some(5) => i += 2,
                        Some(2) => i += 4,
                        _ => {}
                    }
                }
            }
            100..=107 => {}
            _ => {}
        }
        i += 1;
    }
    style
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(segments: &[StyledSegment]) -> Vec<&str> {
        segments.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn plain_text_has_no_segments() {
        assert!(parse_sgr_segments("plain text").is_none());
    }

    #[test]
    fn single_color_segment() {
        let segments = parse_sgr_segments("\x1b[33mfoo").unwrap();
        assert_eq!(texts(&segments), vec!["foo"]);
        assert_eq!(segments[0].style.fg, Some(Color::Indexed(3)));
    }

    #[test]
    fn multiple_segments_split_on_each_sequence() {
        let segments = parse_sgr_segments("\x1b[31mred\x1b[32mgreen").unwrap();
        assert_eq!(texts(&segments), vec!["red", "green"]);
        assert_eq!(segments[0].style.fg, Some(Color::Indexed(1)));
        assert_eq!(segments[1].style.fg, Some(Color::Indexed(2)));
    }

    #[test]
    fn reset_mid_string_clears_prior_style() {
        let segments = parse_sgr_segments("\x1b[1;31mbold red\x1b[0mplain").unwrap();
        assert_eq!(texts(&segments), vec!["bold red", "plain"]);
        assert!(segments[0].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(segments[1].style, Style::default());
    }

    #[test]
    fn truecolor_and_256_color_are_parsed() {
        let segments = parse_sgr_segments("\x1b[38;2;10;20;30mtrue\x1b[38;5;200mindexed").unwrap();
        assert_eq!(segments[0].style.fg, Some(Color::Rgb(10, 20, 30)));
        assert_eq!(segments[1].style.fg, Some(Color::Indexed(200)));
    }

    #[test]
    fn background_codes_are_ignored_but_do_not_desync_following_codes() {
        let segments = parse_sgr_segments("\x1b[48;2;1;2;3;31mred").unwrap();
        assert_eq!(texts(&segments), vec!["red"]);
        assert_eq!(segments[0].style.fg, Some(Color::Indexed(1)));
        assert_eq!(segments[0].style.bg, None);
    }

    #[test]
    fn unsupported_code_is_ignored_gracefully() {
        let segments = parse_sgr_segments("\x1b[9mstrikethrough").unwrap();
        assert_eq!(texts(&segments), vec!["strikethrough"]);
        assert_eq!(segments[0].style, Style::default());
    }

    #[test]
    fn non_sgr_csi_sequence_is_skipped_without_becoming_visible_text() {
        let segments = parse_sgr_segments("\x1b[2Jfoo").unwrap();
        assert_eq!(texts(&segments), vec!["foo"]);
    }

    #[test]
    fn unterminated_trailing_escape_does_not_panic() {
        let segments = parse_sgr_segments("foo\x1b[1;3").unwrap();
        assert_eq!(texts(&segments), vec!["foo"]);
    }

    #[test]
    fn truncate_segments_leaves_content_untouched_when_it_fits() {
        let segments = parse_sgr_segments("\x1b[31mred\x1b[32mgreen").unwrap();
        let truncated = truncate_segments(&segments, 20);
        assert_eq!(
            truncated,
            vec![
                ("red".to_string(), segments[0].style),
                ("green".to_string(), segments[1].style),
            ]
        );
    }

    #[test]
    fn truncate_segments_cuts_across_segment_boundaries_and_keeps_styles() {
        let segments = parse_sgr_segments("\x1b[31mred\x1b[32mgreen").unwrap();
        let truncated = truncate_segments(&segments, 6);
        assert_eq!(
            truncated,
            vec![
                ("red".to_string(), segments[0].style),
                ("gr…".to_string(), segments[1].style),
            ]
        );
    }

    #[test]
    fn truncate_segments_never_counts_escape_bytes_toward_budget() {
        let segments = parse_sgr_segments("\x1b[38;2;1;2;3mhello").unwrap();
        let truncated = truncate_segments(&segments, 5);
        assert_eq!(truncated, vec![("hello".to_string(), segments[0].style)]);
    }
}
