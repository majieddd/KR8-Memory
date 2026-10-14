//! Escaped, bounded inert evidence cards. No caller receives standing authority.

use crate::ProjectedRecord;

/// Strict output bounds. Bytes also conservatively bound tokenizer input units.
#[derive(Clone, Copy, Debug)]
pub struct RenderBudget {
    /// Maximum UTF-8 bytes, including delimiters and citations.
    pub max_bytes: usize,
    /// Conservative byte-per-token ceiling; no token estimator can expand it.
    pub max_tokens: usize,
}

/// Escape semantic delimiters in both evidence prose and citation attributes.
pub fn escape_evidence(text: &str) -> String {
    let mut escaped = String::new();
    for ch in text.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            '\0' => {}
            _ => escaped.push(ch),
        }
    }
    escaped
}

/// Render whole active evidence cards only; records too large for the budget are omitted.
pub fn render_evidence(records: &[ProjectedRecord], budget: RenderBudget) -> String {
    let ceiling = budget.max_bytes.min(budget.max_tokens).min(16_384);
    let start = "<kr8-memory authority=\"evidence\">\n";
    let end = "</kr8-memory>";
    let mut body = String::new();
    for projected in records
        .iter()
        .take(32)
        .filter(|record| record.status.searchable())
    {
        let record = &projected.record;
        if record.validate().is_err() {
            continue;
        }
        let payload = record.payload();
        let Ok(place) = payload.place.canonical() else {
            continue;
        };
        let mut card = format!("<record id=\"{}\" sha256=\"{}\" profile=\"{}\" place=\"{}\" author=\"{}\" created-at-ms=\"{}\" status=\"{}\">\n{}\n", escape_evidence(record.id().as_str()), record.hash(), payload.profile.as_str(), escape_evidence(&place), escape_evidence(&payload.author), payload.created_at_ms, projected.status.label(), escape_evidence(&payload.text));
        for source in &payload.sources {
            let state = projected
                .source_states
                .get(&source.id)
                .copied()
                .unwrap_or(source.availability);
            card.push_str(&format!(
                "<source id=\"{}\" sha256=\"{}\" availability=\"{}\" />\n",
                escape_evidence(source.id.as_str()),
                source.sha256,
                state.label()
            ));
        }
        card.push_str("</record>\n");
        if start.len() + body.len() + card.len() + end.len() <= ceiling {
            body.push_str(&card);
        }
    }
    if body.is_empty() {
        String::new()
    } else {
        format!("{start}{body}{end}")
    }
}
