use booknook_core::feed_xml::{NormalizedFeed, normalize};
const CONTEXT: &[u8] = br#"{"schema":"booknook.feed-normalization-context.v1","source_id":"b41ef5e8-0e06-43e0-aa47-19f382e00c08","provider_id":"reader.public","requested_url":"https://example.org/feed.xml","final_url":"https://example.org/feed.xml","expected_format":"auto"}"#;
fn parse(xml: &str) -> NormalizedFeed {
    normalize(CONTEXT, xml.as_bytes()).unwrap()
}

#[test]
fn literal_string_normalization_format_rejects_all_non_string_shapes() {
    use serde_json::{Value, json};
    for literal in ["auto", "rss", "atom"] {
        let mut context: Value = serde_json::from_slice(CONTEXT).unwrap();
        context["expected_format"] = json!(literal);
        let xml = if literal == "atom" {
            "<feed xmlns='http://www.w3.org/2005/Atom'><title>T</title></feed>"
        } else {
            "<rss version='2.0'><channel/></rss>"
        };
        assert!(normalize(&serde_json::to_vec(&context).unwrap(), xml.as_bytes()).is_ok());
        for bad in [
            Value::Null,
            json!(false),
            json!(1),
            json!([]),
            json!([literal]),
            json!({literal:null}),
        ] {
            context["expected_format"] = bad;
            assert!(
                normalize(&serde_json::to_vec(&context).unwrap(), xml.as_bytes()).is_err(),
                "{context}"
            );
        }
    }
}
#[test]
fn rss_preserves_native_identity_and_dates_without_fetch_time() {
    let feed = parse(
        r#"<rss version="2.0"><channel><title>Essays</title><item><guid> opaque ID </guid><title>Café &amp; sea</title><pubDate>Tue, 03 Jun 2003 09:39:21 GMT</pubDate></item></channel></rss>"#,
    );
    assert_eq!(feed.schema, "booknook.normalized-feed.v1");
    let value = serde_json::to_value(&feed).unwrap();
    assert_eq!(value["entries"][0]["identity"]["value"], " opaque ID ");
    assert_eq!(value["entries"][0]["published"]["status"], "parsed");
    assert_eq!(value["entries"][0]["updated"]["status"], "missing");
    assert_eq!(value["entries"][0]["title"]["text"], "Café & sea");
    assert_eq!(value["provenance"]["raw_sha256"],booknook_core::digest(r#"<rss version="2.0"><channel><title>Essays</title><item><guid> opaque ID </guid><title>Café &amp; sea</title><pubDate>Tue, 03 Jun 2003 09:39:21 GMT</pubDate></item></channel></rss>"#.as_bytes()));
}
#[test]
fn atom_html_is_explicitly_literal_and_xhtml_has_no_active_markup() {
    let feed = parse(
        r#"<feed xmlns="http://www.w3.org/2005/Atom"><title>T</title><entry><id>urn:opaque</id><title type="html">&lt;b onclick="run()"&gt;Title&lt;/b&gt;</title><content type="xhtml"><div xmlns="http://www.w3.org/1999/xhtml"><p>Hello <b>there</b></p><script>bad()</script><style>bad{}</style><img src="https://bad.example/pixel"/></div></content><link href="javascript:bad()"/><link href="/article"/></entry></feed>"#,
    );
    let v = serde_json::to_value(feed).unwrap();
    assert_eq!(v["entries"][0]["title"]["representation"], "literal-markup");
    assert_eq!(v["entries"][0]["content"]["text"], "Hello there");
    assert_eq!(
        v["entries"][0]["links"][0]["url"],
        "https://example.org/article"
    );
    assert_eq!(v["entries"][0]["omitted_links"], 1);
}
#[test]
fn refuses_dtd_deep_and_namespace_spoof_before_dom() {
    for xml in [
        "<!DOCTYPE rss [<!ENTITY x 'bad'>]><rss version='2.0'><channel/></rss>".to_owned(),
        format!(
            "<rss version='2.0'><channel>{}{}</channel></rss>",
            "<x>".repeat(64),
            "</x>".repeat(64)
        ),
        "<feed xmlns='https://attacker.example/Atom'><entry/></feed>".to_owned(),
    ] {
        assert!(normalize(CONTEXT, xml.as_bytes()).is_err());
    }
}
#[test]
fn comments_and_cdata_declaration_lookalikes_are_literal_not_entities() {
    let feed = parse(
        r#"<rss version="2.0"><channel><item><title>A<!--PRIVATE-->B</title><description><![CDATA[<!DOCTYPE rss><script>never executed</script>]]></description></item></channel></rss>"#,
    );
    let v = serde_json::to_value(feed).unwrap();
    assert_eq!(v["entries"][0]["title"]["text"], "AB");
    assert!(
        v["entries"][0]["summary"]["text"]
            .as_str()
            .unwrap()
            .contains("<!DOCTYPE rss>")
    );
}
#[test]
fn nesting_limit_accepts64_and_rejects65_or_very_deep_before_dom() {
    for (depth, accepted) in [(64, true), (65, false), (30_000, false)] {
        let xml = format!(
            "<rss version='2.0'><channel>{}{}</channel></rss>",
            "<extension>".repeat(depth - 2),
            "</extension>".repeat(depth - 2)
        );
        assert_eq!(
            normalize(CONTEXT, xml.as_bytes()).is_ok(),
            accepted,
            "depth {depth}"
        );
    }
    let xml = format!(
        "<rss version='2.0'><channel>{}<empty/>{}</channel></rss>",
        "<x>".repeat(62),
        "</x>".repeat(62)
    );
    assert!(normalize(CONTEXT, xml.as_bytes()).is_err());
}
#[test]
fn whole_candidate_limits_are_not_silent_truncation() {
    for (items, ok) in [(500, true), (501, false)] {
        let xml = format!(
            "<rss version='2.0'><channel>{}</channel></rss>",
            "<item/>".repeat(items)
        );
        assert_eq!(normalize(CONTEXT, xml.as_bytes()).is_ok(), ok);
    }
    for (attributes, ok) in [(63, true), (64, false)] {
        let attrs = (0..attributes)
            .map(|n| format!(" a{n}='x'"))
            .collect::<String>();
        let xml = format!("<rss version='2.0'{attrs}><channel/></rss>");
        assert_eq!(normalize(CONTEXT, xml.as_bytes()).is_ok(), ok);
    }
    for xml in [
        format!(
            "<rss version='2.0'><channel>{}</channel></rss>",
            "<!--x-->".repeat(50_000)
        ),
        format!(
            "<rss version='2.0' x='{}'><channel/></rss>",
            "x".repeat(65_537)
        ),
        format!(
            "<rss version='2.0'><channel><title>{}</title></channel></rss>",
            "x".repeat(65_537)
        ),
    ] {
        assert!(normalize(CONTEXT, xml.as_bytes()).is_err());
    }
    assert!(normalize(CONTEXT, &vec![b' '; 2 * 1024 * 1024 + 1]).is_err());
}
#[test]
fn encoding_wellformedness_and_all_dtd_forms_fail_closed() {
    for xml in [
        "<!DOCTYPE rss><rss version='2.0'><channel/></rss>",
        "<!DOCTYPE rss SYSTEM 'https://example.org/evil'><rss version='2.0'><channel/></rss>",
        "<!DOCTYPE rss [<!ENTITY % x SYSTEM 'file:///etc/passwd'>%x;]><rss/>",
        "<rss version='2.0'><channel>&unknown;</channel></rss>",
        "<?xml version='1.0' encoding='iso-8859-1'?><rss version='2.0'><channel/></rss>",
        "<rss version='2.0'><channel></rss>",
        "<rss version='2.0' version='2.0'><channel/></rss>",
        "<rss version='2.0'><channel/><channel/></rss>",
    ] {
        assert!(normalize(CONTEXT, xml.as_bytes()).is_err(), "{xml}");
    }
    for bytes in [vec![0xff, 0xfe, b'<', 0], vec![0xf0, 0x28, 0x8c, 0x28]] {
        assert!(normalize(CONTEXT, &bytes).is_err());
    }
}
#[test]
fn namespace_aliases_work_but_extension_lookalikes_are_not_promoted() {
    let feed = parse(
        r#"<a:feed xmlns:a="http://www.w3.org/2005/Atom" xmlns:fake="https://evil.example/ns"><a:title>Actual</a:title><fake:title>Fake</fake:title><a:entry><a:id>native</a:id><fake:id>forged</fake:id><fake:title>Hidden</fake:title><a:updated>invalid-date</a:updated></a:entry><fake:entry><a:title>forged</a:title></fake:entry></a:feed>"#,
    );
    let v = serde_json::to_value(feed).unwrap();
    assert_eq!(v["entries"].as_array().unwrap().len(), 1);
    assert_eq!(v["entries"][0]["identity"]["value"], "native");
    assert!(v["entries"][0]["title"].is_null());
    assert_eq!(v["entries"][0]["updated"]["status"], "invalid");
    assert_eq!(v["entries"][0]["updated"]["raw"], "invalid-date");
    let rss = parse(
        r#"<rss version="2.0" xmlns:x="https://evil.example/ns"><channel><item><guid>real</guid><x:guid>forged</x:guid><x:title>hidden</x:title></item></channel></rss>"#,
    );
    assert!(serde_json::to_value(rss).unwrap()["entries"][0]["title"].is_null());
}
#[test]
fn duplicates_conflicts_and_fallback_ids_are_deterministic() {
    let a = "<item><guid>same</guid><title>A</title></item>";
    let b = "<item><guid>same</guid><title>B</title></item>";
    let c = "<item><title>Anonymous</title></item>";
    let mut first = parse(&format!(
        "<rss version='2.0'><channel>{a}{b}{a}{c}</channel></rss>"
    ));
    let mut second = parse(&format!(
        "<rss version='2.0'><channel>{c}{a}{a}{b}</channel></rss>"
    ));
    assert_eq!(
        first
            .entries
            .iter()
            .find(|e| e.duplicate_count == 2)
            .unwrap()
            .first_source_ordinal,
        0
    );
    assert_eq!(
        second
            .entries
            .iter()
            .find(|e| e.duplicate_count == 2)
            .unwrap()
            .first_source_ordinal,
        1
    );
    for item in first.entries.iter_mut().chain(second.entries.iter_mut()) {
        item.first_source_ordinal = 0;
    }
    assert_eq!(
        serde_json::to_value(&first.entries).unwrap(),
        serde_json::to_value(second.entries).unwrap()
    );
    assert_eq!(first.entries.len(), 3);
    assert_eq!(
        first
            .entries
            .iter()
            .filter(|e| e.conflicting_identity)
            .count(),
        2
    );
    assert!(first.entries.iter().any(|e| e.duplicate_count == 2));
    assert!(
        serde_json::to_value(first.entries)
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["identity"]["kind"] == "derived-sha256")
    );
}
#[test]
fn xml_base_links_are_explicit_safe_data_and_external_content_is_omitted() {
    let feed = parse(
        r#"<feed xmlns="http://www.w3.org/2005/Atom" xml:base="https://other.example.org/essays/"><entry xml:base="chapter/"><id>x</id><content src="file:///etc/passwd"/><link href="page"/><link href="data:text/html,bad"/><link href="file:///etc/passwd"/><link href="https://reader:secret@example.org/page"/><link href=""/></entry></feed>"#,
    );
    let v = serde_json::to_value(feed).unwrap();
    assert_eq!(v["entries"][0]["links"].as_array().unwrap().len(), 1);
    assert_eq!(
        v["entries"][0]["links"][0]["url"],
        "https://other.example.org/essays/chapter/page"
    );
    assert_eq!(v["entries"][0]["omitted_links"], 4);
    assert_eq!(v["entries"][0]["omitted_content"], true);
}

#[test]
fn context_reuses_source_port_provider_ids_and_exact_format() {
    let mut context: serde_json::Value = serde_json::from_slice(CONTEXT).unwrap();
    for provider in ["reader:public".to_owned(), "x".repeat(200)] {
        context["provider_id"] = provider.into();
        let raw = serde_json::to_vec(&context).unwrap();
        assert!(normalize(&raw, b"<rss version='2.0'><channel/></rss>").is_ok());
    }
    context["expected_format"] = "atom".into();
    assert!(
        normalize(
            &serde_json::to_vec(&context).unwrap(),
            b"<rss version='2.0'><channel/></rss>"
        )
        .is_err()
    );
    assert!(
        normalize(
            br#"{"schema":"booknook.feed-normalization-context.v1","schema":"duplicate"}"#,
            b"<rss/>"
        )
        .is_err()
    );
}
#[test]
fn shared_synthetic_corpus_matches_explicit_expected_projections() {
    let corpus: serde_json::Value =
        serde_json::from_slice(include_bytes!("fixtures/feed-xml-v1/cases.json")).unwrap();
    let context = serde_json::to_vec(&corpus["context"]).unwrap();
    for case in corpus["cases"].as_array().unwrap() {
        let result = normalize(&context, case["xml"].as_str().unwrap().as_bytes());
        assert_eq!(
            result.is_ok(),
            case["valid"].as_bool().unwrap(),
            "{}: {result:?}",
            case["id"]
        );
        if let Ok(feed) = result {
            let value = serde_json::to_value(feed).unwrap();
            assert_eq!(value["normalizer_profile"], "booknook.rss2-atom.inert.v1");
            for (path, expected) in case["checks"].as_object().unwrap() {
                assert_eq!(
                    value.pointer(path).unwrap(),
                    expected,
                    "{} {path}",
                    case["id"]
                );
            }
        }
    }
}
#[test]
fn article_links_allow_queries_fragments_http_and_relative_xml_base() {
    let feed = parse(
        r#"<rss version="2.0" xml:base="http://books.example.org/reviews/?from=feed"><channel><item><guid>review</guid><link>../review/show/42?book=1&amp;reader=2#comment</link></item></channel></rss>"#,
    );
    let value = serde_json::to_value(feed).unwrap();
    assert_eq!(
        value["entries"][0]["links"][0]["url"],
        "http://books.example.org/review/show/42?book=1&reader=2#comment"
    );
    let context = std::str::from_utf8(CONTEXT).unwrap().replace(
        "https://example.org/feed.xml",
        "https://example.org/feed.xml?query=forbidden",
    );
    assert!(normalize(context.as_bytes(), b"<rss version='2.0'><channel/></rss>").is_err());
}
#[test]
fn normalized_output_limit_refuses_whole_candidate() {
    let mut xml = "<rss version='2.0'><channel>".to_owned();
    for i in 0..32 {
        xml += &format!(
            "<item><guid>{i}</guid><description><![CDATA[{}]]></description></item>",
            "\"".repeat(65_400)
        );
    }
    xml += "</channel></rss>";
    assert!(xml.len() < booknook_core::feed_xml::MAX_XML_BYTES);
    let error = normalize(CONTEXT, xml.as_bytes()).unwrap_err();
    assert!(
        error.to_string().contains("Normalized feed exceeds"),
        "{error}"
    );
}

#[test]
fn raw_event_limit_includes_names_and_combined_start_tags() {
    for xml in [
        format!(
            "<rss version='2.0'><channel><{}/></channel></rss>",
            "x".repeat(65_537)
        ),
        format!(
            "<rss version='2.0' a='{}' b='{}'><channel/></rss>",
            "x".repeat(32_768),
            "x".repeat(32_768)
        ),
    ] {
        let error = normalize(CONTEXT, xml.as_bytes()).unwrap_err();
        assert!(error.to_string().contains("XML event exceeds"), "{error}");
    }
}
