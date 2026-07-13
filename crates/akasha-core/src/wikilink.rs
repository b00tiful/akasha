use std::error::Error;
use std::fmt;

/// One validated wikilink occurrence in a canonical Markdown body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Wikilink<'a> {
    pub(crate) target: Option<&'a str>,
}

/// A malformed wikilink owned by Akasha's limited Markdown subset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WikilinkParseError {
    offset: usize,
    message: &'static str,
}

impl fmt::Display for WikilinkParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "malformed wikilink at byte {}: {}",
            self.offset, self.message
        )
    }
}

impl Error for WikilinkParseError {}

#[derive(Debug, Clone, Copy)]
struct Fence {
    marker: u8,
    length: usize,
}

/// Parse the wikilink subset used for Akasha's semantic note graph.
pub(crate) fn parse_wikilinks(body: &str) -> Result<Vec<Wikilink<'_>>, WikilinkParseError> {
    let bytes = body.as_bytes();
    let mut links = Vec::new();
    let mut offset = 0;
    let mut fence = None;
    let mut line_start = true;

    while offset < bytes.len() {
        let line_end = find_line_end(bytes, offset);
        if line_start {
            let line = &bytes[offset..line_end];
            let marker = fence_marker(line);
            match (fence, marker) {
                (Some(open), Some(marker)) if is_closing_fence(line, marker, open) => {
                    fence = None;
                    offset = next_line_offset(line_end, bytes.len());
                    continue;
                }
                (Some(_), _) => {
                    offset = next_line_offset(line_end, bytes.len());
                    continue;
                }
                (None, Some(marker)) => {
                    fence = Some(Fence {
                        marker: marker.0,
                        length: marker.1,
                    });
                    offset = next_line_offset(line_end, bytes.len());
                    continue;
                }
                (None, None) => line_start = false,
            }
        }

        if bytes[offset] == b'\n' {
            offset += 1;
            line_start = true;
            continue;
        }
        if bytes[offset..].starts_with(b"<!--") {
            offset = bytes[offset + 4..]
                .windows(3)
                .position(|window| window == b"-->")
                .map_or(bytes.len(), |relative| offset + 4 + relative + 3);
            line_start = offset > 0 && bytes[offset - 1] == b'\n';
            continue;
        }
        if bytes[offset..].starts_with(b"%%") {
            offset = bytes[offset + 2..]
                .windows(2)
                .position(|window| window == b"%%")
                .map_or(bytes.len(), |relative| offset + 2 + relative + 2);
            line_start = offset > 0 && bytes[offset - 1] == b'\n';
            continue;
        }
        if bytes[offset] == b'`' {
            let ticks = byte_run(bytes, offset, b'`');
            if let Some(closing) =
                find_matching_run(bytes, offset + ticks, bytes.len(), b'`', ticks)
            {
                offset = closing + ticks;
                continue;
            }
            offset += ticks;
            continue;
        }
        if bytes[offset..].starts_with(b"[[") && !is_escaped(bytes, offset) {
            let Some(relative_end) = bytes[offset + 2..line_end]
                .windows(2)
                .position(|window| window == b"]]")
            else {
                return Err(WikilinkParseError {
                    offset,
                    message: "opening [[ has no closing ]] on the same line",
                });
            };
            let end = offset + 2 + relative_end;
            links.push(parse_link_content(&body[offset + 2..end], offset)?);
            offset = end + 2;
            continue;
        }
        offset += 1;
    }

    Ok(links)
}

fn parse_link_content(content: &str, offset: usize) -> Result<Wikilink<'_>, WikilinkParseError> {
    if content.is_empty() {
        return Err(WikilinkParseError {
            offset,
            message: "link content is empty",
        });
    }

    let (destination, alias) = content
        .split_once('|')
        .map_or((content, None), |(destination, alias)| {
            (destination, Some(alias))
        });
    if alias.is_some_and(|value| value.trim().is_empty() || value.contains('|')) {
        return Err(WikilinkParseError {
            offset,
            message: "link alias must be non-empty and contain at most one | separator",
        });
    }

    let (target, fragment) = destination
        .split_once('#')
        .map_or((destination, None), |(target, fragment)| {
            (target, Some(fragment))
        });
    if fragment.is_some_and(|value| value.trim().is_empty() || value.starts_with('#')) {
        return Err(WikilinkParseError {
            offset,
            message: "link fragment must name one heading or block",
        });
    }
    if target.is_empty() {
        if fragment.is_none() {
            return Err(WikilinkParseError {
                offset,
                message: "link target is empty",
            });
        }
        return Ok(Wikilink { target: None });
    }
    if target.trim() != target {
        return Err(WikilinkParseError {
            offset,
            message: "link target has leading or trailing whitespace",
        });
    }

    Ok(Wikilink {
        target: Some(target),
    })
}

fn fence_marker(line: &[u8]) -> Option<(u8, usize, usize)> {
    let indentation = line.iter().take_while(|byte| **byte == b' ').count();
    if indentation > 3 || indentation == line.len() {
        return None;
    }
    let marker = line[indentation];
    if marker != b'`' && marker != b'~' {
        return None;
    }
    let length = byte_run(line, indentation, marker);
    (length >= 3).then_some((marker, length, indentation + length))
}

fn is_closing_fence(line: &[u8], marker: (u8, usize, usize), open: Fence) -> bool {
    marker.0 == open.marker
        && marker.1 >= open.length
        && line[marker.2..]
            .iter()
            .all(|byte| *byte == b' ' || *byte == b'\t' || *byte == b'\r')
}

fn byte_run(bytes: &[u8], start: usize, needle: u8) -> usize {
    bytes[start..]
        .iter()
        .take_while(|byte| **byte == needle)
        .count()
}

fn find_matching_run(
    bytes: &[u8],
    mut offset: usize,
    end: usize,
    needle: u8,
    length: usize,
) -> Option<usize> {
    while offset < end {
        if bytes[offset] == needle {
            let run = byte_run(&bytes[..end], offset, needle);
            if run == length {
                return Some(offset);
            }
            offset += run;
        } else {
            offset += 1;
        }
    }
    None
}

fn is_escaped(bytes: &[u8], offset: usize) -> bool {
    let backslashes = bytes[..offset]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count();
    backslashes % 2 == 1
}

fn find_line_end(bytes: &[u8], offset: usize) -> usize {
    bytes[offset..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(bytes.len(), |relative| offset + relative)
}

const fn next_line_offset(line_end: usize, body_len: usize) -> usize {
    if line_end < body_len {
        line_end + 1
    } else {
        body_len
    }
}

#[cfg(test)]
mod tests {
    use super::parse_wikilinks;

    #[test]
    fn parses_supported_links_and_ignores_non_semantic_regions() {
        let body = "[[Projects/example/entities/core|Core]] and \\[[escaped]]\n\
                    `inline\n[[inline-code]]\ncode`\n\
                    ```md\n[[fenced-code]]\n```\n\
                    <!-- multiline\n[[html-comment]]\n--> %% multiline\n[[obsidian-comment]]\n%%\n\
                    ![[Projects/example/entities/core.md#Details]] [[#Local heading]]\n";

        let links = parse_wikilinks(body).expect("parse links");
        assert_eq!(links.len(), 3);
        assert_eq!(links[0].target, Some("Projects/example/entities/core"));
        assert_eq!(links[1].target, Some("Projects/example/entities/core.md"));
        assert_eq!(links[2].target, None);
    }

    #[test]
    fn rejects_malformed_links() {
        for body in [
            "[[]]",
            "[[target|]]",
            "[[target|   ]]",
            "[[target|one|two]]",
            "[[target#]]",
            "[[target#   ]]",
            "[[## search]]",
            "[[unterminated",
        ] {
            assert!(parse_wikilinks(body).is_err(), "accepted {body:?}");
        }
    }
}
