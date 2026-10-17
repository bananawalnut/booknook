use crate::{Error, Result, digest, model::*};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

fn id(provider: &str, source: &str) -> String {
    format!("{provider}:{}", digest(source.as_bytes()))
}
fn clean_isbn(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('=')
        .trim_matches('"')
        .replace('-', "")
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ImportReport {
    pub inserted: usize,
    pub updated: usize,
    pub unchanged: usize,
}

fn upsert(
    state: &mut State,
    mut book: Book,
    edition: Edition,
    report: &mut ImportReport,
) -> Result<()> {
    let incoming = book
        .provenance
        .first()
        .ok_or_else(|| Error::Invalid("missing provenance".into()))?;
    if let Some(old) = state.books.get_mut(&book.id) {
        if old.provenance.iter().any(|p| {
            p.digest == incoming.digest
                && p.source == incoming.source
                && p.provider_connection_id == incoming.provider_connection_id
        }) {
            report.unchanged += 1;
            return Ok(());
        }
        if !old.edited_fields.contains("title") {
            old.title = book.title;
        }
        if !old.edited_fields.contains("shelf") {
            old.shelf = book.shelf;
        }
        if !old.edited_fields.contains("rating") {
            old.rating = book.rating;
        }
        if !old.edited_fields.contains("review") {
            old.review = book.review;
        }
        old.provenance.append(&mut book.provenance);
        old.reading_dates.extend(book.reading_dates);
        old.reading_dates.sort();
        old.reading_dates.dedup();
        report.updated += 1;
    } else {
        state.editions.insert(edition.id.clone(), edition);
        state.books.insert(book.id.clone(), book);
        report.inserted += 1;
    }
    Ok(())
}

pub fn goodreads(state: &mut State, input: &str, at: u64) -> Result<ImportReport> {
    goodreads_from_connection(state, input, at, None)
}

pub fn goodreads_from_connection(
    state: &mut State,
    input: &str,
    at: u64,
    connection_id: Option<&str>,
) -> Result<ImportReport> {
    if connection_id.is_some_and(|id| !crate::castalia_connections::valid_reference(id)) {
        return Err(Error::Invalid(
            "Invalid provider connection reference".into(),
        ));
    }
    if input.len() > 32 * 1024 * 1024 {
        return Err(Error::Invalid("CSV exceeds 32 MiB".into()));
    }
    let mut reader = csv::ReaderBuilder::new()
        .flexible(false)
        .from_reader(input.trim_start_matches('\u{feff}').as_bytes());
    let headers = reader
        .headers()
        .map_err(|e| Error::Invalid(e.to_string()))?
        .clone();
    for required in ["Book Id", "Title", "Author"] {
        if !headers.iter().any(|h| h == required) {
            return Err(Error::Invalid(format!(
                "Missing Goodreads column: {required}"
            )));
        }
    }
    let mut staged = state.clone();
    let mut report = ImportReport::default();
    for row in reader.records() {
        let row = row.map_err(|e| Error::Invalid(e.to_string()))?;
        let fields: BTreeMap<String, String> = headers
            .iter()
            .zip(row.iter())
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        let get = |key: &str| fields.get(key).cloned().unwrap_or_default();
        let source = get("Book Id");
        if source.is_empty() || get("Title").trim().is_empty() {
            return Err(Error::Invalid("Book Id and Title cannot be empty".into()));
        }
        let book_id = id("goodreads", &source);
        let edition_id = format!("{book_id}:edition");
        let rating = match get("My Rating").as_str() {
            "" | "0" => None,
            s => Some(
                s.parse::<u8>()
                    .ok()
                    .filter(|r| (1..=5).contains(r))
                    .ok_or_else(|| Error::Invalid(format!("Invalid rating for {source}")))?,
            ),
        };
        let mut identifiers = vec![SourceRef {
            provider: "goodreads".into(),
            id: source.clone(),
        }];
        for key in ["ISBN", "ISBN13"] {
            let isbn = clean_isbn(&get(key));
            if !isbn.is_empty() {
                identifiers.push(SourceRef {
                    provider: key.to_lowercase(),
                    id: isbn,
                });
            }
        }
        let evidence = ImportEvidence {
            provider_connection_id: connection_id.map(str::to_owned),
            source: identifiers[0].clone(),
            imported_at: at,
            digest: digest(&serde_json::to_vec(&fields)?),
            fields: fields.clone(),
        };
        let book = Book {
            id: book_id.clone(),
            title: get("Title"),
            authors: vec![get("Author")],
            subjects: vec![],
            edition_ids: vec![edition_id.clone()],
            sources: vec![identifiers[0].clone()],
            shelf: get("Exclusive Shelf"),
            rating,
            review: get("My Review"),
            reading_dates: [get("Date Read")]
                .into_iter()
                .filter(|d| !d.is_empty())
                .collect(),
            edited_fields: BTreeSet::new(),
            provenance: vec![evidence],
        };
        upsert(
            &mut staged,
            book,
            Edition {
                id: edition_id,
                book_id,
                identifiers,
                language: None,
                assets: vec![],
            },
            &mut report,
        )?;
    }
    *state = staged;
    Ok(report)
}

/// Normalized connector interchange, deliberately distinct from an authenticated Kindle API.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KindleCatalog {
    pub schema: String,
    pub items: Vec<KindleItem>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KindleItem {
    pub asin: String,
    pub title: String,
    pub authors: Vec<String>,
    pub position: Option<String>,
    pub observed_at: Option<u64>,
}

pub fn kindle(state: &mut State, input: &str, at: u64) -> Result<ImportReport> {
    kindle_from_connection(state, input, at, None)
}

pub fn kindle_from_connection(
    state: &mut State,
    input: &str,
    at: u64,
    connection_id: Option<&str>,
) -> Result<ImportReport> {
    if connection_id.is_some_and(|id| !crate::castalia_connections::valid_reference(id)) {
        return Err(Error::Invalid(
            "Invalid provider connection reference".into(),
        ));
    }
    let catalog: KindleCatalog = crate::strict_parse(input.as_bytes())?;
    if catalog.schema != "booknook.kindle-catalog.v1" {
        return Err(Error::Invalid("Unsupported Kindle catalog schema".into()));
    }
    let mut staged = state.clone();
    let mut report = ImportReport::default();
    for item in catalog.items {
        if item.asin.is_empty() || item.title.trim().is_empty() {
            return Err(Error::Invalid("Kindle item requires ASIN and title".into()));
        }
        let book_id = id("kindle", &item.asin);
        let edition_id = format!("{book_id}:edition");
        let source = SourceRef {
            provider: "kindle".into(),
            id: item.asin.clone(),
        };
        let raw = serde_json::to_string(&item)?;
        let book = Book {
            id: book_id.clone(),
            title: item.title,
            authors: item.authors,
            subjects: vec![],
            edition_ids: vec![edition_id.clone()],
            sources: vec![source.clone()],
            shelf: "to-read".into(),
            rating: None,
            review: String::new(),
            reading_dates: vec![],
            edited_fields: BTreeSet::new(),
            provenance: vec![ImportEvidence {
                provider_connection_id: connection_id.map(str::to_owned),
                source: source.clone(),
                imported_at: at,
                digest: digest(raw.as_bytes()),
                fields: BTreeMap::from([("raw".into(), raw)]),
            }],
        };
        upsert(
            &mut staged,
            book,
            Edition {
                id: edition_id,
                book_id: book_id.clone(),
                identifiers: vec![source],
                language: None,
                assets: vec![],
            },
            &mut report,
        )?;
        if let (Some(value), Some(observed_at)) = (item.position, item.observed_at) {
            let pass_id = crate::ensure_notebook(&mut staged, &book_id, at)?.pass_id;
            let source_identity = match connection_id {
                Some(connection) => {
                    serde_json::to_string(&(connection, &item.asin, observed_at, &value))?
                }
                None => format!("{}:{observed_at}:{value}", item.asin),
            };
            let position = ProgressObservation {
                provider_connection_id: connection_id.map(str::to_owned),
                id: id("kindle-position", &source_identity),
                pass_id,
                location: Location::Provider {
                    provider: "kindle".into(),
                    source_id: item.asin,
                    value,
                },
                observed_at,
                received_at: at,
                alignment: Alignment::Unmatched,
                audio_metadata: None,
            };
            staged
                .positions
                .entry(position.id.clone())
                .or_insert(position);
        }
    }
    *state = staged;
    Ok(report)
}

/// Source-neutral metadata ingestion. Never overwrites personal reading state or assets.
pub(crate) fn normalized(
    state: &mut State,
    namespace: &str,
    entries: &[crate::provider_runtime::SourceCatalogEntry],
    at: u64,
    connection_id: Option<&str>,
) -> Result<ImportReport> {
    let mut staged = state.clone();
    let mut report = ImportReport::default();
    for entry in entries {
        let book_id = format!(
            "source:{}",
            crate::digest(&crate::canonical(&(namespace, &entry.source_id))?)
        );
        let edition_id = format!("{book_id}:edition");
        let source = SourceRef {
            provider: namespace.into(),
            id: entry.source_id.clone(),
        };
        let raw = String::from_utf8(crate::canonical(entry)?)
            .map_err(|_| Error::Invalid("Invalid catalog encoding".into()))?;
        let evidence = ImportEvidence {
            provider_connection_id: connection_id.map(str::to_owned),
            source: source.clone(),
            imported_at: at,
            digest: crate::digest(raw.as_bytes()),
            fields: BTreeMap::from([("raw".into(), raw)]),
        };
        if let Some(old) = staged.books.get_mut(&book_id) {
            if old.provenance.iter().any(|p| {
                p.digest == evidence.digest
                    && p.source == evidence.source
                    && p.provider_connection_id == evidence.provider_connection_id
            }) {
                report.unchanged += 1;
                continue;
            }
            if !old.edited_fields.contains("title") {
                old.title = entry.title.clone();
            }
            if !old.edited_fields.contains("authors") {
                old.authors = entry.authors.clone();
            }
            if !old.edited_fields.contains("subjects") {
                old.subjects = entry.subjects.clone();
            }
            old.provenance.push(evidence);
            report.updated += 1;
        } else {
            staged.books.insert(
                book_id.clone(),
                Book {
                    id: book_id.clone(),
                    title: entry.title.clone(),
                    authors: entry.authors.clone(),
                    subjects: entry.subjects.clone(),
                    edition_ids: vec![edition_id.clone()],
                    sources: vec![source.clone()],
                    shelf: "to-read".into(),
                    rating: None,
                    review: String::new(),
                    reading_dates: vec![],
                    edited_fields: BTreeSet::new(),
                    provenance: vec![evidence],
                },
            );
            report.inserted += 1;
        }
        let mut identifiers = vec![source];
        for identifier in &entry.edition_identifiers {
            if !identifiers.contains(identifier) {
                identifiers.push(identifier.clone());
            }
        }
        staged
            .editions
            .entry(edition_id.clone())
            .and_modify(|e| {
                e.identifiers = identifiers.clone();
                e.language = entry.language.clone();
            })
            .or_insert(Edition {
                id: edition_id,
                book_id,
                identifiers,
                language: entry.language.clone(),
                assets: vec![],
            });
    }
    *state = staged;
    Ok(report)
}
