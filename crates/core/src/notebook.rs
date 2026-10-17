use crate::model::InlineStyle;
use crate::{Error, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use yrs::{
    Doc, GetString, OffsetKind, Options, ReadTxn, StateVector, Text, Transact, Update,
    updates::decoder::Decode,
};

fn load(encoded: &str) -> Result<Doc> {
    let doc = Doc::with_options(Options {
        offset_kind: OffsetKind::Utf16,
        ..Default::default()
    });
    doc.get_or_insert_text("content");
    if !encoded.is_empty() {
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|e| Error::Invalid(e.to_string()))?;
        let update = Update::decode_v1(&bytes).map_err(|e| Error::Invalid(e.to_string()))?;
        doc.transact_mut()
            .apply_update(update)
            .map_err(|e| Error::Invalid(e.to_string()))?;
    }
    Ok(doc)
}

pub fn text(encoded: &str) -> Result<String> {
    let doc = load(encoded)?;
    Ok(doc
        .get_or_insert_text("content")
        .get_string(&doc.transact()))
}

/// Apply the minimal scalar-aligned replacement, expressed to Yrs in UTF-16 offsets.
/// Native IME composition remains in TextKit until it commits.
pub fn edit(encoded: &str, replacement: &str) -> Result<String> {
    let doc = load(encoded)?;
    let text = doc.get_or_insert_text("content");
    let old = text.get_string(&doc.transact());
    let left: Vec<char> = old.chars().collect();
    let right: Vec<char> = replacement.chars().collect();
    let prefix = left.iter().zip(&right).take_while(|(a, b)| a == b).count();
    let suffix = left[prefix..]
        .iter()
        .rev()
        .zip(right[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let offset = left[..prefix].iter().map(|c| c.len_utf16() as u32).sum();
    let removed: u32 = left[prefix..left.len() - suffix]
        .iter()
        .map(|c| c.len_utf16() as u32)
        .sum();
    let insert: String = right[prefix..right.len() - suffix].iter().collect();
    {
        let mut txn = doc.transact_mut();
        if removed > 0 {
            text.remove_range(&mut txn, offset, removed);
        }
        if !insert.is_empty() {
            text.insert(&mut txn, offset, &insert);
        }
    }
    Ok(STANDARD.encode(
        doc.transact()
            .encode_state_as_update_v1(&StateVector::default()),
    ))
}

pub fn merge(local: &str, incoming: &str) -> Result<(String, String)> {
    let doc = load(local)?;
    let bytes = STANDARD
        .decode(incoming)
        .map_err(|e| Error::Invalid(e.to_string()))?;
    doc.transact_mut()
        .apply_update(Update::decode_v1(&bytes).map_err(|e| Error::Invalid(e.to_string()))?)
        .map_err(|e| Error::Invalid(e.to_string()))?;
    let text = doc
        .get_or_insert_text("content")
        .get_string(&doc.transact());
    let state = STANDARD.encode(
        doc.transact()
            .encode_state_as_update_v1(&StateVector::default()),
    );
    Ok((state, text))
}

pub fn styles(encoded: &str) -> Result<Vec<InlineStyle>> {
    let doc = load(encoded)?;
    let text = doc.get_or_insert_text("content");
    let txn = doc.transact();
    let mut start = 0;
    let mut result = vec![];
    for part in text.diff(&txn, |_| ()) {
        let yrs::Out::Any(yrs::Any::String(value)) = part.insert else {
            return Err(Error::Invalid("Unsupported notebook embed".into()));
        };
        let length = value.encode_utf16().count() as u32;
        let enabled = |name: &str| {
            part.attributes
                .as_ref()
                .and_then(|a| a.get(name))
                .is_some_and(|v| matches!(v, yrs::Any::Bool(true)))
        };
        let (bold, italic, underline) = (enabled("bold"), enabled("italic"), enabled("underline"));
        if bold || italic || underline {
            result.push(InlineStyle {
                start_utf16: start,
                length_utf16: length,
                bold,
                italic,
                underline,
            });
        }
        start += length;
    }
    Ok(result)
}
/// Validate UTF-16 style boundaries without decoding a CRDT or allocating per-character indices.
pub fn validate_styles(replacement: &str, styles: &[InlineStyle]) -> Result<u32> {
    let mut last_end = 0;
    for style in styles {
        let end = style
            .start_utf16
            .checked_add(style.length_utf16)
            .ok_or_else(|| Error::Invalid("Style range overflow".into()))?;
        if style.start_utf16 < last_end || style.length_utf16 == 0 {
            return Err(Error::Invalid(
                "Invalid or overlapping UTF-16 style range".into(),
            ));
        }
        last_end = end;
    }
    let mut markers = styles
        .iter()
        .flat_map(|s| [s.start_utf16, s.start_utf16 + s.length_utf16])
        .peekable();
    let mut length = 0u32;
    for c in replacement.chars() {
        while markers.peek() == Some(&length) {
            markers.next();
        }
        if markers.peek().is_some_and(|next| *next < length) {
            return Err(Error::Invalid(
                "Style boundary splits a Unicode scalar".into(),
            ));
        }
        length = length
            .checked_add(c.len_utf16() as u32)
            .ok_or_else(|| Error::Invalid("Text too long".into()))?;
    }
    while markers.peek() == Some(&length) {
        markers.next();
    }
    if markers.next().is_some() {
        return Err(Error::Invalid(
            "Style range exceeds text or splits a Unicode scalar".into(),
        ));
    }
    Ok(length)
}
pub fn rich_edit(encoded: &str, replacement: &str, styles: &[InlineStyle]) -> Result<String> {
    let length = validate_styles(replacement, styles)?;
    let edited = edit(encoded, replacement)?;
    let doc = load(&edited)?;
    let text = doc.get_or_insert_text("content");
    {
        let mut txn = doc.transact_mut();
        if length > 0 {
            text.format(
                &mut txn,
                0,
                length,
                yrs::types::Attrs::from([
                    ("bold".into(), yrs::Any::Null),
                    ("italic".into(), yrs::Any::Null),
                    ("underline".into(), yrs::Any::Null),
                ]),
            );
        }
        for style in styles {
            let attrs = yrs::types::Attrs::from([
                ("bold".into(), style.bold.into()),
                ("italic".into(), style.italic.into()),
                ("underline".into(), style.underline.into()),
            ]);
            text.format(&mut txn, style.start_utf16, style.length_utf16, attrs);
        }
    }
    Ok(STANDARD.encode(
        doc.transact()
            .encode_state_as_update_v1(&StateVector::default()),
    ))
}
