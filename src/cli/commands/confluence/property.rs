use serde_json::Value;

use crate::cli::args::*;
use crate::cli::commands::read_body_arg;
use crate::client::ConfluenceClient;

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
            let value: Value = serde_json::from_str(&value_str).unwrap_or(Value::String(value_str));
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
