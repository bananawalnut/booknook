// SPDX-License-Identifier: MIT OR Apache-2.0
//! Bounded, text-only rendering of unprotected legacy MOBI6.

use super::error;
use crate::{NativeError, Result};
use html5ever::{
    tendril::StrTendril,
    tokenizer::{
        BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, states::RawKind,
    },
};
use std::{borrow::Cow, cell::RefCell};

const MAX_FILE: usize = 32 * 1024 * 1024;
const MAX_TEXT: usize = 8 * 1024 * 1024;
const MAX_XHTML: usize = 16 * 1024 * 1024;
const MAX_RECORDS: usize = 8192;
const MAX_RECORD_TEXT: usize = 16 * 1024;
const MAX_METADATA: usize = 256 * 1024;
const MAX_DEPTH: usize = 64;
const MAX_TOKENS: usize = 512 * 1024;
const MAX_ATTRIBUTES: usize = 32;
const TOKEN_CHUNK: usize = 1024;
const MAX_TOKEN_BYTES: usize = 8 * 1024;
const PALM_HEADER: usize = 78;
const PALMDOC_HEADER: usize = 16;
const NO_INDEX: u32 = u32::MAX;

fn malformed() -> NativeError {
    error("The MOBI file is damaged or has invalid record data.")
}

fn unsupported() -> NativeError {
    error(
        "This Kindle file is outside the supported MOBI6 format. Choose a DRM-free MOBI6, EPUB or PDF.",
    )
}

fn protected() -> NativeError {
    error("This Kindle file is protected. The library entry and notes remain available.")
}

fn too_large() -> NativeError {
    error("This MOBI6 book exceeds the supported size or markup limits.")
}

fn word(data: &[u8], offset: usize) -> Result<u16> {
    let bytes = data
        .get(offset..)
        .and_then(<[u8]>::first_chunk)
        .ok_or_else(malformed)?;
    Ok(u16::from_be_bytes(*bytes))
}

fn number(data: &[u8], offset: usize) -> Result<u32> {
    let bytes = data
        .get(offset..)
        .and_then(<[u8]>::first_chunk)
        .ok_or_else(malformed)?;
    Ok(u32::from_be_bytes(*bytes))
}

struct Records<'a> {
    file: &'a [u8],
    count: usize,
}

impl<'a> Records<'a> {
    fn parse(file: &'a [u8]) -> Result<Self> {
        if file.len() > MAX_FILE {
            return Err(too_large());
        }
        if file.get(60..68) != Some(b"BOOKMOBI") || number(file, 72)? != 0 {
            return Err(unsupported());
        }
        let count = usize::from(word(file, 76)?);
        if count > MAX_RECORDS {
            return Err(too_large());
        }
        let table_end = PALM_HEADER + count * 8;
        if count < 2 || table_end >= file.len() {
            return Err(malformed());
        }
        let records = Self { file, count };
        let mut previous = table_end;
        for index in 0..count {
            let offset = records.offset(index)?;
            if offset < previous || offset > file.len() {
                return Err(malformed());
            }
            previous = offset;
        }
        Ok(records)
    }

    fn offset(&self, index: usize) -> Result<usize> {
        if index == self.count {
            return Ok(self.file.len());
        }
        Ok(number(self.file, PALM_HEADER + index * 8)? as usize)
    }

    fn get(&self, index: usize) -> Result<&'a [u8]> {
        self.file
            .get(self.offset(index)?..self.offset(index + 1)?)
            .ok_or_else(malformed)
    }
}

fn has_kf8_boundary(data: &[u8]) -> Result<bool> {
    if data.get(..4) != Some(b"EXTH") {
        return Err(malformed());
    }
    let size = number(data, 4)? as usize;
    let count = number(data, 8)? as usize;
    if size > MAX_METADATA || count > MAX_METADATA / 8 {
        return Err(too_large());
    }
    let mut entries = data.get(12..size).ok_or_else(malformed)?;
    for _ in 0..count {
        let kind = number(entries, 0)?;
        let size = number(entries, 4)? as usize;
        let entry = entries.get(8..size).ok_or_else(malformed)?;
        if kind == 121 {
            // EXTH 121 points at the second, KF8 header in a hybrid book.
            if entry.len() != 4 {
                return Err(malformed());
            }
            if number(entry, 0)? != NO_INDEX {
                return Ok(true);
            }
        }
        entries = &entries[size..];
    }
    Ok(false)
}

fn without_trailers(mut record: &[u8], flags: u16) -> Result<&[u8]> {
    // The higher bits describe length-coded trailers. The low bit describes
    // the copies of UTF-8 continuation bytes taken from the next record.
    for _ in 0..(flags >> 1).count_ones() {
        let mut length = 0;
        let mut terminated = false;
        for (index, &byte) in record.iter().rev().take(4).enumerate() {
            length |= usize::from(byte & 0x7f) << (7 * index);
            if byte & 0x80 != 0 {
                if length <= index {
                    return Err(malformed());
                }
                terminated = true;
                break;
            }
        }
        if !terminated || length > record.len() {
            return Err(malformed());
        }
        record = &record[..record.len() - length];
    }
    if flags & 1 != 0 {
        let last = record.last().ok_or_else(malformed)?;
        let length = usize::from(last & 3) + 1;
        record = record
            .get(..record.len().checked_sub(length).ok_or_else(malformed)?)
            .ok_or_else(malformed)?;
    }
    Ok(record)
}

fn append(data: &mut Vec<u8>, bytes: &[u8], limit: usize) -> Result<()> {
    if bytes.len() > limit - data.len() {
        return Err(malformed());
    }
    data.extend_from_slice(bytes);
    Ok(())
}

fn decompress_palmdoc(mut input: &[u8], output: &mut Vec<u8>, limit: usize) -> Result<()> {
    let record_start = output.len();
    while let Some((&byte, rest)) = input.split_first() {
        input = rest;
        match byte {
            1..=8 => {
                let count = usize::from(byte);
                let (literal, rest) = input.split_at_checked(count).ok_or_else(malformed)?;
                append(output, literal, limit)?;
                input = rest;
            }
            0x80..=0xbf => {
                let (&low, rest) = input.split_first().ok_or_else(malformed)?;
                input = rest;
                let pair = usize::from(u16::from_be_bytes([byte, low]));
                let distance = (pair >> 3) & 0x7ff;
                let count = (pair & 7) + 3;
                if distance == 0
                    || distance > output.len() - record_start
                    || count > limit - output.len()
                {
                    return Err(malformed());
                }
                for _ in 0..count {
                    // A run may refer to bytes appended by the same run.
                    output.push(output[output.len() - distance]);
                }
            }
            0xc0..=0xff => append(output, &[b' ', byte ^ 0x80], limit)?,
            _ => append(output, &[byte], limit)?,
        }
    }
    Ok(())
}

fn markup(bytes: &[u8]) -> Result<(Vec<u8>, u32)> {
    let records = Records::parse(bytes)?;
    let metadata = records.get(0)?;
    if word(metadata, 12)? != 0 {
        return Err(protected());
    }
    let header = metadata.get(PALMDOC_HEADER..).ok_or_else(malformed)?;
    if header.get(..4) != Some(b"MOBI") {
        return Err(unsupported());
    }
    let header_length = number(header, 4)? as usize;
    let mob = header.get(..header_length).ok_or_else(malformed)?;
    if mob.len() < 0x74 || number(mob, 0x14)? != 6 || number(mob, 8)? != 2 {
        return Err(unsupported());
    }
    if ![0, NO_INDEX].contains(&number(mob, 0x18)?) || ![0, NO_INDEX].contains(&number(mob, 0x1c)?)
    {
        return Err(unsupported());
    }
    if mob.len() >= 0xa4 {
        let offset = number(mob, 0x94)?;
        let count = number(mob, 0x98)?;
        if ![0, NO_INDEX].contains(&offset)
            || ![0, NO_INDEX].contains(&count)
            || number(mob, 0x9c)? != 0
            || number(mob, 0xa0)? != 0
        {
            return Err(protected());
        }
    }
    if number(mob, 0x70)? & 0x40 != 0 && has_kf8_boundary(&header[header_length..])? {
        return Err(unsupported());
    }
    let encoding = number(mob, 0x0c)?;
    let compression = word(metadata, 0)?;
    let flags = if mob.len() >= 0xe4 {
        word(mob, 0xe2)?
    } else {
        0
    };
    if ![65001, 1252].contains(&encoding) || ![1, 2].contains(&compression) || flags & !7 != 0 {
        return Err(unsupported());
    }
    let declared_text = number(metadata, 4)? as usize;
    let text_records = usize::from(word(metadata, 8)?);
    let record_size = usize::from(word(metadata, 10)?);
    if declared_text > MAX_TEXT || record_size > MAX_RECORD_TEXT {
        return Err(too_large());
    }
    if declared_text == 0
        || record_size == 0
        || text_records == 0
        || text_records >= records.count
        || declared_text > record_size * text_records
    {
        return Err(malformed());
    }
    for index in text_records + 1..records.count {
        if records.get(index)? == b"BOUNDARY" {
            return Err(unsupported());
        }
    }
    let mut result = Vec::with_capacity(declared_text);
    for index in 1..=text_records {
        let record = without_trailers(records.get(index)?, flags)?;
        let limit = declared_text.min(result.len() + record_size);
        if compression == 2 {
            decompress_palmdoc(record, &mut result, limit)?;
        } else {
            append(&mut result, record, limit)?;
        }
    }
    if result.len() != declared_text {
        return Err(malformed());
    }
    if result.starts_with(b"%MOP") {
        return Err(unsupported());
    }
    Ok((result, encoding))
}

fn decode_markup(data: &[u8], encoding: u32) -> Result<Cow<'_, str>> {
    if encoding == 65001 {
        return std::str::from_utf8(data)
            .map(Cow::Borrowed)
            .map_err(|_| malformed());
    }
    // WHATWG indexes 0x80 through 0x9f for Windows-1252, including its
    // reserved control values. The remaining byte values map directly.
    const WINDOWS_1252: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž',
        '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}',
        'ž', 'Ÿ',
    ];
    let mut result = String::with_capacity(data.len());
    for &byte in data {
        let ch = if (0x80..=0x9f).contains(&byte) {
            WINDOWS_1252[usize::from(byte - 0x80)]
        } else {
            char::from(byte)
        };
        if result.len() + ch.len_utf8() > MAX_TEXT {
            return Err(too_large());
        }
        result.push(ch);
    }
    Ok(Cow::Owned(result))
}

fn allowed(name: &str) -> Option<&'static str> {
    Some(match name {
        "p" => "p",
        "div" => "div",
        "h1" => "h1",
        "h2" => "h2",
        "h3" => "h3",
        "h4" => "h4",
        "h5" => "h5",
        "h6" => "h6",
        "blockquote" => "blockquote",
        "pre" => "pre",
        "ul" => "ul",
        "ol" => "ol",
        "li" => "li",
        "strong" | "b" => "strong",
        "em" | "i" => "em",
        "sup" => "sup",
        "sub" => "sub",
        _ => return None,
    })
}

fn discarded(name: &str) -> Option<(&'static str, Option<RawKind>)> {
    Some(match name {
        "script" => ("script", Some(RawKind::ScriptData)),
        "style" => ("style", Some(RawKind::Rawtext)),
        "title" => ("title", Some(RawKind::Rcdata)),
        "textarea" => ("textarea", Some(RawKind::Rcdata)),
        "iframe" => ("iframe", Some(RawKind::Rawtext)),
        "xmp" => ("xmp", Some(RawKind::Rawtext)),
        "noembed" => ("noembed", Some(RawKind::Rawtext)),
        "noframes" => ("noframes", Some(RawKind::Rawtext)),
        "head" => ("head", None),
        "svg" => ("svg", None),
        "math" => ("math", None),
        "object" => ("object", None),
        "template" => ("template", None),
        "form" => ("form", None),
        _ => return None,
    })
}

#[derive(Default)]
struct Xhtml {
    output: String,
    open: Vec<&'static str>,
    hidden: Vec<&'static str>,
    tokens: usize,
    completed_tokens: usize,
    limited: bool,
    readable: bool,
}

impl Xhtml {
    fn write(&mut self, text: &str) {
        if self.output.len() + text.len() > MAX_XHTML {
            self.limited = true;
        } else if !self.limited {
            self.output.push_str(text);
        }
    }

    fn tag(&mut self, tag: &str, close: bool) {
        self.write(if close { "</" } else { "<" });
        self.write(tag);
        self.write(">");
    }

    fn text(&mut self, text: &str) {
        for ch in text.chars() {
            if self.limited {
                break;
            }
            let mut buffer = [0; 4];
            let escaped = match ch {
                '&' => "&amp;",
                '<' => "&lt;",
                '>' => "&gt;",
                '\t'
                | '\n'
                | '\r'
                | '\u{20}'..='\u{d7ff}'
                | '\u{e000}'..='\u{fffd}'
                | '\u{10000}'..='\u{10ffff}' => ch.encode_utf8(&mut buffer),
                _ => continue,
            };
            self.readable |= !ch.is_whitespace();
            self.write(escaped);
        }
    }

    fn close_from(&mut self, index: usize) {
        while self.open.len() > index {
            if let Some(tag) = self.open.pop() {
                self.tag(tag, true);
            }
        }
    }

    // The supported block tags follow HTML's optional paragraph and list-item
    // end tags. The nearest list boundary keeps a nested item's open tag from
    // closing an item in its parent list.
    fn close_implied(&mut self, name: &str) {
        if name == "li"
            && let Some(index) = self
                .open
                .iter()
                .rposition(|tag| matches!(*tag, "li" | "ol" | "ul"))
            && self.open[index] == "li"
        {
            self.close_from(index);
        }
        if matches!(
            name,
            "p" | "div"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "blockquote"
                | "pre"
                | "ul"
                | "ol"
                | "li"
                | "hr"
                | "mbp:pagebreak"
        ) && let Some(index) = self.open.iter().rposition(|tag| *tag == "p")
        {
            self.close_from(index);
        }
    }

    fn opening(&mut self, name: &str, self_closing: bool) -> TokenSinkResult<()> {
        if let Some((hidden, raw)) = discarded(name) {
            // Only foreign elements honor this flag here. Raw-text HTML
            // elements must wait for a closing tag, even after a slash.
            if !(self_closing && matches!(hidden, "svg" | "math")) {
                if self.hidden.len() == MAX_DEPTH {
                    self.limited = true;
                } else {
                    self.hidden.push(hidden);
                }
                return raw.map_or(TokenSinkResult::Continue, TokenSinkResult::RawData);
            }
        }
        if self.hidden.is_empty() {
            self.close_implied(name);
            match name {
                "br" => self.write("<br/>"),
                "hr" | "mbp:pagebreak" => self.write("<hr/>"),
                _ => {
                    if let Some(tag) = allowed(name) {
                        if self.open.len() == MAX_DEPTH {
                            self.limited = true;
                        } else {
                            self.tag(tag, false);
                            self.open.push(tag);
                        }
                    }
                }
            }
        }
        TokenSinkResult::Continue
    }
}

struct XhtmlSink(RefCell<Xhtml>);

impl TokenSink for XhtmlSink {
    type Handle = ();

    fn process_token(&self, token: Token, _line_number: u64) -> TokenSinkResult<Self::Handle> {
        let mut doc = self.0.borrow_mut();
        doc.tokens += 1;
        if !matches!(&token, Token::ParseError(_)) {
            doc.completed_tokens += 1;
        }
        doc.limited |= doc.tokens > MAX_TOKENS
            || matches!(&token, Token::TagToken(tag) if tag.attrs.len() > MAX_ATTRIBUTES);
        if doc.limited {
            return TokenSinkResult::Continue;
        }
        match token {
            Token::TagToken(tag) if tag.kind == TagKind::StartTag => {
                return doc.opening(tag.name.as_ref(), tag.self_closing);
            }
            Token::TagToken(tag) => {
                let name = tag.name.as_ref();
                if let Some(hidden) = doc.hidden.last() {
                    if *hidden == name {
                        doc.hidden.pop();
                    }
                } else if let Some(name) = allowed(name)
                    && let Some(index) = doc.open.iter().rposition(|tag| *tag == name)
                {
                    doc.close_from(index);
                }
            }
            Token::CharacterTokens(text) if doc.hidden.is_empty() => doc.text(&text),
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

fn render_xhtml(text: &str) -> Result<Vec<u8>> {
    let mut document = Xhtml::default();
    document.write(concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
        "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>Kindle book</title></head><body>"
    ));
    let queue = BufferQueue::default();
    let tokenizer = Tokenizer::new(XhtmlSink(RefCell::new(document)), Default::default());
    let mut start = 0;
    let mut pending = 0;
    let mut completed = 0;
    while start < text.len() {
        let mut end = (start + TOKEN_CHUNK).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        queue.push_back(StrTendril::from_slice(&text[start..end]));
        let _ = tokenizer.feed(&queue);
        let state = tokenizer.sink.0.borrow();
        // The tokenizer keeps attributes until a tag completes. Bound the
        // bytes fed without a completed token so a single tag cannot grow
        // an unbounded attribute list. Parse errors do not count as progress.
        if completed == state.completed_tokens {
            pending += end - start;
        } else {
            completed = state.completed_tokens;
            pending = 0;
        }
        if state.limited || pending > MAX_TOKEN_BYTES {
            return Err(too_large());
        }
        start = end;
    }
    tokenizer.end();
    let mut document = tokenizer.sink.0.into_inner();
    document.close_from(0);
    document.write("</body></html>");
    if document.limited {
        return Err(too_large());
    }
    if !document.readable {
        return Err(error("This MOBI6 file contains no readable text."));
    }
    Ok(document.output.into_bytes())
}

pub(super) fn render_mobi6(bytes: &[u8]) -> Result<Vec<u8>> {
    let (data, encoding) = markup(bytes)?;
    render_xhtml(&decode_markup(&data, encoding)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECORD0: usize = PALM_HEADER + 16;

    fn book(markup: &[u8], codepage: u32) -> Vec<u8> {
        let mut file = vec![0; RECORD0 + 16 + 0xe8];
        let start = file.len() as u32;
        file[60..68].copy_from_slice(b"BOOKMOBI");
        file[76..78].copy_from_slice(&2u16.to_be_bytes());
        file[78..82].copy_from_slice(&(RECORD0 as u32).to_be_bytes());
        file[86..90].copy_from_slice(&start.to_be_bytes());
        file[RECORD0..RECORD0 + 2].copy_from_slice(&1u16.to_be_bytes());
        file[RECORD0 + 4..RECORD0 + 8].copy_from_slice(&(markup.len() as u32).to_be_bytes());
        file[RECORD0 + 8..RECORD0 + 10].copy_from_slice(&1u16.to_be_bytes());
        file[RECORD0 + 10..RECORD0 + 12].copy_from_slice(&4096u16.to_be_bytes());
        let header = RECORD0 + PALMDOC_HEADER;
        file[header..header + 4].copy_from_slice(b"MOBI");
        file[header + 4..header + 8].copy_from_slice(&0xe8u32.to_be_bytes());
        file[header + 8..header + 12].copy_from_slice(&2u32.to_be_bytes());
        file[header + 12..header + 16].copy_from_slice(&codepage.to_be_bytes());
        file[header + 20..header + 24].copy_from_slice(&6u32.to_be_bytes());
        file.extend_from_slice(markup);
        file
    }

    fn message(file: &[u8]) -> String {
        let NativeError::Failure { message } = render_mobi6(file).unwrap_err();
        message
    }

    #[test]
    fn palmdoc_literals_space_and_overlapping_copy() {
        let mut result = Vec::new();
        decompress_palmdoc(b"\x03abc\x80\x1b\xc1\0", &mut result, 12).unwrap();
        assert_eq!(result, b"abcabcabc A\0");
        assert!(decompress_palmdoc(b"\x80\x08", &mut result, 20).is_err());
        assert!(decompress_palmdoc(b"\x02x", &mut result, 20).is_err());
        assert!(decompress_palmdoc(b"\xc1", &mut result, 12).is_err());
    }

    #[test]
    fn strips_known_multibyte_and_index_trailers() {
        assert_eq!(
            without_trailers(b"text\x80\x82\x02\x81", 3).unwrap(),
            b"text"
        );
        assert!(without_trailers(b"\x03", 1).is_err());
        assert!(without_trailers(b"\x7f\x7f", 2).is_err());
        assert!(without_trailers(b"\x85", 2).is_err());
    }

    #[test]
    fn binary_version_encryption_and_offsets_fail_closed() {
        let mut protected_book = book(b"<p>readable</p>", 65001);
        protected_book[RECORD0 + 13] = 2;
        assert!(message(&protected_book).contains("protected"));
        let mut drm = book(b"<p>readable</p>", 65001);
        drm[RECORD0 + PALMDOC_HEADER + 0x9b] = 1;
        assert!(message(&drm).contains("protected"));
        let mut kf8 = book(b"<p>readable</p>", 65001);
        kf8[RECORD0 + PALMDOC_HEADER + 0x17] = 8;
        assert!(message(&kf8).contains("outside"));
        let mut damaged = book(b"<p>readable</p>", 65001);
        damaged[86..90].copy_from_slice(&0u32.to_be_bytes());
        assert!(message(&damaged).contains("damaged"));
    }

    #[test]
    fn truncated_files_and_declared_expansion_reject() {
        let original = book(b"<p>readable</p>", 65001);
        for end in 0..original.len() {
            assert!(render_mobi6(&original[..end]).is_err(), "prefix {end}");
        }
        let mut huge = original;
        huge[RECORD0 + 4..RECORD0 + 8].copy_from_slice(&((MAX_TEXT + 1) as u32).to_be_bytes());
        assert!(message(&huge).contains("limits"));
    }

    #[test]
    fn renders_unicode_with_a_fixed_xhtml_vocabulary() {
        let raw = book(
            b"<head><title>hidden</title></head><h1>Book &amp; cover</h1><p onclick='x'>Caf\xe9 \x80 <b>yes</b><br> <a href='file:/private/read'>link</a></p><script><p>evil</p></script><style>evil</style><svg><text>evil</text></svg>",
            1252,
        );
        let first = render_mobi6(&raw).unwrap();
        assert_eq!(first, render_mobi6(&raw).unwrap());
        let text = std::str::from_utf8(&first).unwrap();
        assert!(text.contains("<h1>Book &amp; cover</h1>"));
        assert!(text.contains("<p>Café € <strong>yes</strong><br/> link</p>"));
        assert!(
            !["hidden", "evil", "onclick", "href", "file:/"]
                .iter()
                .any(|bad| text.contains(bad))
        );
    }

    #[test]
    fn omitted_paragraph_end_tags_keep_shallow_siblings() {
        let count = MAX_DEPTH + 2;
        let markup: String = (0..count)
            .map(|index| format!("<p>Paragraph {index}"))
            .collect();
        let raw = book(markup.as_bytes(), 65001);
        let first = render_mobi6(&raw).unwrap();
        assert_eq!(first, render_mobi6(&raw).unwrap());
        let parsed = roxmltree::Document::parse(std::str::from_utf8(&first).unwrap()).unwrap();
        let body = parsed
            .descendants()
            .find(|node| node.has_tag_name("body"))
            .unwrap();
        let paragraphs: Vec<_> = body
            .children()
            .filter(|node| node.has_tag_name("p"))
            .collect();
        assert_eq!(paragraphs.len(), count);
        for (index, paragraph) in paragraphs.iter().enumerate() {
            assert_eq!(
                paragraph.text(),
                Some(format!("Paragraph {index}").as_str())
            );
        }
    }

    #[test]
    fn omitted_list_item_end_tags_preserve_nested_list_scope() {
        let markup =
            b"<ol><li>First<p>Details<li>Second<ol><li>Nested one<li>Nested two</ol><li>Last</ol>";
        let generated = render_mobi6(&book(markup, 65001)).unwrap();
        let xhtml = std::str::from_utf8(&generated).unwrap();
        assert!(
            xhtml.contains(
                "<ol><li>First<p>Details</p></li><li>Second<ol><li>Nested one</li><li>Nested two</li></ol></li><li>Last</li></ol>"
            ),
            "{xhtml}"
        );
    }

    #[test]
    fn genuine_deep_nesting_remains_bounded() {
        let markup =
            |depth: usize| format!("{}text{}", "<div>".repeat(depth), "</div>".repeat(depth));
        assert!(render_mobi6(&book(markup(MAX_DEPTH).as_bytes(), 65001)).is_ok());
        assert!(message(&book(markup(MAX_DEPTH + 1).as_bytes(), 65001)).contains("limits"));
    }

    #[test]
    fn bounds_incomplete_tags_and_attribute_count_without_cutting_unicode_text() {
        let endless_tag = format!("<p data='{}'>book</p>", "x".repeat(MAX_TOKEN_BYTES * 2));
        assert!(render_xhtml(&endless_tag).is_err());
        let mut attributes = String::from("<p");
        for index in 0..=MAX_ATTRIBUTES {
            attributes.push_str(&format!(" data-{index}='x'"));
        }
        attributes.push_str(">book</p>");
        assert!(render_xhtml(&attributes).is_err());
        let readable = format!("<p>{}</p>", "🌿é".repeat(MAX_TOKEN_BYTES));
        let generated = String::from_utf8(render_xhtml(&readable).unwrap()).unwrap();
        assert!(generated.contains(&readable));
    }
}
