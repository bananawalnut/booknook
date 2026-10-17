use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Resolution {
    Resolved {
        start_utf16: usize,
        end_utf16: usize,
    },
    Ambiguous {
        matches: usize,
        context: String,
    },
    Missing {
        context: String,
    },
}

/// Exact edition/resource must be checked by the caller before invoking quote resolution.
/// Offsets are UTF-16, matching DOM Range and TextKit. Never select an arbitrary duplicate.
pub fn resolve(text: &str, quote: &str, prefix: &str, suffix: &str) -> Resolution {
    if quote.is_empty() {
        return Resolution::Missing {
            context: quote.into(),
        };
    }
    let matches: Vec<_> = text
        .char_indices()
        .map(|(i, _)| i)
        .filter(|&i| {
            text[i..].starts_with(quote)
                && text[..i].ends_with(prefix)
                && text[i + quote.len()..].starts_with(suffix)
        })
        .collect();
    match matches.as_slice() {
        [start] => Resolution::Resolved {
            start_utf16: text[..*start].encode_utf16().count(),
            end_utf16: text[..start + quote.len()].encode_utf16().count(),
        },
        [] => Resolution::Missing {
            context: format!("{prefix}{quote}{suffix}"),
        },
        _ => Resolution::Ambiguous {
            matches: matches.len(),
            context: format!("{prefix}{quote}{suffix}"),
        },
    }
}

pub fn ink_can_place(saved_layout: &str, current_layout: &str, edition_matches: bool) -> bool {
    edition_matches && !saved_layout.is_empty() && saved_layout == current_layout
}
