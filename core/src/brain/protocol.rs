//! Her action blocks: fenced ```maya blocks in a reply, one JSON action each.

use serde_json::Value;

pub struct Parsed {
    /// The reply with every maya block removed; this is what Maya may speak.
    pub say: String,
    pub actions: Vec<Value>,
    pub errors: Vec<String>,
}

pub fn parse(reply: &str) -> Parsed {
    let mut say: Vec<&str> = Vec::new();
    let mut actions = Vec::new();
    let mut errors = Vec::new();
    // `Some` while inside a maya block.
    let mut block: Option<Vec<&str>> = None;
    for line in reply.lines() {
        let t = line.trim();
        match block.as_mut() {
            Some(body) if t == "```" => {
                finish(&body.join("\n"), &mut actions, &mut errors);
                block = None;
            }
            Some(body) => body.push(line),
            None if t == "```maya" => block = Some(Vec::new()),
            None => say.push(line),
        }
    }
    if block.is_some() {
        errors.push("a maya block was never closed".to_string());
    }
    Parsed { say: say.join("\n").trim().to_string(), actions, errors }
}

fn finish(body: &str, actions: &mut Vec<Value>, errors: &mut Vec<String>) {
    match serde_json::from_str::<Value>(body.trim()) {
        Ok(v) if v.is_object() => actions.push(v),
        Ok(_) => errors.push("a maya block must hold one JSON object".to_string()),
        Err(e) => errors.push(format!("a maya block is not valid JSON: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn blocks_become_actions_and_leave_the_spoken_text() {
        let p = parse("On it.\n```maya\n{\"kind\":\"reply\",\"session\":\"api-fix\",\"text\":\"open the PR\"}\n```\n");
        assert_eq!(p.say, "On it.");
        assert_eq!(p.actions, vec![json!({"kind":"reply","session":"api-fix","text":"open the PR"})]);
        assert!(p.errors.is_empty());
    }
    #[test]
    fn a_bad_block_is_reported_and_not_spoken() {
        let p = parse("Done.\n```maya\n{not json\n```");
        assert_eq!(p.say, "Done.");
        assert!(p.actions.is_empty());
        assert_eq!(p.errors.len(), 1);
    }
    #[test]
    fn an_unterminated_block_is_an_error_and_not_spoken() {
        let p = parse("Hi.\n```maya\n{\"kind\":\"report\"}");
        assert_eq!(p.say, "Hi.");
        assert!(p.actions.is_empty());
        assert_eq!(p.errors.len(), 1);
    }
    #[test]
    fn several_blocks_and_text_between_them() {
        let p = parse("A.\n```maya\n{\"kind\":\"report\"}\n```\nB.\n```maya\n{\"kind\":\"focus\",\"session\":\"x\"}\n```\nC.");
        assert_eq!(p.say, "A.\nB.\nC.");
        assert_eq!(p.actions.len(), 2);
    }
    #[test]
    fn other_fences_and_a_non_object_block() {
        let p = parse("See:\n```rust\nfn a() {}\n```\n```maya\n[1]\n```");
        assert!(p.say.contains("fn a() {}"));
        assert_eq!(p.errors.len(), 1);
    }
}
