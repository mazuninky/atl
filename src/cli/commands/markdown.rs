use comrak::{Options, markdown_to_html};

/// Convert Markdown to Confluence storage format (XHTML).
pub fn markdown_to_storage(md: &str) -> String {
    markdown_to_html(md, &Options::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_heading() {
        let result = markdown_to_storage("# Hello");
        assert!(result.contains("<h1>Hello</h1>"));
    }

    #[test]
    fn converts_paragraph() {
        let result = markdown_to_storage("Some text");
        assert!(result.contains("<p>Some text</p>"));
    }

    #[test]
    fn converts_bold() {
        let result = markdown_to_storage("**bold**");
        assert!(result.contains("<strong>bold</strong>"));
    }

    #[test]
    fn converts_list() {
        let result = markdown_to_storage("- item1\n- item2");
        assert!(result.contains("<li>item1</li>"));
        assert!(result.contains("<li>item2</li>"));
    }
}
