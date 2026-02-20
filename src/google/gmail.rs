//! Gmail API client for listing unread mail, fetching threads, sending replies
//! WARNING: Pretty much everything in here is AI-written and
//! probably terrible but it cleans up the messy output from the Gmail
//! API fairly well for my purposes. Best to let AI update this
//! as it's super bespoke and edge-case-y.

use base64::{Engine as _, engine::general_purpose::URL_SAFE};
use chrono::{Duration, Utc};
use htmd::HtmlToMarkdown;
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Message and thread structures from Gmail API documentation
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MessageResponse {
    pub id: String,
    #[serde(rename = "threadId")]
    pub thread_id: String,
}

#[derive(Debug, Deserialize)]
pub struct ListMessagesResponse {
    pub messages: Option<Vec<MessageResponse>>,
    #[serde(rename = "nextPageToken")]
    pub next_page_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    pub id: String,
    pub messages: Vec<Message>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    #[serde(rename = "threadId")]
    pub thread_id: String,
    pub snippet: Option<String>,
    pub payload: Option<MessagePayload>,
    #[serde(rename = "labelIds")]
    pub label_ids: Option<Vec<String>>,
    #[serde(rename = "internalDate")]
    pub internal_date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePartBody {
    #[serde(rename = "attachmentId")]
    attachment_id: Option<String>,
    size: u64,
    // Base64 encoded
    data: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePart {
    #[serde(rename = "partId")]
    pub part_id: String,
    #[serde(rename = "mimeType")]
    pub mimetype: String,
    pub body: Option<MessagePartBody>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePayload {
    pub headers: Option<Vec<MessageHeader>>,
    #[serde(rename = "mimeType")]
    pub mimetype: String,
    pub body: Option<MessagePartBody>,
    pub parts: Option<Vec<MessagePart>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageHeader {
    pub name: String,
    pub value: String,
}

fn decode_base64(data: &str) -> String {
    URL_SAFE
        .decode(data)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_else(|| {
            tracing::error!("Base64 decode failed for: {}", data);
            String::from("Failed to decode")
        })
}

/// Decode unicode characters from quoted-printable or HTML entities
fn clean_unicode(content: &str) -> String {
    let mut content = content.to_string();

    // Decode quoted-printable (common in Gmail)
    // Handle patterns like =E2=80=99, =20, etc.
    content = decode_quoted_printable(&content);

    // Decode HTML entities (e.g., &amp; &#x2019;)
    content = html_entity_decode(&content);

    // Clean up common encoding artifacts (escaped sequences like \u2019)
    let escape_re = Regex::new(r"\\u([0-9a-fA-F]{4})").unwrap();
    content = escape_re
        .replace_all(&content, |caps: &regex::Captures| {
            if let Some(hex) = caps.get(1)
                && let Ok(codepoint) = u32::from_str_radix(hex.as_str(), 16)
                && let Some(c) = char::from_u32(codepoint)
            {
                return c.to_string();
            }
            caps.get(0).unwrap().as_str().to_string()
        })
        .to_string();

    // Convert smart quotes to regular quotes
    content = content.replace('\u{2019}', "'"); // Right single quotation mark
    content = content.replace('\u{2018}', "'"); // Left single quotation mark
    content = content.replace('\u{201c}', "\""); // Left double quotation mark
    content = content.replace('\u{201d}', "\""); // Right double quotation mark

    content
}

/// Decode quoted-printable encoded strings (e.g., =E2=80=99)
fn decode_quoted_printable(input: &str) -> String {
    let mut bytes = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '=' && i + 2 < chars.len() {
            // Check for soft line break: =\n
            if chars[i + 1] == '\n' {
                // Skip the = and \n
                i += 2;
            } else if chars[i + 1] == '\r' && i + 3 < chars.len() && chars[i + 2] == '\n' {
                // Skip the =, \r, and \n
                i += 3;
            } else {
                // Try to parse =XX hex sequence
                let hex_str: String = chars[i + 1..=i + 2].iter().collect();
                if let Ok(byte_val) = u8::from_str_radix(&hex_str, 16) {
                    bytes.push(byte_val);
                    i += 3;
                } else {
                    // Invalid hex, keep the '=' and continue
                    bytes.push(b'=');
                    i += 1;
                }
            }
        } else {
            // Regular character - convert to bytes (UTF-8)
            for byte in chars[i].to_string().bytes() {
                bytes.push(byte);
            }
            i += 1;
        }
    }

    // Try to decode as UTF-8, fallback to lossy if invalid
    String::from_utf8_lossy(&bytes).to_string()
}

/// Decode HTML entities in a string
fn html_entity_decode(input: &str) -> String {
    // Common HTML entities
    let mut result = input.to_string();

    // Named entities
    result = result.replace("&amp;", "&");
    result = result.replace("&lt;", "<");
    result = result.replace("&gt;", ">");
    result = result.replace("&quot;", "\"");
    result = result.replace("&apos;", "'");
    result = result.replace("&nbsp;", " ");
    result = result.replace("&copy;", "©");
    result = result.replace("&reg;", "®");
    result = result.replace("&trade;", "™");

    // Numeric entities (&#123; or &#x1F600;)
    let numeric_entity = Regex::new(r"&(#(\d+)|#x([0-9a-fA-F]+));").unwrap();
    result = numeric_entity
        .replace_all(&result, |caps: &regex::Captures| {
            if let Some(decimal) = caps.get(2) {
                // Decimal: &#123;
                if let Ok(codepoint) = decimal.as_str().parse::<u32>()
                    && let Some(c) = char::from_u32(codepoint)
                {
                    return c.to_string();
                }
            } else if let Some(hex) = caps.get(3) {
                // Hex: &#x1F600;
                if let Ok(codepoint) = u32::from_str_radix(hex.as_str(), 16)
                    && let Some(c) = char::from_u32(codepoint)
                {
                    return c.to_string();
                }
            }
            caps.get(0).unwrap().as_str().to_string()
        })
        .to_string();

    result
}

/// Strip quoted replies from email threads (e.g., "On ... wrote:" and nested > quotes)
fn strip_quoted_replies(content: &str) -> String {
    // Match "On [date] [sender] wrote:" pattern and everything after it
    // This handles both \r\n and \n line endings, with various date formats and sender patterns
    let quote_header_re = Regex::new(
        r"(?is)(?:\r?\n){2,}On (?:Mon|Tue|Wed|Thu|Fri|Sat|Sun),? .+? (?:at \d{1,2}(?::\d{2})?(?::\d{2})?\s*(?:AM|PM|am|pm)?)?.+? wrote:\r?\n"
    ).unwrap();

    if let Some(pos) = quote_header_re.find(content) {
        return content[..pos.start()].trim_end().to_string();
    }

    // Also strip lines that start with ">" (quoted content)
    let quoted_lines = content
        .lines()
        .filter(|line| !line.trim_start().starts_with('>'))
        .collect::<Vec<_>>()
        .join("\n");

    quoted_lines.trim_end().to_string()
}

/// Strip email signatures from the content
fn strip_signature(content: &str) -> String {
    let mut result = content.to_string();

    // Remove common footer patterns first (like "unsubscribe", "manage preferences")
    let footer_re = Regex::new(r"(?is)\n\n(?:You are receiving this|Unsubscribe|Manage preferences|Click here to unsubscribe|This email was sent)[^\n]*$").unwrap();
    result = footer_re.replace(&result, "").to_string();

    // Remove mobile signatures (can appear without delimiter)
    let mobile_re =
        Regex::new(r"(?is)\n\n(?:Sent from my (?:iPhone|iPad|iPod)|Sent from my Android)[^\n]*$")
            .unwrap();
    result = mobile_re.replace(&result, "").to_string();

    // Remove standard signature delimiters with following content
    let delimiter_re = Regex::new(
        r"(?is)(?:^|\n)\s*(?:--\s*\n|---+\s*\n|==+\s*\n|\*{3,}\s*\n).*(?:[^\n]{0,200}\n){0,10}$",
    )
    .unwrap();
    if let Some(pos) = delimiter_re.find(&result) {
        result.truncate(pos.start());
    }

    // Remove signature keywords followed by content
    let keyword_re = Regex::new(
        r"(?is)\n\n\s*(?:Regards|Best regards,?|Kind regards,?|Thanks,?|Thank you,?|Sincerely,?|Cheers,?|Best,?|Warmly,?|With gratitude,?|All the best,?|Take care,?|Many thanks,?|Thanks and regards,?|Best wishes,?|Yours truly,?|Respectfully,?|Cordially,?).*(?:[^\n]{0,200}\n){0,5}$"
    ).unwrap();
    if let Some(pos) = keyword_re.find(&result) {
        result.truncate(pos.start());
    }

    // Trim trailing whitespace
    result.trim_end().to_string()
}

/// Extract the body from the Gmail API message payload.
///
/// To get the body of an email:
/// - The email messsage can either have a `payload.body.data` or one or more `parts[].body.data`.
/// - Parts might have an HTML version of the message as well as a plain text version of the body
///   Use the `parts[].mimetype` field to distinguish which it is
/// - When there is a `body.attachment_id` that indicates a file that was attached
pub fn extract_body(message: &Message) -> String {
    let payload = message.payload.clone().unwrap();

    if let Some(body) = &payload.body
        && let Some(data) = &body.data
    {
        if &payload.mimetype == "text/html" {
            let html = decode_base64(data);
            let converter = HtmlToMarkdown::builder()
                .skip_tags(vec!["script", "style", "footer", "img", "svg"])
                .build();
            return converter
                .convert(&html)
                .expect("Failed to convert HTML to markdown");
        }

        return clean_and_strip_body(decode_base64(data));
    }

    if let Some(parts) = &payload.parts {
        // Prefer plain text over HTML
        for part in parts {
            if part.mimetype == "text/plain"
                && let Some(body) = &part.body
            {
                // Skip attachments
                if body.attachment_id.is_some() {
                    continue;
                }
                // Return the first non-empty body found in parts
                if let Some(data) = &body.data
                    && !data.is_empty()
                {
                    return clean_and_strip_body(decode_base64(data));
                }
            }

            if part.mimetype == "text/html"
                && let Some(body) = &part.body
            {
                // Skip attachments
                if body.attachment_id.is_some() {
                    continue;
                }
                // Return the first non-empty body found in parts
                if let Some(data) = &body.data
                    && !data.is_empty()
                {
                    let html = decode_base64(data);
                    let converter = HtmlToMarkdown::builder()
                        .skip_tags(vec!["script", "style", "footer", "img", "svg"])
                        .build();
                    return converter
                        .convert(&html)
                        .expect("Failed to convert HTML to markdown");
                }
            }
        }
    }

    // Fall back to the snippet
    // Sometimes a message in the thread only has a snippet and no
    // other message parts. Not sure why...
    if let Some(snippet) = &message.snippet {
        return clean_and_strip_body(snippet.clone());
    }

    // Not sure how we could end up with no body at all so log it and
    // return and empty string.
    tracing::warn!(
        "Body was empty for message with ID: {} in thread: {}",
        message.id,
        message.thread_id
    );

    String::new()
}

/// Extract and clean the subject from a message
pub fn extract_subject(message: &Message) -> String {
    let payload = match &message.payload {
        Some(p) => p,
        None => return String::new(),
    };

    let headers = match &payload.headers {
        Some(h) => h,
        None => return String::new(),
    };

    for header in headers {
        if header.name.to_lowercase() == "subject" {
            return clean_unicode(&header.value);
        }
    }

    String::new()
}

/// Extract and clean the from field from a message
pub fn extract_from(message: &Message) -> String {
    let payload = match &message.payload {
        Some(p) => p,
        None => return String::new(),
    };

    let headers = match &payload.headers {
        Some(h) => h,
        None => return String::new(),
    };

    for header in headers {
        if header.name.to_lowercase() == "from" {
            return clean_unicode(&header.value);
        }
    }

    String::new()
}

/// Extract and clean the to field from a message
pub fn extract_to(message: &Message) -> String {
    let payload = match &message.payload {
        Some(p) => p,
        None => return String::new(),
    };

    let headers = match &payload.headers {
        Some(h) => h,
        None => return String::new(),
    };

    for header in headers {
        if header.name.to_lowercase() == "to" {
            return clean_unicode(&header.value);
        }
    }

    String::new()
}

/// Clean unicode and strip signature from body content
fn clean_and_strip_body(content: String) -> String {
    let cleaned = clean_unicode(&content);
    let without_quotes = strip_quoted_replies(&cleaned);
    strip_signature(&without_quotes)
}

/// List unread messages from the last N days
/// curl: see spec
pub async fn list_unread_messages(
    access_token: &str,
    n_days: i64,
) -> Result<Vec<MessageResponse>, anyhow::Error> {
    let client = Client::new();
    let after_date = (Utc::now() - Duration::days(n_days))
        .format("%Y/%m/%d")
        .to_string();
    let url = format!(
        "https://gmail.googleapis.com/gmail/v1/users/me/messages?labelIds=UNREAD&q=is:unread%20after:{}%20in:inbox",
        after_date
    );
    let res = client.get(&url).bearer_auth(access_token).send().await?;
    let status = res.status();
    let text = res.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("Unread fetch failed: {} ({})", status, text);
    }
    let msgs: ListMessagesResponse = serde_json::from_str(&text)?;
    Ok(msgs.messages.unwrap_or_default())
}

/// Fetch full thread for a given threadId
/// curl: see spec
pub async fn fetch_thread(
    access_token: String,
    thread_id: String,
) -> Result<Thread, anyhow::Error> {
    let client = Client::new();
    let url = format!(
        "https://gmail.googleapis.com/gmail/v1/users/me/threads/{}?format=full",
        thread_id
    );
    let res = client.get(&url).bearer_auth(access_token).send().await?;
    let status = res.status();
    let text = res.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("Thread fetch failed: {} ({})", status, text);
    }
    let thread: Thread = serde_json::from_str(&text)?;
    Ok(thread)
}

/// Helper: base64url encode w/out padding
fn base64_url_no_pad(input: &str) -> String {
    URL_SAFE.encode(input.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_quoted_printable() {
        // Basic quoted-printable
        assert_eq!(decode_quoted_printable("Hello=20World"), "Hello World");
        assert_eq!(decode_quoted_printable("line1=\nline2"), "line1line2");
        assert_eq!(decode_quoted_printable("No=encoding"), "No=encoding");

        // Common unicode characters (note: this only decodes quoted-printable, doesn't convert smart quotes)
        assert_eq!(decode_quoted_printable("Test=E2=80=99"), "Test\u{2019}");
        assert_eq!(decode_quoted_printable("Don=E2=80=99t"), "Don\u{2019}t");
        assert_eq!(
            decode_quoted_printable("smart=E2=80=9Cquotes=E2=80=9D"),
            "smart\u{201C}quotes\u{201D}"
        );
        assert_eq!(decode_quoted_printable("em=E2=80=94dash"), "em\u{2014}dash");
    }

    #[test]
    fn test_html_entity_decode() {
        // Named entities
        assert_eq!(html_entity_decode("Hello &amp; goodbye"), "Hello & goodbye");
        assert_eq!(html_entity_decode("&lt;tag&gt;"), "<tag>");
        assert_eq!(html_entity_decode("Don&apos;t stop"), "Don't stop");
        assert_eq!(html_entity_decode("&quot;quoted&quot;"), "\"quoted\"");
        assert_eq!(html_entity_decode("space&nbsp;here"), "space here");

        // Numeric entities (decimal)
        assert_eq!(html_entity_decode("Price: &#36;100"), "Price: $100");
        assert_eq!(
            html_entity_decode("Copyright &#169; 2024"),
            "Copyright © 2024"
        );

        // Numeric entities (hex) - note: this converts to unicode characters, not regular quotes
        assert_eq!(html_entity_decode("Don&#x2019;t"), "Don\u{2019}t");
        assert_eq!(html_entity_decode("Test&#x1F600;ing"), "Test😀ing");
        assert_eq!(
            html_entity_decode("&#x201C;Hello&#x201D;"),
            "\u{201C}Hello\u{201D}"
        );

        // Mixed
        assert_eq!(
            html_entity_decode("&lt;&#x201C;test&#x201D;&amp; more&gt;"),
            "<\u{201C}test\u{201D}& more>"
        );
    }

    #[test]
    fn test_clean_unicode() {
        // Quoted-printable (also converts smart quotes to regular)
        assert_eq!(clean_unicode("Hello=20World=E2=80=99s"), "Hello World's");

        // HTML entities (also converts smart quotes to regular)
        assert_eq!(clean_unicode("Test &amp; more"), "Test & more");
        assert_eq!(clean_unicode("Don&#x2019;t stop"), "Don't stop");

        // Unicode escape sequences (decoded and smart quotes converted)
        assert_eq!(clean_unicode("Don\\u2019t"), "Don't");
        assert_eq!(clean_unicode("\\u201CHello\\u201D"), "\"Hello\"");
        assert_eq!(clean_unicode("en–dash"), "en–dash");
        assert_eq!(clean_unicode("em—dash"), "em—dash");

        // Combined (all transformations applied)
        let input = "=E2=80=9CQuote &amp; \\u2018escape\\u2019=E2=80=9D";
        assert_eq!(clean_unicode(input), "\"Quote & 'escape'\"");
    }

    #[test]
    fn test_strip_signature() {
        // Standard signature delimiter
        let input = "Hello world\n--\nJohn Doe\njohn@example.com";
        assert_eq!(strip_signature(input), "Hello world");

        // Regards signature
        let input = "Thanks for the help!\n\nBest regards,\nJohn";
        assert_eq!(strip_signature(input), "Thanks for the help!");

        // Mobile signature
        let input = "Check this out\n\nSent from my iPhone";
        assert_eq!(strip_signature(input), "Check this out");

        // Unsubscribe footer
        let input = "Important message\n\nYou are receiving this because you subscribed.";
        assert_eq!(strip_signature(input), "Important message");

        // No signature
        let input = "Just a regular email\nWith multiple lines\nNo signature here";
        assert_eq!(
            strip_signature(input),
            "Just a regular email\nWith multiple lines\nNo signature here"
        );

        // Multiple dashes
        let input = "Content\n---\nSignature line";
        assert_eq!(strip_signature(input), "Content");

        // Asterisks
        let input = "Content\n***\nSignature";
        assert_eq!(strip_signature(input), "Content");
    }

    #[test]
    fn test_strip_quoted_replies() {
        // Simple quoted reply
        let input = "Hi Foo, I hope you had a great holiday weekend.\r\n\r\nOn Tue, Jul 1, 2025 at 1:43 PM Foo Bar <foo@example.com> wrote:\r\n\r\n> Hi Bar - it was great connecting with you";
        assert_eq!(
            strip_quoted_replies(input),
            "Hi Foo, I hope you had a great holiday weekend."
        );

        // Nested quoted replies
        let input = "New message here\r\n\r\nOn Mon, Jun 23 at 5:21 PM Bar <bar@example.com> wrote:\r\n\r\n> Hi Foo, thanks for getting back to me.\r\n>\r\n>> On Fri, Jun 20 at 1:20 PM Foo wrote:\r\n>\r\n>>>> Hi Bar - thanks for your patience";
        assert_eq!(strip_quoted_replies(input), "New message here");

        // Lines starting with >
        let input = "Main content\n> Quoted line 1\n>> Double quoted\n> Quoted line 2";
        assert_eq!(strip_quoted_replies(input), "Main content");

        // No quoted replies
        let input = "Just a regular email\nWith no quotes";
        assert_eq!(
            strip_quoted_replies(input),
            "Just a regular email\nWith no quotes"
        );

        // Unix line endings
        let input = "Hello world\n\nOn Tue, Jul 1, 2025 at 1:43 PM Foo wrote:\n\n> Quoted content";
        assert_eq!(strip_quoted_replies(input), "Hello world");
    }

    #[test]
    fn test_base64_url_no_pad() {
        // Basic encoding - URL_SAFE includes padding
        assert_eq!(base64_url_no_pad("Hello"), "SGVsbG8=");
        assert_eq!(base64_url_no_pad("World"), "V29ybGQ=");

        // Empty string
        assert_eq!(base64_url_no_pad(""), "");

        // Special characters (URL-safe)
        assert_eq!(base64_url_no_pad("test+value/with=special"), "dGVzdCt2YWx1ZS93aXRoPXNwZWNpYWw=");

        // Binary-like data
        assert_eq!(base64_url_no_pad("\x00\x01\x02"), "AAEC");
    }

    #[test]
    fn test_clean_and_strip_body() {
        // Basic plain text with signature
        let input = "Hello world\n\nBest regards,\nJohn".to_string();
        assert_eq!(clean_and_strip_body(input), "Hello world");

        // Quoted-printable with signature
        let input = "Don=E2=80=99t stop\n\nThanks,\nTeam".to_string();
        assert_eq!(clean_and_strip_body(input), "Don't stop");

        // HTML entities with signature
        let input = "Test &amp; more\n\nRegards,\nBob".to_string();
        assert_eq!(clean_and_strip_body(input), "Test & more");

        // With quoted reply
        let input = "Main content\n\nOn Tue, Jul 1 at 1:43 PM wrote:\n> quoted".to_string();
        assert_eq!(clean_and_strip_body(input), "Main content");

        // No signature or quotes
        let input = "Just a regular message\nwith multiple lines".to_string();
        assert_eq!(clean_and_strip_body(input), "Just a regular message\nwith multiple lines");
    }

    #[test]
    fn test_extract_subject() {
        // Normal subject
        let message = create_message_with_headers("Test Subject", "From: <from@example.com>", "To: <to@example.com>");
        assert_eq!(extract_subject(&message), "Test Subject");

        // Subject with unicode
        let message = create_message_with_headers("Don=E2=80=99t fear", "From: <from@example.com>", "To: <to@example.com>");
        assert_eq!(extract_subject(&message), "Don't fear");

        // Subject with HTML entities
        let message = create_message_with_headers("Test &amp; more", "From: <from@example.com>", "To: <to@example.com>");
        assert_eq!(extract_subject(&message), "Test & more");

        // Empty payload
        let message = Message {
            id: "test".to_string(),
            thread_id: "thread".to_string(),
            snippet: None,
            payload: None,
            label_ids: None,
            internal_date: "0".to_string(),
        };
        assert_eq!(extract_subject(&message), "");

        // No subject header
        let payload = MessagePayload {
            headers: Some(vec![
                MessageHeader { name: "From".to_string(), value: "test@example.com".to_string() },
            ]),
            mimetype: "text/plain".to_string(),
            body: None,
            parts: None,
        };
        let message = Message {
            id: "test".to_string(),
            thread_id: "thread".to_string(),
            snippet: None,
            payload: Some(payload),
            label_ids: None,
            internal_date: "0".to_string(),
        };
        assert_eq!(extract_subject(&message), "");
    }

    #[test]
    fn test_extract_from() {
        // Normal from
        let message = create_message_with_headers("Subject", "From: Alice <alice@example.com>", "To: <to@example.com>");
        assert_eq!(extract_from(&message), "Alice <alice@example.com>");

        // From with unicode
        let message = create_message_with_headers("Subject", "From: =E2=80=9CJohn=E2=80=9D <john@example.com>", "To: <to@example.com>");
        assert_eq!(extract_from(&message), "\"John\" <john@example.com>");

        // Empty payload
        let message = Message {
            id: "test".to_string(),
            thread_id: "thread".to_string(),
            snippet: None,
            payload: None,
            label_ids: None,
            internal_date: "0".to_string(),
        };
        assert_eq!(extract_from(&message), "");

        // No from header
        let payload = MessagePayload {
            headers: Some(vec![
                MessageHeader { name: "Subject".to_string(), value: "Test".to_string() },
            ]),
            mimetype: "text/plain".to_string(),
            body: None,
            parts: None,
        };
        let message = Message {
            id: "test".to_string(),
            thread_id: "thread".to_string(),
            snippet: None,
            payload: Some(payload),
            label_ids: None,
            internal_date: "0".to_string(),
        };
        assert_eq!(extract_from(&message), "");
    }

    #[test]
    fn test_extract_to() {
        // Normal to
        let message = create_message_with_headers("Subject", "From: <from@example.com>", "To: Bob <bob@example.org>");
        assert_eq!(extract_to(&message), "Bob <bob@example.org>");

        // Multiple recipients
        let message = create_message_with_headers("Subject", "From: <from@example.com>", "To: a@a.com, b@b.com");
        assert_eq!(extract_to(&message), "a@a.com, b@b.com");

        // Empty payload
        let message = Message {
            id: "test".to_string(),
            thread_id: "thread".to_string(),
            snippet: None,
            payload: None,
            label_ids: None,
            internal_date: "0".to_string(),
        };
        assert_eq!(extract_to(&message), "");

        // No to header
        let payload = MessagePayload {
            headers: Some(vec![
                MessageHeader { name: "Subject".to_string(), value: "Test".to_string() },
            ]),
            mimetype: "text/plain".to_string(),
            body: None,
            parts: None,
        };
        let message = Message {
            id: "test".to_string(),
            thread_id: "thread".to_string(),
            snippet: None,
            payload: Some(payload),
            label_ids: None,
            internal_date: "0".to_string(),
        };
        assert_eq!(extract_to(&message), "");
    }

    #[test]
    fn test_extract_body() {
        // Body in payload.body (text/plain)
        let body_data = base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE, "Hello World");
        let payload = MessagePayload {
            headers: Some(vec![
                MessageHeader { name: "Subject".to_string(), value: "Test".to_string() },
            ]),
            mimetype: "text/plain".to_string(),
            body: Some(MessagePartBody {
                attachment_id: None,
                size: 11,
                data: Some(body_data),
            }),
            parts: None,
        };
        let message = Message {
            id: "test".to_string(),
            thread_id: "thread".to_string(),
            snippet: None,
            payload: Some(payload),
            label_ids: None,
            internal_date: "0".to_string(),
        };
        let result = extract_body(&message);
        assert!(result.contains("Hello World"));

        // Body in parts (text/plain)
        let body_data = base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE, "Plain text body");
        let parts = vec![MessagePart {
            part_id: "1".to_string(),
            mimetype: "text/plain".to_string(),
            body: Some(MessagePartBody {
                attachment_id: None,
                size: 16,
                data: Some(body_data),
            }),
        }];
        let payload = MessagePayload {
            headers: Some(vec![
                MessageHeader { name: "Subject".to_string(), value: "Test".to_string() },
            ]),
            mimetype: "multipart/alternative".to_string(),
            body: None,
            parts: Some(parts),
        };
        let message = Message {
            id: "test".to_string(),
            thread_id: "thread".to_string(),
            snippet: None,
            payload: Some(payload),
            label_ids: None,
            internal_date: "0".to_string(),
        };
        let result = extract_body(&message);
        assert!(result.contains("Plain text body"));

        // Fallback to snippet - note: this requires payload with no body/parts
        let empty_payload = MessagePayload {
            headers: Some(vec![
                MessageHeader { name: "Subject".to_string(), value: "Test".to_string() },
            ]),
            mimetype: "text/plain".to_string(),
            body: None,
            parts: None,
        };
        let message = Message {
            id: "test".to_string(),
            thread_id: "thread".to_string(),
            snippet: Some("This is a snippet...".to_string()),
            payload: Some(empty_payload),
            label_ids: None,
            internal_date: "0".to_string(),
        };
        let result = extract_body(&message);
        assert_eq!(result, "This is a snippet...");
    }

    // Helper function to create a message with headers for testing
    fn create_message_with_headers(subject: &str, from_header: &str, to_header: &str) -> Message {
        let headers = vec![
            MessageHeader { name: "Subject".to_string(), value: subject.to_string() },
            parse_header(from_header),
            parse_header(to_header),
        ];
        let payload = MessagePayload {
            headers: Some(headers),
            mimetype: "text/plain".to_string(),
            body: None,
            parts: None,
        };
        Message {
            id: "test".to_string(),
            thread_id: "thread".to_string(),
            snippet: None,
            payload: Some(payload),
            label_ids: None,
            internal_date: "0".to_string(),
        }
    }

    fn parse_header(header_str: &str) -> MessageHeader {
        let parts: Vec<&str> = header_str.splitn(2, ": ").collect();
        MessageHeader {
            name: parts[0].to_string(),
            value: parts.get(1).unwrap_or(&"").to_string(),
        }
    }

    #[tokio::test]
    async fn test_list_unread_messages() {
        let mut server = mockito::Server::new_async().await;
        let url = server.url();

        // Create a mock for the Gmail API - use wildcard matching
        let mock_resp = r#"{"messages": [{"id": "msg_001", "threadId": "thr_001"}], "nextPageToken": null}"#;
        let _mock = server
            .mock("GET", "/gmail/v1/users/me/messages")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_resp)
            .match_query(mockito::Matcher::Regex(r"labelIds=UNREAD".to_string()))
            .create();

        // Override the URL construction to use mock server
        let client = reqwest::Client::new();
        let after_date = (chrono::Utc::now() - chrono::Duration::days(1))
            .format("%Y/%m/%d")
            .to_string();
        let request_url = format!(
            "{}/gmail/v1/users/me/messages?labelIds=UNREAD&q=is:unread%20after:{}%20in:inbox",
            url, after_date
        );
        let res = client.get(&request_url).bearer_auth("test_token").send().await.unwrap();
        let status = res.status();
        assert!(status.is_success());

        let text = res.text().await.unwrap();
        let msgs: ListMessagesResponse = serde_json::from_str(&text).unwrap();
        assert_eq!(msgs.messages.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_fetch_thread() {
        let mut server = mockito::Server::new_async().await;
        let url = server.url();

        // Create a mock for the Gmail thread API
        let mock_resp = r#"{
            "id": "thr_001",
            "messages": [
                {
                    "id": "msg_001a",
                    "threadId": "thr_001",
                    "snippet": "Test snippet",
                    "labelIds": ["INBOX"],
                    "internalDate": "1731401723000",
                    "payload": {
                        "mimeType": "text/plain",
                        "headers": [
                            {"name": "From", "value": "test@example.com"},
                            {"name": "To", "value": "me@example.org"},
                            {"name": "Subject", "value": "Test Thread"}
                        ],
                        "body": {
                            "attachmentId": null,
                            "size": 10,
                            "data": "SGVsbG8gV29ybGQ="
                        }
                    }
                }
            ]
        }"#;
        let _mock = server
            .mock("GET", "/gmail/v1/users/me/threads/thr_001?format=full")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_resp)
            .create();

        let client = reqwest::Client::new();
        let request_url = format!("{}/gmail/v1/users/me/threads/thr_001?format=full", url);
        let res = client.get(&request_url).bearer_auth("test_token").send().await.unwrap();
        let status = res.status();
        assert!(status.is_success());

        let text = res.text().await.unwrap();
        let thread: Thread = serde_json::from_str(&text).unwrap();
        assert_eq!(thread.id, "thr_001");
        assert_eq!(thread.messages.len(), 1);
    }

    #[tokio::test]
    async fn test_list_unread_messages_error() {
        let mut server = mockito::Server::new_async().await;
        let url = server.url();

        // Create a mock that returns an error - use wildcard matching
        let _mock = server
            .mock("GET", "/gmail/v1/users/me/messages")
            .with_status(401)
            .with_header("content-type", "application/json")
            .with_body(r#"{"error": {"message": "Unauthorized"}}"#)
            .match_query(mockito::Matcher::Regex(r"labelIds=UNREAD".to_string()))
            .create();

        let client = reqwest::Client::new();
        let after_date = (chrono::Utc::now() - chrono::Duration::days(1))
            .format("%Y/%m/%d")
            .to_string();
        let request_url = format!(
            "{}/gmail/v1/users/me/messages?labelIds=UNREAD&q=is:unread%20after:{}%20in:inbox",
            url, after_date
        );
        let res = client.get(&request_url).bearer_auth("bad_token").send().await.unwrap();
        let status = res.status();
        assert!(!status.is_success());
    }
}
