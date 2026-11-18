//! Unified search over history and snippets (design section 7.2).
//!
//! - Empty filter: history in `last_used` order.
//! - With a filter: snippets first, then clips, each ranked by FTS score with
//!   a boost (recency for clips, use count for snippets).

use crate::history::HistoryStore;
use crate::model::{Clip, ClipKind, Snippet};
use crate::snippets::SnippetStore;
use crate::Result;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum SearchFilter {
    All,
    Text,
    Image,
    File,
    Snippet,
}

impl SearchFilter {
    pub fn parse(s: &str) -> Option<SearchFilter> {
        match s {
            "all" => Some(SearchFilter::All),
            "snippet" | "snippets" => Some(SearchFilter::Snippet),
            other => ClipKind::parse(other).map(|k| match k {
                ClipKind::Text => SearchFilter::Text,
                ClipKind::Image => SearchFilter::Image,
                ClipKind::File => SearchFilter::File,
            }),
        }
    }

    fn clip_kind(self) -> Option<ClipKind> {
        match self {
            SearchFilter::Text => Some(ClipKind::Text),
            SearchFilter::Image => Some(ClipKind::Image),
            SearchFilter::File => Some(ClipKind::File),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(tag = "type", content = "item", rename_all = "lowercase")]
pub enum SearchHit {
    Clip(Clip),
    Snippet(Snippet),
}

pub fn search(
    history: &HistoryStore,
    snippets: &SnippetStore,
    query: &str,
    filter: SearchFilter,
    limit: usize,
    now: i64,
) -> Result<Vec<SearchHit>> {
    let query = query.trim();
    let mut out = Vec::new();
    let want_snippets = match filter {
        SearchFilter::Snippet => true,
        SearchFilter::All => !query.is_empty(),
        _ => false,
    };
    if want_snippets {
        out.extend(snippets.search(query, limit)?.into_iter().map(|(s, _)| SearchHit::Snippet(s)));
    }
    if filter != SearchFilter::Snippet && out.len() < limit {
        let clips = history.search(query, filter.clip_kind(), limit - out.len(), now)?;
        out.extend(clips.into_iter().map(|(c, _)| SearchHit::Clip(c)));
    }
    Ok(out)
}

pub(crate) enum QueryPlan {
    Empty,
    /// An FTS5 MATCH expression.
    Fts(String),
    /// Tokens to AND together with LIKE; used when any token is too short
    /// for the trigram tokenizer.
    Like(Vec<String>),
}

/// The trigram tokenizer cannot match terms shorter than three characters,
/// so short queries fall back to LIKE.
pub(crate) fn plan(query: &str) -> QueryPlan {
    let tokens: Vec<&str> = query.split_whitespace().collect();
    if tokens.is_empty() {
        return QueryPlan::Empty;
    }
    if tokens.iter().any(|t| t.chars().count() < 3) {
        return QueryPlan::Like(tokens.into_iter().map(String::from).collect());
    }
    let expr = tokens
        .iter()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ");
    QueryPlan::Fts(expr)
}

pub(crate) fn like_pattern(token: &str) -> String {
    let mut p = String::with_capacity(token.len() + 2);
    p.push('%');
    for c in token.chars() {
        if matches!(c, '%' | '_' | '\\') {
            p.push('\\');
        }
        p.push(c);
    }
    p.push('%');
    p
}

/// Weight of the boost relative to normalized text relevance.
const BOOST_WEIGHT: f64 = 0.3;

/// Re-rank hits in place. `hits` carry a raw relevance (higher is better);
/// it is normalized to `0..=1` and a boost in `0..=1` is added with a small
/// weight, so relevance dominates and the boost breaks near-ties.
pub(crate) fn rank<T>(hits: &mut [(T, f64)], boost: impl Fn(&T) -> f64) {
    let max = hits.iter().map(|(_, s)| *s).fold(0.0_f64, f64::max);
    for (item, score) in hits.iter_mut() {
        let rel = if max > 0.0 { (*score / max).clamp(0.0, 1.0) } else { 1.0 };
        *score = rel + BOOST_WEIGHT * boost(item).clamp(0.0, 1.0);
    }
    hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
}
