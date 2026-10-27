use super::*;

// Frozen pre-optimization control flow; preserves the former line-end lookup policy.
fn reference_parse(body: &str) -> Result<Vec<Wikilink<'_>>, WikilinkParseError> {
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
            let content = &body[offset + 2..end];
            let mut link = parse_link_content(content, offset)?;
            link.from = if offset > 0 && bytes[offset - 1] == b'!' {
                offset - 1
            } else {
                offset
            };
            link.to = end + 2;
            link.label_from = offset + 2 + content.find('|').map_or(0, |index| index + 1);
            link.label_to = end;
            links.push(link);
            offset = end + 2;
            continue;
        }
        offset += 1;
    }

    Ok(links)
}

#[test]
fn long_lines_keep_exact_targets_ranges_and_errors() {
    let paragraph = "Synthetic Unicode ☃ 世界 paragraph. ".repeat(96);
    for newline in ["\n", "\r\n", ""] {
        for suffix in [
            "[[Projects/example/entities/core|Ядро]] ![[Global/entity#Heading]] [[#Local]]",
            "\\[[escaped]] \\\\[[visible]] `[[code]]` <!-- [[comment]] --> %%[[comment]]%% [[ok]]",
            "[[unterminated",
            "[[target|]]",
            "[[target#]]",
        ] {
            let body = format!("{paragraph}{suffix}{newline}");
            assert_eq!(parse_wikilinks(&body), reference_parse(&body));
        }
    }
    let body = format!("{paragraph}[[target|☃ 世界]]");
    let links = parse_wikilinks(&body).unwrap();
    assert_eq!(links[0].from, paragraph.len());
    assert_eq!(&body[links[0].label_from..links[0].label_to], "☃ 世界");
    let body = format!("{paragraph}[[broken");
    let error = parse_wikilinks(&body).unwrap_err();
    assert_eq!(error.offset, paragraph.len());
    assert_eq!(
        error.message,
        "opening [[ has no closing ]] on the same line"
    );
}

#[test]
fn multiline_skips_refresh_line_boundaries_without_changing_fences() {
    let cases = [
        "prefix <!-- one\ntwo\n--> [[after]]\n```md\n[[hidden]]\n```\n[[last]]",
        "prefix %%one\r\ntwo\r\n%% [[after]]\r\n~~~\r\n[[hidden]]\r\n~~~\r\n[[last]]",
        "`code\n[[hidden]]\ncode` [[after]]\n    ``` [[visible]]\n[[last]]",
        "<!--one\n-->\n```md\n[[hidden]]\n```\n![[last|末尾]]",
        "%%one\n%%\n~~~\n[[hidden]]\n~~~\n[[last]]",
        "```md\n[[hidden]]\n````\n[[last]]\n",
        "[[first]] <!--\ncomment\n--> [[broken\n[[next]]",
        "[[first]] %%\ncomment\n%% [[broken\r\n[[next]]",
        "``one\ntwo`` [[first]]\n[[broken\n]]",
        "<!--unterminated\n[[hidden]]",
        "%%unterminated\n[[hidden]]",
    ];
    for body in cases {
        assert_eq!(parse_wikilinks(body), reference_parse(body), "{body:?}");
    }
    let links = parse_wikilinks(cases[0]).unwrap();
    assert_eq!(
        links.iter().map(|link| link.target).collect::<Vec<_>>(),
        [Some("after"), Some("last")]
    );
}

#[test]
fn seeded_syntax_corpus_matches_the_previous_parser() {
    let tokens = [
        "plain",
        "☃ 世界",
        "\n",
        "\r\n",
        "[[a]]",
        "[[a|Alias]]",
        "![[a#Heading]]",
        "[[#local]]",
        "\\[[escaped]]",
        "\\\\[[a]]",
        "`",
        "``",
        "```",
        "~~~",
        "   ",
        "<!--",
        "-->",
        "%%",
        "[[broken",
        "[[]]",
        "[[a|]]",
        "[[a#]]",
        "[[a|b|c]]",
    ];
    let mut seed = 0x8a7c_24d1_u64;
    for _ in 0..2_000 {
        let mut body = String::new();
        for _ in 0..48 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            body.push_str(tokens[(seed >> 32) as usize % tokens.len()]);
        }
        assert_eq!(parse_wikilinks(&body), reference_parse(&body), "{body:?}");
    }
}

#[test]
fn empty_and_newline_boundaries_keep_previous_semantics() {
    for body in [
        "",
        "\n",
        "\r\n",
        "\r",
        "a\n\n[[b]]",
        "[[a]]\n[[b]]",
        "[[a\n]]",
        "<!--x-->\n[[a]]",
        "`x`\n[[a]]",
    ] {
        assert_eq!(parse_wikilinks(body), reference_parse(body), "{body:?}");
    }
}

#[test]
#[ignore = "manual wikilink scan measurement; run with --ignored --nocapture"]
fn synthetic_wikilink_runtime() {
    use std::time::Instant;
    for repeats in [64, 512, 2_048] {
        let body = format!(
            "{} [[Projects/example/entities/core|☃ 世界]]\r\n",
            "Synthetic Unicode ☃ 世界 paragraph. ".repeat(repeats)
        );
        let started = Instant::now();
        let expected = std::hint::black_box(reference_parse(&body).unwrap());
        let reference_ns = started.elapsed().as_nanos();
        let started = Instant::now();
        for _ in 0..20 {
            assert_eq!(
                std::hint::black_box(parse_wikilinks(&body).unwrap()),
                expected
            );
        }
        let current_ns = started.elapsed().as_nanos() / 20;
        println!(
            "{}",
            serde_json::json!({"body_bytes": body.len(), "reference_ns": reference_ns, "current_ns": current_ns, "exact_parity": true})
        );
    }
}
