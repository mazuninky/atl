use serde_json::Value;

use crate::cli::args::*;
use crate::cli::commands::read_body_arg;
use crate::client::ConfluenceClient;

/// Parse a property-value string using the "JSON if you can, otherwise string"
/// rule. This lets users pass either `--value 42` (becomes `Number(42)`) or
/// `--value hello` (becomes `String("hello")`) without an explicit type flag.
///
/// Empty input becomes the empty JSON string `""`.
pub(super) fn parse_property_value(value_str: String) -> Value {
    serde_json::from_str(&value_str).unwrap_or(Value::String(value_str))
}

pub(super) async fn dispatch_resource_property(
    type_name: &str,
    cmd: &ConfluenceContentTypePropertySubcommand,
    client: &ConfluenceClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        ConfluenceContentTypePropertySubcommand::List(args) => {
            client
                .get_content_type_sub_v2(type_name, &args.id, "properties", 200)
                .await?
        }
        ConfluenceContentTypePropertySubcommand::Get(args) => {
            client
                .get_content_type_property_v2(type_name, &args.id, &args.key)
                .await?
        }
        ConfluenceContentTypePropertySubcommand::Set(args) => {
            let value_str = read_body_arg(&args.value)?;
            let value = parse_property_value(value_str);
            client
                .set_content_type_property_v2(type_name, &args.id, &args.key, &value)
                .await?
        }
        ConfluenceContentTypePropertySubcommand::Delete(args) => {
            client
                .delete_content_type_property_v2(type_name, &args.id, &args.key)
                .await?;
            Value::String(format!("Property '{}' deleted", args.key))
        }
    })
}

#[cfg(test)]
mod tests {
    // List/Get/Delete arms are pure HTTP delegation — covered by contract
    // tests in tests/contract_confluence_v*.rs. Only `parse_property_value`
    // has local logic and is unit-tested here.

    use super::*;
    use serde_json::json;

    #[test]
    fn parse_value_recognizes_json_object() {
        let value = parse_property_value(r#"{"a": 1}"#.into());
        assert_eq!(value, json!({ "a": 1 }));
    }

    #[test]
    fn parse_value_recognizes_json_array() {
        let value = parse_property_value("[1, 2, 3]".into());
        assert_eq!(value, json!([1, 2, 3]));
    }

    #[test]
    fn parse_value_recognizes_json_number() {
        let value = parse_property_value("42".into());
        assert_eq!(value, json!(42));
    }

    #[test]
    fn parse_value_recognizes_json_bool() {
        let value = parse_property_value("true".into());
        assert_eq!(value, json!(true));
    }

    #[test]
    fn parse_value_recognizes_json_null() {
        let value = parse_property_value("null".into());
        assert_eq!(value, Value::Null);
    }

    #[test]
    fn parse_value_falls_back_to_string_for_non_json() {
        // Bare identifiers are not valid JSON.
        let value = parse_property_value("hello".into());
        assert_eq!(value, Value::String("hello".into()));
    }

    #[test]
    fn parse_value_falls_back_to_string_for_malformed_json() {
        let value = parse_property_value("{not-json".into());
        assert_eq!(value, Value::String("{not-json".into()));
    }

    #[test]
    fn parse_value_quoted_json_string_becomes_unquoted_string() {
        // `"hello"` is a valid JSON string — it parses to a Value::String
        // without the outer quotes.
        let value = parse_property_value(r#""hello""#.into());
        assert_eq!(value, Value::String("hello".into()));
    }

    #[test]
    fn parse_value_empty_input_becomes_empty_string() {
        // serde_json rejects empty input as JSON, so we fall back to
        // String("") rather than failing.
        let value = parse_property_value(String::new());
        assert_eq!(value, Value::String(String::new()));
    }
}
