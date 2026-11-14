//! Input schemas of the tools and the check of a call's arguments against them.
//!
//! The schemas are JSON Schema 2020-12 and use a small part of it on purpose: an object
//! with named properties that are strings, integers, booleans, one of a list of words or
//! an object of strings, some of them required, and nothing else allowed. Every client
//! and every model handles that part the same way.
//!
//! [`check`] is the server's own validation of that part. What it finds wrong is reported
//! as a tool failure the model can read and correct (the specification asks for input
//! validation errors as tool results, not protocol errors), naming every problem at once
//! and the arguments the tool does take. It is forgiving where the meaning is not in
//! doubt: a number where a string is expected (a document named by its id), digits in a
//! string where a number is expected, `"true"` for `true`.

use serde_json::{Map, Value, json};

/// A string property.
pub fn string(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

/// An integer property, at least `minimum`.
pub fn integer(description: &str, minimum: i64) -> Value {
    json!({ "type": "integer", "minimum": minimum, "description": description })
}

/// An integer property from `minimum` to `maximum`, both included.
pub fn integer_between(description: &str, minimum: i64, maximum: i64) -> Value {
    json!({
        "type": "integer",
        "minimum": minimum,
        "maximum": maximum,
        "description": description,
    })
}

/// A boolean property. Leaving it out means `false`.
pub fn boolean(description: &str) -> Value {
    json!({ "type": "boolean", "description": description })
}

/// A string property that is one of `choices`.
pub fn one_of(description: &str, choices: &[&str]) -> Value {
    json!({ "type": "string", "enum": choices, "description": description })
}

/// An object property whose values are strings (`{"<address>": "<file>"}`).
pub fn string_map(description: &str) -> Value {
    json!({
        "type": "object",
        "additionalProperties": { "type": "string" },
        "description": description,
    })
}

/// The schema of a tool's arguments: an object with these properties and no others.
pub fn object(properties: Vec<(&str, Value)>, required: &[&str]) -> Value {
    let mut map = Map::new();
    for (name, schema) in properties {
        map.insert(name.to_string(), schema);
    }
    for name in required {
        debug_assert!(
            map.contains_key(*name),
            "required property {name} is not declared"
        );
    }
    let mut schema = Map::new();
    schema.insert("type".into(), json!("object"));
    schema.insert("properties".into(), Value::Object(map));
    if !required.is_empty() {
        schema.insert("required".into(), json!(required));
    }
    schema.insert("additionalProperties".into(), json!(false));
    Value::Object(schema)
}

/// The arguments of a call after [`check`]: every value has the type its schema names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Args(Map<String, Value>);

impl Args {
    /// A string argument, when given.
    pub fn str(&self, name: &str) -> Option<&str> {
        self.0.get(name).and_then(Value::as_str)
    }

    /// A string argument the schema requires. Empty when it is missing all the same
    /// (it cannot be after [`check`]).
    pub fn required(&self, name: &str) -> &str {
        self.str(name).unwrap_or_default()
    }

    /// An integer argument, when given.
    pub fn int(&self, name: &str) -> Option<i64> {
        self.0.get(name).and_then(Value::as_i64)
    }

    /// A count, when given.
    pub fn count(&self, name: &str) -> Option<usize> {
        self.int(name).and_then(|n| usize::try_from(n).ok())
    }

    /// A switch: `false` unless given as `true`.
    pub fn flag(&self, name: &str) -> bool {
        self.0.get(name).and_then(Value::as_bool).unwrap_or(false)
    }

    /// Whether the argument was given at all.
    pub fn has(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    /// The arguments as a JSON object again.
    pub fn into_map(self) -> Map<String, Value> {
        self.0
    }

    /// The names of the arguments that were given.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
    }
}

/// Checks `arguments` against `schema` (one made by [`object`]). The error is a sentence
/// per problem, then the arguments the tool takes.
pub fn check(schema: &Value, arguments: &Map<String, Value>) -> Result<Args, String> {
    let empty = Map::new();
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let mut problems = Vec::new();
    let mut checked = Map::new();
    for (name, value) in arguments {
        let Some(property) = properties.get(name) else {
            problems.push(format!("There is no argument \"{name}\"."));
            continue;
        };
        // An optional argument given as null is one that was left out.
        if value.is_null() && !required.contains(&name.as_str()) {
            continue;
        }
        match coerce(property, value) {
            Ok(value) => {
                checked.insert(name.clone(), value);
            }
            Err(expected) => problems.push(format!("\"{name}\" must be {expected}.")),
        }
    }
    for name in &required {
        if !arguments.contains_key(*name) {
            problems.push(format!("\"{name}\" is required."));
        }
    }
    if problems.is_empty() {
        return Ok(Args(checked));
    }
    let mut takes: Vec<String> = Vec::new();
    for (name, property) in properties {
        let kind = match property.get("enum").and_then(Value::as_array) {
            Some(choices) => choices
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("|"),
            None => property
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("value")
                .to_string(),
        };
        let mark = if required.contains(&name.as_str()) {
            ", required"
        } else {
            ""
        };
        takes.push(format!("{name} ({kind}{mark})"));
    }
    let takes = if takes.is_empty() {
        "none".to_string()
    } else {
        takes.join(", ")
    };
    Err(format!(
        "{} Arguments of this tool: {takes}.",
        problems.join(" ")
    ))
}

/// `value` as the type `property` names, or what was expected instead.
fn coerce(property: &Value, value: &Value) -> Result<Value, String> {
    if let Some(choices) = property.get("enum").and_then(Value::as_array) {
        if choices.contains(value) {
            return Ok(value.clone());
        }
        let words: Vec<&str> = choices.iter().filter_map(Value::as_str).collect();
        return Err(format!("one of {}", quoted_list(&words)));
    }
    match property.get("type").and_then(Value::as_str) {
        Some("string") => match value {
            Value::String(_) => Ok(value.clone()),
            // A document or a version named by its number.
            Value::Number(number) => Ok(json!(number.to_string())),
            _ => Err("a string".into()),
        },
        Some("integer") => {
            let minimum = property.get("minimum").and_then(Value::as_i64);
            let maximum = property.get("maximum").and_then(Value::as_i64);
            let number = match value {
                Value::Number(number) => number.as_i64(),
                Value::String(digits) => digits.trim().parse::<i64>().ok(),
                _ => None,
            };
            // One sentence for the whole range, whichever end was passed.
            let expected = match (minimum, maximum) {
                (Some(minimum), Some(maximum)) => {
                    format!("a whole number from {minimum} to {maximum}")
                }
                (Some(minimum), None) => format!("a whole number, {minimum} or more"),
                (None, Some(maximum)) => format!("a whole number, {maximum} or less"),
                (None, None) => "a whole number".into(),
            };
            match number {
                Some(number)
                    if minimum.is_some_and(|minimum| number < minimum)
                        || maximum.is_some_and(|maximum| number > maximum) =>
                {
                    Err(expected)
                }
                Some(number) => Ok(json!(number)),
                None => Err(expected),
            }
        }
        Some("object") => match value {
            Value::Object(map) if map.values().all(Value::is_string) => Ok(value.clone()),
            _ => Err("an object whose values are strings".into()),
        },
        Some("boolean") => match value {
            Value::Bool(_) => Ok(value.clone()),
            Value::String(word) if word == "true" => Ok(json!(true)),
            Value::String(word) if word == "false" => Ok(json!(false)),
            _ => Err("true or false".into()),
        },
        _ => Ok(value.clone()),
    }
}

fn quoted_list(words: &[&str]) -> String {
    words
        .iter()
        .map(|word| format!("\"{word}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Value {
        object(
            vec![
                ("document", string("The document.")),
                ("limit", integer("How many.", 1)),
                ("all", boolean("Every occurrence.")),
                ("format", one_of("The format.", &["markdown", "text"])),
            ],
            &["document"],
        )
    }

    fn arguments(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn the_schema_is_a_closed_object() {
        let schema = schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["required"], json!(["document"]));
        assert_eq!(
            schema["properties"]["format"]["enum"],
            json!(["markdown", "text"])
        );
        assert_eq!(schema["properties"]["limit"]["minimum"], 1);
        let none = object(vec![], &[]);
        assert!(none.get("required").is_none());
        assert_eq!(none["properties"], json!({}));
    }

    #[test]
    fn good_arguments_come_back_typed() {
        let args = check(
            &schema(),
            &arguments(
                json!({ "document": "Notes/Plan", "limit": 5, "all": true, "format": "text" }),
            ),
        )
        .unwrap();
        assert_eq!(args.required("document"), "Notes/Plan");
        assert_eq!(args.count("limit"), Some(5));
        assert!(args.flag("all"));
        assert_eq!(args.str("format"), Some("text"));
        assert!(args.has("limit"));

        let few = check(&schema(), &arguments(json!({ "document": "x" }))).unwrap();
        assert_eq!(few.count("limit"), None);
        assert!(!few.flag("all"));
        assert!(!few.has("format"));
    }

    #[test]
    fn values_whose_meaning_is_plain_are_taken() {
        let args = check(
            &schema(),
            &arguments(json!({ "document": 42, "limit": "7", "all": "true", "format": null })),
        )
        .unwrap();
        assert_eq!(args.required("document"), "42");
        assert_eq!(args.count("limit"), Some(7));
        assert!(args.flag("all"));
        assert!(!args.has("format"));
    }

    #[test]
    fn a_range_is_in_the_schema_and_both_ends_are_refused_with_one_sentence() {
        let property = integer_between("How wide.", 5, 100);
        assert_eq!(property["minimum"], json!(5));
        assert_eq!(property["maximum"], json!(100));
        let schema = object(vec![("width", property)], &[]);
        for (given, taken) in [(json!(5), true), (json!(100), true), (json!("50"), true)] {
            assert_eq!(
                check(&schema, &arguments(json!({ "width": given }))).is_ok(),
                taken
            );
        }
        for wrong in [json!(4), json!(101), json!(200), json!("wide")] {
            let error = check(&schema, &arguments(json!({ "width": wrong }))).unwrap_err();
            assert!(
                error.contains("\"width\" must be a whole number from 5 to 100."),
                "{error}"
            );
        }
    }

    #[test]
    fn every_problem_is_named_at_once_with_what_the_tool_takes() {
        let error = check(
            &schema(),
            &arguments(json!({ "doc": "x", "limit": 0, "all": "yes", "format": "pdf" })),
        )
        .unwrap_err();
        assert!(error.contains("There is no argument \"doc\"."), "{error}");
        assert!(
            error.contains("\"limit\" must be a whole number, 1 or more."),
            "{error}"
        );
        assert!(error.contains("\"all\" must be true or false."), "{error}");
        assert!(
            error.contains("\"format\" must be one of \"markdown\", \"text\"."),
            "{error}"
        );
        assert!(error.contains("\"document\" is required."), "{error}");
        assert!(error.contains("document (string, required)"), "{error}");
        assert!(error.contains("format (markdown|text)"), "{error}");

        let wrong_type = check(&schema(), &arguments(json!({ "document": ["a"] }))).unwrap_err();
        assert!(
            wrong_type.contains("\"document\" must be a string."),
            "{wrong_type}"
        );
        let null_required = check(&schema(), &arguments(json!({ "document": null }))).unwrap_err();
        assert!(
            null_required.contains("\"document\" must be a string."),
            "{null_required}"
        );
    }

    #[test]
    fn an_object_of_strings_is_taken_and_anything_else_is_refused() {
        let schema = object(vec![("externals", string_map("Pieces."))], &[]);
        assert_eq!(schema["properties"]["externals"]["type"], "object");
        assert_eq!(
            schema["properties"]["externals"]["additionalProperties"]["type"],
            "string"
        );
        let given = json!({ "externals": { "https://a/b.png": "/tmp/b.png" } });
        let args = check(&schema, &arguments(given.clone())).unwrap();
        assert_eq!(args.into_map()["externals"], given["externals"]);
        for wrong in [json!({ "a": 1 }), json!("x"), json!(["a"])] {
            let error = check(&schema, &arguments(json!({ "externals": wrong }))).unwrap_err();
            assert!(
                error.contains("\"externals\" must be an object whose values are strings."),
                "{error}"
            );
        }
    }
}
