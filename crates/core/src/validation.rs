use crate::{Error, Result, model::*};
use serde::{
    Deserialize,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::Value;
use std::{collections::BTreeSet, fmt};

/// Reject duplicate object keys before typed deserialization (serde_json alone keeps the last).
struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Unique;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("JSON with unique keys")
            }
            fn visit_map<M: MapAccess<'de>>(
                self,
                mut map: M,
            ) -> std::result::Result<Unique, M::Error> {
                let mut out = serde_json::Map::new();
                while let Some((k, v)) = map.next_entry::<String, Unique>()? {
                    if out.insert(k, v.0).is_some() {
                        return Err(de::Error::custom("duplicate key"));
                    }
                }
                Ok(Unique(Value::Object(out)))
            }
            fn visit_seq<S: SeqAccess<'de>>(
                self,
                mut seq: S,
            ) -> std::result::Result<Unique, S::Error> {
                let mut a = vec![];
                while let Some(v) = seq.next_element::<Unique>()? {
                    a.push(v.0)
                }
                Ok(Unique(Value::Array(a)))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::String(v.into())))
            }
            fn visit_string<E: de::Error>(self, v: String) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::String(v)))
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::Bool(v)))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::from(v)))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::from(v)))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<Unique, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| Unique(Value::Number(n)))
                    .ok_or_else(|| de::Error::custom("nonfinite"))
            }
            fn visit_unit<E: de::Error>(self) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
        }
        d.deserialize_any(V)
    }
}
pub fn strict_parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    if bytes.len() > MAX_JSON_BYTES {
        return Err(Error::Invalid("JSON exceeds 128 MiB".into()));
    }
    let value: Unique = serde_json::from_slice(bytes)?;
    Ok(serde_json::from_value(value.0)?)
}
pub const MAX_JSON_BYTES: usize = 128 * 1024 * 1024;
fn require(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error::Invalid(message.into()))
    }
}
pub fn validate(state: &State) -> Result<()> {
    require(
        (state.schema == SCHEMA && state.ontology_version == ONTOLOGY)
            || (state.schema == INK_ARCHIVE_SCHEMA && state.ontology_version == INK_ONTOLOGY)
            || (state.schema == NOTE_ARCHIVE_SCHEMA && state.ontology_version == NOTE_ONTOLOGY),
        "Unsupported archive/ontology version",
    )?;
    require(!state.local_reader.is_empty(), "Missing local attribution")?;
    for (id, b) in &state.books {
        require(id == &b.id && !b.title.trim().is_empty(), "Invalid book")?;
        require(
            b.rating.is_none_or(|r| (1..=5).contains(&r)),
            "Invalid rating",
        )?;
        for evidence in &b.provenance {
            require(
                evidence
                    .provider_connection_id
                    .as_deref()
                    .is_none_or(crate::castalia_connections::valid_reference),
                "Invalid import connection reference",
            )?;
        }
        for e in &b.edition_ids {
            require(
                state.editions.get(e).is_some_and(|e| &e.book_id == id),
                "Missing/mismatched edition",
            )?;
        }
    }
    for (id, e) in &state.editions {
        require(
            id == &e.id
                && state
                    .books
                    .get(&e.book_id)
                    .is_some_and(|b| b.edition_ids.contains(id)),
            "Invalid edition/book link",
        )?;
        for a in &e.assets {
            require(
                a.sha256.len() == 64
                    && a.sha256
                        .bytes()
                        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
                    && a.storage_key == a.sha256,
                "Invalid asset key",
            )?;
        }
    }
    for (id, p) in &state.passes {
        require(
            id == &p.id
                && !p.id.is_empty()
                && !p.reader.is_empty()
                && state.books.contains_key(&p.book_id)
                && p.finished_at.is_none_or(|end| end >= p.started_at),
            "Invalid reading pass",
        )?;
    }
    for (id, s) in &state.sessions {
        require(
            id == &s.id
                && !s.id.is_empty()
                && state.passes.get(&s.pass_id).is_some_and(|pass| {
                    s.started_at >= pass.started_at
                        && s.ended_at.is_none_or(|end| end >= s.started_at)
                        && pass.finished_at.is_none_or(|end| {
                            s.started_at <= end && s.ended_at.is_none_or(|ended| ended <= end)
                        })
                }),
            "Invalid session",
        )?;
    }
    for (id, n) in &state.notebooks {
        require(
            id == &n.id
                && state
                    .passes
                    .get(&n.pass_id)
                    .is_some_and(|p| p.book_id == n.book_id),
            "Invalid notebook",
        )?;
        let mut ids = BTreeSet::new();
        for b in &n.blocks {
            require(ids.insert(&b.id) && !b.id.is_empty(), "Duplicate block ID")?;
            if let Some(ink) = &b.ink {
                require(
                    state.schema == INK_ARCHIVE_SCHEMA || state.schema == NOTE_ARCHIVE_SCHEMA,
                    "Notebook ink requires archive.v2 / ontology 0.2.0",
                )?;
                require(
                    state.schema == NOTE_ARCHIVE_SCHEMA
                        || ink.versions.iter().all(|v| v.canvas_space.is_none()),
                    "Whiteboard coordinates require archive.v3 / ontology 0.3.0",
                )?;
                crate::notebook_ink::validate(state, n, ink)?;
            }
            if let Some(note) = &b.note {
                require(
                    state.schema == NOTE_ARCHIVE_SCHEMA,
                    "Note metadata requires archive.v3 / ontology 0.3.0",
                )?;
                crate::notes::validate(state, n, note)?;
            }
            require(
                crate::notebook::text(&b.update)? == b.text
                    && crate::notebook::styles(&b.update)? == b.styles,
                "Notebook text disagrees with CRDT state",
            )?;
        }
        for r in &n.revisions {
            require(ids.contains(&r.block_id), "Revision has no block")?;
            crate::notebook::text(&r.update)?;
        }
    }
    for (id, e) in &state.entities {
        require(
            id == &e.id
                && !e.id.is_empty()
                && !e.name.trim().is_empty()
                && state.books.contains_key(&e.book_id),
            "Invalid literary entity",
        )?;
    }
    for (id, c) in &state.cards {
        require(id == &c.id && !c.versions.is_empty(), "Invalid card")?;
        let n = state
            .notebooks
            .get(&c.notebook_id)
            .ok_or_else(|| Error::Invalid("Card notebook missing".into()))?;
        require(
            n.blocks.iter().any(|b| b.id == c.block_id),
            "Card block missing",
        )?;
        require(
            state
                .entities
                .get(&c.character_id)
                .is_some_and(|e| e.kind == LiteraryKind::Character && e.book_id == n.book_id),
            "Character is not in this book",
        )?;
        let mut seen = BTreeSet::new();
        for v in &c.versions {
            require(
                v.pass_id == n.pass_id && !seen.contains(&v.id),
                "Invalid card version",
            )?;
            require(
                v.parents.iter().all(|p| seen.contains(p)),
                "Missing/cyclic card parents",
            )?;
            seen.insert(v.id.clone());
            for p in &v.positions {
                validate_book_location(state, p, &n.book_id)?;
            }
            for observation in &v.position_observations {
                require(
                    observation
                        .provider_connection_id
                        .as_deref()
                        .is_none_or(crate::castalia_connections::valid_reference),
                    "Invalid snapshot connection reference",
                )?;
                require(
                    observation.pass_id == v.pass_id,
                    "Snapshot position belongs to another reading pass",
                )?;
                validate_book_location(state, &observation.location, &n.book_id)?;
                if let Some(metadata) = &observation.audio_metadata {
                    require(
                        matches!(observation.location, Location::Audio { .. }),
                        "Audio metadata requires an audio location",
                    )?;
                    metadata.validate()?;
                }
            }
            for passage in &v.interpretation.passages {
                validate_book_location(state, passage, &n.book_id)?;
            }
            for relationship in &v.interpretation.relationships {
                require(
                    state
                        .entities
                        .get(&relationship.target)
                        .is_some_and(|entity| entity.book_id == n.book_id),
                    "Relationship target is not a literary entity in this book",
                )?;
            }
        }
    }
    for (id, a) in &state.annotations {
        require(
            id == &a.id && state.passes.contains_key(&a.pass_id),
            "Invalid annotation",
        )?;
        validate_book_location(state, &a.anchor, &state.passes[&a.pass_id].book_id)?;
        if let Some(geometry) = &a.ink_geometry {
            require(
                !geometry.content_fingerprint.is_empty()
                    && geometry.content_fingerprint.len() <= 4096
                    && [
                        geometry.width,
                        geometry.height,
                        geometry.origin_x,
                        geometry.origin_y,
                    ]
                    .iter()
                    .all(|n| n.is_finite())
                    && geometry.width > 0.0
                    && geometry.height > 0.0
                    && geometry.width <= 10_000_000.0
                    && geometry.height <= 10_000_000.0,
                "Invalid portable ink geometry",
            )?;
            require(
                geometry.text_rects.len() <= 100_000
                    && geometry.text_rects.iter().all(|r| {
                        [r.x, r.y, r.width, r.height].iter().all(|n| n.is_finite())
                            && r.width >= 0.0
                            && r.height >= 0.0
                    }),
                "Invalid ink anchor rectangles",
            )?;
            match (&geometry.space, &a.anchor) {
                (InkSpace::EpubDocument, Location::Text { resource, .. }) => require(
                    geometry.page.is_none() && &geometry.resource == resource,
                    "Ink resource mismatch",
                )?,
                (InkSpace::PdfPage, Location::Page { page, .. }) => {
                    require(geometry.page == Some(*page), "Ink page mismatch")?
                }
                _ => {
                    return Err(Error::Invalid(
                        "Ink coordinate space disagrees with anchor".into(),
                    ));
                }
            }
        }
        for stroke in &a.strokes {
            require(
                stroke.width.is_finite() && stroke.width > 0.0,
                "Invalid stroke width",
            )?;
            for p in &stroke.points {
                require(
                    [p.x, p.y, p.pressure, p.time].iter().all(|v| v.is_finite())
                        && (0.0..=1.0).contains(&p.pressure),
                    "Invalid stroke point",
                )?;
            }
        }
    }
    for (id, p) in &state.positions {
        require(
            p.provider_connection_id
                .as_deref()
                .is_none_or(crate::castalia_connections::valid_reference),
            "Invalid position connection reference",
        )?;
        require(
            id == &p.id && state.passes.contains_key(&p.pass_id),
            "Invalid position",
        )?;
        let book_id = &state.passes[&p.pass_id].book_id;
        validate_book_location(state, &p.location, book_id)?;
        if let Some(metadata) = &p.audio_metadata {
            require(
                matches!(p.location, Location::Audio { .. }),
                "Audio metadata requires an audio location",
            )?;
            metadata.validate()?;
        }
        if let Alignment::Candidate { edition_id, .. } | Alignment::Confirmed { edition_id, .. } =
            &p.alignment
        {
            require(
                state
                    .editions
                    .get(edition_id)
                    .is_some_and(|edition| &edition.book_id == book_id),
                "Position alignment references another book",
            )?;
        }
    }
    for (id, l) in &state.lists {
        require(
            id == &l.id && l.book_ids.iter().all(|id| state.books.contains_key(id)),
            "Invalid list",
        )?;
    }
    for (id, g) in &state.groups {
        require(
            id == &g.id && g.members.contains(&g.owner) && g.moderators.is_subset(&g.members),
            "Invalid group",
        )?;
    }
    for (id, a) in &state.activity {
        require(id == &a.id, "Invalid activity ID")?;
    }
    for (id, d) in &state.drafts {
        require(
            id == &d.id
                && d.book_id
                    .as_ref()
                    .is_none_or(|id| state.books.contains_key(id)),
            "Invalid draft",
        )?;
    }
    Ok(())
}
pub(crate) fn validate_book_location(
    state: &State,
    location: &Location,
    book_id: &str,
) -> Result<()> {
    validate_location(state, location)?;
    if let Location::Text { edition_id, .. } | Location::Page { edition_id, .. } = location {
        require(
            state
                .editions
                .get(edition_id)
                .is_some_and(|edition| edition.book_id == book_id),
            "Location references another book",
        )?;
    }
    Ok(())
}
fn validate_location(state: &State, l: &Location) -> Result<()> {
    match l {
        Location::Text {
            edition_id,
            resource,
            quote,
            ..
        } => require(
            state.editions.contains_key(edition_id) && !resource.is_empty() && !quote.is_empty(),
            "Invalid text anchor",
        ),
        Location::Page { edition_id, .. } => require(
            state.editions.contains_key(edition_id),
            "Invalid page anchor",
        ),
        Location::Audio {
            provider,
            source_id,
            ..
        }
        | Location::Provider {
            provider,
            source_id,
            ..
        } => require(
            !provider.is_empty() && !source_id.is_empty(),
            "Invalid provider position",
        ),
    }
}
