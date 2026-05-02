use clap::{Args, ValueEnum};

/// Which Atlassian product to target for the raw REST call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum ApiService {
    /// Confluence (Cloud or Data Center)
    Confluence,
    /// Jira (Cloud or Data Center)
    Jira,
}

/// Arguments for `atl api <endpoint>` — a generic REST passthrough command
/// mirroring `gh api`.
///
/// The endpoint is a positional argument. Both absolute forms
/// (`/wiki/rest/api/content/123`) and relative forms
/// (`rest/api/2/issue/PROJ-1`) are accepted — the dispatcher normalizes a
/// leading slash automatically.
#[derive(Debug, Args)]
#[command(
    about = "Make an authenticated request against a Confluence or Jira REST endpoint",
    long_about = "Make an authenticated HTTP request against a Confluence or Jira REST endpoint.\n\
                  \n\
                  Both absolute (/wiki/rest/api/content/123) and relative (rest/api/2/myself) \
                  forms are accepted. The response is printed as JSON by default regardless of \
                  the global --format setting. Use --paginate to auto-follow pagination for \
                  Jira search, Jira Agile, and Confluence _links.next style responses.",
    after_help = "EXAMPLES:\n\
                  \n    \
                  atl api --service jira rest/api/2/myself\n    \
                  atl api --service jira rest/api/3/search/jql --query jql='project=TEST'  # Cloud only\n    \
                  atl api --service confluence /wiki/api/v2/pages --query space-id=123 --paginate\n    \
                  atl api --service jira --method POST rest/api/2/issue \\\n        \
                  --raw-field fields='{\"project\":{\"key\":\"TEST\"}}'\n    \
                  atl api --service jira rest/api/2/issue/TEST-1 --preview"
)]
pub struct ApiArgs {
    /// REST endpoint path (e.g. `rest/api/2/myself`). Leading slash optional.
    pub endpoint: String,

    /// Which Atlassian product to target.
    #[arg(long, value_enum, value_name = "SERVICE")]
    pub service: ApiService,

    /// HTTP method (default: GET).
    #[arg(short = 'X', long, value_name = "METHOD", default_value = "GET")]
    pub method: String,

    /// Custom header in the form `KEY:VALUE`. Repeatable.
    #[arg(short = 'H', long = "header", value_name = "KEY:VALUE")]
    pub headers: Vec<String>,

    /// String field in the JSON body, `KEY=VALUE`. `VALUE` may be `@file` to
    /// read the contents of a file, or `-` to read stdin. Repeatable.
    /// Mutually exclusive with `--input`.
    #[arg(
        short = 'f',
        long = "field",
        value_name = "KEY=VALUE",
        conflicts_with = "input"
    )]
    pub fields: Vec<String>,

    /// Raw JSON field in the JSON body, `KEY=VALUE`. `VALUE` is parsed as
    /// JSON (e.g. `--raw-field count=42`, `--raw-field tags=[1,2,3]`).
    /// Repeatable. Mutually exclusive with `--input`. No short form because
    /// `-F` is reserved for `--format`.
    #[arg(long = "raw-field", value_name = "KEY=VALUE", conflicts_with = "input")]
    pub raw_fields: Vec<String>,

    /// Read the request body verbatim from a file, or `-` for stdin.
    /// Mutually exclusive with `--field` / `--raw-field`.
    #[arg(long, value_name = "FILE|-")]
    pub input: Option<String>,

    /// Query parameter in the form `KEY=VALUE`. Repeatable. No short form
    /// because `-q` is reserved for `--quiet`.
    #[arg(long = "query", value_name = "KEY=VALUE")]
    pub queries: Vec<String>,

    /// Auto-follow pagination until the server reports no more pages.
    #[arg(long)]
    pub paginate: bool,

    /// Maximum number of pages to fetch when --paginate is set. Defaults to
    /// 1000. Use 0 for no limit (not recommended).
    #[arg(long, value_name = "N", default_value_t = 1000)]
    pub max_pages: u32,

    /// Print the constructed request to stderr and exit without sending.
    #[arg(long)]
    pub preview: bool,
}
