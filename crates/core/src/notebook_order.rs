// SPDX-License-Identifier: MIT OR Apache-2.0
//! Private notebook append provenance and reviewed legacy order.
use crate::{Error, canonical, digest, model::Notebook};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

type Id = String;
type Order = Vec<Id>;
type Evidence = BTreeSet<Order>;
type Edge = (usize, usize);
type Result<T> = std::result::Result<T, OrderingError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrderingError {
    InvalidId,
    DuplicateBlock,
    MissingBlock,
    IdentityReused,
    InvalidAppend,
    Cycle,
    LegacyBudget,
    FrontierBudget,
    ReviewBudget,
    ReviewStale,
    InvalidReview,
    InvalidEncoding,
    ProjectionMismatch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Append {
    pub id: Id,
    pub after: Option<Id>,
    pub witness: Order,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
struct Insertion {
    after: Option<Id>,
    frontier: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request: Option<Id>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
struct Review {
    evidence: Id,
    #[serde(deserialize_with = "deserialize_unique_set")]
    parents: BTreeSet<Id>,
    selected: Id,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub evidence: Id,
    pub legacy_orders: Evidence,
    pub heads: BTreeSet<Id>,
    pub independently_reviewed: BTreeSet<Order>,
}

#[derive(Clone, Debug, JsonSchema, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PendingNotebookOrderReview {
    pub notebook_id: Id,
    pub evidence: Id,
    pub heads: BTreeSet<Id>,
    pub legacy_orders: Evidence,
    pub choices: BTreeSet<Order>,
    pub provisional_legacy_order: Order,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub ids: Order,
    pub issue: Option<Issue>,
}

/// Rendered notebook blocks remain separate. Paths and frontiers record provenance
/// and cannot be inferred back from a transient presentation order.
#[derive(Clone, Debug, Default, Deserialize, JsonSchema, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NotebookOrder {
    #[serde(deserialize_with = "deserialize_unique_set")]
    legacy: Evidence,
    inserts: BTreeMap<Id, Insertion>,
    frontiers: BTreeMap<Id, Order>,
    reviews: BTreeMap<Id, Review>,
    #[serde(deserialize_with = "deserialize_review_scopes")]
    review_scopes: BTreeMap<Id, Evidence>,
    reviewed_orders: BTreeMap<Id, Order>,
}

pub(crate) fn deserialize_optional_order<'de, D>(
    input: D,
) -> std::result::Result<Option<NotebookOrder>, D::Error>
where
    D: Deserializer<'de>,
{
    NotebookOrder::deserialize(input).map(Some)
}

fn unique_set<T: Ord, E: serde::de::Error>(raw: Vec<T>) -> std::result::Result<BTreeSet<T>, E> {
    let mut set = BTreeSet::new();
    for element in raw {
        if !set.insert(element) {
            return Err(E::custom("Duplicate ordering evidence"));
        }
    }
    Ok(set)
}

fn deserialize_unique_set<'de, D, T>(input: D) -> std::result::Result<BTreeSet<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Ord,
{
    unique_set(Vec::<T>::deserialize(input)?)
}

fn deserialize_review_scopes<'de, D>(
    input: D,
) -> std::result::Result<BTreeMap<Id, Evidence>, D::Error>
where
    D: Deserializer<'de>,
{
    BTreeMap::<Id, Vec<Order>>::deserialize(input)?
        .into_iter()
        .map(|(id, paths)| Ok((id, unique_set(paths)?)))
        .collect()
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn content_id(value: &impl Serialize) -> Result<Id> {
    canonical(value)
        .map(|bytes| digest(&bytes))
        .map_err(|_| OrderingError::InvalidEncoding)
}

const EXTRA_LEGACY_IDS: usize = 4096;
const LEGACY_VARIANTS: usize = 32;
const EXTRA_FRONTIER_IDS: usize = 8192;
const EXTRA_REVIEW_IDS: usize = 4096;
const MAX_REVIEWS: usize = 32;

fn validate_order(order: &[Id]) -> Result<()> {
    if order.iter().any(String::is_empty) {
        return Err(OrderingError::InvalidId);
    }
    if order.iter().collect::<BTreeSet<_>>().len() != order.len() {
        return Err(OrderingError::DuplicateBlock);
    }
    Ok(())
}

fn subsequence(small: &[Id], large: &[Id]) -> bool {
    let mut remaining = small.iter().peekable();
    for id in large {
        if remaining.peek().is_some_and(|next| *next == id) {
            remaining.next();
        }
    }
    remaining.peek().is_none()
}

fn total_slots(paths: &Evidence) -> usize {
    paths
        .iter()
        .fold(0usize, |total, path| total.saturating_add(path.len()))
}

fn preflight_paths(paths: &Evidence, factor: usize) -> Result<()> {
    let longest = paths.iter().map(Vec::len).max().unwrap_or(0);
    let accepted_slots = longest.saturating_add(longest.max(EXTRA_LEGACY_IDS));
    if paths.len() > LEGACY_VARIANTS * factor
        || total_slots(paths) > accepted_slots.saturating_mul(factor)
    {
        return Err(OrderingError::LegacyBudget);
    }
    Ok(())
}

fn legacy_paths(input: BTreeSet<Order>) -> Result<Evidence> {
    // Each admitted replica is bounded. A pair may transiently contain two
    // copies of that evidence before its strictly dominated paths are removed.
    preflight_paths(&input, 2)?;
    for order in &input {
        validate_order(order)?;
    }
    let maximal: BTreeSet<_> = input
        .iter()
        .filter(|order| {
            !order.is_empty()
                && !input
                    .iter()
                    .any(|other| other.len() > order.len() && subsequence(order, other))
        })
        .cloned()
        .collect();
    let baseline = maximal.iter().map(Vec::len).max().unwrap_or(0);
    let slots: usize = maximal.iter().map(Vec::len).sum();
    if maximal.len() > LEGACY_VARIANTS || slots - baseline > baseline.max(EXTRA_LEGACY_IDS) {
        return Err(OrderingError::LegacyBudget);
    }
    Ok(maximal)
}

fn join_records<T: Clone + Eq>(left: &mut BTreeMap<Id, T>, right: &BTreeMap<Id, T>) -> Result<()> {
    for (id, record) in right {
        if let Some(previous) = left.get(id) {
            if previous != record {
                return Err(OrderingError::IdentityReused);
            }
        } else {
            left.insert(id.clone(), record.clone());
        }
    }
    Ok(())
}

#[derive(Default)]
struct Graph {
    outgoing: Vec<Vec<usize>>,
    indegree: Vec<usize>,
    visible: usize,
}

impl Graph {
    fn new(visible: usize, gates: usize) -> Self {
        Self {
            outgoing: vec![Vec::new(); visible + gates],
            indegree: vec![0; visible + gates],
            visible,
        }
    }

    fn add(&mut self, (start, end): Edge) {
        self.outgoing[start].push(end);
        self.indegree[end] += 1;
    }

    fn order(&self) -> Option<Vec<usize>> {
        let mut remaining = self.indegree.clone();
        let mut notes = BinaryHeap::new();
        let mut gates = Vec::new();
        let enqueue = |index, notes: &mut BinaryHeap<_>, gates: &mut Vec<_>| {
            if index < self.visible {
                notes.push(Reverse(index));
            } else {
                gates.push(index);
            }
        };
        for (index, degree) in remaining.iter().enumerate() {
            if *degree == 0 {
                enqueue(index, &mut notes, &mut gates);
            }
        }
        let mut visited = 0;
        let mut output = Vec::with_capacity(self.visible);
        while let Some(index) = gates
            .pop()
            .or_else(|| notes.pop().map(|Reverse(index)| index))
        {
            visited += 1;
            if index < self.visible {
                output.push(index);
            }
            for target in &self.outgoing[index] {
                remaining[*target] -= 1;
                if remaining[*target] == 0 {
                    enqueue(*target, &mut notes, &mut gates);
                }
            }
        }
        (visited == self.outgoing.len()).then_some(output)
    }

    /// Kosaraju with explicit stacks so a long imported chain does not recurse.
    fn components(&self) -> Vec<usize> {
        let mut reverse = vec![Vec::new(); self.outgoing.len()];
        for (source, targets) in self.outgoing.iter().enumerate() {
            for target in targets {
                reverse[*target].push(source);
            }
        }
        let mut seen = vec![false; self.outgoing.len()];
        let mut finished = Vec::new();
        for initial in 0..self.outgoing.len() {
            if std::mem::replace(&mut seen[initial], true) {
                continue;
            }
            let mut trace = vec![(initial, self.outgoing[initial].iter())];
            while let Some((source, edges)) = trace.last_mut() {
                if let Some(target) = edges.next() {
                    if !std::mem::replace(&mut seen[*target], true) {
                        trace.push((*target, self.outgoing[*target].iter()));
                    }
                } else {
                    finished.push(*source);
                    trace.pop();
                }
            }
        }
        let mut component = vec![usize::MAX; self.outgoing.len()];
        for source in finished.into_iter().rev() {
            if component[source] != usize::MAX {
                continue;
            }
            component[source] = source;
            let mut trace = vec![source];
            while let Some(node) = trace.pop() {
                for predecessor in &reverse[node] {
                    if component[*predecessor] == usize::MAX {
                        component[*predecessor] = source;
                        trace.push(*predecessor);
                    }
                }
            }
        }
        component
    }
}

struct LegacyConstraints<'a> {
    uncontested: Vec<(&'a Id, &'a Id)>,
    ids: BTreeSet<&'a Id>,
    cyclic: bool,
}

struct ReviewContext<'a> {
    evidence: Id,
    constraints: LegacyConstraints<'a>,
    heads: BTreeSet<Id>,
}

fn legacy_edges(paths: &Evidence) -> Vec<(&Id, &Id)> {
    paths
        .iter()
        .flat_map(|path| path.windows(2).map(|pair| (&pair[0], &pair[1])))
        .collect()
}

fn legacy_constraints(paths: &Evidence) -> Result<LegacyConstraints<'_>> {
    let ids: BTreeSet<_> = paths.iter().flatten().collect();
    let indices: BTreeMap<_, _> = ids
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, index))
        .collect();
    let edges = legacy_edges(paths);
    let mut graph = Graph::new(ids.len(), 0);
    for (from, to) in &edges {
        graph.add((indices[from], indices[to]));
    }
    let cyclic = graph.order().is_none();
    let uncontested = if cyclic {
        let components = graph.components();
        edges
            .into_iter()
            .filter(|(from, to)| components[indices[from]] != components[indices[to]])
            .collect()
    } else {
        edges
    };
    Ok(LegacyConstraints {
        uncontested,
        ids,
        cyclic,
    })
}

impl NotebookOrder {
    pub fn from_legacy(ids: Order) -> Result<Self> {
        Ok(Self {
            legacy: legacy_paths(BTreeSet::from([ids]))?,
            ..Self::default()
        })
    }

    fn legacy_ids(&self) -> BTreeSet<&Id> {
        self.legacy.iter().flatten().collect()
    }

    fn indices(&self) -> Result<BTreeMap<&Id, usize>> {
        let mut ids = self.legacy_ids();
        for id in self.inserts.keys() {
            if !ids.insert(id) {
                return Err(OrderingError::DuplicateBlock);
            }
        }
        Ok(ids
            .into_iter()
            .enumerate()
            .map(|(index, id)| (id, index))
            .collect())
    }

    fn authored_graph(&self, indices: &BTreeMap<&Id, usize>) -> Result<Graph> {
        let mut graph = Graph::new(indices.len(), self.frontiers.len());
        let mut gates = BTreeMap::new();
        for (offset, (id, frontier)) in self.frontiers.iter().enumerate() {
            let gate = indices.len() + offset;
            gates.insert(id, gate);
            for predecessor in frontier {
                let source = *indices
                    .get(predecessor)
                    .ok_or(OrderingError::MissingBlock)?;
                graph.add((source, gate));
            }
        }
        for (id, entry) in &self.inserts {
            let target = indices[id];
            if let Some(after) = &entry.after {
                let source = *indices.get(after).ok_or(OrderingError::MissingBlock)?;
                graph.add((source, target));
            }
            if let Some(frontier) = &entry.frontier {
                let gate = *gates.get(frontier).ok_or(OrderingError::InvalidAppend)?;
                graph.add((gate, target));
            }
        }
        Ok(graph)
    }

    fn check(&self) -> Result<()> {
        preflight_paths(&self.legacy, 1)?;
        if legacy_paths(self.legacy.clone())? != self.legacy {
            return Err(OrderingError::InvalidEncoding);
        }
        let legacy_ids = self.legacy_ids();
        let legacy_count = legacy_ids.len();
        let witness_slots: usize = self.frontiers.values().map(Vec::len).sum();
        if witness_slots > legacy_count + legacy_count.max(EXTRA_FRONTIER_IDS) {
            return Err(OrderingError::FrontierBudget);
        }
        let referenced: BTreeSet<_> = self
            .inserts
            .values()
            .filter_map(|entry| entry.frontier.as_ref())
            .collect();
        if referenced.len() != self.frontiers.len()
            || self.frontiers.keys().any(|id| !referenced.contains(id))
        {
            return Err(OrderingError::InvalidAppend);
        }
        for (id, prerequisites) in &self.frontiers {
            if prerequisites.is_empty()
                || !prerequisites.windows(2).all(|pair| pair[0] < pair[1])
                || content_id(prerequisites)? != *id
            {
                return Err(OrderingError::InvalidAppend);
            }
        }
        for (id, entry) in &self.inserts {
            if id.is_empty()
                || entry.after.as_ref() == Some(id)
                || entry
                    .request
                    .as_deref()
                    .is_some_and(|request| !is_digest(request))
            {
                return Err(OrderingError::InvalidAppend);
            }
            if let Some(frontier) = &entry.frontier {
                let witnessed = self
                    .frontiers
                    .get(frontier)
                    .ok_or(OrderingError::InvalidAppend)?;
                if witnessed.binary_search(id).is_ok()
                    || entry
                        .after
                        .as_ref()
                        .is_some_and(|after| witnessed.binary_search(after).is_ok())
                {
                    return Err(OrderingError::InvalidAppend);
                }
            }
        }
        if self.authored_graph(&self.indices()?)?.order().is_none() {
            return Err(OrderingError::Cycle);
        }
        self.check_reviews(&legacy_ids)
    }

    fn check_reviews(&self, legacy_ids: &BTreeSet<&Id>) -> Result<()> {
        let scope_budget = total_slots(&self.legacy)
            .saturating_mul(2)
            .saturating_add(EXTRA_REVIEW_IDS);
        let order_budget = legacy_ids
            .len()
            .saturating_mul(2)
            .saturating_add(EXTRA_REVIEW_IDS);
        let scope_slots = self.review_scopes.values().fold(0usize, |used, scope| {
            used.saturating_add(total_slots(scope))
        });
        let choice_slots = self
            .reviewed_orders
            .values()
            .fold(0usize, |used, order| used.saturating_add(order.len()));
        if self.reviews.len() > MAX_REVIEWS
            || self.review_scopes.len() > MAX_REVIEWS
            || self.reviewed_orders.len() > MAX_REVIEWS
            || scope_slots > scope_budget
            || choice_slots > order_budget
            || self
                .reviews
                .values()
                .any(|review| review.parents.len() >= MAX_REVIEWS)
        {
            return Err(OrderingError::ReviewBudget);
        }
        let scopes: BTreeSet<_> = self
            .reviews
            .values()
            .map(|review| &review.evidence)
            .collect();
        let choices: BTreeSet<_> = self
            .reviews
            .values()
            .map(|review| &review.selected)
            .collect();
        if !scopes.into_iter().eq(self.review_scopes.keys())
            || !choices.into_iter().eq(self.reviewed_orders.keys())
        {
            return Err(OrderingError::InvalidReview);
        }

        let mut checked_scopes = BTreeMap::new();
        for (id, scope) in &self.review_scopes {
            preflight_paths(scope, 1)?;
            if content_id(scope)? != *id
                || legacy_paths(scope.clone())? != *scope
                || scope
                    .iter()
                    .any(|path| !self.legacy.iter().any(|current| subsequence(path, current)))
            {
                return Err(OrderingError::InvalidReview);
            }
            let checked = legacy_constraints(scope)?;
            if !checked.cyclic {
                return Err(OrderingError::InvalidReview);
            }
            checked_scopes.insert(id, checked);
        }
        for (id, selected) in &self.reviewed_orders {
            validate_order(selected)?;
            if content_id(selected)? != *id {
                return Err(OrderingError::InvalidReview);
            }
        }
        let indices: BTreeMap<_, _> = self
            .reviews
            .keys()
            .enumerate()
            .map(|(index, id)| (id, index))
            .collect();
        let mut graph = Graph::new(self.reviews.len(), 0);
        for (id, review) in &self.reviews {
            if id.is_empty() {
                return Err(OrderingError::InvalidReview);
            }
            validate_review_order(
                &checked_scopes[&review.evidence],
                &self.reviewed_orders[&review.selected],
            )?;
            for parent in &review.parents {
                if self
                    .reviews
                    .get(parent)
                    .is_none_or(|previous| previous.evidence != review.evidence)
                {
                    return Err(OrderingError::InvalidReview);
                }
                graph.add((indices[parent], indices[id]));
            }
        }
        if graph.order().is_none() {
            return Err(OrderingError::InvalidReview);
        }

        if let Some(context) = self.context()? {
            let active: BTreeSet<_> = context
                .heads
                .iter()
                .map(|id| &self.reviews[id].selected)
                .collect();
            if active.len() != 1 {
                // Choosing an existing head uses no additional full-order slots.
                // For a fresh conflict keep space for one initial total choice.
                let choice_cost = if active.is_empty() {
                    legacy_ids.len()
                } else {
                    0
                };
                let scope_cost = if self.review_scopes.contains_key(&context.evidence) {
                    0
                } else {
                    total_slots(&self.legacy)
                };
                if self.reviews.len() == MAX_REVIEWS
                    || choice_slots.saturating_add(choice_cost) > order_budget
                    || scope_slots.saturating_add(scope_cost) > scope_budget
                {
                    return Err(OrderingError::ReviewBudget);
                }
            }
        }
        Ok(())
    }

    /// Check imported state without promoting or rewriting any notebook.
    pub fn validate_projection<'a>(&self, blocks: impl Iterator<Item = &'a Id>) -> Result<()> {
        self.check()?;
        if self.project()?.ids.iter().eq(blocks) {
            Ok(())
        } else {
            Err(OrderingError::ProjectionMismatch)
        }
    }

    pub fn join(&self, other: &Self) -> Result<Self> {
        self.check()?;
        other.check()?;
        let mut next = self.clone();
        next.legacy = legacy_paths(self.legacy.union(&other.legacy).cloned().collect())?;
        join_records(&mut next.inserts, &other.inserts)?;
        join_records(&mut next.reviews, &other.reviews)?;
        join_records(&mut next.frontiers, &other.frontiers)?;
        join_records(&mut next.review_scopes, &other.review_scopes)?;
        join_records(&mut next.reviewed_orders, &other.reviewed_orders)?;
        next.check()?;
        Ok(next)
    }

    pub fn prepare_append(&self, id: Id) -> Result<Append> {
        self.prepare_visible_append(id, &self.view()?.ids)
    }

    pub fn prepare_visible_append(&self, id: Id, visible: &[Id]) -> Result<Append> {
        self.check()?;
        validate_order(visible)?;
        let indices = self.indices()?;
        let visible_ids: BTreeSet<_> = visible.iter().collect();
        if visible_ids.contains(&id) || visible.iter().any(|block| !indices.contains_key(block)) {
            return Err(OrderingError::InvalidAppend);
        }
        let after = visible.last().cloned();
        let mut parents = BTreeSet::new();
        let mut gates = BTreeSet::new();
        for (block, entry) in &self.inserts {
            if !visible_ids.contains(block) {
                continue;
            }
            parents.extend(
                entry
                    .after
                    .iter()
                    .filter(|parent| visible_ids.contains(parent)),
            );
            gates.extend(entry.frontier.iter());
        }
        for gate in gates {
            parents.extend(
                self.frontiers[gate]
                    .iter()
                    .filter(|parent| visible_ids.contains(parent)),
            );
        }
        // Recompute only authenticated immutable edges within the captured view.
        // Late blocks must not become causal predecessors of an already composed note.
        let witness = visible_ids
            .into_iter()
            .filter(|block| Some(*block) != after.as_ref() && !parents.contains(block))
            .cloned()
            .collect();
        Ok(Append { id, after, witness })
    }

    pub fn apply(&self, intent: Append) -> Result<Self> {
        self.record_append(intent, None)
    }

    pub(crate) fn replayed_request(&self, id: &Id, request: &str) -> Result<bool> {
        match self.inserts.get(id) {
            None => Ok(false),
            Some(existing) if existing.request.as_deref() == Some(request) => Ok(true),
            Some(_) => Err(OrderingError::IdentityReused),
        }
    }

    pub(crate) fn record_append(&self, mut intent: Append, request: Option<Id>) -> Result<Self> {
        self.check()?;
        if intent.id.is_empty() {
            return Err(OrderingError::InvalidId);
        }
        validate_order(&intent.witness)?;
        intent.witness.sort();
        let mut next = self.clone();
        let frontier = if intent.witness.is_empty() {
            None
        } else {
            let key = content_id(&intent.witness)?;
            join_records(
                &mut next.frontiers,
                &BTreeMap::from([(key.clone(), intent.witness)]),
            )?;
            Some(key)
        };
        let record = Insertion {
            after: intent.after,
            frontier,
            request,
        };
        join_records(&mut next.inserts, &BTreeMap::from([(intent.id, record)]))?;
        next.check()?;
        Ok(next)
    }

    fn context(&self) -> Result<Option<ReviewContext<'_>>> {
        let constraints = legacy_constraints(&self.legacy)?;
        if !constraints.cyclic {
            return Ok(None);
        }
        let evidence = content_id(&self.legacy)?;
        let scoped: BTreeMap<_, _> = self
            .reviews
            .iter()
            .filter(|(_, review)| review.evidence == evidence)
            .collect();
        let parents: BTreeSet<_> = scoped.values().flat_map(|review| &review.parents).collect();
        let heads = scoped
            .keys()
            .filter(|id| !parents.contains(*id))
            .map(|id| (*id).clone())
            .collect();
        Ok(Some(ReviewContext {
            evidence,
            constraints,
            heads,
        }))
    }

    pub fn view(&self) -> Result<View> {
        self.check()?;
        self.project()
    }

    fn project(&self) -> Result<View> {
        let indices = self.indices()?;
        let mut graph = self.authored_graph(&indices)?;
        let render = |edges: &[(&Id, &Id)], graph: &mut Graph| -> Result<Order> {
            for (from, to) in edges {
                graph.add((indices[from], indices[to]));
            }
            let sorted: Vec<_> = indices.keys().copied().collect();
            let indices_out = graph.order().ok_or(OrderingError::Cycle)?;
            Ok(indices_out
                .into_iter()
                .map(|index| sorted[index].clone())
                .collect())
        };
        let original = legacy_edges(&self.legacy);
        let Some(context) = self.context()? else {
            return Ok(View {
                ids: render(&original, &mut graph)?,
                issue: None,
            });
        };
        let choices: BTreeSet<_> = context
            .heads
            .iter()
            .map(|id| self.reviewed_orders[&self.reviews[id].selected].clone())
            .collect();
        if choices.len() == 1 {
            let reviewed: Vec<_> = choices
                .first()
                .unwrap()
                .windows(2)
                .map(|pair| (&pair[0], &pair[1]))
                .collect();
            return Ok(View {
                ids: render(&reviewed, &mut graph)?,
                issue: None,
            });
        }
        let ids = render(&context.constraints.uncontested, &mut graph)?;
        Ok(View {
            ids,
            issue: Some(Issue {
                evidence: context.evidence,
                legacy_orders: self.legacy.clone(),
                heads: context.heads,
                independently_reviewed: choices,
            }),
        })
    }

    pub fn review(&self, id: Id, previous: &Issue, selected: Order) -> Result<Self> {
        self.review_order(
            id,
            previous.evidence.clone(),
            previous.heads.clone(),
            selected,
        )
    }

    pub(crate) fn review_order(
        &self,
        id: Id,
        evidence: Id,
        parents: BTreeSet<Id>,
        selected: Order,
    ) -> Result<Self> {
        self.check()?;
        if id.is_empty() {
            return Err(OrderingError::InvalidId);
        }
        validate_order(&selected)?;
        let selected_id = content_id(&selected)?;
        let requested = Review {
            evidence,
            parents,
            selected: selected_id.clone(),
        };
        if let Some(accepted) = self.reviews.get(&id) {
            // An acknowledgement can be retried even after the review has moved on.
            return if accepted == &requested {
                Ok(self.clone())
            } else {
                Err(OrderingError::IdentityReused)
            };
        }
        let context = self.context()?.ok_or(OrderingError::ReviewStale)?;
        if requested.evidence != context.evidence || requested.parents != context.heads {
            return Err(OrderingError::ReviewStale);
        }
        validate_review_order(&context.constraints, &selected)?;
        let mut next = self.clone();
        join_records(
            &mut next.reviewed_orders,
            &BTreeMap::from([(selected_id, selected)]),
        )?;
        join_records(
            &mut next.review_scopes,
            &BTreeMap::from([(requested.evidence.clone(), self.legacy.clone())]),
        )?;
        join_records(&mut next.reviews, &BTreeMap::from([(id, requested)]))?;
        next.check()?;
        Ok(next)
    }
}

/// List lossless legacy interpretations and runnable, bounded total selections.
pub fn unresolved_reviews(
    state: &crate::model::State,
) -> crate::Result<Vec<PendingNotebookOrderReview>> {
    let mut pending = Vec::new();
    for notebook in state.notebooks.values() {
        let Some(order) = &notebook.order else {
            continue;
        };
        let View {
            ids,
            issue: Some(issue),
        } = order.view().map_err(invalid_order)?
        else {
            continue;
        };
        let legacy_ids = order.legacy_ids();
        let provisional_legacy_order = ids
            .into_iter()
            .filter(|id| legacy_ids.contains(id))
            .collect();
        let choices = if issue.independently_reviewed.is_empty() {
            order.legacy_choices().map_err(invalid_order)?
        } else {
            issue.independently_reviewed
        };
        pending.push(PendingNotebookOrderReview {
            notebook_id: notebook.id.clone(),
            evidence: issue.evidence,
            heads: issue.heads,
            legacy_orders: issue.legacy_orders,
            choices,
            provisional_legacy_order,
        });
    }
    Ok(pending)
}

impl NotebookOrder {
    fn legacy_choices(&self) -> Result<BTreeSet<Order>> {
        let context = self.context()?.ok_or(OrderingError::ReviewStale)?;
        let ids: Vec<_> = context.constraints.ids.iter().copied().collect();
        let indices: BTreeMap<_, _> = ids
            .iter()
            .enumerate()
            .map(|(index, id)| (*id, index))
            .collect();
        // Preserve all exact raw interpretations separately. Limit expanded
        // total choices so one legacy path cannot amplify into an unbounded list.
        let max_slots = ids.len().saturating_mul(2).saturating_add(EXTRA_REVIEW_IDS);
        let mut choices = BTreeSet::new();
        for path in &self.legacy {
            if choices.len().saturating_add(1).saturating_mul(ids.len()) > max_slots {
                break;
            }
            let mut graph = Graph::new(ids.len(), 0);
            for (from, to) in &context.constraints.uncontested {
                graph.add((indices[from], indices[to]));
            }
            for pair in path.windows(2) {
                graph.add((indices[&pair[0]], indices[&pair[1]]));
            }
            let candidate = graph
                .order()
                .ok_or(OrderingError::InvalidReview)?
                .into_iter()
                .map(|index| ids[index].clone())
                .collect();
            choices.insert(candidate);
        }
        Ok(choices)
    }
}

pub(crate) fn invalid_order(error: OrderingError) -> Error {
    match error {
        OrderingError::LegacyBudget
        | OrderingError::FrontierBudget
        | OrderingError::ReviewBudget => Error::Conflict(format!(
            "Notebook order exceeds the evidence budget ({error:?}); retain the original verified transfer for retry."
        )),
        OrderingError::ReviewStale | OrderingError::IdentityReused => Error::Conflict(format!(
            "Notebook order changed or reused an identity ({error:?})."
        )),
        _ => Error::Invalid(format!("Invalid notebook order: {error:?}.")),
    }
}

pub(crate) fn snapshot(notebook: &Notebook) -> Result<NotebookOrder> {
    match &notebook.order {
        Some(order) => Ok(order.clone()),
        None => NotebookOrder::from_legacy(
            notebook
                .blocks
                .iter()
                .map(|block| block.id.clone())
                .collect(),
        ),
    }
}

pub(crate) fn materialize(
    notebook: &mut Notebook,
    order: NotebookOrder,
    retain: bool,
) -> Result<()> {
    let view = order.view()?;
    let positions: BTreeMap<_, _> = view
        .ids
        .into_iter()
        .enumerate()
        .map(|(index, id)| (id, index))
        .collect();
    let blocks: BTreeSet<_> = notebook.blocks.iter().map(|block| &block.id).collect();
    if blocks.len() != notebook.blocks.len() || !blocks.into_iter().eq(positions.keys()) {
        return Err(OrderingError::ProjectionMismatch);
    }
    notebook.blocks.sort_by_key(|block| positions[&block.id]);
    if retain {
        notebook.order = Some(order);
    }
    Ok(())
}

pub(crate) fn requires_provenance(order: &NotebookOrder) -> bool {
    order.legacy.len() > 1
}

fn validate_review_order(context: &LegacyConstraints<'_>, selected: &[Id]) -> Result<()> {
    let positions: BTreeMap<_, _> = selected
        .iter()
        .enumerate()
        .map(|(index, block)| (block, index))
        .collect();
    if positions.keys().copied().collect::<BTreeSet<_>>() != context.ids
        || context
            .uncontested
            .iter()
            .any(|(from, to)| positions[from] >= positions[to])
    {
        return Err(OrderingError::InvalidReview);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(ids: &[&str]) -> Order {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    fn old(ids: &[&str]) -> NotebookOrder {
        NotebookOrder::from_legacy(names(ids)).unwrap()
    }

    fn add(state: &NotebookOrder, id: &str) -> NotebookOrder {
        state
            .apply(state.prepare_append(id.to_owned()).unwrap())
            .unwrap()
    }

    #[test]
    fn authored_three_way_join_converges_and_respects_clock_skew_and_late_append() {
        let base = old(&["old-z", "old-a"]);
        let left = add(&add(&base, "left-at-9"), "left-at-8");
        let middle = add(&add(&base, "middle-at-9"), "middle-at-9-later");
        let right = add(&base, "right");
        let expected = left.join(&middle).unwrap().join(&right).unwrap();
        let replicas = [&left, &middle, &right];
        for permutation in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            let [a, b, c] = permutation.map(|index| replicas[index]);
            assert_eq!(a.join(b).unwrap().join(c).unwrap(), expected);
            assert_eq!(a.join(&b.join(c).unwrap()).unwrap(), expected);
        }
        let prior = left.join(&middle).unwrap();
        let before = prior.view().unwrap().ids;
        let after = add(&prior, "later")
            .join(&right)
            .unwrap()
            .view()
            .unwrap()
            .ids;
        assert!(
            before
                .iter()
                .all(|id| after.iter().position(|entry| entry == id).unwrap()
                    < after.iter().position(|entry| entry == "later").unwrap())
        );
        assert_eq!(
            after
                .iter()
                .filter(|id| before.contains(id))
                .cloned()
                .collect::<Order>(),
            before
        );
    }

    #[test]
    fn late_older_writer_and_replay_do_not_change_observed_ancestry() {
        let split = old(&["p", "b"]).join(&old(&["p", "c"])).unwrap();
        let intent = split.prepare_append("a-new".into()).unwrap();
        assert_eq!(intent.after.as_deref(), Some("c"));
        assert_eq!(intent.witness, names(&["b", "p"]));
        let late = old(&["p", "c", "b"]);
        let before = split.apply(intent.clone()).unwrap().join(&late).unwrap();
        let after = split.join(&late).unwrap().apply(intent.clone()).unwrap();
        assert_eq!(before, after);
        assert_eq!(after.view().unwrap().ids, names(&["p", "c", "b", "a-new"]));
        assert_eq!(after.apply(intent.clone()).unwrap(), after);
        let mut reused = intent;
        reused.witness = names(&["b"]);
        assert_eq!(after.apply(reused), Err(OrderingError::IdentityReused));
    }

    #[test]
    fn redundant_older_prefixes_and_subsequences_do_not_change_metadata() {
        let full = old(&["a", "b", "c", "d"]);
        for prefix in [&[][..], &["a"], &["a", "b"], &["a", "c", "d"]] {
            assert_eq!(full.join(&old(prefix)).unwrap(), full);
            assert_eq!(old(prefix).join(&full).unwrap(), full);
        }
    }

    #[test]
    fn conflicting_legacy_orders_stay_readable_and_review_preconditions_are_exact() {
        let a = old(&["common", "text", "scribble", "tail"]);
        let b = old(&["common", "scribble", "text", "tail"]);
        let conflict = a.join(&b).unwrap();
        let view = conflict.view().unwrap();
        assert_eq!(view.ids, names(&["common", "scribble", "text", "tail"]));
        let issue = view.issue.unwrap();
        let edited = add(&conflict, "while-open");
        assert_eq!(edited.view().unwrap().issue.as_ref(), Some(&issue));
        let choose = |id, order| edited.review(id, &issue, order).unwrap();
        let left = choose(
            "left-review".into(),
            names(&["common", "text", "scribble", "tail"]),
        );
        let right = choose(
            "right-review".into(),
            names(&["common", "scribble", "text", "tail"]),
        );
        assert_eq!(left.join(&a).unwrap(), left);
        assert_eq!(left.legacy, issue.legacy_orders);
        assert_eq!(left.view().unwrap().ids.last().unwrap(), "while-open");
        let both = left.join(&right).unwrap();
        let new_issue = both.view().unwrap().issue.unwrap();
        assert_eq!(new_issue.independently_reviewed.len(), 2);
        assert_eq!(
            both.review(
                "stale".into(),
                &issue,
                names(&["common", "text", "scribble", "tail"])
            ),
            Err(OrderingError::ReviewStale)
        );
        assert_eq!(
            both.review(
                "moves-common".into(),
                &new_issue,
                names(&["text", "common", "scribble", "tail"])
            ),
            Err(OrderingError::InvalidReview)
        );
        let reviewed = both
            .review(
                "settled".into(),
                &new_issue,
                names(&["common", "text", "scribble", "tail"]),
            )
            .unwrap();
        assert_eq!(
            reviewed.view().unwrap().ids,
            names(&["common", "text", "scribble", "tail", "while-open"])
        );
        assert_eq!(reviewed.join(&right).unwrap(), reviewed);
        assert_eq!(reviewed.legacy, issue.legacy_orders);
        assert_eq!(
            reviewed
                .review(
                    "right-review".into(),
                    &issue,
                    names(&["common", "scribble", "text", "tail"])
                )
                .unwrap(),
            reviewed,
        );
        assert_eq!(
            reviewed.review(
                "right-review".into(),
                &issue,
                names(&["common", "text", "scribble", "tail"])
            ),
            Err(OrderingError::IdentityReused),
        );
    }

    #[test]
    fn old_notebook_wire_shape_and_missing_optional_order_are_preserved() {
        let old = serde_json::json!({
            "id":"old", "book_id":"book", "pass_id":"pass", "blocks":[], "revisions":[]
        });
        let decoded: crate::model::Notebook = serde_json::from_value(old.clone()).unwrap();
        assert!(decoded.order.is_none());
        assert_eq!(serde_json::to_value(decoded).unwrap(), old);
        let mut with_null = old;
        with_null["order"] = serde_json::Value::Null;
        assert!(serde_json::from_value::<crate::model::Notebook>(with_null).is_err());

        let schema = serde_json::to_value(schemars::schema_for!(crate::model::Notebook)).unwrap();
        assert!(schema["properties"]["order"]["$ref"].is_string());
        assert!(
            !schema["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item == "order")
        );
    }

    #[test]
    fn order_wire_roundtrip_validates_the_frontier_key_and_exact_projection() {
        let base = old(&["legacy-b", "legacy-a"]);
        let appended = add(&base, "new");
        let encoded = serde_json::to_value(&appended).unwrap();
        let decoded: NotebookOrder = serde_json::from_value(encoded.clone()).unwrap();
        assert_eq!(decoded, appended);
        let projection = decoded.view().unwrap().ids;
        decoded.validate_projection(projection.iter()).unwrap();
        assert_eq!(
            decoded.validate_projection(projection.iter().rev()),
            Err(OrderingError::ProjectionMismatch)
        );

        let mut invalid = encoded;
        let shared = decoded.frontiers.keys().next().unwrap();
        invalid["frontiers"][shared] = serde_json::json!(["unknown"]);
        let tampered: NotebookOrder = serde_json::from_value(invalid).unwrap();
        assert_eq!(tampered.view(), Err(OrderingError::InvalidAppend));
    }

    #[test]
    fn missing_review_parent_and_uncontested_order_changes_are_rejected_on_read() {
        let conflict = old(&["p", "b", "a"]).join(&old(&["p", "a", "b"])).unwrap();
        let issue = conflict.view().unwrap().issue.unwrap();
        let resolved = conflict
            .review("choice".into(), &issue, names(&["p", "a", "b"]))
            .unwrap();
        let good = serde_json::to_value(&resolved).unwrap();
        let mut bad = good.clone();
        bad["reviews"]["choice"]["parents"] = serde_json::json!(["missing"]);
        assert_eq!(
            serde_json::from_value::<NotebookOrder>(bad).unwrap().view(),
            Err(OrderingError::InvalidReview)
        );

        let mut bad = good;
        let selected = resolved.reviews["choice"].selected.clone();
        bad["reviewed_orders"][selected] = serde_json::json!(["b", "a", "p"]);
        assert_eq!(
            serde_json::from_value::<NotebookOrder>(bad).unwrap().view(),
            Err(OrderingError::InvalidReview)
        );
    }

    #[test]
    fn large_legacy_notes_share_virtual_frontiers_and_only_additional_evidence_is_bounded() {
        let ids: Order = (0..4103).map(|index| format!("note-{index:05}")).collect();
        let old = NotebookOrder::from_legacy(ids.clone()).unwrap();
        assert_eq!(old.view().unwrap().ids, ids);
        let first = add(&old, "new-a");
        let second = add(&old, "new-b");
        let both = first.join(&second).unwrap();
        assert_eq!(both.frontiers.len(), 1);
        assert_eq!(
            both.inserts["new-a"].frontier,
            both.inserts["new-b"].frontier
        );
        let indices = both.indices().unwrap();
        let graph = both.authored_graph(&indices).unwrap();
        assert_eq!(graph.outgoing.len(), ids.len() + 3);
        assert_eq!(
            graph.outgoing.iter().map(Vec::len).sum::<usize>(),
            ids.len() + 3
        );
        assert_eq!(&both.view().unwrap().ids[..ids.len()], ids);
        let reversed = NotebookOrder::from_legacy(ids.iter().cloned().rev().collect()).unwrap();
        let ambiguous = both.join(&reversed).unwrap();
        let view = ambiguous.view().unwrap();
        assert_eq!(view.ids.len(), ids.len() + 2);
        assert_eq!(view.issue.unwrap().legacy_orders.len(), 2);
        let third =
            NotebookOrder::from_legacy(ids.iter().skip(1).chain(&ids[..1]).cloned().collect())
                .unwrap();
        assert_eq!(ambiguous.join(&third), Err(OrderingError::LegacyBudget));
        assert_eq!(old.view().unwrap().ids, ids);
    }
}

#[cfg(test)]
#[path = "notebook_order_integration_tests.rs"]
mod integration_tests;
