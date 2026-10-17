use crate::{Error, Result, model::*};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub fn friends(state: &State, a: &str, b: &str) -> bool {
    !state.blocked.contains(b)
        && state.friendship_requests.contains(&(a.into(), b.into()))
        && state.friendship_requests.contains(&(b.into(), a.into()))
}

pub fn feed(state: &State, reader: &str) -> Vec<Activity> {
    let mut result: Vec<_> = state
        .activity
        .values()
        .filter(|a| {
            !state.blocked.contains(&a.actor)
                && (a.actor == reader || friends(state, reader, &a.actor))
                && a.group_id.as_ref().is_none_or(|id| {
                    state
                        .groups
                        .get(id)
                        .is_some_and(|g| !g.private || g.members.contains(reader))
                })
        })
        .cloned()
        .collect();
    result.sort_by(|a, b| {
        b.published_at
            .cmp(&a.published_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    result
}

/// Called only after transport signature, Castalia binding and current authority verification.
/// VerifiedAuthorization cannot be constructed by consumers without validating a v2 receipt.
pub fn receive(
    state: &mut State,
    activity: Activity,
    auth: &crate::authority::VerifiedAuthorization,
    now: u64,
) -> Result<bool> {
    auth.require(
        "social.publish",
        &activity.actor,
        &crate::canonical(&activity)?,
        activity.published_at,
    )?;
    auth.require_version(activity.group_epoch.unwrap_or(0))?;
    if state.blocked.contains(&activity.actor) {
        return Err(Error::Denied("Actor is blocked".into()));
    }
    if activity.published_at > now.saturating_add(60) {
        return Err(Error::Invalid("Future activity timestamp".into()));
    }
    if let Some(id) = &activity.group_id {
        let group = state
            .groups
            .get(id)
            .ok_or_else(|| Error::Denied("Unknown group".into()))?;
        if !group.members.contains(&activity.actor) || activity.group_epoch != Some(group.epoch) {
            return Err(Error::Denied("Membership epoch or member mismatch".into()));
        }
        if group.private {
            return Err(Error::Denied(
                "Private group activity requires a qualified encrypted group transport".into(),
            ));
        }
    }
    if let Some(existing) = state.activity.get(&activity.id) {
        if existing != &activity {
            return Err(Error::Conflict("Event ID collision".into()));
        }
        return Ok(false);
    }
    state.activity.insert(activity.id.clone(), activity);
    Ok(true)
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MembershipChange {
    pub group_id: String,
    pub owner: String,
    pub expected_epoch: u64,
    pub members: BTreeSet<String>,
}

/// Membership changes require a fresh receipt for these exact members and the current epoch.
/// Group key distribution is a separate provider operation; this never sends a key.
pub fn update_members(
    group: &mut Group,
    change: &MembershipChange,
    auth: &crate::authority::VerifiedAuthorization,
    now: u64,
) -> Result<()> {
    auth.require(
        "group.membership",
        &change.owner,
        &crate::canonical(change)?,
        now,
    )?;
    auth.require_version(change.expected_epoch)?;
    if change.group_id != group.id
        || change.owner != group.owner
        || change.expected_epoch != group.epoch
        || !change.members.contains(&change.owner)
    {
        return Err(Error::Denied("Invalid membership change".into()));
    }
    let next_epoch = group
        .epoch
        .checked_add(1)
        .ok_or_else(|| Error::Invalid("Group epoch overflow".into()))?;
    group.members = change.members.clone();
    group.moderators.retain(|m| group.members.contains(m));
    group.epoch = next_epoch;
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogCandidate {
    pub book_id: String,
    pub title: String,
    pub subjects: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SharedRating {
    pub actor: String,
    pub book_id: String,
    pub stars: u8,
}
/// This type intentionally has no notebook, annotation or card fields.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecommendationInput {
    pub candidates: Vec<CatalogCandidate>,
    pub preferred_subjects: Vec<String>,
    pub shared_ratings: Vec<SharedRating>,
    pub friends: BTreeSet<String>,
    pub excluded: BTreeSet<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Recommendation {
    pub book_id: String,
    pub score: u32,
    pub reasons: Vec<String>,
}

pub fn recommend(input: &RecommendationInput) -> Vec<Recommendation> {
    let mut results = BTreeMap::new();
    let interests: BTreeSet<_> = input
        .preferred_subjects
        .iter()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    for book in &input.candidates {
        if input.excluded.contains(&book.book_id) {
            continue;
        }
        let mut reasons = Vec::new();
        let mut score = 0;
        let subjects: BTreeSet<_> = book
            .subjects
            .iter()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        for subject in subjects {
            if interests.contains(&subject) {
                score += 2;
                reasons.push(format!("Matches your interest in {subject}"));
            }
        }
        for rating in &input.shared_ratings {
            if rating.book_id == book.book_id
                && (4..=5).contains(&rating.stars)
                && input.friends.contains(&rating.actor)
            {
                score += u32::from(rating.stars);
                reasons.push(format!(
                    "{} shared a {}-star rating",
                    rating.actor, rating.stars
                ));
            }
        }
        if score > 0 {
            results.insert(
                book.book_id.clone(),
                Recommendation {
                    book_id: book.book_id.clone(),
                    score,
                    reasons,
                },
            );
        }
    }
    let mut results: Vec<_> = results.into_values().collect();
    results.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.book_id.cmp(&b.book_id))
    });
    results
}
