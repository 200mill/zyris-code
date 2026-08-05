//! Fits the announced tool definitions into a **token budget**.
//!
//! The doc comments upstream (zyris-caps) become this node's tool descriptions verbatim. Those
//! descriptions ride in the agent's context at session creation and every turn — rich examples,
//! caveats, and path-resolution notes all loaded would let a single file_io eat hundreds of
//! tokens. Keep the name and the schema's value-semantics parts, cut **only the descriptions**,
//! and what the agent needs to pick a tool (what it does) survives while the repetition falls away.
//!
//! `dispatch` does not read descriptions — cutting here only touches what gets announced.

use serde_json::Value;
use zyris::CapabilityDescriptor;

/// Budget for one tool description (bytes). The first sentence is the core.
pub const DESCRIPTION_LIMIT: usize = 200;
/// Budget for parameter descriptions inside the schema.
pub const PARAM_LIMIT: usize = 80;

/// Fits a description to the budget. **Cut at a period or newline** — slicing mid-sentence
/// hides the "why". A single `…` says it was cut.
pub fn clip(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let cut = text.floor_char_boundary(limit);
    let end = text[..cut]
        .rfind(['.', '\n'])
        .map(|i| i + 1)
        .unwrap_or(cut);
    let mut out = text[..end].trim_end().to_string();
    out.push('…');
    out
}

/// Fits the descriptions inside a schema JSON to the budget. Cuts only the strings under the
/// `description` key — leaves what is used to interpret values — types, defaults, enums — alone.
pub fn clip_schema(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(s)) = map.get_mut("description") {
                *s = clip(s, PARAM_LIMIT);
            }
            for child in map.values_mut() {
                clip_schema(child);
            }
        }
        Value::Array(items) => {
            for child in items {
                clip_schema(child);
            }
        }
        _ => {}
    }
}

/// Fits one capability descriptor to the budget.
pub fn trim_descriptor(descriptor: &mut CapabilityDescriptor) {
    for tool in &mut descriptor.tools {
        tool.description = clip(&tool.description, DESCRIPTION_LIMIT);
        clip_schema(&mut tool.request_schema);
        if let Some(schema) = &mut tool.response_schema {
            clip_schema(schema);
        }
        if let Some(schema) = &mut tool.item_schema {
            clip_schema(schema);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use zyris::ServeCapability;

    #[test]
    fn a_short_description_is_left_alone() {
        assert_eq!(clip("Read a file.", DESCRIPTION_LIMIT), "Read a file.");
        assert_eq!(clip("", 10), "");
    }

    /// Does not cut mid-sentence — the cut lands after a period or newline.
    #[test]
    fn a_long_description_is_cut_at_a_sentence_boundary() {
        let text = "Read a file's text. Large files come back truncated, and you read on \
                    by passing an offset, which is described in more detail further down \
                    this sentence that has to go on for a while.";
        let out = clip(text, 40);
        assert_eq!(out, "Read a file's text.…");
        assert!(out.len() <= DESCRIPTION_LIMIT);
    }

    /// Cuts only `description` and leaves the type — what interprets values must not be touched.
    #[test]
    fn schema_descriptions_are_trimmed_but_the_shape_stays() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "A path that goes on and on and on far beyond the budget for a parameter help string, with nothing new to say."
                }
            }
        });
        clip_schema(&mut schema);
        let desc = schema["properties"]["path"]["description"].as_str().unwrap();
        // `…` is 3 bytes, so allow budget + 3.
        assert!(desc.len() <= PARAM_LIMIT + 3, "{desc}");
        assert_eq!(schema["properties"]["path"]["type"], "string");
    }

    /// Does the actually-announced file_io description fit the budget? Gate calls this function,
    /// so passing here means what the agent receives passed.
    #[test]
    fn the_announced_file_io_fits_the_budget() {
        let dir = tempfile::tempdir().unwrap();
        let gate = crate::tools::guard::Gate::new(
            crate::tools::readonly::ReadOnlyFileIo::new(dir.path().to_path_buf()),
            crate::tools::bridge::Bridge::new(),
        );
        for tool in gate.descriptor().tools {
            assert!(
                tool.description.len() <= DESCRIPTION_LIMIT,
                "{}: {}",
                tool.name,
                tool.description
            );
        }
    }
}
