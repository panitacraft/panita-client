//! Small stream context for redacted Log4j output. Raw text remains authoritative.
use std::sync::Arc;

#[derive(Default)]
pub(super) struct LogContext {
    header: Option<Arc<str>>,
    cdata: bool,
}

impl LogContext {
    /// Return the context before this line, then advance without parsing payloads.
    pub fn consume(&mut self, line: &str) -> (Option<Arc<str>>, bool) {
        let previous = (self.header.clone(), self.cdata);
        if self.header.is_none() && !self.cdata && !line.trim_start().starts_with("<log4j:Event") {
            return previous;
        }
        let mut offset = 0;
        while offset < line.len() {
            let rest = &line[offset..];
            if self.cdata {
                let Some(end) = rest.find("]]>") else { break };
                self.cdata = false;
                offset += end + 3;
                continue;
            }
            let Some(start) = rest.find('<') else { break };
            offset += start;
            let rest = &line[offset..];
            if rest.starts_with("<![CDATA[") {
                self.cdata = true;
                offset += 9;
            } else if rest.starts_with("</log4j:Event>") {
                self.header = None;
                offset += 14;
            } else if rest.starts_with("<log4j:Event")
                && rest
                    .as_bytes()
                    .get(12)
                    .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == b'>')
            {
                let mut quote = None;
                let end = rest.char_indices().skip(1).find_map(|(index, ch)| {
                    if quote == Some(ch) {
                        quote = None;
                    } else if quote.is_none() && matches!(ch, '\'' | '"') {
                        quote = Some(ch);
                    } else if quote.is_none() && ch == '>' {
                        return Some(index + 1);
                    }
                    None
                });
                let Some(end) = end else { break };
                self.header = Some(Arc::from(&rest[..end]));
                offset += end;
            } else {
                offset += 1;
            }
        }
        previous
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_context_survives_pages_cdata_and_quoted_angles() {
        let mut context = LogContext::default();
        let header = r#"<log4j:Event logger="a>b" level="INFO" thread="main">"#;
        assert_eq!(context.consume(header), (None, false));
        assert_eq!(
            context
                .consume("<log4j:Message><![CDATA[first")
                .0
                .as_deref(),
            Some(header)
        );
        let middle = context.consume("<log4j:Event is literal CDATA text");
        assert_eq!(middle.0.as_deref(), Some(header));
        assert!(middle.1);
        context.consume("]]></log4j:Message></log4j:Event>");
        assert_eq!(context.consume("ordinary output"), (None, false));
    }
}
