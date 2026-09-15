//! TOON renderer (bead twitter_cli-5o3.5.9).
//!
//! Third renderer over the SAME envelope model as JSON/YAML (plan §5.1): a
//! renderer, not a different data model. Tabular arrays for lists
//! (`tweets[N]{id,text,…}`). Errors always stay JSON/YAML, never TOON.
//!
//! Token savings are workload-dependent — no fixed percentage is claimed
//! anywhere (reviewed constraint on this bead).

use serde::Serialize;

use crate::envelope::Envelope;

/// Render a success envelope's `data` to TOON. Errors are never TOON —
/// callers must route `Envelope::Err` to JSON/YAML.
pub fn render_data_toon<T: Serialize>(envelope: &Envelope<T>) -> Option<String> {
    match envelope {
        Envelope::Ok { data, .. } => {
            let value = serde_json::to_value(data).ok()?;
            Some(toon::encode(&value, None))
        }
        Envelope::Err { .. } => None,
    }
}

/// Emit TOON to stdout (success path) — sibling of [`crate::emit`].
pub fn emit_toon<T: Serialize>(envelope: &Envelope<T>) {
    if let Some(text) = render_data_toon(envelope) {
        println!("{text}");
    } else {
        crate::emit(envelope);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tabular_arrays_render_compactly() {
        let env = Envelope::ok(
            "tweet_list",
            json!({"tweets": [{"id": "1", "text": "hi"}, {"id": "2", "text": "yo"}]}),
        );
        let toon = render_data_toon(&env).unwrap();
        assert!(toon.contains("tweets[2]"), "{toon}");
        assert!(toon.contains("hi"));
    }

    #[test]
    fn errors_never_render_toon() {
        let err = crate::TwrError::new(crate::ErrorKind::NotFound, "gone");
        let env: Envelope<serde_json::Value> = Envelope::err(err);
        assert!(render_data_toon(&env).is_none());
    }
}
