// SPDX-License-Identifier: MIT OR Apache-2.0
//! Portable, unsigned source-metadata history. Graph edges describe observed field values.
use crate::{Error, Result, canonical, digest, model::*};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub type Histories = BTreeMap<String, BTreeMap<String, Graph>>;
type Ids = Vec<String>;
const SCOPES: [(&str, &str); 2] = [("isbn", "ISBN"), ("isbn13", "ISBN13")];
const MAX_NODES: usize = 512;
const MAX_EDGES: usize = 4096;
const MAX_HEADS: usize = 64;
const MAX_TOTAL_NODES: usize = 65536;
const MAX_TOTAL_GRAPHS: usize = 16384;

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Graph {
    pub nodes: BTreeMap<String, Node>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub legacy_absence_unknown: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Node {
    ValueAnchor {
        values: Ids,
    },
    Import {
        parents: Ids,
        values: Ids,
        source_id: String,
        connection_id: Option<String>,
        row_digest: String,
    },
    Review {
        parents: Ids,
        values: Ids,
    },
    Reassert {
        parents: Ids,
        historical_node: String,
        values: Ids,
    },
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct IsbnReview {
    pub edition_id: String,
    pub namespace: String,
    pub heads: Ids,
    pub choices: Vec<Ids>,
    pub provisional_values: Ids,
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct IsbnHistoryView {
    pub edition_id: String,
    pub namespace: String,
    pub heads: Ids,
    pub nodes: BTreeMap<String, Node>,
    pub legacy_absence_unknown: bool,
}
fn invalid(s: &str) -> Error {
    Error::Invalid(format!("ISBN history: {s}"))
}
fn require(ok: bool, s: &str) -> Result<()> {
    if ok { Ok(()) } else { Err(invalid(s)) }
}
fn sorted(v: &Ids) -> bool {
    v.windows(2).all(|w| w[0] < w[1])
}
fn namespace(ns: &str) -> Result<&'static str> {
    SCOPES
        .iter()
        .find(|(n, _)| *n == ns)
        .map(|(_, c)| *c)
        .ok_or_else(|| invalid("unknown namespace"))
}
fn linked(s: &State, e: &Edition) -> bool {
    e.id == format!("{}:edition", e.book_id)
        && s.books.get(&e.book_id).is_some_and(|b| {
            b.sources.iter().any(|r| {
                r.provider == "goodreads"
                    && e.book_id == format!("goodreads:{}", digest(r.id.as_bytes()))
            })
        })
}
fn observed(e: &Edition, ns: &str) -> Ids {
    e.identifiers
        .iter()
        .filter(|i| i.provider == ns)
        .map(|i| i.id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
impl Node {
    fn values(&self) -> &Ids {
        match self {
            Self::ValueAnchor { values }
            | Self::Import { values, .. }
            | Self::Review { values, .. }
            | Self::Reassert { values, .. } => values,
        }
    }
    fn parents(&self) -> &[String] {
        match self {
            Self::ValueAnchor { .. } => &[],
            Self::Import { parents, .. }
            | Self::Review { parents, .. }
            | Self::Reassert { parents, .. } => parents,
        }
    }
    fn id(&self, edition: &str, ns: &str) -> Result<String> {
        Ok(digest(&canonical(&(
            "booknook.isbn-history.v1",
            edition,
            ns,
            self,
        ))?))
    }
}
impl Graph {
    fn heads(&self) -> Ids {
        let referenced: BTreeSet<_> = self.nodes.values().flat_map(|n| n.parents()).collect();
        self.nodes
            .keys()
            .filter(|id| !referenced.contains(id))
            .cloned()
            .collect()
    }
    fn choices(&self, heads: &[String]) -> Vec<Ids> {
        heads
            .iter()
            .map(|id| self.nodes[id].values().clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    fn insert(&mut self, edition: &str, ns: &str, node: Node) -> Result<String> {
        let id = node.id(edition, ns)?;
        if let Some(previous) = self.nodes.get(&id) {
            require(previous == &node, "node identity reused")?;
        } else {
            require(self.nodes.len() < MAX_NODES, "node budget exceeded")?;
            self.nodes.insert(id.clone(), node);
        }
        Ok(id)
    }
}
fn matching_row(p: &ImportEvidence, b: &Book, ns: &str) -> Option<Ids> {
    let column = namespace(ns).ok()?;
    let raw = p.fields.get(column)?;
    if p.source.provider != "goodreads"
        || p.fields.get("Book Id") != Some(&p.source.id)
        || b.id != format!("goodreads:{}", digest(p.source.id.as_bytes()))
        || digest(&serde_json::to_vec(&p.fields).ok()?) != p.digest
    {
        return None;
    }
    let v = crate::imports::clean_isbn(raw);
    Some(if v.is_empty() { vec![] } else { vec![v] })
}
// Conflicting row evidence cannot identify which write left a legacy field empty.
fn legacy_clear(book: &Book, ns: &str) -> bool {
    let mut saw_clear = false;
    for evidence in &book.provenance {
        if let Some(values) = matching_row(evidence, book, ns) {
            if !values.is_empty() {
                return false;
            }
            saw_clear = true;
        }
    }
    saw_clear
}

fn legacy(s: &State, e: &Edition, ns: &str, g: &mut Graph) -> Result<()> {
    let values = observed(e, ns);
    if values.is_empty() && !legacy_clear(&s.books[&e.book_id], ns) {
        g.legacy_absence_unknown = true;
        return Ok(());
    }
    g.insert(&e.id, ns, Node::ValueAnchor { values })?;
    Ok(())
}
fn project(s: &mut State, eid: &str, ns: &str) -> Result<()> {
    let g = &s.isbn_history[eid][ns];
    let choices = g.choices(&g.heads());
    let Some(values) = choices.first() else {
        return Ok(());
    };
    let e = s
        .editions
        .get_mut(eid)
        .ok_or_else(|| invalid("missing edition"))?;
    e.identifiers.retain(|i| i.provider != ns);
    e.identifiers.extend(values.iter().map(|v| SourceRef {
        provider: ns.into(),
        id: v.clone(),
    }));
    Ok(())
}
pub(crate) fn record_import(
    s: &mut State,
    eid: &str,
    p: &ImportEvidence,
    old: Option<&Edition>,
) -> Result<()> {
    for (ns, column) in SCOPES {
        if !p.fields.contains_key(column) {
            continue;
        }
        let e = &s.editions[eid];
        let values = matching_row(p, &s.books[&e.book_id], ns)
            .ok_or_else(|| invalid("unbound source row"))?;
        let mut g = s
            .isbn_history
            .get_mut(eid)
            .and_then(|m| m.remove(ns))
            .unwrap_or_default();
        if g.nodes.is_empty()
            && let Some(old) = old
        {
            // The new observation must not retroactively prove that a legacy empty was an explicit clear.
            let mut prior = s.books[&e.book_id].clone();
            prior.provenance.retain(|v| v != p);
            let oldvals = observed(old, ns);
            if !oldvals.is_empty() || legacy_clear(&prior, ns) {
                g.insert(eid, ns, Node::ValueAnchor { values: oldvals })?;
            } else {
                g.legacy_absence_unknown = true;
            }
        }
        let heads = g.heads();
        let disputed = g.choices(&heads).len() > 1;
        let previous_heads: BTreeSet<_> = heads.iter().cloned().collect();
        let mut parents = BTreeSet::new();
        for head in heads {
            let matching_stream = matches!(&g.nodes[&head], Node::Import {source_id, connection_id, ..}
                if source_id == &p.source.id && connection_id == &p.provider_connection_id);
            if !disputed || matching_stream {
                parents.insert(head);
            }
        }
        let anchor = g.insert(
            eid,
            ns,
            Node::ValueAnchor {
                values: values.clone(),
            },
        )?;
        // A compatibility anchor can already be an independent, disputed legacy
        // head. Recording the same value must not consume that independent head.
        if !disputed || !previous_heads.contains(&anchor) {
            parents.insert(anchor);
        }
        g.insert(
            eid,
            ns,
            Node::Import {
                values,
                parents: parents.into_iter().collect(),
                source_id: p.source.id.clone(),
                connection_id: p.provider_connection_id.clone(),
                row_digest: p.digest.clone(),
            },
        )?;
        s.isbn_history
            .entry(eid.into())
            .or_default()
            .insert(ns.into(), g);
        s.promote_archive(ArchiveVersion::Isbn);
        project(s, eid, ns)?;
    }
    Ok(())
}
pub(crate) fn merge(local: &State, incoming: &State, out: &mut State) -> Result<()> {
    let editions: Vec<_> = out
        .editions
        .iter()
        .filter(|(_, e)| linked(out, e))
        .map(|(k, _)| k.clone())
        .collect();
    for eid in &editions {
        for (ns, _) in SCOPES {
            let get = |s: &State| {
                s.editions
                    .get(eid)
                    .filter(|v| linked(s, v))
                    .map(|v| observed(v, ns))
            };
            let a = get(local);
            let b = get(incoming);
            let graphs = |s: &State| s.isbn_history.get(eid).is_some_and(|m| m.contains_key(ns));
            if !graphs(local) && !graphs(incoming) {
                let (Some(a), Some(b)) = (&a, &b) else {
                    continue;
                };
                if a == b {
                    // Equal materialized empties can represent a proven clear and an
                    // unknown absence. Combining their rows first loses that distinction.
                    let book_id = &local.editions[eid].book_id;
                    if !a.is_empty()
                        || legacy_clear(&local.books[book_id], ns)
                            == legacy_clear(&incoming.books[book_id], ns)
                    {
                        continue;
                    }
                }
            }
            let mut g = Graph::default();
            for s in [local, incoming] {
                if let Some(other) = s.isbn_history.get(eid).and_then(|m| m.get(ns)) {
                    g.legacy_absence_unknown |= other.legacy_absence_unknown;
                    for n in other.nodes.values() {
                        g.insert(eid, ns, n.clone())?;
                    }
                } else if let Some(old) = s.editions.get(eid).filter(|v| linked(s, v)) {
                    legacy(s, old, ns, &mut g)?;
                }
            }
            if !g.nodes.is_empty() {
                out.isbn_history
                    .entry(eid.clone())
                    .or_default()
                    .insert(ns.into(), g);
                out.promote_archive(ArchiveVersion::Isbn);
                project(out, eid, ns)?;
            }
        }
    }
    Ok(())
}
pub fn history(s: &State, edition_id: &str, ns: &str) -> Result<IsbnHistoryView> {
    namespace(ns)?;
    let e = s
        .editions
        .get(edition_id)
        .ok_or_else(|| invalid("unknown edition"))?;
    require(linked(s, e), "not a Goodreads catalog")?;
    let mut g = s
        .isbn_history
        .get(edition_id)
        .and_then(|m| m.get(ns))
        .cloned()
        .unwrap_or_default();
    if g.nodes.is_empty() {
        legacy(s, e, ns, &mut g)?;
    }
    Ok(IsbnHistoryView {
        edition_id: edition_id.into(),
        namespace: ns.into(),
        heads: g.heads(),
        nodes: g.nodes,
        legacy_absence_unknown: g.legacy_absence_unknown,
    })
}
pub fn pending_reviews(s: &State) -> Result<Vec<IsbnReview>> {
    let mut result = vec![];
    for (edition_id, scopes) in &s.isbn_history {
        for (ns, g) in scopes {
            let heads = g.heads();
            let choices = g.choices(&heads);
            if choices.len() > 1 {
                result.push(IsbnReview {
                    edition_id: edition_id.clone(),
                    namespace: ns.clone(),
                    heads,
                    provisional_values: choices[0].clone(),
                    choices,
                });
            }
        }
    }
    Ok(result)
}
pub(crate) fn decide(
    s: &mut State,
    eid: &str,
    ns: &str,
    parents: Ids,
    values: Option<Ids>,
    historical: Option<String>,
) -> Result<serde_json::Value> {
    let g = s
        .isbn_history
        .get_mut(eid)
        .and_then(|m| m.get_mut(ns))
        .ok_or_else(|| invalid("no managed graph"))?;
    require(
        sorted(&parents) && !parents.is_empty() && parents.len() <= MAX_HEADS,
        "invalid reviewed heads",
    )?;
    let node = match (values, historical) {
        (Some(values), None) => Node::Review { parents, values },
        (None, Some(id)) => Node::Reassert {
            values: g
                .nodes
                .get(&id)
                .ok_or_else(|| invalid("unknown historical observation"))?
                .values()
                .clone(),
            parents,
            historical_node: id,
        },
        _ => return Err(invalid("invalid decision")),
    };
    let id = node.id(eid, ns)?;
    if g.nodes.get(&id) == Some(&node) {
        return Ok(serde_json::json!({"node_id":id,"replayed":true}));
    }
    let heads = g.heads();
    require(node.parents() == heads, "stale reviewed heads")?;
    if matches!(node, Node::Review { .. }) {
        let choices = g.choices(&heads);
        require(
            choices.len() > 1 && choices.contains(node.values()),
            "not a current alternative",
        )?;
    }
    if let Node::Reassert {
        historical_node, ..
    } = &node
    {
        require(
            !heads.contains(historical_node),
            "reassertion requires a superseded historical observation",
        )?;
    }
    g.insert(eid, ns, node)?;
    project(s, eid, ns)?;
    Ok(serde_json::json!({"node_id":id,"replayed":false}))
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum IsbnHistoryCommand {
    ReviewIsbn {
        edition_id: String,
        namespace: String,
        expected_heads: Ids,
        values: Ids,
    },
    ReassertIsbn {
        edition_id: String,
        namespace: String,
        expected_heads: Ids,
        historical_node: String,
    },
}
impl IsbnHistoryCommand {
    pub fn apply(self, state: &mut State) -> Result<serde_json::Value> {
        crate::apply_atomic(state, |s| match self {
            Self::ReviewIsbn {
                edition_id,
                namespace,
                expected_heads,
                values,
            } => decide(
                s,
                &edition_id,
                &namespace,
                expected_heads,
                Some(values),
                None,
            ),
            Self::ReassertIsbn {
                edition_id,
                namespace,
                expected_heads,
                historical_node,
            } => decide(
                s,
                &edition_id,
                &namespace,
                expected_heads,
                None,
                Some(historical_node),
            ),
        })
    }
}

pub(crate) fn validate(s: &State) -> Result<()> {
    require(
        s.isbn_history.is_empty() || s.archive_version() == Some(ArchiveVersion::Isbn),
        "unsupported archive version",
    )?;
    let mut total = 0;
    let mut scopes = 0;
    for (eid, graphs) in &s.isbn_history {
        let e = s
            .editions
            .get(eid)
            .ok_or_else(|| invalid("unknown managed edition"))?;
        require(
            linked(s, e) && !graphs.is_empty() && graphs.len() <= 2,
            "invalid managed catalog",
        )?;
        let book = &s.books[&e.book_id];
        for (ns, g) in graphs {
            namespace(ns)?;
            total += g.nodes.len();
            scopes += 1;
            require(
                total <= MAX_TOTAL_NODES
                    && scopes <= MAX_TOTAL_GRAPHS
                    && !g.nodes.is_empty()
                    && g.nodes.len() <= MAX_NODES,
                "history budget exceeded",
            )?;
            let rows: BTreeSet<_> = book
                .provenance
                .iter()
                .filter_map(|p| {
                    matching_row(p, book, ns).map(|v| {
                        (
                            p.source.id.as_str(),
                            p.provider_connection_id.as_deref(),
                            p.digest.as_str(),
                            v,
                        )
                    })
                })
                .collect();
            let index: BTreeMap<_, _> = g.nodes.keys().enumerate().map(|(i, k)| (k, i)).collect();
            let mut successors = vec![vec![]; index.len()];
            let mut indegree = vec![0; index.len()];
            let mut edge_count = 0;
            for (i, (key, n)) in g.nodes.iter().enumerate() {
                require(
                    n.values().len() <= 64
                        && sorted(n.values())
                        && n.values().iter().all(|v| !v.is_empty() && v.len() <= 512),
                    "invalid values",
                )?;
                require(n.id(eid, ns)? == *key, "node ID mismatch")?;
                let parents = n.parents();
                require(
                    parents.len() <= MAX_HEADS + 1 && parents.windows(2).all(|w| w[0] < w[1]),
                    "invalid parent frontier",
                )?;
                if matches!(n, Node::Review { .. } | Node::Reassert { .. }) {
                    require(!parents.is_empty(), "missing parents")?;
                }
                edge_count += parents.len();
                require(edge_count <= MAX_EDGES, "edge budget exceeded")?;
                for p in parents {
                    let j = *index.get(p).ok_or_else(|| invalid("missing parent"))?;
                    successors[j].push(i);
                    indegree[i] += 1;
                }
                if let Node::Import {
                    values,
                    source_id,
                    connection_id,
                    row_digest,
                    ..
                } = n
                {
                    require(
                        rows.contains(&(
                            source_id,
                            connection_id.as_deref(),
                            row_digest,
                            values.clone(),
                        )),
                        "invalid source evidence",
                    )?;
                    let anchor = Node::ValueAnchor {
                        values: values.clone(),
                    }
                    .id(eid, ns)?;
                    require(
                        g.nodes.contains_key(&anchor),
                        "missing compatibility anchor",
                    )?;
                    let observed: Vec<_> = parents.iter().filter(|p| **p != anchor).collect();
                    if observed
                        .iter()
                        .map(|p| g.nodes[*p].values())
                        .collect::<BTreeSet<_>>()
                        .len()
                        > 1
                    {
                        require(
                            observed.iter().all(|p| {
                                matches!(&g.nodes[*p],Node::Import{source_id:s,connection_id:c,..}
                            if s==source_id && c==connection_id)
                            }),
                            "import consumed conflicting streams",
                        )?;
                    }
                }
            }
            // Transitive ancestry, bounded to MAX_NODES² bits per scope. A historical
            // review must have observed an antichain, not every head present today.
            let count = index.len();
            let mut ancestry = vec![vec![0u64; count.div_ceil(64)]; count];
            let mut work: Vec<_> = (0..count).filter(|i| indegree[*i] == 0).collect();
            let mut visited = 0;
            while let Some(i) = work.pop() {
                visited += 1;
                let bits = ancestry[i].clone();
                for &j in &successors[i] {
                    for (dst, src) in ancestry[j].iter_mut().zip(&bits) {
                        *dst |= *src;
                    }
                    ancestry[j][i / 64] |= 1 << (i % 64);
                    indegree[j] -= 1;
                    if indegree[j] == 0 {
                        work.push(j);
                    }
                }
            }
            require(visited == count, "cyclic history")?;
            // Concurrent peers can independently observe one row. A descendant
            // has already seen the row and the ordinary importer skips it.
            let mut observations = BTreeMap::new();
            for (id, node) in &g.nodes {
                if let Node::Import {
                    source_id,
                    connection_id,
                    row_digest,
                    ..
                } = node
                {
                    observations
                        .entry((source_id, connection_id, row_digest))
                        .or_insert_with(Vec::new)
                        .push(index[id]);
                }
            }
            for copies in observations.values().filter(|copies| copies.len() > 1) {
                for &tip in copies {
                    require(
                        copies
                            .iter()
                            .all(|&prior| ancestry[tip][prior / 64] & (1 << (prior % 64)) == 0),
                        "source row replayed in its own causal lineage",
                    )?;
                }
            }
            for (id, n) in &g.nodes {
                // Importers additionally link a deterministic compatibility anchor,
                // which may itself precede another genuinely observed parent.
                let compatibility = if let Node::Import { values, .. } = n {
                    Some(
                        Node::ValueAnchor {
                            values: values.clone(),
                        }
                        .id(eid, ns)?,
                    )
                } else {
                    None
                };
                let parents = n.parents();
                for p in parents
                    .iter()
                    .filter(|p| Some(*p) != compatibility.as_ref())
                {
                    let bits = &ancestry[index[p]];
                    require(
                        parents
                            .iter()
                            .filter(|q| Some(*q) != compatibility.as_ref())
                            .all(|q| {
                                let j = index[q];
                                bits[j / 64] & (1 << (j % 64)) == 0
                            }),
                        "observed parents overlap",
                    )?;
                }
                if matches!(n, Node::Review { .. } | Node::Reassert { .. }) {
                    if let Node::Reassert {
                        historical_node,
                        values,
                        ..
                    } = n
                    {
                        let old = g
                            .nodes
                            .get(historical_node)
                            .ok_or_else(|| invalid("unknown historical value"))?;
                        let j = index[historical_node];
                        let bits = &ancestry[index[id]];
                        require(
                            old.values() == values
                                && !parents.contains(historical_node)
                                && bits[j / 64] & (1 << (j % 64)) != 0,
                            "unobserved historical value",
                        )?;
                    } else {
                        let choices = g.choices(parents);
                        require(
                            choices.len() > 1 && choices.contains(n.values()),
                            "invalid review selection",
                        )?;
                    }
                }
            }
            let heads = g.heads();
            require(
                !heads.is_empty() && heads.len() <= MAX_HEADS,
                "head budget exceeded",
            )?;
            require(
                g.choices(&heads)[0].iter().eq(e
                    .identifiers
                    .iter()
                    .filter(|i| i.provider == *ns)
                    .map(|i| &i.id)),
                "identifier projection mismatch",
            )?;
        }
    }
    Ok(())
}
