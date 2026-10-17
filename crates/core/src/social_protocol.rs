// SPDX-License-Identifier: MIT OR Apache-2.0
//! Versioned, provider-independent social state. Only reviewed selections enter this model.
//! No notebook reference is dereferenced and no transport signature grants authority.
use crate::{
    Error, Result,
    authority::{self, AuthorityPolicy},
    canonical, digest,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA: &str = "booknook.social-state.v1";
pub const ACTION: &str = "social.execute";
pub const MAX_PUBLICATION_BYTES: usize = 60_000; // Fits a NIP-44 v2 plaintext, including receipts.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BookReference {
    /// A portable catalog identifier, such as an ISBN URN or a provider identifier.
    pub id: String,
    pub title: String,
    pub author: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PostKind {
    Review,
    Rating,
    ReadingUpdate,
    Note,
    Discussion,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Post {
    pub kind: PostKind,
    pub book: Option<BookReference>,
    pub body: String,
    pub stars: Option<u8>,
    pub reading_status: Option<String>,
    pub spoiler: bool,
    pub reply_to: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SharedList {
    pub id: String,
    pub owner: String,
    pub name: String,
    pub description: String,
    pub books: Vec<BookReference>,
    pub version: u64,
    pub deleted: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadingGroup {
    pub id: String,
    pub owner: String,
    pub name: String,
    pub description: String,
    pub private: bool,
    pub epoch: u64,
    pub members: BTreeSet<String>,
    pub moderators: BTreeSet<String>,
    pub banned: BTreeSet<String>,
    pub closed: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Invitation {
    pub id: String,
    pub group_id: String,
    pub inviter: String,
    pub invitee: String,
    pub epoch: u64,
    pub expires_at: u64,
    pub accepted: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JoinRequest {
    pub id: String,
    pub group_id: String,
    pub reader: String,
    pub epoch: u64,
    pub expires_at: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Delivery {
    Public,
    Direct { recipients: BTreeSet<String> },
}
/// An explicit, signed permission to show this membership acceptance to the group's
/// current and future admitted members. It never permits sharing the invitation or notes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdmissionConsent {
    pub group_id: String,
    pub epoch: u64,
    pub expires_at: u64,
    pub disclosure: AdmissionDisclosure,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionDisclosure {
    CurrentAndFutureGroupMembers,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum SocialAction {
    /// Both independently signed requests must be true before a friendship exists.
    Friendship {
        peer: String,
        requested: bool,
    },
    Publish {
        group_id: Option<String>,
        post: Post,
    },
    Retract {
        event_id: String,
    },
    CreateList {
        id: String,
        name: String,
        description: String,
        books: Vec<BookReference>,
    },
    UpdateList {
        id: String,
        name: String,
        description: String,
        books: Vec<BookReference>,
    },
    DeleteList {
        id: String,
    },
    CreateGroup {
        id: String,
        name: String,
        description: String,
        private: bool,
    },
    /// An owner-signed snapshot bootstraps an invitee without exposing private discovery.
    Invite {
        id: String,
        group: ReadingGroup,
        invitee: String,
        expires_at: u64,
    },
    AcceptInvitation {
        invitation_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        admission_consent: Option<AdmissionConsent>,
    },
    /// Public consent enables independent verification of public group membership.
    RequestJoin {
        group_id: String,
    },
    /// Adding members requires current consent: a private accepted invitation or public request.
    SetMembers {
        group_id: String,
        members: BTreeSet<String>,
        moderators: BTreeSet<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        admission_proofs: Vec<ApprovedPublication>,
    },
    Moderate {
        group_id: String,
        event_id: Option<String>,
        ban_actor: Option<String>,
    },
    CloseGroup {
        group_id: String,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SocialEvent {
    pub schema: String,
    pub id: String,
    pub actor: String,
    pub at: u64,
    pub resource_version: u64,
    pub delivery: Delivery,
    pub action: SocialAction,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApprovedPublication {
    pub schema: String,
    pub event: SocialEvent,
    pub approved_at: u64,
    /// Original strict JSON, not a permissively parsed map that could erase duplicate keys.
    pub request_json: String,
    pub presentation_json: String,
}

/// Schedule known history by signed causal version, never wall-clock ties or relay arrival.
/// This does not validate or apply records: callers must still verify every receipt and use
/// `receive`, which rejects stale versions, wrong audiences, missing parents and cycles.
/// Epoch-changing actions follow the other records approved in that epoch. A creation must
/// precede an invitation that could otherwise bootstrap the same group. Reply parents precede
/// their children; unknown/cyclic parents stay unresolved under the normal receive checks.
pub fn causal_replay_order<'a>(
    publications: impl IntoIterator<Item = &'a ApprovedPublication>,
) -> Vec<&'a ApprovedPublication> {
    let mut ordered: Vec<_> = publications.into_iter().collect();
    let by_id: BTreeMap<_, _> = ordered.iter().map(|p| (p.event.id.as_str(), *p)).collect();
    let mut depths = BTreeMap::<&str, usize>::new();
    for publication in &ordered {
        let mut path = Vec::new();
        let mut seen = BTreeSet::new();
        let mut cursor = publication.event.id.as_str();
        let base = loop {
            if let Some(depth) = depths.get(cursor) {
                // The cached node is the parent of the last node already in `path`.
                // Account for that edge before unwinding, even if it was visited first.
                break depth.saturating_add(usize::from(!path.is_empty()));
            }
            if !seen.insert(cursor) {
                break usize::MAX;
            }
            let Some(current) = by_id.get(cursor) else {
                break usize::MAX;
            };
            path.push(cursor);
            if let SocialAction::Publish {
                post:
                    Post {
                        reply_to: Some(parent),
                        ..
                    },
                ..
            } = &current.event.action
            {
                cursor = parent;
            } else {
                break 0;
            }
        };
        let mut depth = base;
        for id in path.into_iter().rev() {
            depths.insert(id, depth);
            depth = depth.saturating_add(1);
        }
    }
    let phase = |action: &SocialAction| match action {
        SocialAction::CreateGroup { .. } | SocialAction::CreateList { .. } => 0,
        SocialAction::Invite { .. } => 1,
        SocialAction::AcceptInvitation { .. } | SocialAction::RequestJoin { .. } => 2,
        SocialAction::Publish { .. }
        | SocialAction::Friendship { .. }
        | SocialAction::UpdateList { .. } => 3,
        SocialAction::Retract { .. }
        | SocialAction::Moderate {
            ban_actor: None, ..
        } => 4,
        SocialAction::SetMembers { .. }
        | SocialAction::Moderate {
            ban_actor: Some(_), ..
        }
        | SocialAction::CloseGroup { .. }
        | SocialAction::DeleteList { .. } => 5,
    };
    ordered.sort_by_key(|p| {
        (
            p.event.resource_version,
            phase(&p.event.action),
            depths[p.event.id.as_str()],
            p.approved_at,
            p.event.id.as_str(),
        )
    });
    ordered
}

fn consent(event: &SocialEvent) -> Option<&AdmissionConsent> {
    match &event.action {
        SocialAction::AcceptInvitation {
            admission_consent, ..
        } => admission_consent.as_ref(),
        _ => None,
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OutboxItem {
    pub publication: ApprovedPublication,
    /// Acknowledgments are recorded per intended recipient; no acknowledgment means retry.
    pub acknowledged: BTreeMap<String, BTreeSet<String>>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FriendshipIntent {
    pub requested: bool,
    pub version: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SocialState {
    pub schema: String,
    pub revision: u64,
    pub drafts: BTreeMap<String, SocialEvent>,
    pub outbox: BTreeMap<String, OutboxItem>,
    pub publications: BTreeMap<String, ApprovedPublication>,
    pub friendship_intents: BTreeMap<String, FriendshipIntent>,
    pub lists: BTreeMap<String, SharedList>,
    pub groups: BTreeMap<String, ReadingGroup>,
    pub invitations: BTreeMap<String, Invitation>,
    #[serde(default)]
    pub join_requests: BTreeMap<String, JoinRequest>,
    pub blocked: BTreeSet<String>,
    pub hidden: BTreeSet<String>,
    pub retracted: BTreeSet<String>,
    /// Receipt nonce -> exact event digest. Consumed in the same persistence transaction.
    pub replay: BTreeMap<String, String>,
    pub preferred_subjects: BTreeSet<String>,
}
impl Default for SocialState {
    fn default() -> Self {
        Self {
            schema: SCHEMA.into(),
            revision: 0,
            drafts: Default::default(),
            outbox: Default::default(),
            publications: Default::default(),
            friendship_intents: Default::default(),
            lists: Default::default(),
            groups: Default::default(),
            invitations: Default::default(),
            join_requests: Default::default(),
            blocked: Default::default(),
            hidden: Default::default(),
            retracted: Default::default(),
            replay: Default::default(),
            preferred_subjects: Default::default(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum LocalSocialCommand {
    SaveDraft {
        event: Box<SocialEvent>,
    },
    DeleteDraft {
        id: String,
    },
    /// Stop future retries; published history and receipts remain intact.
    CancelDelivery {
        event_id: String,
    },
    Block {
        actor: String,
        blocked: bool,
    },
    Hide {
        event_id: String,
        hidden: bool,
    },
    SetSubjects {
        subjects: BTreeSet<String>,
    },
}
fn invalid(message: &str) -> Error {
    Error::Invalid(message.into())
}
fn denied(message: &str) -> Error {
    Error::Denied(message.into())
}
fn ensure(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(denied(message))
    }
}
fn bounded(value: &str, max: usize, empty: bool) -> bool {
    (empty || !value.trim().is_empty()) && value.len() <= max && !value.contains('\0')
}
fn actor(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 200 && value.bytes().all(|b| (0x21..=0x7e).contains(&b))
}
fn owned_id(id: &str, owner: &str) -> bool {
    identifier(id) && id.starts_with(&format!("{owner}:")) && id.len() > 65
}
pub fn new_owned_id(owner: &str) -> Result<String> {
    ensure(actor(owner), "Invalid Castalia actor")?;
    Ok(format!("{owner}:{}", crate::new_id()))
}
fn pair(a: &str, b: &str) -> String {
    format!("{a}:{b}")
}
fn bump(n: u64) -> Result<u64> {
    n.checked_add(1)
        .filter(|n| *n <= 9_007_199_254_740_991)
        .ok_or_else(|| invalid("Version overflow"))
}
fn group<'a>(s: &'a SocialState, id: &str) -> Result<&'a ReadingGroup> {
    s.groups
        .get(id)
        .ok_or_else(|| Error::NotFound("Group history is required before this event".into()))
}
fn list<'a>(s: &'a SocialState, id: &str) -> Result<&'a SharedList> {
    s.lists
        .get(id)
        .ok_or_else(|| Error::NotFound("List history is required before this event".into()))
}
fn invitation<'a>(s: &'a SocialState, id: &str) -> Result<&'a Invitation> {
    s.invitations
        .get(id)
        .ok_or_else(|| Error::NotFound("Invitation history is required before this event".into()))
}
fn live_member(g: &ReadingGroup, who: &str) -> Result<()> {
    ensure(
        !g.closed && g.members.contains(who) && !g.banned.contains(who),
        "Group membership is inactive, revoked or banned",
    )
}
fn names(name: &str, description: &str) -> Result<()> {
    ensure(
        bounded(name, 200, false) && bounded(description, 4000, true),
        "Invalid name or description",
    )
}
fn book(b: &BookReference) -> bool {
    identifier(&b.id) && bounded(&b.title, 500, false) && bounded(&b.author, 500, true)
}
fn books(values: &[BookReference]) -> Result<()> {
    ensure(
        values.len() <= 300
            && values.iter().all(book)
            && values.iter().map(|b| &b.id).collect::<BTreeSet<_>>().len() == values.len(),
        "Invalid or duplicate list books",
    )
}
fn validate_group(g: &ReadingGroup) -> Result<()> {
    names(&g.name, &g.description)?;
    ensure(
        owned_id(&g.id, &g.owner)
            && actor(&g.owner)
            && g.epoch > 0
            && g.members.contains(&g.owner)
            && g.members.len() <= 128
            && g.members.iter().all(|v| actor(v))
            && g.moderators.is_subset(&g.members)
            && !g.banned.contains(&g.owner)
            && g.banned.iter().all(|v| actor(v)),
        "Invalid group snapshot",
    )
}
fn direct(recipients: impl IntoIterator<Item = String>) -> Delivery {
    Delivery::Direct {
        recipients: recipients.into_iter().collect(),
    }
}
fn group_delivery(g: &ReadingGroup) -> Delivery {
    if g.private {
        direct(g.members.iter().filter(|m| !g.banned.contains(*m)).cloned())
    } else {
        Delivery::Public
    }
}
/// The expected audience is derived from state, never accepted from a transport envelope.
pub fn expected(s: &SocialState, who: &str, action: &SocialAction) -> Result<(u64, Delivery)> {
    match action {
        SocialAction::Friendship { peer, .. } => Ok((
            s.friendship_intents
                .get(&pair(who, peer))
                .map_or(0, |v| v.version),
            direct([who.into(), peer.clone()]),
        )),
        SocialAction::Publish { group_id, .. } => {
            if let Some(id) = group_id {
                let g = group(s, id)?;
                Ok((g.epoch, group_delivery(g)))
            } else {
                Ok((0, Delivery::Public))
            }
        }
        SocialAction::Retract { event_id } => {
            let p = s
                .publications
                .get(event_id)
                .ok_or_else(|| Error::NotFound("Publication".into()))?;
            if let SocialAction::Publish {
                group_id: Some(id), ..
            } = &p.event.action
            {
                let g = group(s, id)?;
                Ok((g.epoch, group_delivery(g)))
            } else {
                Ok((0, p.event.delivery.clone()))
            }
        }
        SocialAction::CreateList { .. } | SocialAction::CreateGroup { private: false, .. } => {
            Ok((0, Delivery::Public))
        }
        SocialAction::CreateGroup { private: true, .. } => Ok((0, direct([who.into()]))),
        SocialAction::UpdateList { id, .. } | SocialAction::DeleteList { id } => {
            Ok((list(s, id)?.version, Delivery::Public))
        }
        SocialAction::Invite {
            group: g, invitee, ..
        } => {
            let current = s.groups.get(&g.id).unwrap_or(g);
            let mut recipients: BTreeSet<_> = current
                .members
                .difference(&current.banned)
                .cloned()
                .collect();
            recipients.insert(invitee.clone());
            Ok((current.epoch, direct(recipients)))
        }
        SocialAction::AcceptInvitation { invitation_id, .. } => {
            let i = invitation(s, invitation_id)?;
            let g = group(s, &i.group_id)?;
            let mut recipients: BTreeSet<_> = g.members.difference(&g.banned).cloned().collect();
            recipients.insert(who.into());
            Ok((g.epoch, direct(recipients)))
        }
        SocialAction::RequestJoin { group_id } => Ok((group(s, group_id)?.epoch, Delivery::Public)),
        SocialAction::SetMembers {
            group_id, members, ..
        } => {
            let g = group(s, group_id)?;
            let delivery = if g.private {
                direct(g.members.union(members).cloned())
            } else {
                Delivery::Public
            };
            Ok((g.epoch, delivery))
        }
        SocialAction::Moderate { group_id, .. } | SocialAction::CloseGroup { group_id } => {
            let g = group(s, group_id)?;
            Ok((g.epoch, group_delivery(g)))
        }
    }
}
pub fn prepare(
    s: &SocialState,
    who: &str,
    mut action: SocialAction,
    at: u64,
) -> Result<SocialEvent> {
    match &mut action {
        SocialAction::AcceptInvitation {
            invitation_id,
            admission_consent,
        } => {
            let invitation = invitation(s, invitation_id)?;
            let group = group(s, &invitation.group_id)?;
            *admission_consent = group.private.then(|| AdmissionConsent {
                group_id: group.id.clone(),
                epoch: group.epoch,
                expires_at: invitation.expires_at,
                disclosure: AdmissionDisclosure::CurrentAndFutureGroupMembers,
            });
        }
        SocialAction::SetMembers {
            group_id,
            members,
            admission_proofs,
            ..
        } => {
            let group = group(s, group_id)?;
            admission_proofs.clear();
            if group.private {
                for added in members.difference(&group.members) {
                    if let Some(proof) = s.publications.values().find(|p| {
                        p.event.actor == *added
                            && consent(&p.event).is_some_and(|c| {
                                c.group_id == *group_id
                                    && c.epoch == group.epoch
                                    && c.expires_at > at
                            })
                    }) {
                        admission_proofs.push(proof.clone());
                    }
                }
            }
        }
        _ => {}
    }
    let (resource_version, delivery) = expected(s, who, &action)?;
    let event = SocialEvent {
        schema: "booknook.social-event.v1".into(),
        id: crate::new_id(),
        actor: who.into(),
        at,
        resource_version,
        delivery,
        action,
    };
    check_event(s, &event, at)?;
    Ok(event)
}
fn check_event(s: &SocialState, e: &SocialEvent, now: u64) -> Result<()> {
    ensure(
        e.schema == "booknook.social-event.v1"
            && identifier(&e.id)
            && actor(&e.actor)
            && e.at <= now.saturating_add(60)
            && e.at <= 9_007_199_254_740_991,
        "Invalid social event",
    )?;
    ensure(!s.blocked.contains(&e.actor), "Actor is blocked")?;
    let (version, delivery) = expected(s, &e.actor, &e.action)?;
    if e.resource_version != version {
        return Err(Error::Conflict(
            "Resource version changed; missing history or approval must be refreshed".into(),
        ));
    }
    ensure(
        e.delivery == delivery,
        "Audience does not match current membership",
    )?;
    match &e.action {
        SocialAction::Friendship { peer, .. } => ensure(
            actor(peer) && peer != &e.actor && !s.blocked.contains(peer),
            "Invalid or blocked friendship recipient",
        )?,
        SocialAction::Publish { group_id, post } => {
            ensure(
                bounded(&post.body, 12_000, true)
                    && post.book.as_ref().is_none_or(book)
                    && post.stars.is_none_or(|n| (1..=5).contains(&n)),
                "Invalid published selection",
            )?;
            match post.kind {
                PostKind::Rating => ensure(
                    post.book.is_some() && post.stars.is_some(),
                    "Ratings need a book and 1–5 stars",
                )?,
                PostKind::Review => ensure(
                    post.book.is_some() && !post.body.trim().is_empty(),
                    "Reviews need a book and text",
                )?,
                PostKind::ReadingUpdate => ensure(
                    post.book.is_some()
                        && post.reading_status.as_ref().is_some_and(|v| {
                            [
                                "to-read",
                                "currently-reading",
                                "read",
                                "paused",
                                "abandoned",
                            ]
                            .contains(&v.as_str())
                        }),
                    "Reading update needs a book and status",
                )?,
                PostKind::Note | PostKind::Discussion => {
                    ensure(!post.body.trim().is_empty(), "Empty note or discussion")?
                }
            }
            ensure(
                post.reading_status.is_none() || post.kind == PostKind::ReadingUpdate,
                "Status only applies to reading updates",
            )?;
            ensure(
                post.stars.is_none() || matches!(post.kind, PostKind::Rating | PostKind::Review),
                "Stars only apply to ratings and reviews",
            )?;
            if let Some(id) = group_id {
                live_member(group(s, id)?, &e.actor)?;
            }
            ensure(
                post.kind != PostKind::Discussion || group_id.is_some(),
                "Discussions require a group",
            )?;
            if let Some(reply) = &post.reply_to {
                let prior = s
                    .publications
                    .get(reply)
                    .ok_or_else(|| Error::NotFound("Reply target".into()))?;
                ensure(
                    matches!(&prior.event.action,SocialAction::Publish { group_id: prior_group,.. } if prior_group==group_id),
                    "Reply target is in another context",
                )?;
            }
        }
        SocialAction::Retract { event_id } => {
            let prior = s
                .publications
                .get(event_id)
                .ok_or_else(|| Error::NotFound("Publication".into()))?;
            ensure(
                prior.event.actor == e.actor
                    && matches!(prior.event.action, SocialAction::Publish { .. }),
                "Only the author can retract their post",
            )?;
        }
        SocialAction::CreateList {
            id,
            name,
            description,
            books: bs,
        } => {
            ensure(
                owned_id(id, &e.actor) && !s.lists.contains_key(id),
                "List ID must be new and scoped to its owner",
            )?;
            names(name, description)?;
            books(bs)?;
        }
        SocialAction::UpdateList {
            id,
            name,
            description,
            books: bs,
        } => {
            let l = list(s, id)?;
            ensure(
                l.owner == e.actor && !l.deleted,
                "Only the owner can update an active list",
            )?;
            names(name, description)?;
            books(bs)?;
        }
        SocialAction::DeleteList { id } => {
            let l = list(s, id)?;
            ensure(
                l.owner == e.actor && !l.deleted,
                "Only the owner can delete an active list",
            )?;
        }
        SocialAction::CreateGroup {
            id,
            name,
            description,
            ..
        } => {
            ensure(
                owned_id(id, &e.actor) && !s.groups.contains_key(id),
                "Group ID must be new and scoped to its owner",
            )?;
            names(name, description)?;
        }
        SocialAction::Invite {
            id,
            group: g,
            invitee,
            expires_at,
        } => {
            validate_group(g)?;
            live_member(g, &e.actor)?;
            ensure(
                identifier(id)
                    && !s.invitations.contains_key(id)
                    && g.owner == e.actor
                    && actor(invitee)
                    && !g.members.contains(invitee)
                    && !g.banned.contains(invitee)
                    && *expires_at > e.at
                    && *expires_at <= e.at.saturating_add(30 * 86400),
                "Invalid group invitation",
            )?;
            if let Some(current) = s.groups.get(&g.id) {
                ensure(
                    current == g,
                    "Invitation snapshot differs from current group",
                )?;
            }
        }
        SocialAction::AcceptInvitation {
            invitation_id,
            admission_consent,
        } => {
            let i = invitation(s, invitation_id)?;
            let g = group(s, &i.group_id)?;
            ensure(
                i.invitee == e.actor
                    && !i.accepted
                    && i.expires_at > e.at
                    && i.epoch == g.epoch
                    && !g.closed
                    && !g.banned.contains(&e.actor),
                "Invitation expired, revoked, accepted or belongs to another reader",
            )?;
            if let Some(consent) = admission_consent {
                ensure(
                    g.private
                        && consent.group_id == g.id
                        && consent.epoch == g.epoch
                        && consent.expires_at == i.expires_at,
                    "Admission consent does not match the private invitation",
                )?;
            }
        }
        SocialAction::RequestJoin { group_id } => {
            let g = group(s, group_id)?;
            ensure(
                !g.private
                    && !g.closed
                    && !g.members.contains(&e.actor)
                    && !g.banned.contains(&e.actor),
                "Only nonmembers of an open public group may request membership",
            )?;
        }
        SocialAction::SetMembers {
            group_id,
            members,
            moderators,
            admission_proofs,
        } => {
            let g = group(s, group_id)?;
            live_member(g, &e.actor)?;
            ensure(
                g.owner == e.actor
                    && members.contains(&g.owner)
                    && members.len() <= 128
                    && members.iter().all(|v| actor(v))
                    && moderators.is_subset(members)
                    && members.is_disjoint(&g.banned),
                "Only the owner can approve valid group membership",
            )?;
            let mut proven = BTreeSet::new();
            for proof in admission_proofs {
                let consent = consent(&proof.event).ok_or_else(|| {
                    Error::Denied("Admission proof lacks explicit disclosure consent".into())
                })?;
                ensure(
                    g.private
                        && !g.members.contains(&proof.event.actor)
                        && members.contains(&proof.event.actor)
                        && proven.insert(proof.event.actor.clone())
                        && proof.event.schema == "booknook.social-event.v1"
                        && consent.group_id == g.id
                        && consent.epoch == g.epoch
                        && proof.event.resource_version == g.epoch
                        && consent.expires_at > e.at
                        && proof.approved_at <= e.at
                        && proof.event.delivery
                            == direct(
                                g.members
                                    .difference(&g.banned)
                                    .cloned()
                                    .chain([proof.event.actor.clone()]),
                            ),
                    "Admission proof has the wrong group, audience, epoch, expiry or reader",
                )?;
            }
            if g.private && members.difference(&g.members).count() > 1 {
                ensure(
                    proven.len() == members.difference(&g.members).count(),
                    "Batch admission needs every new reader's transferable consent; send a fresh invitation to readers with legacy acceptances",
                )?;
            }
            for added in members.difference(&g.members) {
                ensure(
                    proven.contains(added)
                        || (g.private
                            && s.invitations.values().any(|i| {
                                i.group_id == *group_id
                                    && i.invitee == *added
                                    && i.accepted
                                    && i.epoch == g.epoch
                                    && i.expires_at > e.at
                            }))
                        || (!g.private
                            && s.join_requests.values().any(|r| {
                                r.group_id == *group_id
                                    && r.reader == *added
                                    && r.epoch == g.epoch
                                    && r.expires_at > e.at
                            })),
                    "New members must approve a current private invitation or public membership request",
                )?;
            }
        }
        SocialAction::Moderate {
            group_id,
            event_id,
            ban_actor,
        } => {
            let g = group(s, group_id)?;
            live_member(g, &e.actor)?;
            ensure(
                g.owner == e.actor || g.moderators.contains(&e.actor),
                "Moderator permission required",
            )?;
            ensure(
                event_id.is_some() || ban_actor.is_some(),
                "Empty moderation action",
            )?;
            if let Some(id) = event_id {
                let prior = s
                    .publications
                    .get(id)
                    .ok_or_else(|| Error::NotFound("Moderated post".into()))?;
                ensure(
                    matches!(&prior.event.action,SocialAction::Publish { group_id:Some(id),.. } if id==group_id),
                    "Moderation target is outside this group",
                )?;
            }
            if let Some(who) = ban_actor {
                ensure(
                    actor(who)
                        && who != &g.owner
                        && (g.owner == e.actor || !g.moderators.contains(who)),
                    "Cannot ban the owner or a peer moderator",
                )?;
            }
        }
        SocialAction::CloseGroup { group_id } => {
            let g = group(s, group_id)?;
            live_member(g, &e.actor)?;
            ensure(g.owner == e.actor, "Only the owner can close the group")?;
        }
    }
    ensure(
        canonical(e)?.len() <= 45_000,
        "Social event exceeds the publication limit",
    )
}
fn apply(s: &mut SocialState, e: &SocialEvent) -> Result<()> {
    match &e.action {
        SocialAction::Friendship { peer, requested } => {
            s.friendship_intents.insert(
                pair(&e.actor, peer),
                FriendshipIntent {
                    requested: *requested,
                    version: bump(e.resource_version)?,
                },
            );
        }
        SocialAction::Publish { .. } => {}
        SocialAction::Retract { event_id } => {
            s.retracted.insert(event_id.clone());
            s.outbox.remove(event_id);
        }
        SocialAction::CreateList {
            id,
            name,
            description,
            books,
        } => {
            s.lists.insert(
                id.clone(),
                SharedList {
                    id: id.clone(),
                    owner: e.actor.clone(),
                    name: name.clone(),
                    description: description.clone(),
                    books: books.clone(),
                    version: 1,
                    deleted: false,
                },
            );
        }
        SocialAction::UpdateList {
            id,
            name,
            description,
            books,
        } => {
            let l = s
                .lists
                .get_mut(id)
                .ok_or_else(|| Error::NotFound("List".into()))?;
            l.name = name.clone();
            l.description = description.clone();
            l.books = books.clone();
            l.version = bump(l.version)?;
        }
        SocialAction::DeleteList { id } => {
            let l = s
                .lists
                .get_mut(id)
                .ok_or_else(|| Error::NotFound("List".into()))?;
            l.deleted = true;
            l.version = bump(l.version)?;
        }
        SocialAction::CreateGroup {
            id,
            name,
            description,
            private,
        } => {
            s.groups.insert(
                id.clone(),
                ReadingGroup {
                    id: id.clone(),
                    owner: e.actor.clone(),
                    name: name.clone(),
                    description: description.clone(),
                    private: *private,
                    epoch: 1,
                    members: [e.actor.clone()].into(),
                    moderators: Default::default(),
                    banned: Default::default(),
                    closed: false,
                },
            );
        }
        SocialAction::Invite {
            id,
            group: g,
            invitee,
            expires_at,
        } => {
            s.groups.entry(g.id.clone()).or_insert_with(|| g.clone());
            s.invitations.insert(
                id.clone(),
                Invitation {
                    id: id.clone(),
                    group_id: g.id.clone(),
                    inviter: e.actor.clone(),
                    invitee: invitee.clone(),
                    epoch: g.epoch,
                    expires_at: *expires_at,
                    accepted: false,
                },
            );
        }
        SocialAction::AcceptInvitation { invitation_id, .. } => {
            s.invitations
                .get_mut(invitation_id)
                .ok_or_else(|| Error::NotFound("Invitation".into()))?
                .accepted = true;
        }
        SocialAction::RequestJoin { group_id } => {
            let key = pair(group_id, &e.actor);
            let request = JoinRequest {
                id: e.id.clone(),
                group_id: group_id.clone(),
                reader: e.actor.clone(),
                epoch: e.resource_version,
                expires_at: e.at.saturating_add(7 * 86400),
            };
            // Repeated valid consent may arrive in either order. Choose the newest signed
            // request (ID breaks timestamp ties), preserving every receipt in publications.
            // check_event has already rejected requests for any stale membership epoch.
            if s.join_requests.get(&key).is_none_or(|prior| {
                (request.epoch, request.expires_at, &request.id)
                    > (prior.epoch, prior.expires_at, &prior.id)
            }) {
                s.join_requests.insert(key, request);
            }
        }
        SocialAction::SetMembers {
            group_id,
            members,
            moderators,
            ..
        } => {
            let g = s
                .groups
                .get_mut(group_id)
                .ok_or_else(|| Error::NotFound("Group".into()))?;
            g.members = members.clone();
            g.moderators = moderators.clone();
            g.epoch = bump(g.epoch)?;
        }
        SocialAction::Moderate {
            group_id,
            event_id,
            ban_actor,
        } => {
            if let Some(id) = event_id {
                s.retracted.insert(id.clone());
                s.outbox.remove(id);
            }
            if let Some(who) = ban_actor {
                let g = s
                    .groups
                    .get_mut(group_id)
                    .ok_or_else(|| Error::NotFound("Group".into()))?;
                g.banned.insert(who.clone());
                g.moderators.remove(who);
                g.epoch = bump(g.epoch)?;
            }
        }
        SocialAction::CloseGroup { group_id } => {
            let g = s
                .groups
                .get_mut(group_id)
                .ok_or_else(|| Error::NotFound("Group".into()))?;
            g.closed = true;
            g.epoch = bump(g.epoch)?;
        }
    }
    Ok(())
}
/// Validate a previously approved publication at its signed creation time. Pin revocation is
/// evaluated using the caller's current independently configured policy on every invocation.
pub fn verify_publication(
    p: &ApprovedPublication,
    policy: &AuthorityPolicy,
    now: u64,
) -> Result<authority::VerifiedAuthorization> {
    ensure(
        p.schema == "booknook.approved-publication.v1"
            && policy.action == ACTION
            && policy.actor == p.event.actor
            && p.approved_at <= now.saturating_add(60)
            && p.event.at <= p.approved_at
            && p.approved_at - p.event.at <= 60,
        "Publication policy mismatch",
    )?;
    ensure(
        canonical(p)?.len() <= MAX_PUBLICATION_BYTES,
        "Publication is too large",
    )?;
    if let SocialAction::SetMembers {
        admission_proofs, ..
    } = &p.event.action
    {
        for proof in admission_proofs {
            // Only a leaf acceptance with explicit redistribution consent is legal. This
            // prevents recursive membership proofs and never imports invitation plaintext.
            ensure(
                consent(&proof.event).is_some(),
                "Admission proof lacks explicit disclosure consent",
            )?;
            let mut consent_policy = policy.clone();
            consent_policy.actor = proof.event.actor.clone();
            verify_publication(proof, &consent_policy, now)?;
        }
    }
    authority::verify_receipt(
        p.request_json.as_bytes(),
        p.presentation_json.as_bytes(),
        policy,
        &canonical(&p.event)?,
        p.event.resource_version,
        p.approved_at,
    )
}
/// Receiver must call this only after verifying the transport author binding and decryption.
/// An exact duplicate is idempotent; an altered ID or reused approval nonce is denied.
pub fn receive(
    s: &mut SocialState,
    p: ApprovedPublication,
    policy: &AuthorityPolicy,
    now: u64,
) -> Result<bool> {
    let auth = verify_publication(&p, policy, now)?;
    ensure(!s.blocked.contains(&p.event.actor), "Actor is blocked")?;
    if let Some(old) = s.publications.get(&p.event.id) {
        ensure(old == &p, "Social event ID collision")?;
        return Ok(false);
    }
    check_event(s, &p.event, now)?;
    let hash = digest(&canonical(&p.event)?);
    ensure(
        !s.replay.contains_key(auth.replay_key()),
        "Approval nonce already consumed",
    )?;
    let mut next = s.clone();
    apply(&mut next, &p.event)?;
    next.replay.insert(auth.replay_key().into(), hash);
    next.publications.insert(p.event.id.clone(), p);
    next.revision = bump(s.revision)?;
    *s = next;
    Ok(true)
}
/// Local publication is a single state transaction: approval, projection, replay consumption,
/// and durable outbox insertion. The platform commits this before sending any network request.
pub fn approve(
    s: &mut SocialState,
    event: SocialEvent,
    request_json: String,
    presentation_json: String,
    policy: &AuthorityPolicy,
    now: u64,
) -> Result<ApprovedPublication> {
    check_event(s, &event, now)?;
    let auth = authority::verify_receipt(
        request_json.as_bytes(),
        presentation_json.as_bytes(),
        policy,
        &canonical(&event)?,
        event.resource_version,
        now,
    )?;
    auth.require(ACTION, &event.actor, &canonical(&event)?, now)?;
    ensure(
        event.at <= now && now - event.at <= 60,
        "Refresh the publication timestamp before approving",
    )?;
    let publication = ApprovedPublication {
        schema: "booknook.approved-publication.v1".into(),
        event,
        approved_at: now,
        request_json,
        presentation_json,
    };
    let mut next = s.clone();
    receive(&mut next, publication.clone(), policy, now)?;
    next.drafts.remove(&publication.event.id);
    next.outbox.insert(
        publication.event.id.clone(),
        OutboxItem {
            publication: publication.clone(),
            acknowledged: Default::default(),
        },
    );
    *s = next;
    Ok(publication)
}
/// Recheck authorization/membership before retrying an outbox item after an outage. Receipts
/// remain historical approval evidence; changed membership/ownership invalidates the retry.
pub fn validate_outgoing(
    s: &SocialState,
    p: &ApprovedPublication,
    policy: &AuthorityPolicy,
    now: u64,
) -> Result<()> {
    verify_publication(p, policy, now)?;
    ensure(
        s.outbox
            .get(&p.event.id)
            .is_some_and(|i| i.publication == *p),
        "Publication is not an approved outbox item",
    )?;
    ensure(!s.blocked.contains(&p.event.actor), "Actor is blocked")?;
    ensure(
        !s.retracted.contains(&p.event.id),
        "Publication was retracted or removed by moderation",
    )?;

    if let Delivery::Direct { recipients } = &p.event.delivery {
        ensure(
            recipients.is_disjoint(&s.blocked),
            "A recipient was blocked before delivery",
        )?;
    }
    if let SocialAction::Publish {
        group_id: Some(id), ..
    } = &p.event.action
    {
        let g = group(s, id)?;
        live_member(g, &p.event.actor)?;
        ensure(
            p.event.resource_version == g.epoch && p.event.delivery == group_delivery(g),
            "Membership changed before sending; review a new publication",
        )?;
    }
    Ok(())
}
pub fn mark_delivered(
    s: &mut SocialState,
    event_id: &str,
    recipient: &str,
    relay_ids: BTreeSet<String>,
) -> Result<bool> {
    ensure(
        !relay_ids.is_empty() && relay_ids.iter().all(|r| r.starts_with("wss://")),
        "A relay acknowledgment is required",
    )?;
    let mut next = s.clone();
    let item = next
        .outbox
        .get_mut(event_id)
        .ok_or_else(|| Error::NotFound("Outbox item".into()))?;
    let recipients = match &item.publication.event.delivery {
        Delivery::Public => ["public".into()].into(),
        Delivery::Direct { recipients } => recipients.clone(),
    };
    ensure(
        recipients.contains(recipient),
        "Acknowledgment recipient mismatch",
    )?;
    item.acknowledged
        .entry(recipient.into())
        .or_default()
        .extend(relay_ids);
    let complete = recipients
        .iter()
        .all(|r| item.acknowledged.get(r).is_some_and(|a| !a.is_empty()));
    if complete {
        next.outbox.remove(event_id);
    }
    next.revision = bump(next.revision)?;
    *s = next;
    Ok(complete)
}
pub fn local(s: &mut SocialState, command: LocalSocialCommand) -> Result<()> {
    let mut next = s.clone();
    match command {
        LocalSocialCommand::SaveDraft { event } => {
            check_event(s, &event, event.at)?;
            ensure(
                !s.publications.contains_key(&event.id),
                "Published events cannot become drafts",
            )?;
            next.drafts.insert(event.id.clone(), *event);
        }
        LocalSocialCommand::DeleteDraft { id } => {
            next.drafts.remove(&id);
        }
        LocalSocialCommand::CancelDelivery { event_id } => {
            next.outbox.remove(&event_id);
        }
        LocalSocialCommand::Block {
            actor: who,
            blocked,
        } => {
            ensure(actor(&who), "Invalid blocked actor")?;
            if blocked {
                next.blocked.insert(who);
            } else {
                next.blocked.remove(&who);
            }
        }
        LocalSocialCommand::Hide { event_id, hidden } => {
            if hidden {
                next.hidden.insert(event_id);
            } else {
                next.hidden.remove(&event_id);
            }
        }
        LocalSocialCommand::SetSubjects { subjects } => {
            ensure(
                subjects.len() <= 100 && subjects.iter().all(|v| bounded(v, 100, false)),
                "Invalid preferred subjects",
            )?;
            next.preferred_subjects = subjects;
        }
    }
    next.revision = bump(next.revision)?;
    *s = next;
    Ok(())
}
pub fn friends(s: &SocialState, a: &str, b: &str) -> bool {
    !s.blocked.contains(b)
        && !s.blocked.contains(a)
        && [pair(a, b), pair(b, a)]
            .iter()
            .all(|p| s.friendship_intents.get(p).is_some_and(|v| v.requested))
}
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FeedItem {
    pub id: String,
    pub actor: String,
    pub at: u64,
    pub group_id: Option<String>,
    pub post: Post,
    pub spoiler_hidden: bool,
}
pub fn feed(s: &SocialState, viewer: &str, reveal_spoilers: bool) -> Vec<FeedItem> {
    let mut items = Vec::new();
    for p in s.publications.values() {
        let e = &p.event;
        let SocialAction::Publish { group_id, post } = &e.action else {
            continue;
        };
        if s.blocked.contains(&e.actor) || s.hidden.contains(&e.id) || s.retracted.contains(&e.id) {
            continue;
        }
        let visible = if let Some(id) = group_id {
            s.groups.get(id).is_some_and(|g| {
                !g.closed && !g.banned.contains(viewer) && g.members.contains(viewer)
            })
        } else {
            e.actor == viewer || friends(s, viewer, &e.actor)
        };
        if !visible {
            continue;
        }
        let spoiler_hidden = post.spoiler && !reveal_spoilers;
        let mut post = post.clone();
        if spoiler_hidden {
            post.body.clear();
        }
        items.push(FeedItem {
            id: e.id.clone(),
            actor: e.actor.clone(),
            at: e.at,
            group_id: group_id.clone(),
            post,
            spoiler_hidden,
        });
    }
    items.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.id.cmp(&b.id)));
    items
}
pub fn discover_groups(s: &SocialState, query: &str) -> Vec<ReadingGroup> {
    let q = query.to_lowercase();
    s.groups
        .values()
        .filter(|g| {
            !g.private
                && !g.closed
                && !s.blocked.contains(&g.owner)
                && (g.name.to_lowercase().contains(&q) || g.description.to_lowercase().contains(&q))
        })
        .cloned()
        .collect()
}
/// Rebuild shared projections from signed history using independently configured policies.
/// Local preferences, drafts and delivery bookkeeping are not shared authority.
pub fn audit(
    state: &SocialState,
    mut policy_for_actor: impl FnMut(&str) -> Result<AuthorityPolicy>,
    at: u64,
) -> Result<()> {
    validate(state)?;
    let mut rebuilt = SocialState::default();
    let mut remaining: Vec<_> = causal_replay_order(state.publications.values())
        .into_iter()
        .cloned()
        .collect();
    while !remaining.is_empty() {
        let mut deferred = Vec::new();
        let previous = remaining.len();
        for publication in remaining {
            let policy = policy_for_actor(&publication.event.actor)?;
            match receive(&mut rebuilt, publication.clone(), &policy, at) {
                Ok(_) => {}
                Err(Error::NotFound(_) | Error::Conflict(_)) => deferred.push(publication),
                Err(error) => return Err(error),
            }
        }
        if deferred.len() == previous {
            return Err(denied(
                "Social history cannot be verified with current identity settings; missing dependency or authorization",
            ));
        }
        remaining = deferred;
    }
    if state.groups != rebuilt.groups
        || state.lists != rebuilt.lists
        || state.invitations != rebuilt.invitations
        || state.join_requests != rebuilt.join_requests
        || state.friendship_intents != rebuilt.friendship_intents
        || state.retracted != rebuilt.retracted
        || state.replay != rebuilt.replay
    {
        return Err(denied(
            "Stored social projection does not match its signed history",
        ));
    }
    Ok(())
}

/// Safe recommendation entry point for a host's persisted or imported social snapshot.
/// The policy's actor is the viewer. Other authors use the same independent issuer pins,
/// audience/caller and action, matching the native reference application's trust model.
pub fn audited_recommendations(
    state: &SocialState,
    policy: &AuthorityPolicy,
    candidates: Vec<crate::social::CatalogCandidate>,
    excluded: BTreeSet<String>,
    now: u64,
) -> Result<Vec<crate::social::Recommendation>> {
    ensure(
        policy.action == ACTION && actor(&policy.actor),
        "Invalid recommendation policy",
    )?;
    audit(
        state,
        |who| {
            let mut author_policy = policy.clone();
            author_policy.actor = who.into();
            Ok(author_policy)
        },
        now,
    )?;
    Ok(recommendations(state, &policy.actor, candidates, excluded))
}

/// Recommendations use only public friend ratings/reviews and the reader's chosen interests.
/// Private group posts and private notebook contents cannot enter this input.
pub fn recommendations(
    s: &SocialState,
    viewer: &str,
    candidates: Vec<crate::social::CatalogCandidate>,
    excluded: BTreeSet<String>,
) -> Vec<crate::social::Recommendation> {
    crate::social::recommend(&recommendation_input(s, viewer, candidates, excluded))
}
/// Build the minimized input after the caller audits social evidence. Contains no notebook data.
pub fn recommendation_input(
    s: &SocialState,
    viewer: &str,
    candidates: Vec<crate::social::CatalogCandidate>,
    excluded: BTreeSet<String>,
) -> crate::social::RecommendationInput {
    let mut latest: BTreeMap<(String, String), (u64, String, u8)> = BTreeMap::new();
    let mut fs = BTreeSet::new();
    for p in s.publications.values() {
        let e = &p.event;
        if !matches!(e.delivery, Delivery::Public)
            || s.retracted.contains(&e.id)
            || !friends(s, viewer, &e.actor)
        {
            continue;
        }
        if let SocialAction::Publish {
            group_id: None,
            post,
        } = &e.action
            && let (Some(b), Some(stars)) = (&post.book, post.stars)
        {
            fs.insert(e.actor.clone());
            let key = (e.actor.clone(), b.id.clone());
            let value = (e.at, e.id.clone(), stars);
            if latest
                .get(&key)
                .is_none_or(|old| (old.0, &old.1) < (value.0, &value.1))
            {
                latest.insert(key, value);
            }
        }
    }
    crate::social::RecommendationInput {
        candidates,
        preferred_subjects: s.preferred_subjects.iter().cloned().collect(),
        shared_ratings: latest
            .into_iter()
            .map(
                |((actor, book_id), (_, _, stars))| crate::social::SharedRating {
                    actor,
                    book_id,
                    stars,
                },
            )
            .collect(),
        friends: fs,
        excluded,
    }
}
/// Storage-level structural validation. Network inputs must still pass receive/approval.
pub fn validate(s: &SocialState) -> Result<()> {
    ensure(s.schema == SCHEMA, "Unsupported social state schema")?;
    for (id, p) in &s.publications {
        ensure(
            id == &p.event.id
                && p.schema == "booknook.approved-publication.v1"
                && canonical(p)?.len() <= MAX_PUBLICATION_BYTES,
            "Invalid stored publication",
        )?;
    }
    for (id, d) in &s.drafts {
        ensure(
            id == &d.id && !s.publications.contains_key(id),
            "Invalid stored draft",
        )?;
    }
    for (id, g) in &s.groups {
        ensure(id == &g.id, "Group key mismatch")?;
        validate_group(g)?;
    }
    for (id, l) in &s.lists {
        ensure(
            id == &l.id && owned_id(id, &l.owner) && l.version > 0,
            "Invalid stored list",
        )?;
        names(&l.name, &l.description)?;
        books(&l.books)?;
    }
    for (key, request) in &s.join_requests {
        ensure(
            key == &pair(&request.group_id, &request.reader)
                && identifier(&request.id)
                && actor(&request.reader)
                && request.epoch > 0
                && request.expires_at <= 9_007_199_254_740_991
                && s.groups.get(&request.group_id).is_some_and(|g| !g.private),
            "Invalid stored membership request",
        )?;
    }
    for (id, item) in &s.outbox {
        ensure(
            id == &item.publication.event.id && s.publications.get(id) == Some(&item.publication),
            "Invalid outbox projection",
        )?;
    }
    ensure(s.blocked.iter().all(|a| actor(a)), "Invalid block list")
}
