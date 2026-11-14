//! Describes a tool call in one short line.
//!
//! A thread shows what an agent did, not the full input of every tool: that
//! can be a whole file. Both programs name the interesting part of an input in
//! similar ways, so one list of field names serves both.

use serde_json::Value;

/// Fields that say what a tool was used on, most telling first.
const TELLING_FIELDS: [&str; 9] = [
    "description",
    "file_path",
    "path",
    "command",
    "pattern",
    "url",
    "query",
    "prompt",
    "target_directory",
];

const LONGEST_SUMMARY: usize = 200;

/// Picks the most telling text field of a tool input, on one line.
pub fn summarise_input(input: &Value) -> String {
    let text = TELLING_FIELDS
        .iter()
        .find_map(|field| input.get(field).and_then(Value::as_str))
        .unwrap_or_default();
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");

    if one_line.chars().count() <= LONGEST_SUMMARY {
        one_line
    } else {
        let shortened: String = one_line.chars().take(LONGEST_SUMMARY).collect();
        format!("{shortened}…")
    }
}

#[cfg(test)]
mod tests {
    use lets_expect::lets_expect;
    use serde_json::json;

    use super::*;

    lets_expect! {
        expect(summarise_input(&input)) {
            when(input = json!({"command": "ls", "description": "List files"})) as prefer_the_description to hold {
                equal("List files".to_string())
            }
            when(input = json!({"file_path": "/work/a.txt", "content": "long"})) as name_the_file to hold {
                equal("/work/a.txt".to_string())
            }
            when(input = json!({"command": "cargo test"})) as show_the_command to hold {
                equal("cargo test".to_string())
            }
            when(input = json!({"command": "a\n  b\tc"})) as put_it_on_one_line to hold {
                equal("a b c".to_string())
            }
            when(input = json!({"content": "only content"})) as be_empty_without_a_telling_field to hold {
                equal(String::new())
            }
            when(input = json!({"command": 5})) as be_empty_when_the_field_is_not_text to hold {
                equal(String::new())
            }
            when(input = json!(null)) as be_empty_without_input to hold {
                equal(String::new())
            }
            when(input = json!({"command": "é".repeat(200)})) as keep_exactly_the_longest_length to hold {
                equal("é".repeat(200))
            }
            when(input = json!({"command": "é".repeat(201)})) as shorten_longer_text to hold {
                equal(format!("{}…", "é".repeat(200)))
            }
        }
    }
}
